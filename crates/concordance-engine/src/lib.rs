//! The incremental reconciliation engine.
//!
//! One engine task owns all mutation (single writer, no write races). It processes
//! commands serially: scans, coalesced file-change batches, analysis requests and
//! analysis results. CPU-heavy parsing runs on the blocking pool with bounded
//! parallelism; semantic analyses run as bounded background tasks whose results come
//! back as commands and are applied only if every input they depended on is still
//! current.
//!
//! State is published three ways, all consistent with each other:
//! * the in-memory [`Model`] behind a `RwLock` (HTTP snapshots),
//! * typed events on the [`EventBus`] (published while the write lock is held, so a
//!   snapshot plus `bus.last_seq()` read under the read lock is consistent),
//! * SQLite via [`Store`] (one transaction per change batch).

mod apply;
mod discover;
mod scan;
mod semantic;
pub mod source;
mod watch;

use concordance_core::config::Config;
use concordance_core::event::{Event, ScanPhase, ScanReason};
use concordance_core::{ClaimId, Revision, Timestamp};
use concordance_events::EventBus;
use concordance_graph::Model;
use concordance_semantic::Backend;
use concordance_storage::Store;
use serde::Serialize;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};
use tokio::sync::{mpsc, oneshot, watch as tokio_watch};

pub use concordance_storage::{FindingTransition, RevisionRecord};

/// Observable engine state for `/status` and headless summaries.
#[derive(Clone, Debug, Default, Serialize)]
pub struct EngineStatus {
    pub scan: Option<ScanStatus>,
    pub last_scan: Option<LastScan>,
    pub semantic: SemanticStatus,
    pub watching: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct ScanStatus {
    pub id: u64,
    pub reason: ScanReason,
    pub phase: ScanPhase,
    pub done: u32,
    pub total: u32,
    pub started_at: Timestamp,
}

#[derive(Clone, Debug, Serialize)]
pub struct LastScan {
    pub revision: Revision,
    pub at: Timestamp,
    pub duration_ms: u64,
    pub changed_files: u32,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct SemanticStatus {
    pub enabled: bool,
    /// `None` until probed; `Some(false)` when the CLI is missing or unusable.
    pub available: Option<bool>,
    pub detail: Option<String>,
    pub model: String,
    pub queued: usize,
    pub running: usize,
    pub completed: u64,
    pub failed: u64,
    pub cached: u64,
    pub superseded: u64,
}

/// State shared between the engine task and readers.
pub struct Shared {
    pub config: Config,
    pub model: RwLock<Model>,
    pub bus: EventBus,
    pub store: Mutex<Store>,
    pub status: RwLock<EngineStatus>,
}

impl Shared {
    /// Run `f` against a consistent snapshot; returns the event seq it is consistent with.
    pub fn read<T>(&self, f: impl FnOnce(&Model) -> T) -> (T, u64) {
        let m = self.model.read().unwrap_or_else(|e| e.into_inner());
        let seq = self.bus.last_seq();
        (f(&m), seq)
    }

    pub fn status(&self) -> EngineStatus {
        self.status
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    fn update_status(&self, f: impl FnOnce(&mut EngineStatus)) {
        f(&mut self.status.write().unwrap_or_else(|e| e.into_inner()));
    }
}

#[derive(Debug)]
pub enum EngineError {
    Store(String),
    Io(String),
}

impl std::fmt::Display for EngineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EngineError::Store(e) => write!(f, "state store: {e}"),
            EngineError::Io(e) => write!(f, "io: {e}"),
        }
    }
}

impl std::error::Error for EngineError {}

#[derive(Default)]
pub struct EngineOptions {
    /// Watch the roots for changes after the initial scan.
    pub watch: bool,
    /// Override the semantic backend (tests). `None` = Claude CLI when enabled.
    pub backend: Option<Arc<dyn Backend>>,
    /// Override where the database lives (tests); default `<state_dir>/state.db`.
    pub db_path: Option<PathBuf>,
}

pub(crate) enum Command {
    Scan {
        force: bool,
        reason: ScanReason,
    },
    FilesChanged(Vec<PathBuf>),
    Analyze {
        claims: Vec<ClaimId>,
        all: bool,
        reply: oneshot::Sender<Result<usize, String>>,
    },
    AnalysisDone(Box<semantic::JobOutcome>),
    WhenIdle(oneshot::Sender<()>),
}

/// Handle to a running engine. Cheap to clone.
#[derive(Clone)]
pub struct EngineHandle {
    pub shared: Arc<Shared>,
    cmd: mpsc::Sender<Command>,
    shutdown: tokio_watch::Sender<bool>,
}

impl EngineHandle {
    pub async fn request_scan(&self, force: bool) {
        let _ = self
            .cmd
            .send(Command::Scan {
                force,
                reason: ScanReason::Manual,
            })
            .await;
    }

    /// Queue semantic analysis for specific claims (or every candidate with `all`).
    pub async fn request_analysis(&self, claims: Vec<ClaimId>, all: bool) -> Result<usize, String> {
        let (tx, rx) = oneshot::channel();
        self.cmd
            .send(Command::Analyze {
                claims,
                all,
                reply: tx,
            })
            .await
            .map_err(|_| "engine stopped".to_string())?;
        rx.await.map_err(|_| "engine stopped".to_string())?
    }

    /// Resolves when no scan is running and no semantic analysis is queued or running.
    pub async fn wait_idle(&self) {
        let (tx, rx) = oneshot::channel();
        if self.cmd.send(Command::WhenIdle(tx)).await.is_ok() {
            let _ = rx.await;
        }
    }

    /// Feed paths as if the watcher had reported them (tests, editors integrations).
    pub async fn notify_changed(&self, paths: Vec<PathBuf>) {
        let _ = self.cmd.send(Command::FilesChanged(paths)).await;
    }

    pub fn shutdown(&self) {
        self.shutdown.send_replace(true);
    }

    pub fn subscribe_shutdown(&self) -> tokio_watch::Receiver<bool> {
        self.shutdown.subscribe()
    }
}

/// The built-in source-language plugins.
pub fn languages() -> &'static concordance_core::lang::Languages {
    static LANGS: std::sync::OnceLock<concordance_core::lang::Languages> =
        std::sync::OnceLock::new();
    LANGS.get_or_init(|| {
        concordance_core::lang::Languages::new(vec![
            std::sync::Arc::new(concordance_rust::RustPlugin),
            std::sync::Arc::new(concordance_typescript::TypeScriptPlugin),
        ])
    })
}

/// Identifies the extractor build; persisted so that extractor changes force re-indexing.
pub fn extractor_version() -> String {
    format!(
        "{}+{}+{}",
        concordance_markdown::EXTRACTOR_VERSION,
        languages().extractor_versions(),
        concordance_reconcile::RULES_VERSION
    )
}

/// Load the persisted model without starting the engine (headless `report`).
/// Returns `None` when no state exists yet for this repository.
pub fn load_persisted(config: &Config) -> Result<Option<Model>, EngineError> {
    let db = config.state_dir.join("state.db");
    if !db.exists() {
        return Ok(None);
    }
    let store = Store::open(&db).map_err(|e| EngineError::Store(e.to_string()))?;
    let snap = store
        .load()
        .map_err(|e| EngineError::Store(e.to_string()))?;
    Ok(Some(apply::model_from_snapshot(snap)))
}

/// Open state, restore the persisted model, start the engine task, and queue the
/// initial scan. Returns immediately; progress is observable through events.
pub async fn start(
    config: Config,
    bus: EventBus,
    options: EngineOptions,
) -> Result<(EngineHandle, tokio::task::JoinHandle<()>), EngineError> {
    let db_path = options
        .db_path
        .clone()
        .unwrap_or_else(|| config.state_dir.join("state.db"));
    let store = tokio::task::spawn_blocking(move || Store::open(&db_path))
        .await
        .map_err(|e| EngineError::Io(e.to_string()))?
        .map_err(|e| EngineError::Store(e.to_string()))?;
    let snapshot = store
        .load()
        .map_err(|e| EngineError::Store(e.to_string()))?;
    let restored = snapshot.revision.0 > 0;
    let reindex =
        restored && snapshot.extractor_version.as_deref() != Some(extractor_version().as_str());
    // Events published but not yet committed before a crash are gone; jump well past any
    // such window so no sequence number is ever reused. Old cursors then resync.
    let resume = if snapshot.last_seq > 0 {
        snapshot.last_seq + 100_000
    } else {
        0
    };
    bus.resume_after(resume, snapshot.revision);
    let model = apply::model_from_snapshot(snapshot);

    let status = EngineStatus {
        semantic: SemanticStatus {
            enabled: config.semantic.enabled,
            model: config.semantic.model.clone(),
            ..Default::default()
        },
        ..Default::default()
    };
    let shared = Arc::new(Shared {
        config,
        model: RwLock::new(model),
        bus,
        store: Mutex::new(store),
        status: RwLock::new(status),
    });
    let (cmd_tx, cmd_rx) = mpsc::channel(256);
    let (shutdown_tx, shutdown_rx) = tokio_watch::channel(false);
    let handle = EngineHandle {
        shared: shared.clone(),
        cmd: cmd_tx.clone(),
        shutdown: shutdown_tx,
    };

    let semantic =
        semantic::SemanticRunner::new(shared.clone(), options.backend.clone(), cmd_tx.clone());
    let mut engine = Engine {
        shared: shared.clone(),
        cmd_tx: cmd_tx.clone(),
        semantic,
        scan_counter: 0,
        idle_waiters: Vec::new(),
        watcher: None,
    };
    let reason = if restored {
        ScanReason::Restart
    } else {
        ScanReason::Initial
    };
    let watch_enabled = options.watch;
    let task = tokio::spawn(async move {
        engine.semantic.probe().await;
        if reindex {
            engine.publish_now(Event::SystemWarning {
                message: "extractors changed since the last run; re-indexing every file".into(),
            });
        }
        engine.scan(reindex, reason).await;
        if watch_enabled {
            engine.start_watching();
        }
        engine.run(cmd_rx, shutdown_rx).await;
    });
    Ok((handle, task))
}

struct Engine {
    shared: Arc<Shared>,
    cmd_tx: mpsc::Sender<Command>,
    semantic: semantic::SemanticRunner,
    scan_counter: u64,
    idle_waiters: Vec<oneshot::Sender<()>>,
    watcher: Option<watch::WatchHandle>,
}

impl Engine {
    async fn run(
        &mut self,
        mut rx: mpsc::Receiver<Command>,
        mut shutdown: tokio_watch::Receiver<bool>,
    ) {
        loop {
            self.release_idle_waiters();
            let cmd = tokio::select! {
                c = rx.recv() => match c { Some(c) => c, None => break },
                _ = shutdown.changed() => break,
            };
            match cmd {
                Command::Scan { force, reason } => self.scan(force, reason).await,
                Command::FilesChanged(mut paths) => {
                    // Coalesce every change already queued behind this one.
                    let mut deferred = Vec::new();
                    while let Ok(next) = rx.try_recv() {
                        match next {
                            Command::FilesChanged(more) => paths.extend(more),
                            other => deferred.push(other),
                        }
                    }
                    self.files_changed(paths).await;
                    for c in deferred {
                        self.handle_deferred(c).await;
                    }
                }
                other => self.handle_deferred(other).await,
            }
        }
        self.semantic.cancel_all();
        drop(self.watcher.take());
        for w in self.idle_waiters.drain(..) {
            let _ = w.send(());
        }
    }

    async fn handle_deferred(&mut self, cmd: Command) {
        match cmd {
            Command::Scan { force, reason } => self.scan(force, reason).await,
            Command::FilesChanged(paths) => self.files_changed(paths).await,
            Command::Analyze { claims, all, reply } => {
                let r = self.semantic.request(claims, all);
                let _ = reply.send(r);
            }
            Command::AnalysisDone(outcome) => self.semantic.finish(*outcome).await,
            Command::WhenIdle(tx) => self.idle_waiters.push(tx),
        }
    }

    fn release_idle_waiters(&mut self) {
        if self.semantic.is_idle() {
            for w in self.idle_waiters.drain(..) {
                let _ = w.send(());
            }
        }
    }

    fn start_watching(&mut self) {
        match watch::start(&self.shared.config, self.cmd_tx.clone()) {
            Ok(h) => {
                self.watcher = Some(h);
                self.shared.update_status(|s| s.watching = true);
            }
            Err(e) => {
                self.publish_now(Event::SystemError {
                    message: format!("file watching unavailable: {e}"),
                });
            }
        }
    }

    fn publish_now(&self, event: Event) {
        self.shared.bus.publish(event);
    }
}
