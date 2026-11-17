//! An Exegra JSONC specification through the engine: discovery, requirement ids defined
//! by rows and authored claims, `Verifies:` links from TypeScript tests, locations in the
//! `.jsonc`, and incremental re-derivation.

use concordance_core::finding::FindingSubject;
use concordance_core::graph::{EdgeKind, NodeRef};
use concordance_graph::Model;
use concordance_tests::*;

const SPEC: &str = "docs/spec.jsonc";

fn claim_of<'a>(m: &'a Model, req: &str) -> &'a concordance_core::doc::Claim {
    m.claims
        .values()
        .find(|c| c.requirement.as_ref().is_some_and(|r| r.id == req))
        .unwrap_or_else(|| panic!("no claim defines {req}"))
}

fn verified_by(m: &Model, req: &str) -> Vec<String> {
    let c = claim_of(m, req);
    m.edges_from(&NodeRef::Claim(c.id.clone()))
        .filter(|e| e.kind == EdgeKind::VerifiedBy)
        .filter_map(|e| match e.to.clone() {
            Some(NodeRef::Symbol(s)) => Some(m.symbols[&s].qualified_name()),
            _ => None,
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn exegra_spec_is_indexed_and_linked_to_typescript_tests() {
    let repo = fixture_copy("exegra");
    let state = temp_dir("exegra-state");
    let r = start(config_for(&repo, &state, false), None, 8192).await;
    r.idle().await;
    let m = r.engine.shared.read(|m| m.clone()).0;

    // The JSONC is the document; other JSON under the spec root is ignored.
    let paths: Vec<_> = m.documents.values().map(|d| d.path.as_str()).collect();
    assert_eq!(paths, vec![SPEC]);
    let doc = m.documents.values().next().unwrap();
    let lines = std::fs::read_to_string(repo.join(SPEC))
        .unwrap()
        .lines()
        .count() as u32;
    assert_eq!(doc.line_count, lines);
    for c in m.claims.values() {
        assert_eq!(c.range.path.as_str(), SPEC);
        assert!(
            c.range.start_line >= 1 && c.range.end_line <= lines,
            "{c:?}"
        );
    }

    // Rows and authored claims define requirements that TypeScript tests verify.
    assert!(verified_by(&m, "TEN-1")
        .iter()
        .any(|n| n.ends_with("reads the tenant header")));
    assert!(verified_by(&m, "FR-ISO-1")
        .iter()
        .any(|n| n.ends_with("rejects a request without a tenant")));
    assert!(verified_by(&m, "TEN-2").is_empty());
    assert!(
        !m.findings
            .values()
            .any(|f| f.is_open() && matches!(&f.subject, FindingSubject::Verification { .. })),
        "every Verifies: names a defined requirement"
    );

    // An edit elsewhere in the file re-derives the document and keeps claim identity.
    let iso = claim_of(&m, "FR-ISO-1").id.clone();
    let ten2 = claim_of(&m, "TEN-2").id.clone();
    let iso_line = claim_of(&m, "FR-ISO-1").range.start_line;
    let path = repo.join(SPEC);
    let text = std::fs::read_to_string(&path).unwrap();
    let edited = text
        .replacen(
            "Each organization has its own database.",
            "Each organization has its own PostgreSQL database.",
            1,
        )
        .replacen(
            "  \"tables\": [",
            "  \"notes\": [\"added\"],\n  \"tables\": [",
            1,
        );
    std::fs::write(&path, edited).unwrap();
    r.engine.notify_changed(vec![path]).await;
    r.idle().await;
    let m = r.engine.shared.read(|m| m.clone()).0;
    assert!(claim_of(&m, "TEN-1").text.contains("PostgreSQL"));
    assert_eq!(claim_of(&m, "FR-ISO-1").id, iso);
    assert_eq!(claim_of(&m, "FR-ISO-1").range.start_line, iso_line + 1);
    assert_eq!(claim_of(&m, "TEN-2").id, ten2);
    assert!(!verified_by(&m, "TEN-1").is_empty());
    r.stop().await;
}
