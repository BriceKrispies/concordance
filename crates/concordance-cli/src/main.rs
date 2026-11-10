//! `concordance` command-line interface.
//!
//! Exit codes (stable contract, see docs/cli.md):
//! * 0: success (and the `--fail-on` gate, if any, passed)
//! * 1: the `--fail-on` gate failed
//! * 2: usage or configuration error
//! * 3: runtime error (state store, I/O)

mod report;

use clap::{Args, Parser, Subcommand};
use concordance_core::config::{Config, ConfigFile, Overrides};
use concordance_core::event::Event;
use concordance_engine::{EngineHandle, EngineOptions};
use concordance_events::{EventBus, Next, DEFAULT_CAPACITY};
use report::FailOn;
use std::io::Write;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Parser)]
#[command(
    name = "concordance",
    version,
    about = "Continuous specification <-> Rust code reconciliation"
)]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Index, reconcile, watch for changes, and serve the web UI + API.
    Serve(ServeArgs),
    /// Scan once and print findings (CI-friendly).
    Scan(ScanArgs),
    /// Scan, then watch for changes and stream events.
    Watch(WatchArgs),
    /// Print the persisted findings without scanning.
    Report(ReportArgs),
}

#[derive(Args, Clone)]
struct Common {
    /// Repository root (default: current directory, or `repo` from the config file).
    #[arg(long)]
    repo: Option<PathBuf>,
    /// Markdown specification root (repeatable; absolute or relative to the repository).
    #[arg(long = "spec-root")]
    spec_roots: Vec<PathBuf>,
    /// Rust source root (repeatable; absolute or relative to the repository).
    #[arg(long = "src-root")]
    src_roots: Vec<PathBuf>,
    /// Configuration file (default: `concordance.toml` in the repository, if present).
    #[arg(long)]
    config: Option<PathBuf>,
    /// Where Concordance keeps its state (default: per-user data directory, outside the repository).
    #[arg(long)]
    state_dir: Option<PathBuf>,
    /// Test-run output to ingest as evidence (repeatable): libtest text, libtest JSON
    /// lines, or cargo-nextest output. May live outside the repository.
    #[arg(long = "test-results")]
    test_results: Vec<PathBuf>,
    /// Enable semantic analysis through the local Claude Code CLI.
    #[arg(long, conflicts_with = "no_semantic")]
    semantic: bool,
    /// Disable semantic analysis even if the config file enables it.
    #[arg(long)]
    no_semantic: bool,
    /// Model alias for semantic analysis (e.g. `sonnet`, `opus`, `haiku`).
    #[arg(long)]
    model: Option<String>,
}

#[derive(Args)]
struct ServeArgs {
    #[command(flatten)]
    common: Common,
    #[arg(long)]
    port: Option<u16>,
    /// Bind address (default 127.0.0.1). Non-loopback addresses expose the UI to the network.
    #[arg(long)]
    host: Option<String>,
    /// Do not watch for file changes.
    #[arg(long)]
    no_watch: bool,
}

#[derive(Args)]
struct ScanArgs {
    #[command(flatten)]
    common: Common,
    /// Print the report as JSON.
    #[arg(long)]
    json: bool,
    /// Stream all events as JSON lines on stdout, then the report as the last line.
    #[arg(long, conflicts_with = "json")]
    jsonl: bool,
    /// Re-parse every file even if its content hash is unchanged.
    #[arg(long)]
    force: bool,
    /// Exit with code 1 when findings of this kind exist.
    #[arg(long, value_enum, default_value_t = FailOn::None)]
    fail_on: FailOn,
    /// List every finding in human output.
    #[arg(long, short)]
    verbose: bool,
    /// Include resolved findings.
    #[arg(long)]
    include_resolved: bool,
}

#[derive(Args)]
struct WatchArgs {
    #[command(flatten)]
    common: Common,
    /// Emit events as JSON lines (default: human-readable lines).
    #[arg(long)]
    jsonl: bool,
}

#[derive(Args)]
struct ReportArgs {
    #[command(flatten)]
    common: Common,
    #[arg(long, value_enum, default_value_t = Format::Text)]
    format: Format,
    #[arg(long, value_enum, default_value_t = FailOn::None)]
    fail_on: FailOn,
    #[arg(long, short)]
    verbose: bool,
    #[arg(long)]
    include_resolved: bool,
}

#[derive(Clone, Copy, clap::ValueEnum)]
enum Format {
    Text,
    Json,
}

fn default_state_base() -> PathBuf {
    if let Some(d) = std::env::var_os("LOCALAPPDATA") {
        return PathBuf::from(d).join("concordance");
    }
    if let Some(d) = std::env::var_os("XDG_DATA_HOME") {
        return PathBuf::from(d).join("concordance");
    }
    if let Some(h) = std::env::var_os("HOME") {
        return PathBuf::from(h)
            .join(".local")
            .join("share")
            .join("concordance");
    }
    std::env::temp_dir().join("concordance")
}

fn load_config(c: &Common, host: Option<String>, port: Option<u16>) -> Result<Config, String> {
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    let explicit = c.config.clone();
    let candidate = explicit.clone().or_else(|| {
        let repo = c
            .repo
            .clone()
            .map(|r| if r.is_absolute() { r } else { cwd.join(r) })
            .unwrap_or_else(|| cwd.clone());
        let p = repo.join("concordance.toml");
        p.exists().then_some(p)
    });
    let (file, dir) = match candidate {
        Some(p) => {
            let text = std::fs::read_to_string(&p)
                .map_err(|e| format!("cannot read {}: {e}", p.display()))?;
            let f: ConfigFile = toml::from_str(&text)
                .map_err(|e| format!("invalid config {}: {e}", p.display()))?;
            (f, p.parent().map(Path::to_path_buf))
        }
        None => (ConfigFile::default(), None),
    };
    let overrides = Overrides {
        repo: c.repo.clone(),
        spec_roots: c.spec_roots.clone(),
        src_roots: c.src_roots.clone(),
        state_dir: c.state_dir.clone(),
        test_results: c.test_results.clone(),
        semantic: if c.semantic {
            Some(true)
        } else if c.no_semantic {
            Some(false)
        } else {
            None
        },
        model: c.model.clone(),
        host,
        port,
    };
    Config::resolve(file, dir.as_deref(), overrides, &cwd, &default_state_base())
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let rt = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(3);
        }
    };
    rt.block_on(async {
        match cli.command {
            Cmd::Serve(a) => serve(a).await,
            Cmd::Scan(a) => scan(a).await,
            Cmd::Watch(a) => watch(a).await,
            Cmd::Report(a) => report_cmd(a),
        }
    })
}

fn config_or_exit(c: &Common, host: Option<String>, port: Option<u16>) -> Result<Config, ExitCode> {
    load_config(c, host, port).map_err(|e| {
        eprintln!("error: {e}");
        ExitCode::from(2)
    })
}

async fn start_engine(
    config: Config,
    watch: bool,
) -> Result<(EngineHandle, tokio::task::JoinHandle<()>, EventBus), ExitCode> {
    let bus = EventBus::new(DEFAULT_CAPACITY);
    match concordance_engine::start(
        config,
        bus.clone(),
        EngineOptions {
            watch,
            ..Default::default()
        },
    )
    .await
    {
        Ok((h, t)) => Ok((h, t, bus)),
        Err(e) => {
            eprintln!("error: {e}");
            Err(ExitCode::from(3))
        }
    }
}

/// Subscribe from the first event this process published (the ring holds nothing older).
fn subscribe_all(bus: &EventBus) -> concordance_events::Subscription {
    bus.subscribe(bus.oldest_seq().map(|s| s - 1))
}

async fn serve(a: ServeArgs) -> ExitCode {
    let config = match config_or_exit(&a.common, a.host.clone(), a.port) {
        Ok(c) => c,
        Err(code) => return code,
    };
    let addr: SocketAddr = match format!("{}:{}", config.host, config.port)
        .parse()
        .or_else(|_| {
            use std::net::ToSocketAddrs;
            format!("{}:{}", config.host, config.port)
                .to_socket_addrs()
                .ok()
                .and_then(|mut i| i.next())
                .ok_or(())
        }) {
        Ok(a) => a,
        Err(_) => {
            eprintln!("error: invalid host/port {}:{}", config.host, config.port);
            return ExitCode::from(2);
        }
    };
    let repo = config.repo_root.display().to_string();
    let state = config.state_dir.display().to_string();
    let (engine, task, bus) = match start_engine(config, !a.no_watch).await {
        Ok(v) => v,
        Err(c) => return c,
    };
    let mut shutdown_rx = engine.subscribe_shutdown();
    let server_shutdown = async move {
        let _ = shutdown_rx.changed().await;
    };
    let local = match concordance_server::serve(engine.clone(), addr, server_shutdown).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("error: cannot listen on {addr}: {e}");
            return ExitCode::from(3);
        }
    };
    if !local.ip().is_loopback() {
        eprintln!(
            "warning: listening on a non-loopback address; the UI and API have no authentication."
        );
    }
    eprintln!("Concordance serving {repo}\n  UI:    http://{local}/\n  API:   http://{local}/api/v1/status\n  state: {state}\nPress Ctrl+C to stop.");
    let mut sub = subscribe_all(&bus);
    let printer = tokio::spawn(async move {
        loop {
            match sub.next().await {
                Next::Event(env) => match &env.event {
                    Event::ScanCompleted { counts, changed_files, revision, .. } => eprintln!(
                        "revision {}: {} file(s) changed — {} violated, {} stale, {} ambiguous, {} unverified, {} verified, {} undocumented",
                        revision.0, changed_files, counts.violated, counts.stale, counts.ambiguous, counts.unverified, counts.verified, counts.undocumented
                    ),
                    Event::SystemWarning { message } => eprintln!("warning: {message}"),
                    Event::SystemError { message } => eprintln!("error: {message}"),
                    _ => {}
                },
                Next::Lagged { .. } => {}
                Next::Closed => break,
            }
        }
    });
    let _ = tokio::signal::ctrl_c().await;
    eprintln!("shutting down…");
    engine.shutdown();
    bus.close();
    let _ = tokio::time::timeout(std::time::Duration::from_secs(10), task).await;
    let _ = printer.await;
    ExitCode::SUCCESS
}

async fn scan(a: ScanArgs) -> ExitCode {
    let config = match config_or_exit(&a.common, None, None) {
        Ok(c) => c,
        Err(code) => return code,
    };
    let repo = config.repo_root.display().to_string();
    let (engine, task, bus) = match start_engine(config, false).await {
        Ok(v) => v,
        Err(c) => return c,
    };
    if a.force {
        engine.request_scan(true).await;
    }
    let mut sub = subscribe_all(&bus);
    let jsonl = a.jsonl;
    let human = !a.json && !a.jsonl;
    let streamer = tokio::spawn(async move {
        let stdout = std::io::stdout();
        loop {
            match sub.next().await {
                Next::Event(env) => {
                    if jsonl {
                        let mut lock = stdout.lock();
                        let _ = serde_json::to_writer(&mut lock, &*env);
                        let _ = lock.write_all(b"\n");
                    } else if human {
                        match &env.event {
                            Event::ScanStarted { files, .. } => {
                                eprintln!("scanning {files} file(s)…")
                            }
                            Event::SystemWarning { message } => eprintln!("warning: {message}"),
                            Event::SystemError { message } => eprintln!("error: {message}"),
                            Event::AnalysisCompleted {
                                outcome, verdict, ..
                            } => eprintln!(
                                "semantic analysis {outcome:?}{}",
                                verdict
                                    .as_ref()
                                    .map(|v| format!(": {v}"))
                                    .unwrap_or_default()
                            ),
                            _ => {}
                        }
                    }
                }
                Next::Lagged { latest } => {
                    if jsonl {
                        println!(
                            "{}",
                            serde_json::json!({ "type": "resync", "data": { "latest": latest } })
                        );
                    }
                }
                Next::Closed => break,
            }
        }
    });
    engine.wait_idle().await;
    let status = engine.shared.status();
    let (rep, _) = engine
        .shared
        .read(|m| report::build(m, &repo, &status.semantic, a.fail_on, a.include_resolved));
    engine.shutdown();
    bus.close();
    let _ = task.await;
    let _ = streamer.await;
    if a.json || a.jsonl {
        let out = serde_json::to_string(&rep).unwrap_or_default();
        println!("{out}");
    } else {
        print!("{}", report::human(&rep, a.verbose));
    }
    if rep.gate.failed {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

async fn watch(a: WatchArgs) -> ExitCode {
    let config = match config_or_exit(&a.common, None, None) {
        Ok(c) => c,
        Err(code) => return code,
    };
    let (engine, task, bus) = match start_engine(config, true).await {
        Ok(v) => v,
        Err(c) => return c,
    };
    let mut sub = subscribe_all(&bus);
    let jsonl = a.jsonl;
    let streamer = tokio::spawn(async move {
        let stdout = std::io::stdout();
        loop {
            match sub.next().await {
                Next::Event(env) => {
                    let mut lock = stdout.lock();
                    if jsonl {
                        let _ = serde_json::to_writer(&mut lock, &*env);
                        let _ = lock.write_all(b"\n");
                    } else if !matches!(
                        env.event,
                        Event::ScanProgress { .. }
                            | Event::SymbolCreated { .. }
                            | Event::AnalysisProgress { .. }
                    ) {
                        let _ = writeln!(
                            lock,
                            "[{}] {} {}",
                            env.seq,
                            env.event.type_name(),
                            describe(&env.event)
                        );
                    }
                    let _ = lock.flush();
                }
                Next::Lagged { latest } => {
                    println!(
                        "{}",
                        serde_json::json!({ "type": "resync", "data": { "latest": latest } })
                    );
                }
                Next::Closed => break,
            }
        }
    });
    let _ = tokio::signal::ctrl_c().await;
    engine.shutdown();
    bus.close();
    let _ = tokio::time::timeout(std::time::Duration::from_secs(10), task).await;
    let _ = streamer.await;
    ExitCode::SUCCESS
}

fn describe(e: &Event) -> String {
    match e {
        Event::FindingCreated { finding } | Event::FindingUpdated { finding, .. } => format!(
            "{} [{}] {}",
            finding.id,
            finding.status.as_str(),
            finding.title
        ),
        Event::FindingResolved {
            finding,
            previous_status,
        } => format!("{finding} (was {})", previous_status.as_str()),
        Event::DocumentIndexed { path, claims, .. } => format!("{path} ({claims} claims)"),
        Event::SourceIndexed { path, symbols, .. } => format!("{path} ({symbols} symbols)"),
        Event::WatchChangeDetected { changes } => changes
            .iter()
            .map(|(p, k)| format!("{p} {k:?}"))
            .collect::<Vec<_>>()
            .join(", "),
        Event::EvidenceInvalidated { reason, .. } => reason.clone(),
        Event::SystemWarning { message } | Event::SystemError { message } => message.clone(),
        Event::ScanCompleted {
            revision,
            changed_files,
            ..
        } => format!("revision {} ({} changed)", revision.0, changed_files),
        _ => String::new(),
    }
}

fn report_cmd(a: ReportArgs) -> ExitCode {
    let config = match config_or_exit(&a.common, None, None) {
        Ok(c) => c,
        Err(code) => return code,
    };
    let model = match concordance_engine::load_persisted(&config) {
        Ok(Some(m)) => m,
        Ok(None) => {
            eprintln!(
                "error: no persisted state for {} (run `concordance scan` first)",
                config.repo_root.display()
            );
            return ExitCode::from(2);
        }
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(3);
        }
    };
    let semantic = concordance_engine::SemanticStatus {
        enabled: config.semantic.enabled,
        available: None,
        model: config.semantic.model.clone(),
        detail: Some("report reads persisted state; semantic availability not probed".into()),
        ..Default::default()
    };
    let rep = report::build(
        &model,
        &config.repo_root.display().to_string(),
        &semantic,
        a.fail_on,
        a.include_resolved,
    );
    match a.format {
        Format::Json => println!("{}", serde_json::to_string(&rep).unwrap_or_default()),
        Format::Text => print!("{}", report::human(&rep, a.verbose)),
    }
    if rep.gate.failed {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}
