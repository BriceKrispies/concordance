use super::*;
use concordance_core::doc::{Authority, ClaimKind, Modality, RequirementForm};

const SPEC: &str = include_str!("../../../fixtures/exegra/docs/spec.jsonc");

const RENDERED: &str = "# Tenancy

The service MUST reject a request without a tenant.

## Rules

| ID | Rule | State |
|---|---|---|
| TEN-1 | Each organization has its own database. | Built. |
|TEN-2|  A request MUST carry the `Tenant` header.  | Partly built. |

The \"resolver\" `resolveTenant()` MUST NOT return `null`.

## Status of \"tenancy\"

| Item | Value |
|---|---|
| Status | Draft |
| Owner | |
";

/// The fixture with line endings as committed (git may check it out with CRLF).
fn spec() -> String {
    SPEC.replace("\r\n", "\n")
}

fn index(text: &str) -> DocumentIndex {
    index_exegra(
        &RepoPath::from_relative_str("docs/spec.jsonc").unwrap(),
        text,
        None,
        &Languages::new(vec![]),
        &Options::default(),
    )
    .unwrap()
}

/// 1-based line of the first occurrence of `needle` in the fixture.
fn line_of(text: &str, needle: &str) -> u32 {
    let at = text.find(needle).unwrap_or_else(|| panic!("{needle:?}"));
    text[..at].matches('\n').count() as u32 + 1
}

#[test]
fn renders_sections_as_exegra_does() {
    assert_eq!(render_markdown(&spec()).unwrap(), RENDERED);
}

#[test]
fn only_exegra_json_is_recognised() {
    assert!(is_exegra(&spec()));
    assert!(!is_exegra("{ \"name\": \"pkg\", \"exegraLike\": 1 }"));
    assert!(index_exegra(
        &RepoPath::from_relative_str("a.json").unwrap(),
        "{\"name\": 1}",
        None,
        &Languages::new(vec![]),
        &Options::default()
    )
    .is_err());
}

#[test]
fn converted_claims_point_at_their_entities() {
    let text = spec();
    let idx = index(&text);
    assert_eq!(idx.document.authority.level, Authority::Normative);
    assert_eq!(idx.document.title.as_deref(), Some("Tenancy"));
    assert_eq!(idx.document.line_count, text.lines().count() as u32);

    let claim = |needle: &str| {
        idx.claims
            .iter()
            .find(|c| c.text.contains(needle))
            .unwrap_or_else(|| panic!("no claim {needle:?}: {:#?}", idx.claims))
    };
    // Prose: the section entity's line.
    let prose = claim("reject a request without a tenant");
    let sec_line = line_of(&text, "\"id\":\"sec-tenancy\"");
    assert_eq!(
        (prose.range.start_line, prose.range.end_line),
        (sec_line, sec_line)
    );

    // A reference keeps its column inside the body string, past the escaped quotes.
    let r = claim("resolveTenant()")
        .references
        .iter()
        .find(|r| r.text == "resolveTenant()")
        .and_then(|r| r.range.clone())
        .unwrap();
    let line = text.lines().nth(r.start_line as usize - 1).unwrap();
    let at: String = line
        .chars()
        .skip(r.start_col as usize - 1)
        .take(15)
        .collect();
    assert_eq!(at, "`resolveTenant(");

    // Rows: the row entity's line, and the row defines its requirement.
    for (id, needle) in [
        ("TEN-1", "its own database"),
        ("TEN-2", "carry the `Tenant` header"),
    ] {
        let c = claim(needle);
        let row = line_of(&text, &format!("\"ref\":\"{id}\""));
        assert_eq!((c.range.start_line, c.range.end_line), (row, row), "{id}");
        let req = c.requirement.as_ref().unwrap();
        assert_eq!((req.id.as_str(), req.form), (id, RequirementForm::Inline));
    }
    assert_eq!(claim("its own database").modality, Modality::Declarative);

    // Every location is inside the JSONC.
    let n = idx.document.line_count;
    for c in &idx.claims {
        assert!(c.range.path.as_str() == "docs/spec.jsonc" && c.range.end_line <= n);
    }
    for s in &idx.sections {
        assert!(s.range.start_line >= 1 && s.range.end_line <= n, "{s:?}");
    }
}

#[test]
fn authored_claims_define_requirements_and_seams_are_informative() {
    let text = spec();
    let idx = index(&text);
    let by_req = |id: &str| {
        idx.claims
            .iter()
            .find(|c| c.requirement.as_ref().is_some_and(|r| r.id == id))
            .unwrap_or_else(|| panic!("{id}"))
    };
    let iso = by_req("FR-ISO-1");
    assert_eq!(iso.kind, ClaimKind::Invariant);
    assert_eq!(iso.text, "No request reads data of another organization.");
    assert_eq!(iso.range.start_line, line_of(&text, "No request reads"));
    let agt = by_req("FR-AGT-1");
    assert_eq!(agt.kind, ClaimKind::ArchitecturalConstraint);
    assert_eq!(agt.authority, Authority::Normative);

    let seam = idx
        .claims
        .iter()
        .find(|c| c.text.contains("stay reversible"))
        .unwrap();
    assert_eq!(seam.authority, Authority::Informative);
    assert!(seam.requirement.is_none());
    assert!(!seam.is_obligation());
    assert!(idx.sections.iter().any(|s| s.heading == "Seam S-01"));
    assert!(!idx
        .claims
        .iter()
        .any(|c| c.requirement.as_ref().is_some_and(|r| r.id == "S-01")));
}

#[test]
fn requirement_kinds_are_configurable() {
    let idx = index_exegra(
        &RepoPath::from_relative_str("docs/spec.jsonc").unwrap(),
        &spec(),
        None,
        &Languages::new(vec![]),
        &Options {
            requirement_kinds: vec!["claims".into()],
        },
    )
    .unwrap();
    let ids: Vec<_> = idx
        .claims
        .iter()
        .filter_map(|c| c.requirement.as_ref().map(|r| r.id.as_str()))
        .collect();
    assert_eq!(ids, vec!["FR-ISO-1", "FR-AGT-1"]);
}

#[test]
fn claim_identity_survives_edits_elsewhere() {
    let text = spec();
    let before = index(&text);
    let edited = text.replace("\"Built.\"", "\"Built in 2026.\"").replace(
        "  \"tables\": [",
        "  \"notes\": [\"an added line\"],\n  \"tables\": [",
    );
    let after = index(&edited);
    for needle in [
        "No request reads",
        "reject a request",
        "carry the `Tenant` header",
    ] {
        let id = |idx: &DocumentIndex| {
            idx.claims
                .iter()
                .find(|c| c.text.contains(needle))
                .map(|c| (c.id.clone(), c.range.start_line))
                .unwrap()
        };
        let (a, la) = id(&before);
        let (b, lb) = id(&after);
        assert_eq!(a, b, "{needle}");
        assert_eq!(lb, la + 1, "{needle} moved down one line");
    }
}

#[test]
fn malformed_input_is_an_error_not_a_panic() {
    for t in [
        "",
        "{",
        "{\"exegra\": ",
        "{\"exegra\":1,\"sections\":[{\"body\":\"{{entity:x}}\\n\",\"level\":1}]}",
    ] {
        let r = index_exegra(
            &RepoPath::from_relative_str("s.jsonc").unwrap(),
            t,
            None,
            &Languages::new(vec![]),
            &Options::default(),
        );
        if t.contains("entity:x") {
            assert!(r.is_ok(), "an unresolved marker stays as text");
        } else {
            assert!(r.is_err(), "{t}");
        }
    }
}
