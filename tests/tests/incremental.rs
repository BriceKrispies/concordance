//! Incremental reconciliation: selective invalidation, cached reuse, stale analyses,
//! and restart recovery.

use concordance_core::event::Event;
use concordance_core::evidence::{Evidence, MethodKind};
use concordance_core::finding::FindingStatus;
use concordance_core::graph::NodeRef;
use concordance_tests::*;
use std::collections::BTreeMap;
use std::sync::Arc;

fn claim_id(m: &concordance_graph::Model, needle: &str) -> concordance_core::ClaimId {
    m.claims
        .values()
        .find(|c| c.text.contains(needle))
        .unwrap_or_else(|| panic!("claim {needle}"))
        .id
        .clone()
}

fn evidence_of(
    m: &concordance_graph::Model,
    c: &concordance_core::ClaimId,
) -> BTreeMap<String, Evidence> {
    m.evidence_for(&NodeRef::Claim(c.clone()))
        .map(|e| (e.id.to_string(), e.clone()))
        .collect()
}

fn edit(path: &std::path::Path, from: &str, to: &str) {
    let s = std::fs::read_to_string(path).unwrap();
    assert!(s.contains(from), "{} contains {from:?}", path.display());
    std::fs::write(path, s.replacen(from, to, 1)).unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn editing_one_symbol_invalidates_only_dependent_evidence() {
    let repo = fixture_copy("incremental");
    let state = temp_dir("incr-state");
    let backend = Arc::new(FixedBackend::insufficient());
    let mut r = start(config_for(&repo, &state, true), Some(backend.clone()), 8192).await;
    r.idle().await;
    let (before, _) = r.engine.shared.read(|m| m.clone());
    let add = claim_id(&before, "math::add()");
    let clamp = claim_id(&before, "clamp_percent()");
    let shout = claim_id(&before, "text::shout()");
    let add_ev = evidence_of(&before, &add);
    let clamp_ev = evidence_of(&before, &clamp);
    assert!(
        add_ev
            .values()
            .any(|e| e.method.kind() == MethodKind::Semantic && e.is_valid()),
        "semantic evidence recorded for math claim"
    );
    assert!(evidence_of(&before, &shout)
        .values()
        .any(|e| e.method.kind() == MethodKind::Semantic && e.is_valid()));
    let calls_before = backend.calls.load(std::sync::atomic::Ordering::SeqCst);
    r.drain();

    // Change only `text::shout`'s body.
    edit(
        &repo.join("src/text.rs"),
        "to_uppercase()",
        "to_uppercase().trim().to_string()",
    );
    r.engine
        .notify_changed(vec![repo.join("src/text.rs")])
        .await;
    r.idle().await;
    let events = r.drain();
    let (after, _) = r.engine.shared.read(|m| m.clone());

    // Unrelated math evidence: byte-for-byte unchanged (same ids, validity, revision).
    assert_eq!(
        evidence_of(&after, &add),
        add_ev,
        "math::add evidence untouched"
    );
    assert_eq!(
        evidence_of(&after, &clamp),
        clamp_ev,
        "clamp_percent evidence untouched"
    );
    // The shout claim's semantic evidence was invalidated and re-established (new run).
    assert!(
        has(
            &events,
            |e| matches!(e, Event::EvidenceInvalidated { subject: NodeRef::Claim(c), .. } if *c == shout)
        ),
        "shout evidence invalidated: {:?}",
        types(&events)
    );
    assert!(!has(
        &events,
        |e| matches!(e, Event::EvidenceInvalidated { subject: NodeRef::Claim(c), .. } if *c == add || *c == clamp)
    ));
    let calls_after = backend.calls.load(std::sync::atomic::Ordering::SeqCst);
    assert_eq!(
        calls_after - calls_before,
        1,
        "exactly one re-analysis (the affected claim)"
    );
    // Only the changed file was re-parsed.
    let indexed: Vec<String> = events
        .iter()
        .filter_map(|e| match &e.event {
            Event::SourceIndexed { path, .. } => Some(path.to_string()),
            _ => None,
        })
        .collect();
    assert_eq!(indexed, vec!["src/text.rs".to_string()]);
    assert!(!has(&events, |e| matches!(
        e,
        Event::DocumentIndexed { .. }
    )));
    r.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn removing_a_referenced_function_turns_its_claims_into_violations_and_back() {
    let repo = fixture_copy("incremental");
    let state = temp_dir("rm-state");
    let mut r = start(config_for(&repo, &state, false), None, 8192).await;
    r.idle().await;
    let (m, _) = r.engine.shared.read(|m| m.clone());
    let wc = claim_id(&m, "word_count()");
    assert_ne!(m.claim_status(&wc), Some(FindingStatus::Violated));
    r.drain();

    edit(
        &repo.join("src/text.rs"),
        "pub fn word_count",
        "pub fn count_words",
    );
    r.engine
        .notify_changed(vec![repo.join("src/text.rs")])
        .await;
    r.idle().await;
    let events = r.drain();
    let (m, _) = r.engine.shared.read(|m| m.clone());
    assert_eq!(m.claim_status(&wc), Some(FindingStatus::Violated));
    assert!(
        has(&events, |e| matches!(
            e,
            Event::RelationshipInvalidated { .. }
        )),
        "relationship invalidated: {:?}",
        types(&events)
    );
    let f = m.claim_finding(&wc).unwrap();
    let first_seen = f.first_seen;

    edit(
        &repo.join("src/text.rs"),
        "pub fn count_words",
        "pub fn word_count",
    );
    r.engine
        .notify_changed(vec![repo.join("src/text.rs")])
        .await;
    r.idle().await;
    let (m, _) = r.engine.shared.read(|m| m.clone());
    let f = m.claim_finding(&wc).unwrap();
    assert_ne!(f.status, FindingStatus::Violated);
    assert_eq!(
        f.id,
        m.claim_finding(&wc).unwrap().id,
        "stable finding id across scans"
    );
    assert_eq!(f.first_seen, first_seen, "history preserved");
    r.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn restart_restores_state_without_reparsing_or_reanalyzing() {
    let repo = fixture_copy("structural-drift");
    let state = temp_dir("restart-state");
    let backend = Arc::new(FixedBackend::insufficient());
    let r = start(config_for(&repo, &state, true), Some(backend.clone()), 8192).await;
    r.idle().await;
    let (m1, _) = r.engine.shared.read(|m| m.clone());
    let calls1 = backend.calls.load(std::sync::atomic::Ordering::SeqCst);
    assert!(calls1 > 0);
    r.stop().await;

    let mut r = start(config_for(&repo, &state, true), Some(backend.clone()), 8192).await;
    r.idle().await;
    let events = r.drain();
    let (m2, _) = r.engine.shared.read(|m| m.clone());
    assert_eq!(m1.claims, m2.claims);
    assert_eq!(m1.symbols, m2.symbols);
    assert_eq!(
        m1.findings.keys().collect::<Vec<_>>(),
        m2.findings.keys().collect::<Vec<_>>()
    );
    for (id, f) in &m1.findings {
        assert_eq!(f.status, m2.findings[id].status, "{}", f.title);
    }
    assert_eq!(m1.evidence, m2.evidence, "evidence restored exactly");
    assert!(
        !has(&events, |e| matches!(
            e,
            Event::SourceIndexed { .. } | Event::DocumentIndexed { .. }
        )),
        "unchanged files not re-parsed: {:?}",
        types(&events)
    );
    assert_eq!(
        backend.calls.load(std::sync::atomic::Ordering::SeqCst),
        calls1,
        "no semantic re-analysis after restart"
    );
    r.stop().await;
}
