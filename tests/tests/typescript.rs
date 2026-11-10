//! What the TypeScript case rows (`fixtures/typescript/EXPECTED.md`) cannot say: the package
//! is indexed as TypeScript, and the findings explain themselves.

use concordance_tests::conformance::scan_with;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn typescript_package_is_indexed_and_findings_explain_themselves() {
    let s = scan_with("typescript", None).await;
    let m = &s.model;
    let file = m
        .files
        .values()
        .find(|f| f.path.as_str() == "src/mixer.ts")
        .expect("mixer.ts indexed");
    assert_eq!(file.language, "typescript");
    assert_eq!(file.crate_name.as_deref(), Some("volume"));
    assert!(m
        .symbols
        .values()
        .any(|s| s.qualified_name() == "volume::mixer::Mixer::setVolume"));

    let explain = |needle: &str| {
        m.findings
            .values()
            .filter(|f| f.is_open() && f.title.contains(needle))
            .map(|f| f.explanation.clone())
            .collect::<Vec<_>>()
            .join("\n")
    };
    assert!(
        explain("parseLevel(text: string)").contains("radix = 10"),
        "{}",
        explain("parseLevel(text: string)")
    );
    // VOL-001 is linked to the vitest test, no run yet.
    assert!(
        explain("SHALL reject a level above 100").contains("rejects levels above 100"),
        "{}",
        explain("SHALL reject a level above 100")
    );
}
