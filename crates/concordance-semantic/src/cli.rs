//! The production backend: Claude Code CLI in print mode.

use crate::model::*;
use crate::validate::truncate_chars;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, Command};

const STDOUT_CAP: u64 = 4 * 1024 * 1024;
const STDERR_CAP: u64 = 16 * 1024;
const PROBE_TIMEOUT: Duration = Duration::from_secs(20);
const NOT_FOUND: &str = "Claude Code CLI not found on PATH (configure semantic.claude_path)";

/// Production backend: the locally installed Claude Code CLI in print mode.
pub struct ClaudeCli {
    pub program: String,
    pub timeout: Duration,
    /// Neutral working directory (never the analyzed repository), so repository
    /// CLAUDE.md files and settings are not loaded.
    pub workdir: std::path::PathBuf,
}

impl ClaudeCli {
    pub fn new(program: impl Into<String>, timeout: Duration, workdir: std::path::PathBuf) -> Self {
        ClaudeCli {
            program: program.into(),
            timeout,
            workdir,
        }
    }

    /// Spawn the CLI. On Windows, `claude` is often a `.cmd` shim, which a plain spawn
    /// of "claude" cannot find: retry as `<program>.cmd`, then via `cmd /C`.
    fn spawn(&self, args: &[String], piped_stdin: bool) -> std::io::Result<Child> {
        let _ = std::fs::create_dir_all(&self.workdir);
        let make = |mut c: Command| {
            c.args(args)
                .current_dir(&self.workdir)
                .stdin(if piped_stdin {
                    Stdio::piped()
                } else {
                    Stdio::null()
                })
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .kill_on_drop(true);
            c.spawn()
        };
        match make(Command::new(&self.program)) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && cfg!(windows) => {
                let cmd_name = format!("{}.cmd", self.program);
                match make(Command::new(&cmd_name)) {
                    Err(e2) if e2.kind() == std::io::ErrorKind::NotFound => {
                        let mut c = Command::new("cmd");
                        c.arg("/C").arg(&self.program);
                        make(c)
                    }
                    other => other,
                }
            }
            other => other,
        }
    }

    /// `claude --version`; `Err` with a human-readable reason when unusable.
    pub async fn probe(&self) -> Result<String, String> {
        let child = match self.spawn(&["--version".to_string()], false) {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(NOT_FOUND.to_string()),
            Err(e) => return Err(format!("could not start `{}`: {e}", self.program)),
        };
        match tokio::time::timeout(PROBE_TIMEOUT, child.wait_with_output()).await {
            Err(_) => Err("`claude --version` timed out".to_string()),
            Ok(Err(e)) => Err(format!("`claude --version` failed: {e}")),
            Ok(Ok(out)) => {
                let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if out.status.success() && !stdout.is_empty() {
                    Ok(stdout)
                } else {
                    let err = String::from_utf8_lossy(&out.stderr);
                    Err(format!(
                        "`claude --version` exited with {}: {}",
                        out.status,
                        truncate_chars(err.trim(), 500)
                    ))
                }
            }
        }
    }

    fn args(req: &BackendRequest<'_>) -> Result<Vec<String>, AnalysisError> {
        let schema = serde_json::to_string(req.schema)
            .map_err(|e| AnalysisError::Invocation(format!("schema serialization: {e}")))?;
        Ok(vec![
            "-p".into(),
            "--output-format".into(),
            "json".into(),
            "--json-schema".into(),
            schema,
            "--tools".into(),
            String::new(),
            "--no-session-persistence".into(),
            "--model".into(),
            req.model.to_string(),
            "--setting-sources".into(),
            String::new(),
            "--system-prompt".into(),
            req.system.to_string(),
        ])
    }

    async fn run(
        &self,
        req: BackendRequest<'_>,
        mut cancel: CancelRx,
    ) -> Result<BackendResponse, AnalysisError> {
        let args = Self::args(&req)?;
        let mut child = match self.spawn(&args, true) {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(AnalysisError::Unavailable(NOT_FOUND.to_string()))
            }
            Err(e) => {
                return Err(AnalysisError::Invocation(format!(
                    "could not start `{}`: {e}",
                    self.program
                )))
            }
        };
        let mut stdin = child.stdin.take();
        let mut stdout = child.stdout.take();
        let mut stderr = child.stderr.take();
        let prompt = req.prompt.as_bytes().to_vec();

        // `child` moves into this future, so dropping it (timeout/cancel) kills the process.
        let work = async move {
            let write = async {
                if let Some(mut s) = stdin.take() {
                    let _ = s.write_all(&prompt).await;
                    let _ = s.shutdown().await;
                }
            };
            let read_out = async {
                let mut buf = Vec::new();
                if let Some(o) = stdout.as_mut() {
                    let _ = o.take(STDOUT_CAP + 1).read_to_end(&mut buf).await;
                }
                buf
            };
            let read_err = async {
                let mut buf = Vec::new();
                if let Some(e) = stderr.as_mut() {
                    let _ = e.take(STDERR_CAP).read_to_end(&mut buf).await;
                }
                buf
            };
            let (_, out, err) = tokio::join!(write, read_out, read_err);
            let status = child.wait().await;
            (out, err, status)
        };
        let cancelled = async {
            loop {
                if *cancel.borrow() {
                    return;
                }
                if cancel.changed().await.is_err() {
                    std::future::pending::<()>().await;
                }
            }
        };

        let (out, err, status) = tokio::select! {
            r = tokio::time::timeout(self.timeout, work) => match r {
                Ok(v) => v,
                Err(_) => return Err(AnalysisError::Timeout),
            },
            _ = cancelled => return Err(AnalysisError::Cancelled),
        };

        if out.len() as u64 > STDOUT_CAP {
            return Err(AnalysisError::Invocation("CLI output exceeded 4 MB".into()));
        }
        let stdout_text = String::from_utf8_lossy(&out).to_string();
        let stderr_text = truncate_chars(String::from_utf8_lossy(&err).trim(), 500);
        let status =
            status.map_err(|e| AnalysisError::Invocation(format!("waiting for CLI: {e}")))?;
        if !status.success() {
            let detail = parse_cli_output(&stdout_text)
                .err()
                .map(|e| e.to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_default();
            return Err(AnalysisError::Invocation(format!(
                "CLI exited with {status}: {}",
                if stderr_text.is_empty() {
                    truncate_chars(&detail, 500)
                } else {
                    stderr_text
                }
            )));
        }
        parse_cli_output(&stdout_text)
    }
}

impl Backend for ClaudeCli {
    fn name(&self) -> &str {
        "claude-cli"
    }

    fn invoke<'a>(
        &'a self,
        req: BackendRequest<'a>,
        cancel: CancelRx,
    ) -> BoxFuture<'a, Result<BackendResponse, AnalysisError>> {
        Box::pin(self.run(req, cancel))
    }
}

/// Parse the CLI's single JSON object.
pub fn parse_cli_output(text: &str) -> Result<BackendResponse, AnalysisError> {
    let v: serde_json::Value = serde_json::from_str(text.trim())
        .map_err(|e| AnalysisError::InvalidResponse(format!("CLI output is not JSON: {e}")))?;
    if v.get("is_error").and_then(|b| b.as_bool()).unwrap_or(false) {
        let msg = v
            .get("result")
            .and_then(|r| r.as_str())
            .or_else(|| v.get("subtype").and_then(|s| s.as_str()))
            .unwrap_or("unknown error");
        return Err(AnalysisError::Invocation(truncate_chars(msg, 500)));
    }
    let structured = match v.get("structured_output") {
        Some(s) if !s.is_null() => s.clone(),
        _ => {
            let subtype = v
                .get("subtype")
                .and_then(|s| s.as_str())
                .unwrap_or("unknown");
            return Err(AnalysisError::InvalidResponse(format!(
                "CLI response has no structured_output (subtype: {subtype})"
            )));
        }
    };
    Ok(BackendResponse {
        structured,
        reported_model: v
            .get("modelUsage")
            .and_then(|m| m.as_object())
            .and_then(|m| m.keys().next().cloned()),
        duration_ms: v.get("duration_ms").and_then(|d| d.as_u64()).unwrap_or(0),
        cost_usd: v.get("total_cost_usd").and_then(|d| d.as_f64()),
        input_tokens: v.pointer("/usage/input_tokens").and_then(|d| d.as_u64()),
        output_tokens: v.pointer("/usage/output_tokens").and_then(|d| d.as_u64()),
        session_id: v
            .get("session_id")
            .and_then(|s| s.as_str())
            .map(str::to_string),
    })
}
