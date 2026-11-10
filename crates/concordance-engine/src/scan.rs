//! Scans and change batches: discovery, hash comparison, parallel parsing, chunked
//! application, reconciliation and the atomic commit.

use crate::apply::Pending;
use crate::discover::{AuthorityRules, FileKind, PathFilter};
use crate::source::{read_text, RepoFs};
use crate::{Engine, LastScan, ScanStatus};
use concordance_core::code::{SourceIndex, WorkspaceInfo};
use concordance_core::doc::{AuthoritySource, DocumentIndex};
use concordance_core::event::{ChangeKind, Envelope, Event, ScanPhase, ScanReason};
use concordance_core::{ContentHash, RepoPath, Timestamp};
use concordance_reconcile::BatchContext;
use concordance_storage::RevisionRecord;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

/// Files applied per write-lock acquisition during the index phase.
const APPLY_CHUNK: usize = 48;

enum Parsed {
    Doc(Box<DocumentIndex>),
    Src(Box<SourceIndex>),
}

struct Target {
    rel: RepoPath,
    kind: FileKind,
    abs: PathBuf,
}

/// What the model currently knows about a path (to skip unchanged files).
#[derive(Clone)]
struct Known {
    hash: ContentHash,
    configured_authority: Option<concordance_core::doc::AuthorityAssessment>,
}

/// Modification time of `p` in milliseconds since the epoch.
fn mtime_ms(p: &std::path::Path) -> Option<u64> {
    let t = std::fs::metadata(p).ok()?.modified().ok()?;
    Some(t.duration_since(std::time::UNIX_EPOCH).ok()?.as_millis() as u64)
}

impl Engine {
    /// Re-read the configured test-run outputs and recompute their freshness against the
    /// indexed sources. Returns whether anything a finding depends on changed.
    pub(crate) fn reload_test_runs(&mut self) -> bool {
        use concordance_core::testrun::{self, TestRun};
        let paths = self.shared.config.test_results.clone();
        let root = self.shared.config.repo_root.clone();
        let mut runs: Vec<TestRun> = Vec::new();
        for p in &paths {
            let (Some(modified), Ok(text)) = (mtime_ms(p), std::fs::read_to_string(p)) else {
                continue; // not produced yet: no evidence, nothing to report
            };
            runs.push(TestRun {
                path: p.display().to_string(),
                modified: Timestamp(modified),
                content_hash: ContentHash::of_str(&text),
                results: testrun::parse(&text),
                newer_sources: 0,
                newest_source: None,
            });
        }
        if !runs.is_empty() {
            let files: Vec<RepoPath> = self
                .shared
                .read(|m| m.files.values().map(|f| f.path.clone()).collect::<Vec<_>>())
                .0;
            let stamped: Vec<(u64, RepoPath)> = files
                .into_iter()
                .filter_map(|f| mtime_ms(&f.to_fs(&root)).map(|t| (t, f)))
                .collect();
            for run in &mut runs {
                let mut newest: Option<(u64, RepoPath)> = None;
                for (t, f) in &stamped {
                    if *t > run.modified.0 {
                        run.newer_sources += 1;
                        if newest.as_ref().is_none_or(|n| *t > n.0) {
                            newest = Some((*t, f.clone()));
                        }
                    }
                }
                run.newest_source = newest.map(|(_, f)| f);
            }
        }
        let mut m = self.shared.model.write().unwrap_or_else(|e| e.into_inner());
        if m.test_runs == runs {
            return false;
        }
        m.test_runs = runs;
        true
    }

    /// Whether `p` is one of the configured test-run outputs.
    pub(crate) fn is_test_results(&self, p: &std::path::Path) -> bool {
        self.shared.config.test_results.iter().any(|r| {
            r.to_string_lossy()
                .eq_ignore_ascii_case(&p.to_string_lossy())
        })
    }
    pub(crate) async fn scan(&mut self, force: bool, reason: ScanReason) {
        self.scan_counter += 1;
        let scan_id = self.scan_counter;
        let started = Instant::now();
        self.set_scan(scan_id, reason, ScanPhase::Discover, 0, 0);
        let cfg = self.shared.config.clone();
        let discovered = tokio::task::spawn_blocking(move || {
            let ws = crate::languages().discover(&cfg.repo_root, &cfg.src_roots);
            let files = PathFilter::new(&cfg).discover();
            (ws, files)
        })
        .await;
        let (ws, files) = match discovered {
            Ok(v) => v,
            Err(e) => {
                self.publish_now(Event::SystemError {
                    message: format!("discovery failed: {e}"),
                });
                self.clear_scan();
                return;
            }
        };
        for w in &ws.warnings {
            self.publish_now(Event::SystemWarning { message: w.clone() });
        }
        let c = &self.shared.config;
        self.publish_now(Event::RepositoryDiscovered {
            root: c.repo_root.display().to_string(),
            spec_roots: c
                .spec_roots
                .iter()
                .map(|p| p.display().to_string())
                .collect(),
            src_roots: c
                .src_roots
                .iter()
                .map(|p| p.display().to_string())
                .collect(),
            markdown_files: files.iter().filter(|f| f.1 == FileKind::Markdown).count() as u32,
            rust_files: files
                .iter()
                .filter(|f| matches!(f.1, FileKind::Source(_)))
                .count() as u32,
            crates: ws.crates.len() as u32,
        });
        let present: BTreeSet<RepoPath> = files.iter().map(|f| f.0.clone()).collect();
        let removed: Vec<(RepoPath, FileKind)> = {
            let (r, _) = self.shared.read(|m| {
                let mut r: Vec<(RepoPath, FileKind)> = m
                    .documents
                    .values()
                    .filter(|d| !present.contains(&d.path))
                    .map(|d| (d.path.clone(), FileKind::Markdown))
                    .collect();
                r.extend(
                    m.files
                        .values()
                        .filter(|f| !present.contains(&f.path))
                        .map(|f| (f.path.clone(), FileKind::source(&f.language))),
                );
                r
            });
            r
        };
        let targets = files
            .into_iter()
            .map(|(rel, kind, abs)| Target { rel, kind, abs })
            .collect();
        self.process(scan_id, reason, targets, removed, force, Some(ws), started)
            .await;
    }

    /// Paths reported by the watcher (absolute). Coalesced and classified here.
    pub(crate) async fn files_changed(&mut self, paths: Vec<PathBuf>) {
        let filter = PathFilter::new(&self.shared.config);
        let mut targets: BTreeMap<RepoPath, Target> = BTreeMap::new();
        let mut removed: BTreeMap<RepoPath, FileKind> = BTreeMap::new();
        let mut results_touched = false;
        for p in paths {
            if self.is_test_results(&p) {
                results_touched = true;
                continue;
            }
            if p.is_file() {
                if let Some((rel, kind)) = filter.classify(&p) {
                    removed.remove(&rel);
                    targets.insert(rel.clone(), Target { rel, kind, abs: p });
                }
                continue;
            }
            if p.exists() {
                continue; // a directory that still exists: its files report themselves
            }
            // Deleted file or directory: drop everything known under it.
            let Some(rel) = RepoPath::from_fs(filter.root(), &p) else {
                continue;
            };
            let (gone, _) = self.shared.read(|m| {
                let mut g: Vec<(RepoPath, FileKind)> = m
                    .documents
                    .values()
                    .filter(|d| d.path.starts_with_dir(&rel))
                    .map(|d| (d.path.clone(), FileKind::Markdown))
                    .collect();
                g.extend(
                    m.files
                        .values()
                        .filter(|f| f.path.starts_with_dir(&rel))
                        .map(|f| (f.path.clone(), FileKind::source(&f.language))),
                );
                g
            });
            for (r, k) in gone {
                targets.remove(&r);
                removed.insert(r, k);
            }
        }
        if targets.is_empty() && removed.is_empty() && !results_touched {
            return;
        }
        self.scan_counter += 1;
        let id = self.scan_counter;
        self.process(
            id,
            ScanReason::Watch,
            targets.into_values().collect(),
            removed.into_iter().collect(),
            false,
            None,
            Instant::now(),
        )
        .await;
    }

    #[allow(clippy::too_many_arguments)]
    async fn process(
        &mut self,
        scan_id: u64,
        reason: ScanReason,
        targets: Vec<Target>,
        removed: Vec<(RepoPath, FileKind)>,
        force: bool,
        workspace: Option<WorkspaceInfo>,
        started: Instant,
    ) {
        let shared = self.shared.clone();
        let rules = Arc::new(AuthorityRules::new(&shared.config));
        let (known, ws_changed): (HashMap<RepoPath, Known>, bool) = {
            let (r, _) = shared.read(|m| {
                let mut k: HashMap<RepoPath, Known> = HashMap::new();
                for d in m.documents.values() {
                    let configured = (d.authority.source == AuthoritySource::Configured)
                        .then(|| d.authority.clone());
                    k.insert(
                        d.path.clone(),
                        Known {
                            hash: d.content_hash.clone(),
                            configured_authority: configured,
                        },
                    );
                }
                for f in m.files.values() {
                    k.insert(
                        f.path.clone(),
                        Known {
                            hash: f.content_hash.clone(),
                            configured_authority: None,
                        },
                    );
                }
                (
                    k,
                    workspace
                        .as_ref()
                        .is_some_and(|w| w.crates != m.workspace.crates),
                )
            });
            r
        };
        let ws_for_parse: Arc<WorkspaceInfo> = Arc::new(match &workspace {
            Some(w) => w.clone(),
            None => shared.read(|m| m.workspace.clone()).0,
        });

        let total = targets.len() as u32;
        let revision = shared.read(|m| m.revision).0.next();
        shared.bus.set_revision(revision);
        let mut pending = Pending::new(BatchContext {
            revision,
            now: Timestamp::now(),
            changed_files: Vec::new(),
            semantic_enabled: self.semantic.enabled(),
            languages: crate::languages().clone(),
        });
        let fs = RepoFs {
            root: shared.config.repo_root.clone(),
        };
        if ws_changed {
            if let Some(w) = &workspace {
                let mut m = shared.model.write().unwrap_or_else(|e| e.into_inner());
                m.workspace = w.clone();
                pending.dirty.workspace = true;
            }
        }
        if reason != ScanReason::Watch {
            self.publish_now(Event::ScanStarted {
                scan: scan_id,
                reason,
                files: total,
            });
        }
        self.set_scan(scan_id, reason, ScanPhase::Parse, 0, total);

        // ---- parse (bounded parallelism on the blocking pool), apply in chunks as results arrive
        let workers = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4)
            .clamp(1, 8);
        let chunk = ((targets.len() / (workers * 4)).max(1)).min(64);
        let mut set = tokio::task::JoinSet::new();
        let mut queue: std::collections::VecDeque<Vec<Target>> = std::collections::VecDeque::new();
        let mut iter = targets.into_iter().peekable();
        while iter.peek().is_some() {
            queue.push_back(iter.by_ref().take(chunk).collect());
        }
        let spawn = |set: &mut tokio::task::JoinSet<
            Vec<(RepoPath, FileKind, Result<Option<Parsed>, String>)>,
        >,
                     batch: Vec<Target>| {
            let known = known.clone();
            let rules = rules.clone();
            let ws = ws_for_parse.clone();
            set.spawn_blocking(move || {
                batch
                    .into_iter()
                    .map(|t| {
                        let r = parse_one(
                            &t,
                            known.get(&t.rel),
                            &rules,
                            &ws,
                            force || (ws_changed && matches!(t.kind, FileKind::Source(_))),
                        );
                        (t.rel, t.kind, r)
                    })
                    .collect()
            });
        };
        for _ in 0..workers {
            if let Some(b) = queue.pop_front() {
                spawn(&mut set, b);
            }
        }
        let mut done = 0u32;
        let mut changed: Vec<(RepoPath, ChangeKind)> = Vec::new();
        let mut buffer: Vec<(RepoPath, Parsed)> = Vec::new();
        let mut last_progress = Instant::now();
        while let Some(res) = set.join_next().await {
            if let Some(b) = queue.pop_front() {
                spawn(&mut set, b);
            }
            let Ok(results) = res else { continue };
            for (rel, _kind, r) in results {
                done += 1;
                match r {
                    Ok(Some(p)) => {
                        let kind = if known.contains_key(&rel) {
                            ChangeKind::Modified
                        } else {
                            ChangeKind::Created
                        };
                        changed.push((rel.clone(), kind));
                        buffer.push((rel, p));
                    }
                    Ok(None) => {}
                    Err(e) => self.publish_now(Event::SystemWarning {
                        message: format!("{rel}: {e}"),
                    }),
                }
            }
            if buffer.len() >= APPLY_CHUNK && reason != ScanReason::Watch {
                self.apply_chunk(&mut pending, std::mem::take(&mut buffer));
            }
            if last_progress.elapsed().as_millis() >= 150 || done == total {
                last_progress = Instant::now();
                self.set_scan(scan_id, reason, ScanPhase::Parse, done, total);
                if reason != ScanReason::Watch {
                    self.publish_now(Event::ScanProgress {
                        scan: scan_id,
                        phase: ScanPhase::Parse,
                        done,
                        total,
                    });
                }
            }
        }
        for (rel, _) in &removed {
            changed.push((rel.clone(), ChangeKind::Removed));
        }
        // Watch batches announce what changed before the resulting index events.
        if reason == ScanReason::Watch && !changed.is_empty() {
            self.publish_now(Event::WatchChangeDetected {
                changes: changed.clone(),
            });
        }
        self.apply_chunk(&mut pending, buffer);
        // Test outcomes depend on the run files and on how fresh they are relative to the
        // sources, so they are re-read on every batch.
        let tests_changed = self.reload_test_runs();
        if tests_changed || !changed.is_empty() {
            let m = shared.model.read().unwrap_or_else(|e| e.into_inner());
            pending
                .affected
                .claims
                .extend(m.requirement_claims().map(|c| c.id.clone()));
            pending
                .affected
                .symbols
                .extend(m.verifying_symbols().map(|s| s.id.clone()));
        }

        if changed.is_empty() && !pending.dirty.workspace && !tests_changed {
            // Nothing changed (e.g. an editor rewrote identical content).
            if reason != ScanReason::Watch {
                let counts = shared.read(|m| m.counts()).0;
                self.publish_now(Event::ScanCompleted {
                    scan: scan_id,
                    revision: revision
                        .0
                        .checked_sub(1)
                        .map(concordance_core::Revision)
                        .unwrap_or_default(),
                    duration_ms: started.elapsed().as_millis() as u64,
                    changed_files: 0,
                    counts,
                });
                self.finish_scan_status(revision, started, 0, false);
            }
            self.clear_scan();
            if reason != ScanReason::Watch {
                self.semantic.enqueue_candidates(None);
            }
            return;
        }

        if force {
            // A forced scan (manual `force`, extractor or rule change) re-derives everything,
            // not only what the re-parse reported as changed.
            let m = shared.model.read().unwrap_or_else(|e| e.into_inner());
            pending.affected.claims.extend(m.claims.keys().cloned());
            pending.affected.docs.extend(m.documents.keys().cloned());
            pending.affected.symbols.extend(m.symbols.keys().cloned());
            pending
                .affected
                .symbol_edges
                .extend(m.symbols.keys().cloned());
            pending.affected.conflicts = true;
            // ...and re-persists the whole model (recovers from an earlier failed commit).
            let d = &mut pending.dirty;
            d.documents.extend(m.documents.keys().cloned());
            d.sections.extend(m.sections.keys().cloned());
            d.claims.extend(m.claims.keys().cloned());
            d.prose_docs.extend(m.documents.keys().cloned());
            d.files.extend(m.files.keys().cloned());
            d.import_files.extend(m.files.keys().cloned());
            d.symbols.extend(m.symbols.keys().cloned());
            d.edges.extend(m.edges.keys().cloned());
            d.evidence.extend(m.evidence.keys().cloned());
            d.findings.extend(m.findings.keys().cloned());
            d.workspace = true;
        }

        // ---- removals + reconcile under one write lock, events published inside it
        self.set_scan(scan_id, reason, ScanPhase::Reconcile, done, total);
        if reason != ScanReason::Watch {
            self.publish_now(Event::ScanProgress {
                scan: scan_id,
                phase: ScanPhase::Reconcile,
                done,
                total,
            });
        }
        if reason != ScanReason::Initial {
            pending.ctx.changed_files = changed.iter().map(|(p, _)| p.clone()).collect();
        }
        let (candidates, invalidated, delta) = {
            let mut m = shared.model.write().unwrap_or_else(|e| e.into_inner());
            let mut events = Vec::new();
            for (rel, kind) in &removed {
                match kind {
                    FileKind::Markdown => pending.remove_document(&mut m, rel, &mut events),
                    FileKind::Source(_) => pending.remove_source(&mut m, rel, &mut events),
                }
            }
            let candidates = pending.reconcile(&mut m, &fs, &mut events);
            m.revision = revision;
            let counts = m.counts();
            let mut delta = pending.dirty.to_delta(&m);
            let mut envelopes: Vec<Arc<Envelope>> =
                events.into_iter().map(|e| shared.bus.publish(e)).collect();
            let completed = shared.bus.publish(Event::ScanCompleted {
                scan: scan_id,
                revision,
                duration_ms: started.elapsed().as_millis() as u64,
                changed_files: changed.len() as u32,
                counts: counts.clone(),
            });
            envelopes.push(completed);
            delta.last_seq = shared.bus.last_seq();
            delta.extractor_version = Some(crate::extractor_version());
            delta.events = envelopes
                .iter()
                .filter(|e| e.event.is_state_change())
                .map(|e| (**e).clone())
                .collect();
            delta.revision_record = Some(RevisionRecord {
                revision,
                at: pending.ctx.now,
                reason,
                changed: changed.clone(),
                counts,
            });
            (
                candidates,
                std::mem::take(&mut pending.affected.semantic_invalidated),
                delta,
            )
        };
        self.commit(delta).await;
        self.finish_scan_status(revision, started, changed.len() as u32, true);
        self.clear_scan();
        self.semantic.cancel_for(&invalidated);
        self.semantic.reset_budget();
        self.semantic.enqueue_candidates(Some(candidates));
    }

    fn apply_chunk(&mut self, pending: &mut Pending, parsed: Vec<(RepoPath, Parsed)>) {
        if parsed.is_empty() {
            return;
        }
        let shared = self.shared.clone();
        let mut m = shared.model.write().unwrap_or_else(|e| e.into_inner());
        let mut events = Vec::new();
        for (_, p) in parsed {
            match p {
                Parsed::Doc(d) => pending.apply_document(&mut m, *d, &mut events),
                Parsed::Src(s) => pending.apply_source(&mut m, *s, &mut events),
            }
        }
        for e in events {
            shared.bus.publish(e);
        }
    }

    /// Persist a delta, retrying transient failures (e.g. a locked database). If it still
    /// fails, the next scan is forced to re-derive and re-persist everything, so the
    /// database cannot silently stay behind the in-memory model.
    pub(crate) async fn commit(&mut self, delta: concordance_storage::Delta) {
        let delta = Arc::new(delta);
        let mut last_err = String::new();
        for attempt in 0..3u64 {
            if attempt > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(200 * attempt)).await;
            }
            let shared = self.shared.clone();
            let d = delta.clone();
            let res = tokio::task::spawn_blocking(move || {
                shared
                    .store
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .commit(&d)
            })
            .await;
            match res {
                Ok(Ok(())) => return,
                Ok(Err(e)) => last_err = e.to_string(),
                Err(e) => last_err = e.to_string(),
            }
        }
        self.publish_now(Event::SystemError {
            message: format!(
                "failed to persist revision {}: {last_err}; a full re-scan is scheduled",
                delta.revision.0
            ),
        });
        let _ = self.cmd_tx.try_send(crate::Command::Scan {
            force: true,
            reason: ScanReason::Manual,
        });
    }

    fn set_scan(&self, id: u64, reason: ScanReason, phase: ScanPhase, done: u32, total: u32) {
        self.shared.update_status(|s| {
            let started_at = s
                .scan
                .as_ref()
                .filter(|x| x.id == id)
                .map(|x| x.started_at)
                .unwrap_or_else(Timestamp::now);
            s.scan = Some(ScanStatus {
                id,
                reason,
                phase,
                done,
                total,
                started_at,
            });
        });
    }

    fn clear_scan(&self) {
        self.shared.update_status(|s| s.scan = None);
    }

    fn finish_scan_status(
        &self,
        revision: concordance_core::Revision,
        started: Instant,
        changed: u32,
        committed: bool,
    ) {
        let revision = if committed {
            revision
        } else {
            concordance_core::Revision(revision.0.saturating_sub(1))
        };
        self.shared.update_status(|s| {
            s.last_scan = Some(LastScan {
                revision,
                at: Timestamp::now(),
                duration_ms: started.elapsed().as_millis() as u64,
                changed_files: changed,
            });
        });
    }
}

/// Read, hash and (if changed) parse one file. `Ok(None)` = unchanged.
fn parse_one(
    t: &Target,
    known: Option<&Known>,
    rules: &AuthorityRules,
    ws: &WorkspaceInfo,
    force: bool,
) -> Result<Option<Parsed>, String> {
    let root_rel = &t.rel;
    let text = match read_text(&repo_root_of(&t.abs, root_rel), root_rel) {
        Ok(s) => s,
        Err(e) => return Err(format!("could not read: {e}")),
    };
    let hash = ContentHash::of_str(&text);
    match t.kind {
        FileKind::Markdown => {
            let configured = rules.assess(&t.rel);
            if !force
                && known.is_some_and(|k| k.hash == hash && k.configured_authority == configured)
            {
                return Ok(None);
            }
            Ok(Some(Parsed::Doc(Box::new(
                concordance_markdown::index_document(&t.rel, &text, configured, crate::languages()),
            ))))
        }
        FileKind::Source(lang) => {
            if !force && known.is_some_and(|k| k.hash == hash) {
                return Ok(None);
            }
            let plugin = crate::languages()
                .by_id(lang)
                .ok_or_else(|| format!("no language plugin `{lang}`"))?;
            Ok(Some(Parsed::Src(Box::new(plugin.index(&t.rel, &text, ws)))))
        }
    }
}

/// Recover the repository root from an absolute file path and its repository path.
fn repo_root_of(abs: &std::path::Path, rel: &RepoPath) -> PathBuf {
    let mut root = abs.to_path_buf();
    for _ in rel.as_str().split('/') {
        root.pop();
    }
    root
}
