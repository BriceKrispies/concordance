use super::*;

/// A white paper, a requirements file and a case table. One entity per line; the line numbers
/// below are asserted in `origin_maps_every_line_to_its_entity`.
const SAMPLE: &str = r##"{
  // header comment
  "exegra": "0.1",
  "documents": [
    {"id":"d-wp","path":"docs/whitepaper.md","role":"whitepaper"},
    {"id":"d-kernel","path":"spec/kernel.md","role":"requirements"},
    {"id":"d-cases","path":"spec/cases/k.md","role":"cases"},
  ],
  "sections": [
    {"id":"s-wp-1","title":"White paper","level":1,"document":"d-wp","order":1,"body":"\n{{entity:g-1}}\n{{entity:ng-1}}\n\n{{entity:b-1}}\n{{entity:b-2}}\n\nGlossary\n\n{{entity:t-1}}\n"},
    {"id":"s-0","title":"","level":0,"document":"d-kernel","order":1,"body":"Status: draft\n\n"},
    {"id":"s-1","title":"Kernel","level":1,"document":"d-kernel","order":2,"body":"\nIntro.\n\n{{entity:krn-001}}\n\n{{entity:krn-002}}\n"}
  ],
  "caseTables": [
    {"id":"c-1","path":"spec/cases/k.md","document":"d-cases","body":"# Cases\n\n## Cases\n\n| Case |\n"}
  ],
  "goals": [
    {"id":"g-1","ref":"G-1","statement":"A player can play.","stage":"R1","raw":"STALE goal"}
  ],
  "nonGoals": [
    {"id":"ng-1","ref":"NG-1","statement":"No voice chat.","horizon":"Not at launch."}
  ],
  "boundaries": [
    {"id":"b-1","ref":"B-1","name":"Web app","interfaceKind":"UI","usedBy":"players"},
    {"id":"b-2","ref":"B-2","name":"API | edge","interfaceKind":"HTTP","usedBy":"games"}
  ],
  "terms": [
    {"id":"t-1","term":"Fence","definition":"a token."}
  ],
  "claims": [
    {"id":"krn-001","kind":"requirement","ref":"KRN-001","statement":"The kernel SHALL boot.","rationale":"Boot first.","evaluate":{"method":"conformance"},"boundary":"b-1","boundaryDetail":"`kernel boot`","traces":["g-1"],"dependsOn":["krn-002"],"milestone":3,"seam":1,"lifecycle":"active","cases":["c-1"],"inSection":"s-1","raw":"**KRN-001** STALE text"},
    {"id":"krn-002","kind":"requirement","ref":"KRN-002","statement":"The kernel SHALL halt.","lifecycle":"retired","evaluate":{"method":"example"},"boundary":"b-1","traces":["g-1"]}
  ]
}
"##;

const WHITEPAPER_TEXT: &str = "\
# White paper

- **G-1** A player can play. *(R1)*
- **NG-1** *Not at launch.* No voice chat.

| ID | Boundary | Interface | Used by |
|---|---|---|---|
| B-1 | Web app | UI | players |
| B-2 | API \\| edge | HTTP | games |

Glossary

- **Fence**: a token.
";

const KERNEL_TEXT: &str = "\
Status: draft

# Kernel

Intro.

**KRN-001** The kernel SHALL boot.
- Verify: conformance
- Cases: spec/cases/k.md
- Boundary: B-1 `kernel boot`
- Traces: G-1
- Depends: KRN-002
- Rationale: Boot first.
- Milestone: 3
- Seam: 1

**KRN-002** The kernel SHALL halt.
- Status: retired
- Verify: example
- Boundary: B-1
- Traces: G-1
";

fn doc<'a>(x: &'a Expansion, path: &str) -> &'a VirtualDocument {
    x.documents.iter().find(|d| d.path == path).unwrap()
}

#[test]
fn renders_each_kind_to_exact_markdown() {
    let x = expand("spec.jsonc", SAMPLE).unwrap();
    assert!(x.diagnostics.is_empty(), "{:?}", x.diagnostics);
    assert_eq!(x.documents.len(), 3);
    assert_eq!(doc(&x, "docs/whitepaper.md").text, WHITEPAPER_TEXT);
    assert_eq!(doc(&x, "spec/kernel.md").text, KERNEL_TEXT);
    assert_eq!(
        doc(&x, "spec/cases/k.md").text,
        "# Cases\n\n## Cases\n\n| Case |\n"
    );
    let reqs: Vec<(&str, &str)> = x
        .requirements
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    assert_eq!(reqs, [("KRN-001", "krn-001"), ("KRN-002", "krn-002")]);
}

#[test]
fn editing_a_property_changes_the_text_and_raw_is_never_read() {
    let a = expand("s.jsonc", SAMPLE).unwrap();
    let edited = SAMPLE.replace("The kernel SHALL boot.", "The kernel SHALL start.");
    assert_ne!(edited, SAMPLE);
    let b = expand("s.jsonc", &edited).unwrap();
    let ka = &doc(&a, "spec/kernel.md").text;
    let kb = &doc(&b, "spec/kernel.md").text;
    assert!(ka.contains("The kernel SHALL boot."));
    assert!(kb.contains("**KRN-001** The kernel SHALL start."));
    assert!(!kb.contains("boot."));
    // The stale `raw` of the goal and of the requirement appear nowhere.
    for d in a.documents.iter().chain(&b.documents) {
        assert!(!d.text.contains("STALE"), "{}", d.path);
    }
    // Only the edited document differs.
    assert_eq!(doc(&a, "docs/whitepaper.md"), doc(&b, "docs/whitepaper.md"));
    assert_eq!(doc(&a, "spec/cases/k.md"), doc(&b, "spec/cases/k.md"));
}

#[test]
fn origin_maps_every_line_to_its_entity() {
    let x = expand("spec.jsonc", SAMPLE).unwrap();
    let k = doc(&x, "spec/kernel.md");
    assert_eq!(k.origin_of(1), Some(("s-0", 11)));
    assert_eq!(k.origin_of(2), Some(("s-0", 11)));
    assert_eq!(k.origin_of(3), Some(("s-1", 12)));
    assert_eq!(k.origin_of(5), Some(("s-1", 12)));
    assert_eq!(k.origin_of(7), Some(("krn-001", 31)));
    assert_eq!(k.origin_of(15), Some(("krn-001", 31)));
    assert_eq!(k.origin_of(16), Some(("s-1", 12)));
    assert_eq!(k.origin_of(17), Some(("krn-002", 32)));
    assert_eq!(k.origin_of(21), Some(("krn-002", 32)));
    assert_eq!(k.origin_of(22), None);
    assert_eq!(k.origin_of(0), None);
    assert_eq!(k.origin.len(), k.text.lines().count());

    let w = doc(&x, "docs/whitepaper.md");
    assert_eq!(w.origin_of(1), Some(("s-wp-1", 10)));
    assert_eq!(w.origin_of(3), Some(("g-1", 18)));
    assert_eq!(w.origin_of(4), Some(("ng-1", 21)));
    // The table header and separator belong to the first row's entity.
    assert_eq!(w.origin_of(6), Some(("b-1", 24)));
    assert_eq!(w.origin_of(7), Some(("b-1", 24)));
    assert_eq!(w.origin_of(8), Some(("b-1", 24)));
    assert_eq!(w.origin_of(9), Some(("b-2", 25)));
    assert_eq!(w.origin_of(13), Some(("t-1", 28)));

    let c = doc(&x, "spec/cases/k.md");
    assert_eq!(c.origin_of(1), Some(("c-1", 15)));
    assert_eq!(c.origin_of(5), Some(("c-1", 15)));
    assert_eq!(k.document_id, "d-kernel");
    assert_eq!(k.source_line, 6);
}

#[test]
fn unusable_files_are_errors() {
    assert!(matches!(
        expand("x.jsonc", "{"),
        Err(ExegraError::Syntax { .. })
    ));
    assert!(matches!(
        expand("x.jsonc", "{\"a\":1}"),
        Err(ExegraError::NotExegra { .. })
    ));
    assert!(matches!(
        expand("x.jsonc", "[1]"),
        Err(ExegraError::NotExegra { .. })
    ));
    let e = expand("x.jsonc", "{\"a\":1}").unwrap_err();
    assert!(e.to_string().starts_with("x.jsonc: not an Exegra"));
}

#[test]
fn a_requirement_that_no_section_places_is_reported_not_dropped() {
    let text = r#"{
  "exegra": "0.1",
  "documents": [{"id":"d-k","path":"spec/k.md","role":"requirements"}],
  "sections": [{"id":"s-1","title":"K","level":1,"document":"d-k","order":1,"body":"\n{{entity:krn-001}}\n"}],
  "claims": [
    {"id":"krn-001","kind":"requirement","ref":"KRN-001","statement":"The kernel SHALL boot.","evaluate":{"method":"example"}},
    {"id":"krn-002","kind":"requirement","ref":"KRN-002","statement":"The kernel SHALL halt.","evaluate":{"method":"example"}}
  ]
}"#;
    let x = expand("specs/spec.jsonc", text).unwrap();
    let unplaced: Vec<&Diagnostic> = x
        .diagnostics
        .iter()
        .filter(|d| d.kind == DiagnosticKind::UnplacedEntity)
        .collect();
    assert_eq!(unplaced.len(), 1, "diagnostics: {:?}", x.diagnostics);
    assert_eq!(unplaced[0].entity, "krn-002");
    assert_eq!(unplaced[0].line, 7);
    assert_eq!(unplaced[0].kind.severity(), Severity::Error);
    assert!(!doc(&x, "spec/k.md").text.contains("KRN-002"));
    assert!(doc(&x, "spec/k.md").text.contains("KRN-001"));
}

#[test]
fn a_multi_line_statement_keeps_its_lines_under_the_item() {
    let src = r#"{"exegra":"0.1",
"documents":[{"id":"d","path":"wp.md","role":"whitepaper"}],
"sections":[{"id":"s","title":"Decisions","level":2,"document":"d","order":1,"body":"{{entity:d-1}}\n"}],
"decisions":[{"id":"d-1","ref":"D-1","title":"Rust","statement":"**Rust.** Decided by the founder.\n- Every component in Rust.\n\n  A nested paragraph.\n- Lean for proofs."}]}"#;
    let x = expand("spec.jsonc", src).unwrap();
    assert!(x.diagnostics.is_empty(), "{:?}", x.diagnostics);
    assert_eq!(
        doc(&x, "wp.md").text,
        "## Decisions\n- **D-1** **Rust.** Decided by the founder.\n  - Every component in Rust.\n\n    A nested paragraph.\n  - Lean for proofs.\n"
    );
}

#[test]
fn a_row_under_a_header_in_the_body_gets_no_second_header() {
    let src = r#"{"exegra":"0.1",
"documents":[{"id":"d","path":"wp.md","role":"whitepaper"}],
"sections":[{"id":"s","title":"Topology","level":2,"document":"d","order":1,"body":"| ID | Component | Owns | Talks to |\n|---|---|---|---|\n{{entity:c-1}}\n"}],
"components":[{"id":"c-1","ref":"C-1","name":"Kernel (`k|x`)","category":"logical","purpose":"Decides | plans","talksTo":"C-2"}]}"#;
    let x = expand("spec.jsonc", src).unwrap();
    assert_eq!(
        doc(&x, "wp.md").text,
        "## Topology\n| ID | Component | Owns | Talks to |\n|---|---|---|---|\n| C-1 | Kernel (`k|x`) | Decides \\| plans | C-2 |\n"
    );
}

#[test]
fn a_misspelt_requirement_kind_is_reported_not_rendered_silently() {
    let text = r#"{
  "exegra": "0.1",
  "documents": [{"id":"d-k","path":"spec/k.md","role":"requirements"}],
  "sections": [{"id":"s-1","title":"K","level":1,"document":"d-k","order":1,"body":"\n{{entity:krn-001}}\n{{entity:krn-002}}\n"}],
  "claims": [
    {"id":"krn-001","kind":"requirement","ref":"KRN-001","statement":"The kernel SHALL boot.","evaluate":{"method":"example"}},
    {"id":"krn-002","kind":"Requirment","ref":"KRN-002","statement":"The kernel halts on power loss.","evaluate":{"method":"example"}}
  ]
}"#;
    let x = expand("specs/spec.jsonc", text).unwrap();
    let misspelt: Vec<&Diagnostic> = x
        .diagnostics
        .iter()
        .filter(|d| d.kind == DiagnosticKind::MisspeltKind)
        .collect();
    assert_eq!(misspelt.len(), 1, "diagnostics: {:?}", x.diagnostics);
    assert_eq!(misspelt[0].entity, "krn-002");
    assert_eq!(misspelt[0].kind.severity(), Severity::Error);
    // Only the misspelt kind is reported: the correct spelling and an unrelated kind are not.
    assert!(super::is_misspelling_of_requirement("Requirement"));
    assert!(!super::is_misspelling_of_requirement("requirement"));
    assert!(!super::is_misspelling_of_requirement("note"));
}
