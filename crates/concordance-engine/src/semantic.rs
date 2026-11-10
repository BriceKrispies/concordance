//! Semantic analysis scheduling (Layers B and C).
//!
//! * Candidates: obligation claims that structural analysis cannot decide, that have
//!   linked implementation symbols, and that lack valid semantic evidence.
//! * Bounded: at most `max_concurrent` running, at most `max_analyses_per_scan` model
//!   invocations per batch (explicit requests add to the budget).
//! * Cached: identical inputs (prompt version, mode, model, claim and symbol hashes) reuse
//!   the stored result, after re-checking that its dependencies are still current.
//! * Fresh-or-discarded: a result is applied only if, under the model write lock, the
//!   claim and every symbol it depended on still have the hashes it was computed from.
//!   Otherwise it is reported as `superseded` and never published as current.

use crate::apply::Pending;
use crate::source::read_text;
use crate::{Command, Shared};
use concordance_core::code::Symbol;
use concordance_core::doc::Claim;
use concordance_core::event::{AnalysisOutcome, Envelope, Event};
use concordance_core::evidence::{
    Citation, CitationRole, Evidence, Method, MethodKind, Stance, Strength, Validity,
};
use concordance_core::finding::{FindingCategory, FindingStatus};
use concordance_core::graph::{EdgeKind, NodeRef};
use concordance_core::{AnalysisId, ClaimId, ContentHash, EvidenceId, Timestamp};
use concordance_graph::Model;
use concordance_reconcile::{derive, BatchContext};
use concordance_semantic::{
    AnalysisError, AnalysisMode, AnalysisRequest, AnalysisResult, Assessment, Backend,
    ChallengeOutcome, ClaimContext, ClaudeCli, ContextProvider, ContextWanted, SymbolContext,
    Verdict, PROMPT_VERSION,
};
use concordance_storage::AnalysisRecord;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, watch};

/// Max symbol excerpts in the initial context.
const MAX_INITIAL_SYMBOLS: usize = 4;
/// Max lines of one symbol excerpt.
const MAX_SYMBOL_LINES: u32 = 150;
const CLI_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Clone, Debug)]
pub(crate) enum JobMode {
    Assess,
    /// `prior_analysis` = the assessment being challenged; the result is discarded if
    /// that assessment is no longer the claim's current semantic evidence.
    Challenge {
        prior: Assessment,
        prior_analysis: AnalysisId,
    },
}

impl JobMode {
    fn is_challenge(&self) -> bool {
        matches!(self, JobMode::Challenge { .. })
    }
}

pub(crate) struct JobOutcome {
    claim: ClaimId,
    analysis: AnalysisId,
    mode: JobMode,
    cache_key: String,
    cached: bool,
    result: Result<AnalysisResult, AnalysisError>,
}

struct Running {
    cancel: watch::Sender<bool>,
}

pub(crate) struct SemanticRunner {
    shared: Arc<Shared>,
    backend: Option<Arc<dyn Backend>>,
    probe_cli: Option<ClaudeCli>,
    cmd_tx: mpsc::Sender<Command>,
    queue: VecDeque<(ClaimId, JobMode, AnalysisId)>,
    queued: HashSet<(ClaimId, bool)>,
    running: HashMap<ClaimId, Running>,
    budget_left: usize,
    available: bool,
}

impl SemanticRunner {
    pub fn new(
        shared: Arc<Shared>,
        backend: Option<Arc<dyn Backend>>,
        cmd_tx: mpsc::Sender<Command>,
    ) -> Self {
        let cfg = &shared.config.semantic;
        let (backend, probe_cli, available): (Option<Arc<dyn Backend>>, Option<ClaudeCli>, bool) =
            match backend {
                Some(b) => (Some(b), None, cfg.enabled),
                None if cfg.enabled => {
                    let workdir = shared.config.state_dir.join("claude-workdir");
                    let cli = ClaudeCli::new(cfg.claude_path.clone(), CLI_TIMEOUT, workdir.clone());
                    (
                        Some(Arc::new(ClaudeCli::new(
                            cfg.claude_path.clone(),
                            CLI_TIMEOUT,
                            workdir,
                        ))),
                        Some(cli),
                        false,
                    )
                }
                None => (None, None, false),
            };
        let budget_left = cfg.max_analyses_per_scan;
        SemanticRunner {
            shared,
            backend,
            probe_cli,
            cmd_tx,
            queue: VecDeque::new(),
            queued: HashSet::new(),
            running: HashMap::new(),
            budget_left,
            available,
        }
    }

    /// Probe the Claude CLI once at startup.
    pub async fn probe(&mut self) {
        let Some(cli) = self.probe_cli.take() else {
            let enabled = self.shared.config.semantic.enabled;
            let available = self.available;
            self.shared.update_status(|s| {
                s.semantic.available = enabled.then_some(available);
                if !enabled {
                    s.semantic.detail = Some(
                        "disabled: structural mode (behavioral claims cannot be verified)".into(),
                    );
                }
            });
            return;
        };
        let _ = std::fs::create_dir_all(&cli.workdir);
        match cli.probe().await {
            Ok(v) => {
                self.available = true;
                self.shared.update_status(|s| {
                    s.semantic.available = Some(true);
                    s.semantic.detail = Some(format!("Claude Code CLI {v}"));
                });
            }
            Err(e) => {
                self.available = false;
                self.shared.update_status(|s| {
                    s.semantic.available = Some(false);
                    s.semantic.detail = Some(e.clone());
                });
                self.shared.bus.publish(Event::SystemWarning {
                    message: format!("Semantic analysis is enabled but unavailable ({e}); continuing in structural mode. Behavioral claims will remain unverified."),
                });
            }
        }
    }

    /// Enabled and usable.
    pub fn enabled(&self) -> bool {
        self.shared.config.semantic.enabled && self.available && self.backend.is_some()
    }

    pub fn is_idle(&self) -> bool {
        self.queue.is_empty() && self.running.is_empty()
    }

    pub fn reset_budget(&mut self) {
        self.budget_left = self.shared.config.semantic.max_analyses_per_scan;
    }

    fn sync_status(&self) {
        let (q, r) = (self.queue.len(), self.running.len());
        self.shared.update_status(|s| {
            s.semantic.queued = q;
            s.semantic.running = r;
        });
    }

    pub fn cancel_for(&mut self, claims: &std::collections::BTreeSet<ClaimId>) {
        for c in claims {
            if let Some(r) = self.running.get(c) {
                r.cancel.send_replace(true);
            }
        }
    }

    pub fn cancel_all(&mut self) {
        for r in self.running.values() {
            r.cancel.send_replace(true);
        }
        self.queue.clear();
        self.queued.clear();
    }

    /// Queue candidate claims (`None` = consider every claim).
    pub fn enqueue_candidates(&mut self, claims: Option<Vec<ClaimId>>) {
        if !self.enabled() {
            return;
        }
        let picked: Vec<ClaimId> = self
            .shared
            .read(|m| {
                let ids: Vec<ClaimId> = match claims {
                    Some(c) => c,
                    None => m.claims.keys().cloned().collect(),
                };
                let mut cands: Vec<&Claim> = ids
                    .iter()
                    .filter_map(|id| m.claims.get(id))
                    .filter(|c| is_candidate(m, c))
                    .collect();
                cands.sort_by_key(|c| {
                    (
                        !derive::is_high_impact(c),
                        c.authority != concordance_core::doc::Authority::Normative,
                        c.id.clone(),
                    )
                });
                cands.into_iter().map(|c| c.id.clone()).collect()
            })
            .0;
        for id in picked {
            self.push(id, JobMode::Assess, false);
        }
        self.pump();
    }

    fn push(&mut self, claim: ClaimId, mode: JobMode, front: bool) {
        let key = (claim.clone(), mode.is_challenge());
        if self.queued.contains(&key) || self.running.contains_key(&claim) {
            return;
        }
        self.queued.insert(key);
        let analysis = AnalysisId::from_key(&[
            claim.as_str(),
            &Timestamp::now().0.to_string(),
            if mode.is_challenge() { "c" } else { "a" },
        ]);
        self.shared.bus.publish(Event::AnalysisQueued {
            analysis: analysis.clone(),
            claim: claim.clone(),
        });
        if front {
            self.queue.push_front((claim, mode, analysis));
        } else {
            self.queue.push_back((claim, mode, analysis));
        }
        self.sync_status();
    }

    /// Explicit analysis request from the API/CLI.
    pub fn request(&mut self, claims: Vec<ClaimId>, all: bool) -> Result<usize, String> {
        if !self.shared.config.semantic.enabled {
            return Err("semantic analysis is disabled (enable with --semantic or `[semantic] enabled = true`)".into());
        }
        if !self.enabled() {
            let detail = self
                .shared
                .status()
                .semantic
                .detail
                .unwrap_or_else(|| "backend unavailable".into());
            return Err(format!("semantic analysis is unavailable: {detail}"));
        }
        let before = self.queue.len();
        if all {
            self.budget_left = self
                .budget_left
                .max(self.shared.config.semantic.max_analyses_per_scan);
            self.enqueue_candidates(None);
        } else {
            let known: Vec<ClaimId> = self
                .shared
                .read(|m| {
                    claims
                        .iter()
                        .filter(|c| m.claims.get(*c).is_some_and(|c| c.is_obligation()))
                        .cloned()
                        .collect()
                })
                .0;
            self.budget_left += known.len() * 2;
            for c in known {
                self.push(c, JobMode::Assess, true);
            }
            self.pump();
        }
        Ok(self.queue.len().saturating_sub(before) + self.running.len())
    }

    fn pump(&mut self) {
        let max = self.shared.config.semantic.max_concurrent;
        while self.running.len() < max {
            let Some((claim, mode, analysis)) = self.queue.pop_front() else {
                break;
            };
            self.queued.remove(&(claim.clone(), mode.is_challenge()));
            self.start(claim, mode, analysis);
        }
        self.sync_status();
    }

    fn start(&mut self, claim: ClaimId, mode: JobMode, analysis: AnalysisId) {
        let Some(backend) = self.backend.clone() else {
            return;
        };
        let model_name = if mode.is_challenge() {
            self.shared.config.semantic.challenger_model.clone()
        } else {
            self.shared.config.semantic.model.clone()
        };
        let root = self.shared.config.repo_root.clone();
        let built = self
            .shared
            .read(|m| build_request(m, &root, &claim, &mode, &model_name, analysis.clone()))
            .0;
        let Some((request, key)) = built else {
            self.shared.bus.publish(Event::AnalysisCompleted {
                analysis,
                claim,
                outcome: AnalysisOutcome::Cancelled,
                verdict: None,
                detail: Some(
                    "no current context to analyze (file changed or no linked symbols)".into(),
                ),
            });
            return;
        };

        // Cache: identical inputs, and the cached result's dependencies still current.
        let cached = {
            let store = self.shared.store.lock().unwrap_or_else(|e| e.into_inner());
            store.cached_analysis(&key).ok().flatten()
        };
        if let Some(rec) = cached {
            if let Ok(result) = serde_json::from_value::<AnalysisResult>(
                rec.payload.get("result").cloned().unwrap_or_default(),
            ) {
                let fresh = self
                    .shared
                    .read(|m| {
                        result
                            .dependencies
                            .iter()
                            .all(|d| m.content_hash(&d.node).as_ref() == Some(&d.content_hash))
                    })
                    .0;
                if fresh {
                    let (tx, _) = watch::channel(false);
                    self.running.insert(claim.clone(), Running { cancel: tx });
                    let outcome = Command::AnalysisDone(Box::new(JobOutcome {
                        claim: claim.clone(),
                        analysis,
                        mode,
                        cache_key: key,
                        cached: true,
                        result: Ok(result),
                    }));
                    if self.cmd_tx.try_send(outcome).is_err() {
                        // Channel full: drop the reservation so the claim can be queued again.
                        self.running.remove(&claim);
                    }
                    return;
                }
            }
        }
        if self.budget_left == 0 {
            self.shared.bus.publish(Event::AnalysisCompleted {
                analysis,
                claim,
                outcome: AnalysisOutcome::Cancelled,
                verdict: None,
                detail: Some("per-batch analysis budget exhausted (max_analyses_per_scan); request it explicitly to run".into()),
            });
            return;
        }
        self.budget_left -= 1;

        let (cancel_tx, cancel_rx) = watch::channel(false);
        self.running
            .insert(claim.clone(), Running { cancel: cancel_tx });
        self.shared.bus.publish(Event::AnalysisStarted {
            analysis: analysis.clone(),
            claim: claim.clone(),
            model: model_name,
            challenge: mode.is_challenge(),
        });
        let provider = ModelProvider {
            shared: self.shared.clone(),
        };
        let tx = self.cmd_tx.clone();
        let bus = self.shared.bus.clone();
        tokio::spawn(async move {
            bus.publish(Event::AnalysisProgress {
                analysis: analysis.clone(),
                stage: format!(
                    "invoking {} with {} excerpt(s)",
                    backend.name(),
                    request.symbols.len()
                ),
            });
            let result =
                concordance_semantic::analyze(backend.as_ref(), request, &provider, cancel_rx)
                    .await;
            let _ = tx
                .send(Command::AnalysisDone(Box::new(JobOutcome {
                    claim,
                    analysis,
                    mode,
                    cache_key: key,
                    cached: false,
                    result,
                })))
                .await;
        });
    }

    /// Apply (or discard) a finished analysis.
    pub async fn finish(&mut self, o: JobOutcome) {
        self.running.remove(&o.claim);
        let shared = self.shared.clone();
        match o.result {
            Err(AnalysisError::Cancelled) => {
                shared.bus.publish(Event::AnalysisCompleted {
                    analysis: o.analysis,
                    claim: o.claim.clone(),
                    outcome: AnalysisOutcome::Cancelled,
                    verdict: None,
                    detail: Some("inputs changed while running".into()),
                });
                self.requeue_if_candidate(o.claim);
            }
            Err(e) => {
                if matches!(e, AnalysisError::Unavailable(_)) {
                    self.available = false;
                    shared.update_status(|s| {
                        s.semantic.available = Some(false);
                        s.semantic.detail = Some(e.to_string());
                    });
                }
                shared.update_status(|s| s.semantic.failed += 1);
                shared.bus.publish(Event::AnalysisCompleted {
                    analysis: o.analysis,
                    claim: o.claim,
                    outcome: AnalysisOutcome::Failed,
                    verdict: None,
                    detail: Some(concordance_graph::truncate(&e.to_string(), 300)),
                });
            }
            Ok(result) => {
                let applied = self
                    .apply(&o.claim, &o.analysis, &o.mode, &result, o.cached)
                    .await;
                match applied {
                    Some(follow_up) => {
                        if !o.cached {
                            let rec = AnalysisRecord {
                                key: o.cache_key,
                                analysis: o.analysis.clone(),
                                claim: o.claim.clone(),
                                revision: shared.read(|m| m.revision).0,
                                created_at: Timestamp::now(),
                                payload: serde_json::json!({ "prompt_version": PROMPT_VERSION, "challenge": o.mode.is_challenge(), "result": result }),
                            };
                            let s2 = shared.clone();
                            let _ = tokio::task::spawn_blocking(move || {
                                s2.store
                                    .lock()
                                    .unwrap_or_else(|e| e.into_inner())
                                    .put_analysis(&rec)
                            })
                            .await;
                        }
                        shared.update_status(|s| {
                            if o.cached {
                                s.semantic.cached += 1
                            } else {
                                s.semantic.completed += 1
                            }
                        });
                        if let Some(prior) = follow_up {
                            self.push(
                                o.claim,
                                JobMode::Challenge {
                                    prior,
                                    prior_analysis: o.analysis.clone(),
                                },
                                true,
                            );
                        }
                    }
                    None => {
                        shared.update_status(|s| s.semantic.superseded += 1);
                        shared.bus.publish(Event::AnalysisCompleted {
                            analysis: o.analysis,
                            claim: o.claim.clone(),
                            outcome: AnalysisOutcome::Superseded,
                            verdict: None,
                            detail: Some("claim or implementation changed while the analysis ran; result discarded".into()),
                        });
                        self.requeue_if_candidate(o.claim);
                    }
                }
            }
        }
        self.pump();
    }

    fn requeue_if_candidate(&mut self, claim: ClaimId) {
        if !self.enabled() {
            return;
        }
        let ok = self
            .shared
            .read(|m| m.claims.get(&claim).is_some_and(|c| is_candidate(m, c)))
            .0;
        if ok {
            self.push(claim, JobMode::Assess, false);
        }
    }

    /// Returns `None` if stale (discarded), `Some(challenge_prior)` if applied.
    async fn apply(
        &mut self,
        claim_id: &ClaimId,
        analysis: &AnalysisId,
        mode: &JobMode,
        result: &AnalysisResult,
        cached: bool,
    ) -> Option<Option<Assessment>> {
        let shared = self.shared.clone();
        let (delta, follow_up) = {
            let mut m = shared.model.write().unwrap_or_else(|e| e.into_inner());
            let claim = m.claims.get(claim_id).cloned()?;
            let fresh = result
                .dependencies
                .iter()
                .all(|d| m.content_hash(&d.node).as_ref() == Some(&d.content_hash));
            if !fresh {
                return None;
            }
            if let JobMode::Challenge { prior_analysis, .. } = mode {
                // The challenged assessment must still be the current one.
                let current = m.evidence_for(&NodeRef::Claim(claim_id.clone())).any(|e| {
                    e.is_valid() && matches!(&e.method, Method::Semantic { analysis, .. } if analysis == prior_analysis)
                });
                if !current {
                    return None;
                }
            }
            let revision = m.revision.next();
            shared.bus.set_revision(revision);
            let mut pending = Pending::new(BatchContext {
                revision,
                now: Timestamp::now(),
                changed_files: vec![],
                semantic_enabled: true,
                languages: crate::languages().clone(),
            });
            let ev = to_evidence(&claim, analysis, mode, result, &pending.ctx);
            let mut events = vec![Event::EvidenceCreated {
                evidence: (&ev).into(),
            }];
            if !mode.is_challenge() {
                // A new assessment supersedes any challenge of the previous one.
                let old: Vec<EvidenceId> = m
                    .evidence_for(&NodeRef::Claim(claim_id.clone()))
                    .filter(|e| e.method.kind() == MethodKind::Adversarial)
                    .map(|e| e.id.clone())
                    .collect();
                for id in old {
                    m.remove_evidence(&id);
                    pending.dirty.evidence.insert(id);
                }
            }
            pending.dirty.evidence.insert(ev.id.clone());
            m.upsert_evidence(ev);
            pending.derive_claim(&mut m, claim_id, &mut events);
            m.revision = revision;
            let verdict = match mode {
                JobMode::Assess => result.assessment.verdict.as_str().to_string(),
                JobMode::Challenge { .. } => challenge_str(result.assessment.challenge).to_string(),
            };
            events.push(Event::AnalysisCompleted {
                analysis: analysis.clone(),
                claim: claim_id.clone(),
                outcome: if cached {
                    AnalysisOutcome::Cached
                } else {
                    AnalysisOutcome::Applied
                },
                verdict: Some(verdict),
                detail: result.assessment.downgraded.clone(),
            });
            let envs: Vec<Arc<Envelope>> =
                events.into_iter().map(|e| shared.bus.publish(e)).collect();
            let mut delta = pending.dirty.to_delta(&m);
            delta.last_seq = shared.bus.last_seq();
            delta.events = envs
                .iter()
                .filter(|e| e.event.is_state_change())
                .map(|e| (**e).clone())
                .collect();
            let follow_up = match mode {
                JobMode::Assess => {
                    let v = result.assessment.verdict;
                    let needs = v == Verdict::Disagrees
                        || (v == Verdict::Agrees && derive::is_high_impact(&claim));
                    needs.then(|| result.assessment.clone())
                }
                JobMode::Challenge { .. } => None,
            };
            (delta, follow_up)
        };
        let s2 = shared.clone();
        let res = tokio::task::spawn_blocking(move || {
            s2.store
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .commit(&delta)
        })
        .await;
        if let Ok(Err(e)) = res {
            shared.bus.publish(Event::SystemError {
                message: format!("failed to persist analysis: {e}"),
            });
        }
        Some(follow_up)
    }
}

fn challenge_str(c: Option<ChallengeOutcome>) -> &'static str {
    match c {
        Some(ChallengeOutcome::Upheld) => "upheld",
        Some(ChallengeOutcome::Refuted) => "refuted",
        _ => "inconclusive",
    }
}

/// Should semantic analysis look at this claim?
pub(crate) fn is_candidate(m: &Model, c: &Claim) -> bool {
    if !c.is_obligation() || c.documented_signature.is_some() {
        return false;
    }
    let node = NodeRef::Claim(c.id.clone());
    if let Some(f) = m.claim_finding(&c.id) {
        if f.status == FindingStatus::Violated
            && f.category == FindingCategory::StructuralContradiction
        {
            return false;
        }
        if f.status == FindingStatus::Verified && f.methods == [MethodKind::Structural] {
            return false;
        }
    }
    if m.evidence_for(&node)
        .any(|e| e.is_valid() && e.method.kind() == MethodKind::Semantic)
    {
        return false;
    }
    m.edges_from(&node).any(|e| {
        matches!(
            e.kind,
            EdgeKind::ReferencesSymbol | EdgeKind::MentionsSymbol
        ) && e.to.is_some()
    })
}

fn symbol_context(
    m: &Model,
    root: &Path,
    s: &Symbol,
    reason: String,
    cache: &mut HashMap<String, Option<String>>,
) -> Option<SymbolContext> {
    let file = m.files.get(&s.file)?;
    let text = cache
        .entry(file.path.to_string())
        .or_insert_with(|| {
            read_text(root, &file.path)
                .ok()
                .filter(|t| ContentHash::of_str(t) == file.content_hash)
        })
        .as_ref()?;
    let lines: Vec<&str> = text.lines().collect();
    let start = s.range.start_line.max(1);
    let end = s
        .range
        .end_line
        .min(start + MAX_SYMBOL_LINES - 1)
        .min(lines.len() as u32);
    if start > end {
        return None;
    }
    Some(SymbolContext {
        symbol: s.id.clone(),
        qualified_name: s.qualified_name(),
        kind: s.kind,
        file: file.path.clone(),
        file_hash: file.content_hash.clone(),
        content_hash: s.content_hash.clone(),
        signature: s.signature.clone(),
        doc: s.doc.clone(),
        facts: s.facts.iter().take(30).cloned().collect(),
        start_line: start,
        lines: lines[(start - 1) as usize..end as usize]
            .iter()
            .map(|l| l.to_string())
            .collect(),
        reason,
    })
}

/// Scoped context for one claim. `None` when there is nothing to analyze or a file
/// changed since indexing (a change batch is pending and will re-queue the claim).
fn build_request(
    m: &Model,
    root: &Path,
    claim_id: &ClaimId,
    mode: &JobMode,
    model_name: &str,
    id: AnalysisId,
) -> Option<(AnalysisRequest, String)> {
    let claim = m.claims.get(claim_id)?;
    let doc = m.documents.get(&claim.document)?;
    let doc_text = read_text(root, &doc.path)
        .ok()
        .filter(|t| ContentHash::of_str(t) == doc.content_hash)?;
    let doc_lines: Vec<&str> = doc_text.lines().collect();
    let from = claim.range.start_line.saturating_sub(3).max(1);
    let to = (claim.range.end_line + 3)
        .min(doc_lines.len() as u32)
        .min(from + 40);
    let passage: Vec<String> = if from <= to {
        doc_lines[(from - 1) as usize..to as usize]
            .iter()
            .map(|l| l.to_string())
            .collect()
    } else {
        vec![]
    };

    let node = NodeRef::Claim(claim_id.clone());
    let mut picks: Vec<(&Symbol, String)> = Vec::new();
    let mut edges: Vec<_> = m
        .edges_from(&node)
        .filter(|e| {
            matches!(
                e.kind,
                EdgeKind::ReferencesSymbol | EdgeKind::MentionsSymbol
            )
        })
        .collect();
    edges.sort_by(|a, b| {
        (a.kind != EdgeKind::ReferencesSymbol)
            .cmp(&(b.kind != EdgeKind::ReferencesSymbol))
            .then(
                b.confidence
                    .partial_cmp(&a.confidence)
                    .unwrap_or(std::cmp::Ordering::Equal),
            )
    });
    for e in edges {
        let Some(NodeRef::Symbol(id)) = &e.to else {
            continue;
        };
        let Some(s) = m.symbols.get(id) else { continue };
        if picks.iter().any(|(p, _)| p.id == s.id) {
            continue;
        }
        let reason = if e.kind == EdgeKind::ReferencesSymbol {
            format!("explicit reference `{}`", e.target_text)
        } else {
            format!("name similarity with \"{}\"", e.target_text)
        };
        picks.push((s, reason));
        if picks.len() >= MAX_INITIAL_SYMBOLS {
            break;
        }
    }
    let mut cache = HashMap::new();
    let mut symbols = Vec::new();
    for (s, reason) in picks {
        // Every excerpt must come from the indexed file content; a changed file aborts.
        symbols.push(symbol_context(m, root, s, reason, &mut cache)?);
    }
    if symbols.is_empty() {
        return None;
    }
    let mut key_parts: Vec<String> = vec![
        PROMPT_VERSION.into(),
        model_name.into(),
        claim.content_hash.to_string(),
        ContentHash::of_str(&passage.join(
            "
",
        ))
        .to_string(),
    ];
    key_parts.push(match mode {
        JobMode::Assess => "assess".into(),
        JobMode::Challenge { prior, .. } => format!(
            "challenge:{}",
            ContentHash::of_str(&serde_json::to_string(prior).unwrap_or_default())
        ),
    });
    let mut syms: Vec<String> = symbols
        .iter()
        .map(|s| format!("{}={}", s.symbol, s.content_hash))
        .collect();
    syms.sort();
    key_parts.extend(syms);
    let refs: Vec<&str> = key_parts.iter().map(String::as_str).collect();
    let key = concordance_core::short_hash(&refs) + &concordance_core::short_hash(&refs[1..]);
    let request = AnalysisRequest {
        id,
        claim: ClaimContext {
            claim: claim.clone(),
            document_path: doc.path.clone(),
            document_authority: doc.authority.level,
            passage_start_line: from,
            passage,
        },
        symbols,
        mode: match mode {
            JobMode::Assess => AnalysisMode::Assess,
            JobMode::Challenge { prior, .. } => AnalysisMode::Challenge {
                prior: prior.clone(),
            },
        },
        model: model_name.to_string(),
    };
    Some((request, key))
}

/// Context expansion against the live model (reads current, hash-checked files).
struct ModelProvider {
    shared: Arc<Shared>,
}

impl ContextProvider for ModelProvider {
    fn expand(&self, wanted: &ContextWanted) -> Option<SymbolContext> {
        let ContextWanted::Symbol { path } = wanted;
        let segs: Vec<String> = path
            .split("::")
            .map(|s| s.trim().trim_end_matches("()").to_string())
            .filter(|s| !s.is_empty())
            .collect();
        let root = self.shared.config.repo_root.clone();
        self.shared
            .read(|m| {
                let id = m.resolve_path(&segs, None).best().cloned()?;
                let s = m.symbols.get(&id)?;
                symbol_context(
                    m,
                    &root,
                    s,
                    format!("requested by analysis: `{path}`"),
                    &mut HashMap::new(),
                )
            })
            .0
    }
}

/// Convert a validated analysis result into evidence about the claim.
fn to_evidence(
    claim: &Claim,
    analysis: &AnalysisId,
    mode: &JobMode,
    r: &AnalysisResult,
    ctx: &BatchContext,
) -> Evidence {
    let a = &r.assessment;
    let model = r
        .invocation
        .reported_model
        .clone()
        .unwrap_or_else(|| r.invocation.requested_model.clone());
    let (stance, method, key) = match mode {
        JobMode::Assess => {
            let stance = match a.verdict {
                Verdict::Agrees => Stance::Supports,
                Verdict::Disagrees => Stance::Contradicts,
                _ => Stance::Inconclusive,
            };
            (
                stance,
                Method::Semantic {
                    analysis: analysis.clone(),
                    model,
                    verdict: a.verdict.as_str().into(),
                },
                "assess",
            )
        }
        JobMode::Challenge { prior, .. } => {
            // Stance toward the claim, given what the challenger did to the prior assessment.
            let stance = match (prior.verdict, a.challenge) {
                (Verdict::Disagrees, Some(ChallengeOutcome::Upheld)) => Stance::Contradicts,
                (Verdict::Agrees, Some(ChallengeOutcome::Upheld)) => Stance::Supports,
                (Verdict::Agrees, Some(ChallengeOutcome::Refuted)) => Stance::Contradicts,
                _ => Stance::Inconclusive,
            };
            (
                stance,
                Method::Adversarial {
                    analysis: analysis.clone(),
                    model,
                    verdict: challenge_str(a.challenge).into(),
                },
                "challenge",
            )
        }
    };
    let has_cites = !a.supporting.is_empty() || !a.contradicting.is_empty();
    let strength = if has_cites && stance != Stance::Inconclusive {
        Strength::Moderate
    } else {
        Strength::Weak
    };
    let mut citations = vec![Citation {
        role: CitationRole::Specification,
        range: claim.range.clone(),
        excerpt: concordance_graph::truncate(&claim.text, 2000),
        node: Some(NodeRef::Claim(claim.id.clone())),
    }];
    for c in a.contradicting.iter().chain(&a.supporting) {
        citations.push(Citation {
            role: c.role,
            range: c.range.clone(),
            excerpt: concordance_graph::truncate(&c.excerpt, 2000),
            node: c.symbol.clone().map(NodeRef::Symbol),
        });
    }
    let mut summary = a.explanation.clone();
    if !a.ambiguities.is_empty() {
        summary.push_str(&format!(" Ambiguities: {}.", a.ambiguities.join("; ")));
    }
    // In challenge mode only the challenge outcome is used, so only its downgrades matter.
    let downgrade = a.downgraded.as_ref().map(|d| match mode {
        JobMode::Assess => d.clone(),
        JobMode::Challenge { .. } => d
            .split("; ")
            .filter(|p| p.contains("challenge"))
            .collect::<Vec<_>>()
            .join("; "),
    });
    if let Some(d) = downgrade.filter(|d| !d.is_empty()) {
        summary.push_str(&format!(" [Validation: {d}.]"));
    }
    if !a.rejected.is_empty() {
        summary.push_str(&format!(
            " [{} citation(s) rejected by validation.]",
            a.rejected.len()
        ));
    }
    Evidence {
        id: EvidenceId::from_key(&[claim.id.as_str(), "semantic", key]),
        subject: NodeRef::Claim(claim.id.clone()),
        stance,
        strength,
        method,
        summary,
        citations,
        dependencies: r.dependencies.clone(),
        validity: Validity::Valid,
        revision: ctx.revision,
        created_at: ctx.now,
    }
}
