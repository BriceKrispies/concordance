//! What the Lean case rows (`fixtures/lean/EXPECTED.md`) cannot say: the Lake package is
//! indexed as Lean, theorems are proofs, and requirements name the theorems that prove them.

use concordance_tests::conformance::scan_with;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn lean_package_is_indexed_and_requirements_name_their_theorems() {
    let s = scan_with("lean", None).await;
    let m = &s.model;
    let file = m
        .files
        .values()
        .find(|f| f.path.as_str() == "Volume/Mixer.lean")
        .expect("Mixer.lean indexed");
    assert_eq!(file.language, "lean");
    assert_eq!(file.crate_name.as_deref(), Some("volume"));
    assert!(m
        .symbols
        .values()
        .any(|s| s.qualified_name() == "volume::Volume::Mixer::Volume::clampLevel"));
    let thm = m
        .symbols
        .values()
        .find(|s| s.name == "clamp_idempotent")
        .expect("theorem indexed");
    assert!(thm.is_test);
    assert!(thm
        .facts
        .iter()
        .any(|f| f.kind == concordance_core::code::FactKind::Unproven));

    for (needle, thm) in [
        ("SHALL never return a level above", "clamp_le_max"),
        ("SHALL be idempotent", "clamp_idempotent"),
    ] {
        let explanation: String = m
            .findings
            .values()
            .filter(|f| f.is_open() && f.title.contains(needle))
            .map(|f| f.explanation.clone())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(explanation.contains(thm), "{explanation}");
    }
}
