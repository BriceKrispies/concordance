//! Implementation: Rust crates, files, symbols and syntactic facts.
//!
//! Everything here comes from syntactic analysis (`syn`) and `cargo metadata`. Nothing
//! here is compiler-verified; relationships that depend on name resolution carry an
//! explicit [`crate::graph::Resolution`].

use crate::{ContentHash, FileId, RepoPath, SourceRange, SymbolId};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SymbolKind {
    Module,
    Function,
    /// Function inside an `impl` block with a `self` receiver.
    Method,
    /// Function inside an `impl` block without a receiver.
    AssociatedFunction,
    Struct,
    Enum,
    Variant,
    /// Named struct field.
    Field,
    Union,
    Trait,
    /// Function declared inside a `trait` block.
    TraitMethod,
    Impl,
    Const,
    Static,
    TypeAlias,
    MacroRules,
    /// Class (TypeScript, C#, ...).
    Class,
    /// Interface (TypeScript, C#) — the counterpart of a Rust trait.
    Interface,
    /// Class/interface property or field accessor (TypeScript, C#).
    Property,
    /// Class constructor.
    Constructor,
    /// Namespace (TypeScript `namespace`, C# `namespace`).
    Namespace,
}

impl SymbolKind {
    /// Stable snake_case name (also part of symbol id keys).
    pub fn as_str(self) -> &'static str {
        match self {
            SymbolKind::Module => "module",
            SymbolKind::Function => "function",
            SymbolKind::Method => "method",
            SymbolKind::AssociatedFunction => "associated_function",
            SymbolKind::Struct => "struct",
            SymbolKind::Enum => "enum",
            SymbolKind::Variant => "variant",
            SymbolKind::Field => "field",
            SymbolKind::Union => "union",
            SymbolKind::Trait => "trait",
            SymbolKind::TraitMethod => "trait_method",
            SymbolKind::Impl => "impl",
            SymbolKind::Const => "const",
            SymbolKind::Static => "static",
            SymbolKind::TypeAlias => "type_alias",
            SymbolKind::MacroRules => "macro_rules",
            SymbolKind::Class => "class",
            SymbolKind::Interface => "interface",
            SymbolKind::Property => "property",
            SymbolKind::Constructor => "constructor",
            SymbolKind::Namespace => "namespace",
        }
    }

    pub fn is_callable(self) -> bool {
        matches!(
            self,
            SymbolKind::Function
                | SymbolKind::Method
                | SymbolKind::AssociatedFunction
                | SymbolKind::TraitMethod
                | SymbolKind::Constructor
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Visibility {
    Public,
    Crate,
    /// `pub(super)`, `pub(in path)`.
    Restricted,
    Private,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceFile {
    pub id: FileId,
    pub path: RepoPath,
    /// Id of the language plugin that indexed the file (`rust`, `typescript`).
    #[serde(default = "rust_id")]
    pub language: String,
    /// Cargo package name normalized to `snake_case` (`my-crate` -> `my_crate`), if the
    /// file belongs to a known crate.
    pub crate_name: Option<String>,
    /// Module path of the file inside its crate (`src/a/b.rs` -> `["a","b"]`).
    pub module_path: Vec<String>,
    pub content_hash: ContentHash,
    pub line_count: u32,
    /// Set when `syn` could not parse the file. The file then has no symbols.
    pub parse_error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImplInfo {
    /// Self type as written, normalized token string (`Foo<T>`).
    pub self_ty: String,
    /// Implemented trait path as written (`Display`, `std::fmt::Display`).
    pub trait_path: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReturnContract {
    /// Return type as written, normalized token string.
    pub text: String,
    /// `(ok, err)` when the return type's last segment is `Result`.
    pub result: Option<(String, String)>,
    pub is_option: bool,
}

/// A syntactic fact about a symbol body or item.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Fact {
    pub kind: FactKind,
    pub range: SourceRange,
    /// Normalized source text of the construct (bounded, <= 200 chars), e.g. `x.unwrap()`,
    /// `count > 100`, `const MAX: u32 = 100`, `std::env::var("PORT")`.
    pub detail: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FactKind {
    /// A failure site: an operation that aborts the call instead of returning.
    /// Rust: `panic!`, `unreachable!`, `todo!`, `unimplemented!`, `assert!` family,
    /// `.unwrap()`, `.expect(..)`. TypeScript/C#: `throw`.
    PanicSite,
    /// Code that opts out of the language's checks. Rust/C#: `unsafe`. TypeScript:
    /// `any` annotations and `as any` casts, `@ts-ignore` / `@ts-expect-error`.
    UnsafeCode,
    /// Comparison against a numeric literal (`x > 100`, `len <= MAX`).
    LiteralComparison,
    /// Value of a `const`/`static` initialized with a literal. `detail` holds `NAME = value`.
    ConstValue,
    /// `.min(lit)`, `.max(lit)`, `.clamp(a, b)`, `.saturating_*` with literal operands.
    Clamp,
    /// Environment read. Rust: `std::env::var("..")` / `env!` / `option_env!`.
    /// TypeScript: `process.env.X`. C#: `Environment.GetEnvironmentVariable`.
    EnvRead,
    /// `#[cfg(feature = "..")]` / `cfg!(feature = "..")`.
    FeatureGate,
    /// Explicit error return: `return Err(..)`, `Err(..)` tail, `bail!`, `?` on a Result.
    ErrorReturn,
    /// Any other macro invocation (`detail` = macro path).
    MacroInvocation,
}

/// A call expression found in a body. Not resolved by the indexer: resolution happens
/// in the graph and is labelled with how it was resolved.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CallSite {
    pub target: CallTarget,
    pub range: SourceRange,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CallTarget {
    /// `a::b::c(..)` or `Type::method(..)`; segments as written, generics stripped.
    Path { segments: Vec<String> },
    /// `expr.method(..)`; receiver type unknown syntactically.
    Method { name: String },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Symbol {
    pub id: SymbolId,
    pub file: FileId,
    pub kind: SymbolKind,
    pub name: String,
    /// Crate-qualified path: `["my_crate","module","Type","method"]`. Impl methods use the
    /// self type's last segment; trait impls are disambiguated in the id key, not the path.
    pub path: Vec<String>,
    pub parent: Option<SymbolId>,
    pub visibility: Visibility,
    /// Approximate public API: `Public` and every ancestor module/type `Public`
    /// (trait impl items inherit from the self type). Syntactic approximation.
    pub is_public_api: bool,
    /// Normalized signature (tokens joined by single spaces with `quote` formatting),
    /// without the body: `pub fn set_volume(&mut self, v: u8) -> Result<(), Error>`.
    /// `None` for modules/impls.
    pub signature: Option<String>,
    pub doc: Option<String>,
    /// Non-doc outer attributes, normalized (`#[derive(Debug, Clone)]`, `#[must_use]`).
    pub attributes: Vec<String>,
    /// `cfg(..)` predicates on this item and all ancestors, outermost first.
    pub cfg: Vec<String>,
    pub range: SourceRange,
    /// Hash of the item's full normalized token stream (changes when body changes).
    pub content_hash: ContentHash,
    /// Hash of `signature` (changes only when the interface changes).
    pub signature_hash: Option<ContentHash>,
    pub impl_info: Option<ImplInfo>,
    pub returns: Option<ReturnContract>,
    pub facts: Vec<Fact>,
    pub calls: Vec<CallSite>,
    /// A test function (`#[test]`, `#[tokio::test]`, …).
    #[serde(default)]
    pub is_test: bool,
    /// Requirement IDs this item declares it verifies, from `Verifies: ID, ID` lines in
    /// its doc comment. Meaningful on tests; recorded on any item.
    #[serde(default)]
    pub verifies: Vec<String>,
}

impl Symbol {
    pub fn qualified_name(&self) -> String {
        self.path.join("::")
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Import {
    /// `use a::b::{c as d}` -> path `["a","b","c"]`, alias `Some("d")`.
    pub path: Vec<String>,
    pub alias: Option<String>,
    pub glob: bool,
    pub range: SourceRange,
}

/// Everything extracted from one Rust file. Produced by `concordance-rust`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SourceIndex {
    pub file: SourceFile,
    pub symbols: Vec<Symbol>,
    pub imports: Vec<Import>,
}

/// A Cargo package found by `cargo metadata` (or by manifest fallback).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CrateInfo {
    /// Normalized (`-` -> `_`).
    pub name: String,
    /// Language plugin that discovered the package.
    #[serde(default = "rust_id")]
    pub language: String,
    pub manifest: RepoPath,
    /// Directory containing the manifest; `None` when it is the repository root.
    pub root_dir: Option<RepoPath>,
    /// Target root files (`src/lib.rs`, `src/main.rs`, `src/bin/*.rs`, ...).
    pub targets: Vec<RepoPath>,
    /// Normalized names of declared dependencies (normal + build; dev excluded).
    pub dependencies: Vec<String>,
    pub features: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceInfo {
    pub crates: Vec<CrateInfo>,
    /// True when `cargo metadata` ran successfully; false when crates were found by
    /// reading manifests directly (dependency and feature data then partial).
    pub from_cargo_metadata: bool,
    pub warnings: Vec<String>,
}

impl WorkspaceInfo {
    /// Crate whose root directory most specifically contains `path`.
    pub fn crate_for(&self, path: &RepoPath) -> Option<&CrateInfo> {
        self.crates
            .iter()
            .filter(|c| c.root_dir.as_ref().is_none_or(|d| path.starts_with_dir(d)))
            .max_by_key(|c| c.root_dir.as_ref().map_or(0, |d| d.as_str().len()))
    }
}

fn rust_id() -> String {
    "rust".to_string()
}
