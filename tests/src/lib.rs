//! Shared helpers for the integration tests: temporary fixture copies, engine startup,
//! scripted semantic backends and a minimal HTTP/SSE client.

use concordance_core::config::{Config, ConfigFile, Overrides};
use concordance_core::event::{Envelope, Event};
use concordance_engine::{EngineHandle, EngineOptions};
use concordance_events::{EventBus, Next, Subscription};
use concordance_semantic::{
    AnalysisError, Backend, BackendRequest, BackendResponse, BoxFuture, CancelRx,
};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}

static COUNTER: AtomicUsize = AtomicUsize::new(0);

/// A fresh temporary directory unique to this test process + call.
pub fn temp_dir(tag: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let d = std::env::temp_dir().join(format!(
        "concordance-test-{}-{}-{tag}",
        std::process::id(),
        n
    ));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap().flatten() {
        let name = e.file_name();
        if name == "target" || name == "Cargo.lock" {
            continue;
        }
        let p = e.path();
        if p.is_dir() {
            copy_dir(&p, &to.join(&name));
        } else {
            std::fs::copy(&p, to.join(&name)).unwrap();
        }
    }
}

/// Copy `fixtures/<name>` into a temp directory (so tests can edit it).
pub fn fixture_copy(name: &str) -> PathBuf {
    let dst = temp_dir(name).join("repo");
    copy_dir(&repo_root().join("fixtures").join(name), &dst);
    dst
}

pub fn config_for(repo: &Path, state: &Path, semantic: bool) -> Config {
    let mut file = ConfigFile::default();
    file.semantic.enabled = Some(semantic);
    file.semantic.max_concurrent = Some(2);
    Config::resolve(
        file,
        None,
        Overrides {
            repo: Some(repo.to_path_buf()),
            spec_roots: vec![PathBuf::from("docs")],
            src_roots: vec![PathBuf::from("src")],
            state_dir: Some(state.to_path_buf()),
            ..Default::default()
        },
        repo,
        state,
    )
    .unwrap()
}

pub struct Running {
    pub engine: EngineHandle,
    pub bus: EventBus,
    pub task: tokio::task::JoinHandle<()>,
    pub sub: Subscription,
}

pub async fn start(config: Config, backend: Option<Arc<dyn Backend>>, capacity: usize) -> Running {
    let bus = EventBus::new(capacity);
    let (engine, task) = concordance_engine::start(
        config,
        bus.clone(),
        EngineOptions {
            watch: false,
            backend,
            db_path: None,
        },
    )
    .await
    .expect("engine starts");
    let sub = bus.subscribe(bus.oldest_seq().map(|s| s - 1));
    Running {
        engine,
        bus,
        task,
        sub,
    }
}

impl Running {
    pub async fn idle(&self) {
        tokio::time::timeout(Duration::from_secs(60), self.engine.wait_idle())
            .await
            .expect("engine became idle");
    }

    /// All events published so far that this subscription has not consumed.
    pub fn drain(&mut self) -> Vec<Arc<Envelope>> {
        let mut out = Vec::new();
        while let Some(n) = self.sub.try_next() {
            match n {
                Next::Event(e) => out.push(e),
                Next::Lagged { .. } => panic!("test subscription lagged"),
                Next::Closed => break,
            }
        }
        out
    }

    pub async fn stop(self) {
        self.engine.shutdown();
        self.bus.close();
        let _ = tokio::time::timeout(Duration::from_secs(10), self.task).await;
    }
}

pub fn types(events: &[Arc<Envelope>]) -> Vec<&'static str> {
    events.iter().map(|e| e.event.type_name()).collect()
}

pub fn has<F: Fn(&Event) -> bool>(events: &[Arc<Envelope>], f: F) -> bool {
    events.iter().any(|e| f(&e.event))
}

/// Semantic backend returning a fixed structured response, optionally waiting for a
/// gate first (to hold an analysis "in flight"). Ignores cancellation on purpose, so the
/// engine's freshness check (not cancellation) is what must reject stale results.
pub struct FixedBackend {
    pub response: serde_json::Value,
    pub gate: Option<Arc<tokio::sync::Semaphore>>,
    pub calls: AtomicUsize,
}

impl FixedBackend {
    pub fn insufficient() -> Self {
        FixedBackend {
            response: serde_json::json!({
                "verdict": "insufficient_evidence",
                "explanation": "scripted: not enough evidence",
                "supporting": [], "contradicting": [], "ambiguities": [], "needed_context": []
            }),
            gate: None,
            calls: AtomicUsize::new(0),
        }
    }
}

impl Backend for FixedBackend {
    fn name(&self) -> &str {
        "fixed-test-backend"
    }
    fn invoke<'a>(
        &'a self,
        _req: BackendRequest<'a>,
        _cancel: CancelRx,
    ) -> BoxFuture<'a, Result<BackendResponse, AnalysisError>> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if let Some(g) = &self.gate {
                g.acquire().await.expect("gate").forget();
            }
            Ok(BackendResponse {
                structured: self.response.clone(),
                reported_model: Some("test-model".into()),
                duration_ms: 1,
                cost_usd: None,
                input_tokens: None,
                output_tokens: None,
                session_id: None,
            })
        })
    }
}

// ------------------------------------------------------------------ minimal HTTP client

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// GET/POST returning (status, body). `Connection: close`, content-length bodies.
pub async fn http(
    addr: std::net::SocketAddr,
    method: &str,
    path: &str,
    host: &str,
    body: Option<&str>,
) -> (u16, String) {
    let mut s = TcpStream::connect(addr).await.unwrap();
    let body = body.unwrap_or("");
    let req = format!(
        "{method} {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    s.write_all(req.as_bytes()).await.unwrap();
    let mut buf = Vec::new();
    tokio::time::timeout(Duration::from_secs(10), s.read_to_end(&mut buf))
        .await
        .unwrap()
        .unwrap();
    let text = String::from_utf8_lossy(&buf).into_owned();
    let status: u16 = text
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let body = text
        .split_once("\r\n\r\n")
        .map(|(_, b)| b.to_string())
        .unwrap_or_default();
    (status, body)
}

/// One parsed SSE message.
#[derive(Debug, Clone)]
pub struct SseMsg {
    pub id: Option<u64>,
    pub event: String,
    pub data: String,
}

/// Open an SSE stream and collect messages until `until` returns true or `timeout`.
pub async fn sse_collect(
    addr: std::net::SocketAddr,
    path: &str,
    timeout: Duration,
    until: impl Fn(&[SseMsg]) -> bool,
) -> Vec<SseMsg> {
    let mut s = TcpStream::connect(addr).await.unwrap();
    let req =
        format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nAccept: text/event-stream\r\n\r\n");
    s.write_all(req.as_bytes()).await.unwrap();
    let mut raw = String::new();
    let mut msgs: Vec<SseMsg> = Vec::new();
    let deadline = tokio::time::Instant::now() + timeout;
    let mut buf = [0u8; 8192];
    loop {
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        if left.is_zero() {
            break;
        }
        match tokio::time::timeout(left, s.read(&mut buf)).await {
            Ok(Ok(0)) | Err(_) | Ok(Err(_)) => break,
            Ok(Ok(n)) => raw.push_str(&String::from_utf8_lossy(&buf[..n])),
        }
        msgs = parse_sse(&raw);
        if until(&msgs) {
            break;
        }
    }
    msgs
}

fn parse_sse(raw: &str) -> Vec<SseMsg> {
    let body = raw.split_once("\r\n\r\n").map(|(_, b)| b).unwrap_or("");
    let mut out = Vec::new();
    let mut cur = SseMsg {
        id: None,
        event: String::new(),
        data: String::new(),
    };
    for line in body.lines() {
        let line = line.trim_end_matches('\r');
        if let Some(v) = line.strip_prefix("id:") {
            cur.id = v.trim().parse().ok();
        } else if let Some(v) = line.strip_prefix("event:") {
            cur.event = v.trim().to_string();
        } else if let Some(v) = line.strip_prefix("data:") {
            cur.data.push_str(v.trim_start());
        } else if line.is_empty() && !cur.event.is_empty() {
            // Only count complete messages (data present or control events).
            out.push(std::mem::replace(
                &mut cur,
                SseMsg {
                    id: None,
                    event: String::new(),
                    data: String::new(),
                },
            ));
        }
    }
    out
}
