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
fn explanatory_text_makes_no_claims_and_examples_are_informative() {
    let idx = index("specs/limiter.md", SPEC);
    assert!(!idx.claims.iter().any(|c| c.text.contains("small library")));
    let ex = idx
        .claims
        .iter()
        .find(|c| c.text.contains("created once"))
        .unwrap();
    assert_eq!(ex.authority, Authority::Informative);
    let sec = idx
        .sections
        .iter()
        .find(|s| s.heading == "Examples")
        .unwrap();
    assert_eq!(sec.role, SectionRole::Example);
    assert_eq!(sec.anchor, "examples");
}

#[test]
fn rust_signature_becomes_api_contract() {
    let idx = index("specs/limiter.md", SPEC);
    let api = idx
        .claims
        .iter()
        .find(|c| c.block == BlockKind::CodeBlock)
        .unwrap();
    assert_eq!(api.kind, ClaimKind::ApiContract);
    assert_eq!(api.modality, Modality::Declarative);
    assert_eq!(
        api.documented_signature.as_deref(),
        Some("pub fn check(&self, key: &str) -> Result<(), Error>;")
    );
    assert_eq!(api.range.start_line, 15);
    assert!(
        matches!(&api.references[0].target, ReferenceTarget::Symbol { path } if path == &["check"])
    );
}

#[test]
fn prose_links_and_code_spans_are_references() {
    let idx = index("specs/limiter.md", SPEC);
    let targets: Vec<&ReferenceTarget> = idx
        .prose_references
        .iter()
        .map(|p| &p.reference.target)
        .collect();
    assert!(targets.iter().any(|t| matches!(t,
        ReferenceTarget::Section { document, anchor } if document.as_str() == "guide.md" && anchor.as_deref() == Some("start"))));
    assert!(targets
        .iter()
        .any(|t| matches!(t, ReferenceTarget::File { path, .. } if path.as_str() == "src/lib.rs")));
}

#[test]
fn empty_and_odd_inputs_do_not_panic() {
    for text in [
        "",
        "\n",
        "---\nconcordance: informative\n---",
        "# \u{e9}\u{4e2d}\n\n| a |\n|---|\n| must `x::y` |\n",
        "`",
    ] {
        let idx = index("a.md", text);
        assert!(!idx.sections.is_empty());
    }
    assert_eq!(
        index("docs/archive/a.md", "plain text")
            .document
            .authority
            .level,
        Authority::Informative
    );
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

#[test]
fn requirement_id_shape() {
    use concordance_core::doc::is_requirement_id as ok;
    for good in ["PSS-003", "LANG-006.3", "UI-12", "WEB-SEC-004"] {
        assert!(ok(good), "{good}");
    }
    for bad in [
        "pss-003",
        "P-1",
        "PSS",
        "PSS-",
        "PSS-abc",
        "§6",
        "R4307",
        "PSS-003.x",
    ] {
        assert!(!ok(bad), "{bad}");
    }
}

#[test]
fn dotted_type_paths_and_throw_constraints_are_language_neutral() {
    use crate::refs::classify_code;
    let sym = |c: &str| match classify_code(c) {
        Some((ReferenceTarget::Symbol { path }, _)) => path,
        other => panic!("{c}: {other:?}"),
    };
    assert_eq!(sym("Mixer.setVolume()"), vec!["Mixer", "setVolume"]);
    assert_eq!(
        sym("Codec.Header.parse(bytes)"),
        vec!["Codec", "Header", "parse"]
    );
    assert_eq!(
        sym("mixer.setVolume(3)"),
        vec!["setVolume"],
        "a variable receiver is not a path"
    );
    assert_eq!(sym("Mixer::set_volume()"), vec!["Mixer", "set_volume"]);
    assert!(
        classify_code("Math.min(v, 100)").is_none(),
        "built-in members are not project symbols"
    );
    assert!(classify_code("RangeError").is_none());

    let d = index(
        "docs/spec.md",
        "# Spec\n\n`parse()` MUST NOT throw.\n\nThe decoder MUST NOT panic.\n",
    );
    let forms: Vec<_> = d.claims.iter().map(|c| c.constraint.clone()).collect();
    assert!(
        forms
            .iter()
            .all(|f| matches!(f, Some(ConstraintForm::NoPanic { .. }))),
        "throw and panic map to the same failure-site check: {forms:?}"
    );
}
