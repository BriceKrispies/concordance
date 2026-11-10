use crate::*;
use concordance_core::code::*;

fn ws() -> WorkspaceInfo {
    let rp = |s: &str| RepoPath::from_relative_str(s).unwrap();
    WorkspaceInfo {
        crates: vec![CrateInfo {
            name: "demo".into(),
            language: "rust".into(),
            manifest: rp("Cargo.toml"),
            root_dir: None,
            targets: vec![rp("src/lib.rs")],
            dependencies: vec![],
            features: vec![],
        }],
        from_cargo_metadata: true,
        warnings: vec![],
    }
}

fn index(path: &str, text: &str) -> SourceIndex {
    index_source(&RepoPath::from_relative_str(path).unwrap(), text, &ws())
}

const SAMPLE: &str = r#"
use std::fmt;

pub const MAX: u32 = 100;

pub struct Foo;

impl fmt::Display for Foo {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result { Ok(()) }
}
impl fmt::Debug for Foo {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result { Ok(()) }
}

pub fn set(&mut self, v: u8) -> Result<(), E> {
    let x = v.unwrap();
    if v > 100 { return Err(E); }
    v.min(150)
}
"#;

#[test]
fn ids_stable_when_code_moves() {
    let a = index("src/lib.rs", SAMPLE);
    let moved = format!("{}{}", "\n".repeat(10), SAMPLE);
    let b = index("src/lib.rs", &moved);
    let ids = |i: &SourceIndex| i.symbols.iter().map(|s| s.id.clone()).collect::<Vec<_>>();
    assert_eq!(ids(&a), ids(&b));
    assert_ne!(a.symbols[0].range.start_line, b.symbols[0].range.start_line);
}

#[test]
fn trait_impl_methods_are_disambiguated() {
    let i = index("src/lib.rs", SAMPLE);
    let fmts: Vec<_> = i.symbols.iter().filter(|s| s.name == "fmt").collect();
    assert_eq!(fmts.len(), 2);
    assert_ne!(fmts[0].id, fmts[1].id);
    assert_eq!(fmts[0].path, fmts[1].path);
}

#[test]
fn facts_detected() {
    let i = index("src/lib.rs", SAMPLE);
    let set = i.symbols.iter().find(|s| s.name == "set").unwrap();
    let has = |k: FactKind, d: &str| {
        set.facts
            .iter()
            .any(|f| f.kind == k && f.detail.contains(d))
    };
    assert!(has(FactKind::PanicSite, "unwrap"));
    assert!(has(FactKind::LiteralComparison, "v > 100"));
    assert!(has(FactKind::Clamp, "min(150)"));
    assert!(has(FactKind::ErrorReturn, "Err"));
    let max = i.symbols.iter().find(|s| s.name == "MAX").unwrap();
    assert!(max
        .facts
        .iter()
        .any(|f| f.kind == FactKind::ConstValue && f.detail == "MAX = 100"));
    assert_eq!(
        set.returns.as_ref().unwrap().result,
        Some(("()".to_string(), "E".to_string()))
    );
}

#[test]
fn documented_signature_matches_indexed() {
    let i = index("src/lib.rs", SAMPLE);
    let set = i.symbols.iter().find(|s| s.name == "set").unwrap();
    let (name, sig) =
        normalize_signature("pub fn set(&mut self, v: u8) -> Result<(), E>;").unwrap();
    assert_eq!(name, "set");
    assert_eq!(Some(&sig), set.signature.as_ref());
    assert_eq!(
        strip_visibility(&sig),
        "fn set(&mut self, v: u8) -> Result<(), E>"
    );
}

#[test]
fn module_paths() {
    let mp = |p: &str| index(p, "").file.module_path;
    assert!(mp("src/lib.rs").is_empty());
    assert_eq!(mp("src/a.rs"), vec!["a"]);
    assert_eq!(mp("src/a/mod.rs"), vec!["a"]);
    assert_eq!(mp("src/a/b.rs"), vec!["a", "b"]);
    assert_eq!(mp("tests/it.rs"), vec!["tests", "it"]);
}

#[test]
fn parse_error_is_reported() {
    let i = index("src/lib.rs", "fn (");
    assert!(i.file.parse_error.is_some());
    assert!(i.symbols.is_empty());
}

#[test]
fn test_functions_record_the_requirements_they_verify() {
    let src = r#"
/// Nested comments close at the balancing `*/`.
///
/// Verifies: LANG-006.3, LANG-006.4.
#[test]
fn nested_block_comments() {}

/// Verifies `PSS-003`
#[tokio::test]
async fn canonical() {}

/// Not a test, but documents a requirement link anyway. Verifies: nothing-here
fn helper() {}
"#;
    let idx = index("tests/lexing.rs", src);
    let sym = |n: &str| idx.symbols.iter().find(|s| s.name == n).unwrap();
    let t = sym("nested_block_comments");
    assert!(t.is_test);
    assert_eq!(t.verifies, vec!["LANG-006.3", "LANG-006.4"]);
    let c = sym("canonical");
    assert!(c.is_test);
    assert_eq!(c.verifies, vec!["PSS-003"]);
    let h = sym("helper");
    assert!(!h.is_test);
    assert!(h.verifies.is_empty(), "\"Verifies:\" must open a line");
}
