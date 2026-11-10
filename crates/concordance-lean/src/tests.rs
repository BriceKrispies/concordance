use crate::buildlog;
use crate::index::index_snippet;
use crate::lex::{lex, CommentKind, TokKind};
use crate::workspace::{parse_lakefile_lean, parse_lakefile_toml};
use concordance_core::code::*;
use concordance_core::testrun::{TestOutcome, TestSite};
use concordance_core::RepoPath;

fn idx(text: &str) -> SourceIndex {
    index_snippet(&RepoPath::from_relative_str("T.lean").unwrap(), text)
}

fn find<'a>(i: &'a SourceIndex, path: &str) -> &'a Symbol {
    i.symbols
        .iter()
        .find(|s| s.path.join(".") == path)
        .unwrap_or_else(|| {
            panic!(
                "no symbol {path}; have {:?}",
                i.symbols
                    .iter()
                    .map(|s| s.path.join("."))
                    .collect::<Vec<_>>()
            )
        })
}

#[test]
fn comments_nest_and_literals_hide_keywords() {
    let src = "/- outer /- inner def hidden -/ still def hidden2 -/\n\
               -- def line_hidden\n\
               def real := \"def in_string\" ++ \"\\\"def\"\n\
               def c := 'x'\n\
               def d' := 'a'\n\
               def «odd name» := r#\"theorem raw\"#\n";
    let i = idx(src);
    let names: Vec<_> = i.symbols.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["real", "c", "d'", "odd name"]);
    let l = lex(src);
    assert_eq!(
        l.comments.iter().map(|c| c.kind).collect::<Vec<_>>(),
        [CommentKind::Block, CommentKind::Line]
    );
    assert!(l.toks.iter().any(|t| t.kind == TokKind::Char));
}

#[test]
fn declarations_namespaces_and_visibility() {
    let src = "\
namespace A.B
def one : Nat := 1
section Helpers
private def hidden (n : Nat) : Nat := n
protected theorem t : True := trivial
end Helpers
abbrev Level := Nat
end A.B
def Top.inner := 2
structure P where
  x : Nat
  y z : String
  deriving Repr
inductive Color where
  | red
  | green (n : Nat) : Color
  deriving Repr
class Shape (α : Type) where
  area : α → Nat
instance : Shape Nat where
  area n := n
instance named : Shape Int := ⟨fun n => n.toNat⟩
axiom ax : False
opaque op : Nat
example : 1 = 1 := rfl
";
    let i = idx(src);
    assert_eq!(find(&i, "A.B.one").kind, SymbolKind::Function);
    assert_eq!(
        find(&i, "A.B.one").signature.as_deref(),
        Some("def one : Nat")
    );
    let h = find(&i, "A.B.hidden");
    assert_eq!(h.visibility, Visibility::Private);
    assert_eq!(h.signature.as_deref(), Some("def hidden (n : Nat) : Nat"));
    assert_eq!(find(&i, "A.B.t").visibility, Visibility::Restricted);
    assert!(i.symbols.iter().any(|s| s.path.join(".") == "A.B.Level"));
    // `end A.B` closed both levels; `Top.inner` is two segments at the top.
    assert_eq!(find(&i, "Top.inner").name, "inner");
    assert_eq!(find(&i, "P").kind, SymbolKind::Struct);
    assert_eq!(find(&i, "P.x").kind, SymbolKind::Field);
    assert_eq!(find(&i, "P.z").signature.as_deref(), Some("z : String"));
    assert_eq!(find(&i, "Color").kind, SymbolKind::Enum);
    assert_eq!(find(&i, "Color.red").kind, SymbolKind::Variant);
    assert_eq!(
        find(&i, "Color.green").signature.as_deref(),
        Some("green (n : Nat) : Color")
    );
    assert_eq!(find(&i, "Shape").kind, SymbolKind::Trait);
    assert_eq!(find(&i, "Shape.area").kind, SymbolKind::Field);
    let anon = find(&i, "instShapeNat");
    assert_eq!(anon.kind, SymbolKind::Impl);
    assert_eq!(anon.signature.as_deref(), Some("instance : Shape Nat"));
    assert_eq!(find(&i, "named").kind, SymbolKind::Impl);
    assert_eq!(find(&i, "ax").kind, SymbolKind::Const);
    assert_eq!(find(&i, "op").kind, SymbolKind::Const);
    assert_eq!(find(&i, "example").kind, SymbolKind::Function);
    // Ranges run to the next command.
    let one = find(&i, "A.B.one");
    assert_eq!((one.range.start_line, one.range.end_line), (2, 2));
    let p = find(&i, "P");
    assert_eq!((p.range.start_line, p.range.end_line), (10, 13));
}

#[test]
fn modifiers_attributes_and_facts() {
    let src = "\
/-- Doc line one.
Verifies: DOC-001 -/
@[simp] noncomputable def f (n : Nat) : Nat := sorry

-- Verifies: LEAN-002, LEAN-003
-- more words
theorem g : True := by
  trivial

unsafe def danger : Nat := panic! \"x\"
theorem untagged : True := trivial
";
    let i = idx(src);
    let f = find(&i, "f");
    assert_eq!(f.signature.as_deref(), Some("def f (n : Nat) : Nat"));
    assert_eq!(f.attributes, ["@[simp]", "noncomputable"]);
    assert_eq!(f.range.start_line, 3);
    assert_eq!(f.verifies, ["DOC-001"]);
    assert!(!f.is_test, "a def is never a test");
    let sorry = f
        .facts
        .iter()
        .find(|x| x.kind == FactKind::Unproven)
        .unwrap();
    assert_eq!(
        (sorry.range.start_line, sorry.detail.as_str()),
        (3, "sorry")
    );

    let g = find(&i, "g");
    assert!(g.is_test);
    assert_eq!(g.verifies, ["LEAN-002", "LEAN-003"]);
    assert!(g.facts.is_empty());
    assert!(!find(&i, "untagged").is_test);

    let d = find(&i, "danger");
    assert!(d.facts.iter().any(|x| x.kind == FactKind::UnsafeCode));
    assert!(d.facts.iter().any(|x| x.kind == FactKind::PanicSite));
}

#[test]
fn documented_signature_matches_indexed_one() {
    let sigs = crate::signature::documented("def  clamp (n : Nat)\n    (m : Nat) : Nat\n");
    assert_eq!(sigs.len(), 1);
    assert_eq!(sigs[0].name, "clamp");
    assert_eq!(sigs[0].text, "def clamp (n : Nat) (m : Nat) : Nat");
    assert_eq!(sigs[0].name_col, 5);
    let one = crate::signature::normalize_documented("theorem foo : True").unwrap();
    assert_eq!(one, ("foo".to_string(), "theorem foo : True".to_string()));
}

#[test]
fn lakefiles() {
    let l = parse_lakefile_lean(
        "import Lake\nopen Lake DSL\n\npackage «my-pkg» where\n  x := 1\n\nrequire mathlib from git \"u\"\n\n@[default_target]\nlean_lib Volume\n",
    );
    assert_eq!(l.name.as_deref(), Some("my_pkg"));
    assert_eq!(l.requires, ["mathlib"]);
    assert_eq!(l.libs, ["Volume"]);
    let t = parse_lakefile_toml(
        "name = \"volume\"\n[[require]]\nname = \"batteries\"\n[[lean_lib]]\nname = \"Volume\"\n",
    )
    .unwrap();
    assert_eq!(t.name.as_deref(), Some("volume"));
    assert_eq!(t.requires, ["batteries"]);
    assert_eq!(t.libs, ["Volume"]);
}

fn site(name: &str, file: &str, a: u32, b: u32) -> TestSite {
    let path = RepoPath::from_relative_str(file).unwrap();
    TestSite {
        name: name.into(),
        file: path.clone(),
        range: concordance_core::SourceRange {
            path,
            start_line: a,
            start_col: 1,
            end_line: b,
            end_col: 1,
        },
    }
}

#[test]
fn build_log_outcomes() {
    let tests = [
        site("v::Volume::Mixer::good", "Volume/Mixer.lean", 10, 14),
        site("v::Volume::Mixer::sorried", "Volume/Mixer.lean", 16, 20),
        site("v::Volume::Other::t", "Volume/Other.lean", 3, 5),
        site("v::Volume::Broken::t", "Volume/Broken.lean", 3, 5),
        site("v::Volume::Unbuilt::t", "Volume/Unbuilt.lean", 3, 5),
    ];
    let log = "\
\u{2714} [2/5] Built Volume.Other
\u{26a0} [3/5] Built Volume.Mixer
warning: ./Volume/Mixer.lean:17:8: declaration uses 'sorry'
\u{2716} [4/5] Building Volume.Broken
error: ./Volume/Broken.lean:9:2: unknown identifier 'x'
error: build failed
";
    let o = buildlog::outcomes(log, &tests);
    assert_eq!(o["v::Volume::Mixer::good"], TestOutcome::Passed);
    assert_eq!(o["v::Volume::Mixer::sorried"], TestOutcome::Failed);
    assert_eq!(o["v::Volume::Other::t"], TestOutcome::Passed);
    assert_eq!(o["v::Volume::Broken::t"], TestOutcome::Failed);
    assert!(
        !o.contains_key("v::Volume::Unbuilt::t"),
        "not mentioned: no evidence"
    );

    // A whole-build success covers modules Lake did not need to rebuild.
    let ok = buildlog::outcomes("Build completed successfully.\n", &tests);
    assert!(ok.values().all(|o| *o == TestOutcome::Passed));
    assert_eq!(ok.len(), 5);

    // Logs of other tools say nothing.
    assert!(buildlog::outcomes("test a ... ok\n", &tests).is_empty());
}
