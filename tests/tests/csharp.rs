//! What the C# case rows (`fixtures/csharp/EXPECTED.md`) cannot say: the project is indexed
//! as C#, and the findings explain themselves.

use concordance_tests::conformance::scan_with;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn csharp_project_is_indexed_and_findings_explain_themselves() {
    let s = scan_with("csharp", None).await;
    let m = &s.model;
    let file = m
        .files
        .values()
        .find(|f| f.path.as_str() == "src/Mixer.cs")
        .expect("Mixer.cs indexed");
    assert_eq!(file.language, "csharp");
    assert_eq!(file.crate_name.as_deref(), Some("Volume"));
    assert!(m
        .symbols
        .values()
        .any(|s| s.qualified_name() == "Volume::Volume::Mixer::SetVolume"));

    let explain = |needle: &str| {
        m.findings
            .values()
            .filter(|f| f.is_open() && f.title.contains(needle))
            .map(|f| f.explanation.clone())
            .collect::<Vec<_>>()
            .join("\n")
    };
    assert!(
        explain("int ParseLevel(string text)").contains("radix = 10"),
        "{}",
        explain("int ParseLevel(string text)")
    );
    // VOL-001 is linked to the xUnit test, no run yet.
    assert!(
        explain("SHALL reject a level above 100").contains("Rejects_level_above_100"),
        "{}",
        explain("SHALL reject a level above 100")
    );
}
