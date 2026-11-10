use concordance_core::code::{Symbol, SymbolKind, Visibility};
use concordance_core::doc::{
    Authority, AuthorityAssessment, AuthoritySource, BlockKind, Claim, ClaimKind, Document,
    Modality,
};
use concordance_core::event::{Envelope, Event};
use concordance_core::evidence::{
    Evidence, Method, MethodKind, Stance, Strength, StructuralCheck, Validity,
};
use concordance_core::finding::{
    EvidenceQuality, Finding, FindingCategory, FindingStatus, FindingSubject, Lifecycle, Severity,
};
use concordance_core::graph::NodeRef;
use concordance_core::{
    ClaimId, ContentHash, DocumentId, EvidenceId, FileId, RepoPath, Revision, SectionId,
    SourceRange, SymbolId, Timestamp,
};
use concordance_storage::{Delta, Store, StoreError};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// A unique scratch directory under the system temp dir, removed on drop.
struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "concordance-storage-{label}-{}-{nanos}",
            std::process::id()
        ));
        TempDir(dir)
    }

    fn db(&self) -> PathBuf {
        self.0.join("nested").join("state.sqlite")
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn doc_path() -> RepoPath {
    RepoPath::from_relative_str("docs/spec.md").unwrap()
}

fn src_path() -> RepoPath {
    RepoPath::from_relative_str("src/lib.rs").unwrap()
}

fn document_id() -> DocumentId {
    DocumentId::from_key(&["docs/spec.md"])
}

fn claim_id() -> ClaimId {
    ClaimId::from_key(&["docs/spec.md", "Intro", "must be fast"])
}

fn symbol_id() -> SymbolId {
    SymbolId::from_key(&["fn", "demo::run"])
}

fn sample_document() -> Document {
    Document {
        id: document_id(),
        path: doc_path(),
        title: Some("Spec".into()),
        authority: AuthorityAssessment {
            level: Authority::Normative,
            source: AuthoritySource::Configured,
            rationale: "matched configured glob `docs/**`".into(),
        },
        content_hash: ContentHash::of_str("doc body"),
        line_count: 12,
    }
}

fn sample_claim() -> Claim {
    Claim {
        id: claim_id(),
        document: document_id(),
        section: SectionId::from_key(&["docs/spec.md", "Intro"]),
        heading_path: vec!["Spec".into(), "Intro".into()],
        text: "`run` must be fast".into(),
        kind: ClaimKind::Requirement,
        modality: Modality::Must,
        authority: Authority::Normative,
        block: BlockKind::Paragraph,
        range: SourceRange::lines(doc_path(), 3, 3),
        content_hash: ContentHash::of_str("run must be fast"),
        references: vec![],
        bounds: vec![],
        ambiguity: vec![],
        documented_signature: None,
        signature_language: None,
        constraint: None,
        requirement: None,
    }
}

fn sample_symbol() -> Symbol {
    Symbol {
        id: symbol_id(),
        file: FileId::from_key(&["src/lib.rs"]),
        kind: SymbolKind::Function,
        name: "run".into(),
        path: vec!["demo".into(), "run".into()],
        parent: None,
        visibility: Visibility::Public,
        is_public_api: true,
        signature: Some("pub fn run() -> u32".into()),
        doc: None,
        attributes: vec![],
        cfg: vec![],
        range: SourceRange::lines(src_path(), 1, 3),
        content_hash: ContentHash::of_str("fn run() -> u32 { 1 }"),
        signature_hash: Some(ContentHash::of_str("pub fn run() -> u32")),
        impl_info: None,
        returns: None,
        facts: vec![],
        calls: vec![],
        is_test: false,
        verifies: vec![],
    }
}

fn sample_evidence() -> Evidence {
    Evidence {
        id: EvidenceId::from_key(&["evidence", "claim-symbol"]),
        subject: NodeRef::Claim(claim_id()),
        stance: Stance::Inconclusive,
        strength: Strength::Moderate,
        method: Method::Structural {
            check: StructuralCheck::SymbolResolution,
        },
        summary: "claim names `run`, which exists".into(),
        citations: vec![],
        dependencies: vec![],
        validity: Validity::Valid,
        revision: Revision(1),
        created_at: Timestamp(1_000),
    }
}

fn sample_finding() -> Finding {
    let subject = FindingSubject::Claim { claim: claim_id() };
    Finding {
        id: subject.finding_id(),
        subject,
        status: FindingStatus::Unverified,
        category: FindingCategory::MissingEvidence,
        severity: Severity::Medium,
        title: "No evidence for `run` being fast".into(),
        explanation: "Nothing establishes the performance claim.".into(),
        claims: vec![claim_id()],
        symbols: vec![symbol_id()],
        documents: vec![document_id()],
        locations: vec![SourceRange::lines(doc_path(), 3, 3)],
        supporting: vec![],
        contradicting: vec![],
        confidence: 0.5,
        quality: EvidenceQuality::Weak,
        methods: vec![MethodKind::Structural],
        limitations: vec!["callees not analyzed".into()],
        dependencies: vec![NodeRef::Symbol(symbol_id())],
        proposals: vec![],
        revision: Revision(1),
        introduced_in: Revision(1),
        introduced_by: vec![],
        first_seen: Timestamp(1_000),
        last_updated: Timestamp(1_000),
        lifecycle: Lifecycle::Open,
        crate_name: Some("demo".into()),
        module: None,
    }
}

fn sample_envelope() -> Envelope {
    Envelope {
        seq: 1,
        revision: Revision(1),
        at: Timestamp(1_000),
        event: Event::ClaimRemoved { claim: claim_id() },
    }
}

fn sample_delta() -> Delta {
    Delta {
        revision: Revision(1),
        last_seq: 1,
        upsert_documents: vec![sample_document()],
        upsert_claims: vec![sample_claim()],
        upsert_symbols: vec![sample_symbol()],
        upsert_findings: vec![sample_finding()],
        upsert_evidence: vec![sample_evidence()],
        events: vec![sample_envelope()],
        ..Default::default()
    }
}

#[test]
fn committed_delta_loads_back_equal() {
    let dir = TempDir::new("roundtrip");
    let mut store = Store::open(&dir.db()).unwrap();
    let delta = sample_delta();
    store.commit(&delta).unwrap();

    let snap = store.load().unwrap();
    assert_eq!(snap.revision, Revision(1));
    assert_eq!(snap.last_seq, 1);
    assert_eq!(snap.documents, delta.upsert_documents);
    assert_eq!(snap.claims, delta.upsert_claims);
    assert_eq!(snap.symbols, delta.upsert_symbols);
    assert_eq!(snap.findings, delta.upsert_findings);
    assert_eq!(snap.evidence, delta.upsert_evidence);

    let events = store.events_after(0, 10).unwrap();
    assert_eq!(
        serde_json::to_value(&events).unwrap(),
        serde_json::to_value(&delta.events).unwrap()
    );
}

#[test]
fn delete_removes_row_and_keeps_findings() {
    let dir = TempDir::new("delete");
    let mut store = Store::open(&dir.db()).unwrap();
    store.commit(&sample_delta()).unwrap();

    let second = Delta {
        revision: Revision(2),
        last_seq: 2,
        delete_claims: vec![claim_id()],
        ..Default::default()
    };
    store.commit(&second).unwrap();

    let snap = store.load().unwrap();
    assert!(snap.claims.is_empty());
    assert_eq!(snap.findings, vec![sample_finding()]);
}

#[test]
fn newer_schema_version_is_refused() {
    let dir = TempDir::new("schema");
    let path = dir.db();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    {
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.pragma_update(None, "user_version", 999).unwrap();
    }
    let result = Store::open(&path);
    assert!(matches!(
        result,
        Err(StoreError::SchemaTooNew { found: 999, .. })
    ));
}
