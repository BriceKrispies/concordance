use super::*;
use concordance_core::code::{
    CallTarget, CrateInfo, Symbol, SymbolKind, WorkspaceInfo, EXTERNAL_MOD_ATTRIBUTE,
};

const TEST_FILE: &str = "crates/strata-simulation/tests/transitions.rs";

fn workspace() -> WorkspaceInfo {
    WorkspaceInfo {
        crates: vec![CrateInfo {
            name: "strata_simulation".into(),
            language: "rust".into(),
            manifest: RepoPath::from_relative_str("crates/strata-simulation/Cargo.toml").unwrap(),
            root_dir: Some(RepoPath::from_relative_str("crates/strata-simulation").unwrap()),
            targets: vec![],
            dependencies: vec![],
            features: vec![],
        }],
        ..WorkspaceInfo::default()
    }
}

fn index(path: &str, text: &str) -> SourceIndex {
    index_source(
        &RepoPath::from_relative_str(path).unwrap(),
        text,
        &workspace(),
    )
}

fn symbol<'a>(idx: &'a SourceIndex, name: &str) -> Option<&'a Symbol> {
    idx.symbols.iter().find(|s| s.name == name)
}

fn calls_path(sym: &Symbol, segments: &[&str]) -> bool {
    let want: Vec<String> = segments.iter().map(|s| s.to_string()).collect();
    sym.calls.iter().any(|c| {
        c.target
            == CallTarget::Path {
                segments: want.clone(),
            }
    })
}

const PROPTEST_BLOCK: &str = r#"
use strata_simulation::notes::notes_world;

fn helper(_w: &u32) {}

proptest! {
    #![proptest_config(ProptestConfig { cases: 96, ..ProptestConfig::default() })]

    /// Some prose.
    /// Verifies: KRN-001
    #[test]
    fn transitions_are_deterministic(ops in proptest::collection::vec(op_strategy(), 1..30)) {
        let mut world = notes_world();
        helper(&world);
    }
}
"#;

#[test]
fn proptest_test_is_indexed_like_a_plain_test() {
    let idx = index(TEST_FILE, PROPTEST_BLOCK);
    assert!(idx.file.parse_error.is_none());
    let sym = symbol(&idx, "transitions_are_deterministic").expect("proptest test is indexed");
    assert_eq!(sym.kind, SymbolKind::Function);
    assert!(sym.is_test);
    assert_eq!(sym.verifies, vec!["KRN-001".to_string()]);
    assert!(calls_path(sym, &["helper"]), "calls: {:?}", sym.calls);
    assert!(calls_path(sym, &["notes_world"]), "calls: {:?}", sym.calls);

    // The span covers the real function: from its doc comment to the closing brace of its body.
    let doc_line = PROPTEST_BLOCK
        .lines()
        .position(|l| l.contains("/// Some prose."))
        .unwrap() as u32
        + 1;
    let close_line = PROPTEST_BLOCK.lines().position(|l| l == "    }").unwrap() as u32 + 1;
    assert_eq!(sym.range.start_line, doc_line);
    assert_eq!(sym.range.end_line, close_line);
}

#[test]
fn proptest_block_with_several_tests_indexes_each_one() {
    let text = r#"
proptest! {
    #[test]
    fn first_property(a in 0u8..10) {
        check_first(a);
    }

    #[test]
    fn second_property(b in any::<u16>()) {
        check_second(b);
    }
}
"#;
    let idx = index(TEST_FILE, text);
    let first = symbol(&idx, "first_property").expect("first test is indexed");
    let second = symbol(&idx, "second_property").expect("second test is indexed");
    assert!(first.is_test && second.is_test);
    assert!(calls_path(first, &["check_first"]));
    assert!(calls_path(second, &["check_second"]));
}

#[test]
fn malformed_proptest_block_is_skipped_and_later_items_still_indexed() {
    let text = r#"
proptest! {
    #[test]
    fn broken(x in 0u8..10) {
        let y = ;
    }
}

#[test]
fn ordinary() {
    plain_helper();
}
"#;
    let idx = index(TEST_FILE, text);
    assert!(idx.file.parse_error.is_none());
    assert!(
        symbol(&idx, "broken").is_none(),
        "malformed block must not index"
    );
    let ordinary = symbol(&idx, "ordinary").expect("the ordinary test after it is indexed");
    assert!(ordinary.is_test);
    assert!(calls_path(ordinary, &["plain_helper"]));
}

const ASSERTION_CALLS: &str = r#"
use strata_simulation::notes::notes_world;

#[test]
fn calls_inside_assertions() {
    assert_eq!(notes_world(1), 2);
    assert!(Thing::check(&"x"), "message {}", other(3));
    println!("{}", not_recorded());
}
"#;

#[test]
fn calls_in_the_arguments_of_assertion_macros_are_recorded() {
    let idx = index(TEST_FILE, ASSERTION_CALLS);
    assert!(idx.file.parse_error.is_none());
    let sym = symbol(&idx, "calls_inside_assertions").expect("the test is indexed");
    assert!(calls_path(sym, &["notes_world"]), "calls: {:?}", sym.calls);
    assert!(
        calls_path(sym, &["Thing", "check"]),
        "calls: {:?}",
        sym.calls
    );
    assert!(calls_path(sym, &["other"]), "calls: {:?}", sym.calls);
    // Other macros stay opaque: only the assertion family is read as expressions.
    assert!(
        !calls_path(sym, &["not_recorded"]),
        "calls: {:?}",
        sym.calls
    );
}

#[test]
fn an_out_of_line_module_is_marked_and_an_inline_one_is_not() {
    let idx = index(
        "crates/strata-simulation/src/lib.rs",
        "mod declared;\nmod inline { pub fn f() {} }\n",
    );
    let declared = symbol(&idx, "declared").expect("the declaration is indexed");
    assert!(declared
        .attributes
        .iter()
        .any(|a| a == EXTERNAL_MOD_ATTRIBUTE));
    let inline = symbol(&idx, "inline").expect("the inline module is indexed");
    assert!(!inline
        .attributes
        .iter()
        .any(|a| a == EXTERNAL_MOD_ATTRIBUTE));
}
