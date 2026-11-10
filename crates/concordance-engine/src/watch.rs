//! File watching with debouncing.
//!
//! Raw notify events (which arrive in bursts: editor atomic saves write a temp file and
//! rename it, formatters rewrite files, git checkouts touch hundreds) are collected
//! into a path set. The set is flushed to the engine once the filesystem has been quiet
//! for `QUIET`, or after `MAX_WAIT` of continuous activity. Irrelevant paths are
//! dropped early; the engine re-classifies, re-hashes and skips unchanged content, so
//! duplicate or spurious events cost one hash.

use crate::discover::PathFilter;
use crate::Command;
use concordance_core::config::Config;
use notify::{RecursiveMode, Watcher};
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

const QUIET: Duration = Duration::from_millis(300);
const MAX_WAIT: Duration = Duration::from_secs(2);

pub struct WatchHandle {
    _watcher: notify::RecommendedWatcher,
    stop: std::sync::mpsc::Sender<()>,
}

impl Drop for WatchHandle {
    fn drop(&mut self) {
        let _ = self.stop.send(());
    }
}

pub fn start(config: &Config, engine: mpsc::Sender<Command>) -> Result<WatchHandle, String> {
    let (raw_tx, raw_rx) = std::sync::mpsc::channel::<PathBuf>();
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if let Ok(ev) = res {
            if matches!(ev.kind, notify::EventKind::Access(_)) {
                return;
            }
            for p in ev.paths {
                let _ = raw_tx.send(p);
            }
        }
    })
    .map_err(|e| e.to_string())?;
    let mut roots: Vec<&PathBuf> = config.spec_roots.iter().chain(&config.src_roots).collect();
    roots.sort();
    roots.dedup();
    // Watch only top-most roots (nested roots are covered recursively).
    let tops: Vec<&PathBuf> = roots
        .iter()
        .copied()
        .filter(|r| !roots.iter().any(|o| o != r && r.starts_with(o)))
        .collect();
    for r in &tops {
        watcher
            .watch(r, RecursiveMode::Recursive)
            .map_err(|e| format!("{}: {e}", r.display()))?;
    }
    // Test-run outputs may live anywhere (often outside the repository): watch their
    // directories, non-recursively, and keep only events for the files themselves.
    let results: Vec<PathBuf> = config.test_results.clone();
    let mut dirs: Vec<PathBuf> = results
        .iter()
        .filter_map(|p| p.parent().map(PathBuf::from))
        .collect();
    dirs.sort();
    dirs.dedup();
    for d in dirs
        .iter()
        .filter(|d| d.is_dir() && !tops.iter().any(|t| d.starts_with(t)))
    {
        watcher
            .watch(d, RecursiveMode::NonRecursive)
            .map_err(|e| format!("{}: {e}", d.display()))?;
    }

    let filter = PathFilter::new(config);
    let (stop_tx, stop_rx) = std::sync::mpsc::channel::<()>();
    std::thread::Builder::new()
        .name("concordance-watch".into())
        .spawn(move || {
            let mut pending: BTreeSet<PathBuf> = BTreeSet::new();
            let mut first: Option<Instant> = None;
            let mut last = Instant::now();
            loop {
                if stop_rx.try_recv().is_ok() {
                    return;
                }
                match raw_rx.recv_timeout(Duration::from_millis(100)) {
                    Ok(p) => {
                        // Keep files we analyze, and anything that vanished (could be a
                        // deleted file or directory we know about).
                        let is_results = results.iter().any(|r| {
                            r.to_string_lossy()
                                .eq_ignore_ascii_case(&p.to_string_lossy())
                        });
                        if is_results || filter.classify(&p).is_some() || !p.exists() {
                            pending.insert(p);
                            first.get_or_insert_with(Instant::now);
                            last = Instant::now();
                        }
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
                }
                let due = first.is_some_and(|f| last.elapsed() >= QUIET || f.elapsed() >= MAX_WAIT);
                if due && !pending.is_empty() {
                    let batch: Vec<PathBuf> = std::mem::take(&mut pending).into_iter().collect();
                    first = None;
                    if engine.blocking_send(Command::FilesChanged(batch)).is_err() {
                        return;
                    }
                }
            }
        })
        .map_err(|e| e.to_string())?;
    Ok(WatchHandle {
        _watcher: watcher,
        stop: stop_tx,
    })
}
