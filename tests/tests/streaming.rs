//! HTTP API and SSE streaming: snapshot + cursor consistency, reconnection without
//! gaps or duplicates, resync when history is gone, and request safety.

use concordance_tests::*;
use std::time::Duration;

async fn serve(name: &str, capacity: usize) -> (Running, std::net::SocketAddr, std::path::PathBuf) {
    let repo = fixture_copy(name);
    let state = temp_dir(&format!("{name}-http"));
    let r = start(config_for(&repo, &state, false), None, capacity).await;
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    std::mem::forget(tx); // server lives for the test
    let addr = concordance_server::serve(
        r.engine.clone(),
        "127.0.0.1:0".parse().unwrap(),
        async move {
            let _ = rx.await;
        },
    )
    .await
    .unwrap();
    (r, addr, repo)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reconnecting_client_resumes_from_cursor_without_gaps_or_duplicates() {
    let (r, addr, repo) = serve("incremental", 8192).await;
    r.idle().await;
    let (code, body) = http(addr, "GET", "/api/v1/status", "127.0.0.1", None).await;
    assert_eq!(code, 200);
    let status: serde_json::Value = serde_json::from_str(&body).unwrap();
    let snapshot_seq = status["seq"].as_u64().unwrap();
    assert!(status["counts"]["claims"].as_u64().unwrap() > 0);

    // Connect with the snapshot cursor, then trigger a change and read some events.
    let path = format!("/api/v1/events?cursor={snapshot_seq}");
    let reader = tokio::spawn(async move {
        sse_collect(addr, &path, Duration::from_secs(20), |m| {
            m.iter().filter(|x| x.id.is_some()).count() >= 3
        })
        .await
    });
    tokio::time::sleep(Duration::from_millis(300)).await;
    std::fs::write(
        repo.join("src/text.rs"),
        std::fs::read_to_string(repo.join("src/text.rs")).unwrap() + "\npub fn extra() {}\n",
    )
    .unwrap();
    r.engine
        .notify_changed(vec![repo.join("src/text.rs")])
        .await;
    let first = reader.await.unwrap();
    assert_eq!(first[0].event, "hello");
    let first_ids: Vec<u64> = first.iter().filter_map(|m| m.id).collect();
    assert!(first_ids.len() >= 3, "{first:?}");
    assert_eq!(
        first_ids[0],
        snapshot_seq + 1,
        "stream continues exactly after the snapshot"
    );
    assert!(
        first_ids.windows(2).all(|w| w[1] == w[0] + 1),
        "ordered, gapless: {first_ids:?}"
    );
    r.idle().await;

    // Disconnected. More changes happen meanwhile.
    std::fs::write(
        repo.join("src/math.rs"),
        std::fs::read_to_string(repo.join("src/math.rs")).unwrap() + "\npub fn extra_math() {}\n",
    )
    .unwrap();
    r.engine
        .notify_changed(vec![repo.join("src/math.rs")])
        .await;
    r.idle().await;
    // Reconnect from the last seen id: get everything missed, exactly once.
    let last = *first_ids.last().unwrap();
    assert!(r.bus.last_seq() > last);
    let head = r.bus.last_seq();
    let path = format!("/api/v1/events?cursor={last}");
    let second = sse_collect(addr, &path, Duration::from_secs(10), |m| {
        m.iter().filter_map(|x| x.id).any(|id| id >= head)
    })
    .await;
    let second_ids: Vec<u64> = second
        .iter()
        .filter(|m| m.event != "hello" && m.event != "resync")
        .filter_map(|m| m.id)
        .collect();
    assert!(
        !second.iter().any(|m| m.event == "resync"),
        "replay available, no resync"
    );
    assert_eq!(second_ids.first().copied(), Some(last + 1));
    assert_eq!(second_ids.last().copied(), Some(head));
    assert!(second_ids.windows(2).all(|w| w[1] == w[0] + 1));
    // Payloads are typed envelopes whose seq matches the SSE id.
    for m in second
        .iter()
        .filter(|m| m.id.is_some() && m.event != "resync")
    {
        let v: serde_json::Value = serde_json::from_str(&m.data).unwrap();
        assert_eq!(v["seq"].as_u64(), m.id);
        assert_eq!(v["type"].as_str(), Some(m.event.as_str()));
    }
    r.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cursor_outside_replay_window_gets_resync() {
    // A tiny ring: the initial scan alone overflows it.
    let (r, addr, _) = serve("incremental", 16).await;
    r.idle().await;
    let msgs = sse_collect(
        addr,
        "/api/v1/events?cursor=1",
        Duration::from_secs(5),
        |m| m.iter().any(|x| x.event == "resync"),
    )
    .await;
    let resync = msgs
        .iter()
        .find(|m| m.event == "resync")
        .expect("resync sent");
    let v: serde_json::Value = serde_json::from_str(&resync.data).unwrap();
    assert_eq!(v["latest"].as_u64(), Some(r.bus.last_seq()));
    // A cursor from the future (e.g. from before a restart) also resyncs.
    let msgs = sse_collect(
        addr,
        "/api/v1/events?cursor=999999",
        Duration::from_secs(5),
        |m| m.iter().any(|x| x.event == "resync"),
    )
    .await;
    assert!(msgs.iter().any(|m| m.event == "resync"));
    r.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn api_serves_ui_and_scoped_queries_and_refuses_unsafe_requests() {
    let (r, addr, _) = serve("structural-drift", 8192).await;
    r.idle().await;
    let (code, body) = http(addr, "GET", "/", "127.0.0.1", None).await;
    assert_eq!(code, 200);
    assert!(body.contains("<html") || body.contains("<!doctype") || body.contains("<!DOCTYPE"));

    let (code, body) = http(
        addr,
        "GET",
        "/api/v1/findings?status=violated&limit=5",
        "localhost",
        None,
    )
    .await;
    assert_eq!(code, 200);
    let page: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(page["total"].as_u64().unwrap() >= 3);
    let id = page["items"][0]["id"].as_str().unwrap().to_string();
    let (code, body) = http(
        addr,
        "GET",
        &format!("/api/v1/findings/{id}"),
        "127.0.0.1",
        None,
    )
    .await;
    assert_eq!(code, 200);
    let detail: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(
        !detail["spec_excerpts"].as_array().unwrap().is_empty()
            || !detail["evidence"].as_array().unwrap().is_empty()
    );

    // Neighborhood is scoped, not the whole graph.
    let claim = detail["finding"]["claims"][0]
        .as_str()
        .or(detail["finding"]["documents"][0].as_str())
        .unwrap()
        .to_string();
    let (code, body) = http(
        addr,
        "GET",
        &format!("/api/v1/graph/neighborhood?node={claim}&depth=1"),
        "127.0.0.1",
        None,
    )
    .await;
    assert_eq!(code, 200, "{body}");

    // Safety: DNS-rebinding Host, garbage ids, traversal.
    assert_eq!(
        http(addr, "GET", "/api/v1/status", "evil.example", None)
            .await
            .0,
        403
    );
    assert_eq!(
        http(
            addr,
            "GET",
            "/api/v1/source/..%2F..%2FCargo.toml",
            "127.0.0.1",
            None
        )
        .await
        .0,
        400
    );
    assert_eq!(
        http(addr, "GET", "/../Cargo.toml", "127.0.0.1", None)
            .await
            .0,
        404
    );
    assert_eq!(
        http(
            addr,
            "GET",
            "/api/v1/graph/neighborhood?node=C:/Windows",
            "127.0.0.1",
            None
        )
        .await
        .0,
        400
    );
    // Semantic disabled: analysis request refused, not silently accepted.
    assert_eq!(
        http(
            addr,
            "POST",
            "/api/v1/analyses",
            "127.0.0.1",
            Some(r#"{"all":true}"#)
        )
        .await
        .0,
        409
    );
    r.stop().await;
}
