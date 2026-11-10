//! Server-Sent Events: cursor replay, lag-driven resync, heartbeats.

use crate::AppState;
use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use concordance_events::{Next, Subscription};
use futures_util::stream::{self, Stream};
use std::collections::{HashMap, VecDeque};
use std::convert::Infallible;
use std::time::Duration;

struct StreamState {
    sub: Subscription,
    initial: VecDeque<SseEvent>,
}

pub async fn events(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Sse<impl Stream<Item = Result<SseEvent, Infallible>>> {
    let bus = state.engine.shared.bus.clone();
    let cursor: Option<u64> = q.get("cursor").and_then(|c| c.parse().ok()).or_else(|| {
        headers
            .get("last-event-id")
            .and_then(|h| h.to_str().ok())
            .and_then(|s| s.parse().ok())
    });
    let head = bus.last_seq();
    let mut initial = VecDeque::new();
    let sub = match cursor {
        Some(c) if bus.can_replay_from(c) => {
            initial.push_back(hello(head, Some(c)));
            bus.subscribe(Some(c))
        }
        Some(_) => {
            initial.push_back(hello(head, None));
            initial.push_back(resync(head));
            bus.subscribe(Some(head))
        }
        None => {
            initial.push_back(hello(head, None));
            bus.subscribe(Some(head))
        }
    };
    let s = stream::unfold(StreamState { sub, initial }, |mut st| async move {
        if let Some(ev) = st.initial.pop_front() {
            return Some((Ok(ev), st));
        }
        match st.sub.next().await {
            Next::Event(env) => {
                let data = serde_json::to_string(&*env).unwrap_or_else(|_| "{}".into());
                let ev = SseEvent::default()
                    .id(env.seq.to_string())
                    .event(env.event.type_name())
                    .data(data);
                Some((Ok(ev), st))
            }
            Next::Lagged { latest } => Some((Ok(resync(latest)), st)),
            Next::Closed => None,
        }
    });
    Sse::new(s).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("heartbeat"),
    )
}

fn hello(seq: u64, replayed_from: Option<u64>) -> SseEvent {
    SseEvent::default()
        .event("hello")
        .data(serde_json::json!({ "seq": seq, "replayed_from": replayed_from }).to_string())
}

fn resync(latest: u64) -> SseEvent {
    SseEvent::default()
        .event("resync")
        .id(latest.to_string())
        .data(serde_json::json!({ "latest": latest }).to_string())
}
