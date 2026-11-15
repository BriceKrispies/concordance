use super::*;
use concordance_core::doc::*;
use concordance_core::lang::Languages;

fn index(path: &str, text: &str) -> DocumentIndex {
    let langs = Languages::new(vec![std::sync::Arc::new(concordance_rust::RustPlugin)]);
    index_document(
        &RepoPath::from_relative_str(path).unwrap(),
        text,
        None,
        &langs,
    )
}

const SPEC: &str = "# Rate limiter\r\n\r\nThe `RateLimiter` MUST reject requests over the limit.\r\nA client must never exceed 100 requests per window.\r\n\r\nThis crate is a small library for throttling.\r\n\r\n## Examples\r\n\r\nThe limiter must be created once.\r\n\r\n## API\r\n\r\n```rust\r\npub fn check(&self, key: &str) -> Result<(), Error>;\r\n```\r\n\r\nSee [the guide](../guide.md#start) and `src/lib.rs`.\r\n";

#[test]
fn normative_spec_yields_claims_with_kinds_and_lines() {
    let idx = index("specs/limiter.md", SPEC);
    assert_eq!(idx.document.title.as_deref(), Some("Rate limiter"));
    assert_eq!(idx.document.authority.level, Authority::Normative);
    let c = &idx.claims;
    let first = c
        .iter()
        .find(|c| c.text.starts_with("The `RateLimiter` MUST"))
        .unwrap();
    assert_eq!(
        (first.modality, first.kind),
        (Modality::Must, ClaimKind::Requirement)
    );
    assert_eq!((first.range.start_line, first.range.end_line), (3, 3));
    assert!(first
        .references
        .iter()
        .any(|r| r.strength == ReferenceStrength::Probable));

    let bound = c.iter().find(|c| c.text.contains("never exceed")).unwrap();
    assert_eq!(bound.kind, ClaimKind::Invariant);
    assert_eq!(bound.range.start_line, 4);
    assert_eq!(bound.bounds[0].op, BoundOp::Le);
    assert_eq!(bound.bounds[0].value, 100.0);
    assert_eq!(bound.bounds[0].unit.as_deref(), Some("requests"));
}

#[test]
fn requirement_ids_come_from_bold_openers_and_id_headings_only() {
    let text = "\
# Language

## 6. Comments

**LANG-006.3** A block comment SHALL end at the `*/` that balances its opening `/*`.

The lexer MUST accept `//` comments. PROJ-2147 tracks the history.

## PSS-003

The canonical encoding SHALL be deterministic.

### Detail

Floats MUST be finite.
";
    let d = index("docs/spec.md", text);
    let req = |needle: &str| {
        d.claims
            .iter()
            .find(|c| c.text.contains(needle))
            .unwrap_or_else(|| panic!("no claim containing {needle}"))
            .requirement
            .clone()
    };
    let inline = req("balances its opening").expect("bold opener defines a requirement");
    assert_eq!(inline.id, "LANG-006.3");
    assert_eq!(inline.form, RequirementForm::Inline);
    let c = d
        .claims
        .iter()
        .find(|c| c.text.contains("balances"))
        .unwrap();
    assert!(
        !c.text.contains("LANG-006.3"),
        "the label is stripped from the statement: {}",
        c.text
    );
    assert!(
        req("accept `//` comments").is_none(),
        "an unlabelled paragraph has no requirement"
    );
    let h = req("deterministic").expect("ID heading");
    assert_eq!(
        (h.id.as_str(), h.form),
        ("PSS-003", RequirementForm::Heading)
    );
    assert_eq!(
        req("finite").map(|r| r.id),
        Some("PSS-003".into()),
        "subsections inherit"
    );
}
