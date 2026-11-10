//! HTTP server: versioned JSON API, SSE event stream, embedded static frontend.
//!
//! Read handlers take a consistent model snapshot (`Shared::read`) and report the
//! event `seq` it corresponds to. No handler accepts filesystem paths: files are only
//! addressed by ids the model knows.

mod api;
mod assets;
mod sse;
mod views;

use axum::extract::{Request, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::Router;
use concordance_engine::EngineHandle;
use std::future::Future;
use std::net::SocketAddr;

#[derive(Clone)]
pub struct AppState {
    pub engine: EngineHandle,
    /// Bound to a loopback address: enforce a loopback `Host` header (DNS-rebinding guard).
    pub loopback_only: bool,
}

pub fn router(state: AppState) -> Router {
    let api = Router::new()
        .route("/status", get(api::status))
        .route("/repository", get(api::repository))
        .route("/documents", get(api::documents))
        .route("/documents/:id", get(api::document))
        .route("/claims", get(api::claims))
        .route("/claims/:id", get(api::claim))
        .route("/crates", get(api::crates))
        .route("/symbols", get(api::symbols))
        .route("/symbols/:id", get(api::symbol))
        .route("/findings", get(api::findings))
        .route("/findings/:id", get(api::finding))
        .route("/graph/neighborhood", get(api::neighborhood))
        .route("/history", get(api::history))
        .route("/source/:id", get(api::source))
        .route("/events", get(sse::events))
        .route("/events/recent", get(api::recent_events))
        .route("/scans", post(api::post_scan))
        .route("/analyses", post(api::post_analysis))
        .fallback(api::not_found);
    Router::new()
        .nest("/api/v1", api)
        .fallback(get(assets::serve_asset))
        .layer(middleware::from_fn_with_state(state.clone(), guard))
        .with_state(state)
}

/// Security headers + Host validation for loopback deployments.
async fn guard(State(state): State<AppState>, req: Request, next: Next) -> Response {
    if state.loopback_only {
        let host = req
            .headers()
            .get(header::HOST)
            .and_then(|h| h.to_str().ok())
            .unwrap_or("");
        let name = if let Some(rest) = host.strip_prefix('[') {
            rest.split(']').next().unwrap_or("")
        } else {
            host.split(':').next().unwrap_or("")
        };
        if !matches!(name, "localhost" | "127.0.0.1" | "::1") {
            return (StatusCode::FORBIDDEN, "host not allowed").into_response();
        }
    }
    // Cross-site requests must not trigger scans or spend model budget.
    if req.method() != axum::http::Method::GET && req.method() != axum::http::Method::HEAD {
        let host = req
            .headers()
            .get(header::HOST)
            .and_then(|h| h.to_str().ok())
            .unwrap_or("")
            .to_string();
        if let Some(origin) = req
            .headers()
            .get(header::ORIGIN)
            .and_then(|h| h.to_str().ok())
        {
            let origin_host = origin.split("://").nth(1).unwrap_or("");
            if origin_host != host {
                return (StatusCode::FORBIDDEN, "cross-origin request refused").into_response();
            }
        }
        if req
            .headers()
            .get("sec-fetch-site")
            .and_then(|h| h.to_str().ok())
            .is_some_and(|v| v == "cross-site")
        {
            return (StatusCode::FORBIDDEN, "cross-site request refused").into_response();
        }
    }
    let mut res = next.run(req).await;
    let h = res.headers_mut();
    h.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    h.insert("referrer-policy", HeaderValue::from_static("no-referrer"));
    h.insert(
        "content-security-policy",
        HeaderValue::from_static(
            "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'",
        ),
    );
    res
}

/// Serve until `shutdown` resolves. SSE streams end when the event bus is closed.
pub async fn serve(
    engine: EngineHandle,
    addr: SocketAddr,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> std::io::Result<SocketAddr> {
    let listener = tokio::net::TcpListener::bind(addr).await?;
    let local = listener.local_addr()?;
    let state = AppState {
        engine,
        loopback_only: local.ip().is_loopback(),
    };
    let app = router(state);
    tokio::spawn(async move {
        let _ = axum::serve(listener, app)
            .with_graceful_shutdown(shutdown)
            .await;
    });
    Ok(local)
}
