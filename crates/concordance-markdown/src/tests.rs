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

fn index_as(path: &str, text: &str, level: Authority, whitepaper: bool) -> DocumentIndex {
    let langs = Languages::new(vec![std::sync::Arc::new(concordance_rust::RustPlugin)]);
    let configured = AuthorityAssessment {
        level,
        source: AuthoritySource::Configured,
        rationale: "test".into(),
    };
    index_document_for(
        &RepoPath::from_relative_str(path).unwrap(),
        text,
        Some(configured),
        &langs,
        whitepaper,
    )
}

const REGISTER_LINE: &str =
    "# Vision\n\n## Goals\n\n- **G-47** Tenant automation cannot reach the platform network.\n";

#[test]
fn register_id_opener_is_no_requirement_in_a_descriptive_document_or_the_white_paper() {
    let d = index_as(
        "specifications/00-vision/vision.md",
        REGISTER_LINE,
        Authority::Descriptive,
        false,
    );
    assert!(
        d.claims.iter().all(|c| c.requirement.is_none()),
        "a descriptive document defines no requirement by an ID opener"
    );
    let claim = d
        .claims
        .iter()
        .find(|c| c.text.contains("Tenant automation cannot reach"))
        .expect("the line stays an ordinary prose claim");
    assert!(
        claim.text.contains("G-47"),
        "the ID is kept as text, not stripped as a label: {}",
        claim.text
    );

    // The same line in the file named by `[lint] whitepaper` is never a requirement either,
    // even when the file is configured normative.
    let w = index_as(
        "docs/whitepaper.md",
        REGISTER_LINE,
        Authority::Normative,
        true,
    );
    assert!(w.claims.iter().all(|c| c.requirement.is_none()));
}

#[test]
fn register_id_opener_defines_a_requirement_in_a_normative_document() {
    let d = index_as(
        "specifications/spec.md",
        REGISTER_LINE,
        Authority::Normative,
        false,
    );
    let claim = d
        .claims
        .iter()
        .find(|c| c.text.contains("Tenant automation cannot reach"))
        .expect("claim");
    let req = claim
        .requirement
        .as_ref()
        .expect("normative opener defines G-47");
    assert_eq!(
        (req.id.as_str(), req.form),
        ("G-47", RequirementForm::Inline)
    );
    assert!(
        !claim.text.contains("G-47"),
        "label stripped: {}",
        claim.text
    );
}

#[test]
fn failure_constraints_need_a_named_or_whole_code_scope() {
    let d = index(
        "docs/spec.md",
        "# Spec\n\nThe client never throws.\n\nThe decoder MUST NOT panic.\n\n\
         Unsafe code is forbidden in the parser.\n\nThis crate must not use unsafe code.\n\n\
         Unsafe code is forbidden.\n\n`Decoder` MUST NOT panic.\n",
    );
    let form = |t: &str| {
        d.claims
            .iter()
            .find(|c| c.text.starts_with(t))
            .unwrap_or_else(|| panic!("no claim {t}"))
            .constraint
            .clone()
    };
    assert_eq!(
        form("The client never throws"),
        None,
        "unresolvable part, not the codebase"
    );
    assert_eq!(form("The decoder MUST NOT"), None);
    assert_eq!(form("Unsafe code is forbidden in the parser"), None);
    assert_eq!(
        form("This crate must not"),
        Some(ConstraintForm::NoUnsafe { scope: vec![] })
    );
    assert_eq!(
        form("Unsafe code is forbidden."),
        Some(ConstraintForm::NoUnsafe { scope: vec![] })
    );
    assert_eq!(
        form("`Decoder` MUST NOT"),
        Some(ConstraintForm::NoPanic {
            scope: vec!["Decoder".into()]
        })
    );
}

#[test]
fn table_labels_and_quoted_cells_do_not_drive_modality_or_vagueness() {
    let d = index(
        "docs/spec.md",
        "# Spec\n\n\
         | Role | May do | Where |\n|---|---|---|\n\
         | app | It reads rows. It cannot connect to the control DB. | its own DB |\n\n\
         | Field | Type | Required | Meaning |\n|---|---|---|---|\n\
         | version | int >= 1 | no | The payload version |\n\n\
         | Key | Summary | Business need |\n|---|---|---|\n\
         | T-1 | Some images are missing | Each book must show a cover. |\n\
         | T-2 | Show stats | The list must load quickly. |\n",
    );
    let claim = |t: &str| d.claims.iter().find(|c| c.text.contains(t));
    let role = claim("cannot connect").expect("row claim");
    assert_eq!(role.modality, Modality::MustNot);
    assert!(
        role.ambiguity.is_empty(),
        "the `May do` label is not a weak qualifier: {:?}",
        role.ambiguity
    );
    assert!(
        claim("payload version").is_none(),
        "a `Required: no` cell is not an obligation"
    );
    let quoted = claim("Some images").expect("row claim");
    assert!(
        quoted.ambiguity.is_empty(),
        "the Summary cell quotes a ticket title"
    );
    let own = claim("load quickly").expect("row claim");
    assert!(
        own.ambiguity.iter().any(|a| a.detail.contains("quickly")),
        "the author's own column is still graded"
    );
}

#[test]
fn bounds_do_not_take_a_unit_from_the_next_cell() {
    let d = index(
        "docs/spec.md",
        "# Spec\n\n| Field | Type | Need |\n|---|---|---|\n| n | int >= 1 | must be set |\n",
    );
    let b = &d.claims[0].bounds[0];
    assert_eq!((b.op, b.value, b.unit.as_deref()), (BoundOp::Ge, 1.0, None));
}

#[test]
fn leading_pronoun_after_a_sentence_has_an_antecedent() {
    let d = index(
        "docs/spec.md",
        "# Spec\n\nThe request must have an empty body. It must not contain an identity header.\n\n\
         It must be banned.\n\nIt is not known if the ranker must apply.\n",
    );
    let notes = |t: &str| {
        d.claims
            .iter()
            .find(|c| c.text == t)
            .map(|c| c.ambiguity.clone())
            .unwrap()
    };
    assert!(notes("It must not contain an identity header.").is_empty());
    assert!(
        notes("It is not known if the ranker must apply.").is_empty(),
        "expletive `it` has no antecedent to miss"
    );
    assert!(notes("It must be banned.")
        .iter()
        .any(|a| a.kind == AmbiguityKind::UnresolvedSubject));
}

#[test]
fn not_required_is_not_an_obligation() {
    let d = index(
        "docs/spec.md",
        "# Spec\n\nThis step is not required by the process. Comments may hold later decisions.\n\n\
         A token is required for each call.\n",
    );
    let m = |t: &str| {
        d.claims
            .iter()
            .find(|c| c.text.starts_with(t))
            .map(|c| c.modality)
    };
    assert_ne!(m("This step is not required"), Some(Modality::Must));
    assert_eq!(m("A token is required"), Some(Modality::Must));
}
