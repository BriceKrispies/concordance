//! Intent: documents, sections and claims extracted from Markdown.

use crate::{ClaimId, ContentHash, DocumentId, RepoPath, SectionId, SourceRange};
use serde::{Deserialize, Serialize};

/// How much a document (or claim) is allowed to define intended behavior.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Authority {
    /// Defines intended behavior. Never rewritten to match code without an explicit decision.
    Normative,
    /// Describes behavior. May be considered outdated when reliable implementation evidence changes.
    Descriptive,
    /// Examples, background, rationale, history. Imposes no obligation.
    Informative,
    /// Could not be established. Never silently treated as normative.
    Unknown,
}

/// Where an authority assessment came from. Shown to users so authority is never invented.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthoritySource {
    /// `[authority]` globs in the configuration file.
    Configured,
    /// `concordance: normative|descriptive|informative` in YAML-style front matter (optional).
    FrontMatter,
    /// Document uses upper-case RFC 2119 keywords (MUST, SHALL, ...).
    Rfc2119Keywords,
    /// Path naming convention (e.g. a `spec/`, `specifications/` or `rfcs/` directory).
    PathConvention,
    /// Section heading role (Example, Background, History, ...).
    SectionRole,
    /// The file format declares itself the specification (an Exegra JSONC file).
    SpecFormat,
    /// Nothing established it.
    Default,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthorityAssessment {
    pub level: Authority,
    pub source: AuthoritySource,
    /// One sentence a human can check, e.g. "matched configured glob `docs/spec/**`".
    pub rationale: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Document {
    pub id: DocumentId,
    pub path: RepoPath,
    /// First level-1 heading, if any.
    pub title: Option<String>,
    pub authority: AuthorityAssessment,
    pub content_hash: ContentHash,
    pub line_count: u32,
}

/// What kind of content a section holds, from its heading. Claims under
/// non-`Body` sections are informative.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SectionRole {
    Body,
    Example,
    Background,
    Rationale,
    History,
    ImplementationNotes,
    NonGoals,
    OpenQuestions,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Section {
    pub id: SectionId,
    pub document: DocumentId,
    pub parent: Option<SectionId>,
    pub heading: String,
    pub level: u8,
    /// Heading titles from the top of the document down to this section (inclusive).
    pub heading_path: Vec<String>,
    /// GitHub-style anchor slug (`## Rate limits` -> `rate-limits`).
    pub anchor: String,
    /// Effective role (inherited from ancestors unless this heading sets its own).
    pub role: SectionRole,
    /// From the heading line to the line before the next heading of equal or higher level.
    pub range: SourceRange,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClaimKind {
    /// Positive obligation ("must", "shall", "should").
    Requirement,
    /// Negative obligation ("must not", "never", "is forbidden").
    Prohibition,
    /// Something true at all times ("at most", "always holds", "invariant").
    Invariant,
    /// A promise to callers ("guarantees", "is guaranteed to", "will always").
    Guarantee,
    /// Structural rule about crates/modules/dependencies/layers.
    ArchitecturalConstraint,
    /// A documented API surface: signature, parameters, return/error contract.
    ApiContract,
    /// Declarative description of what a referenced symbol does ("`parse` returns ...").
    Behavior,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Modality {
    Must,
    MustNot,
    Should,
    ShouldNot,
    May,
    /// No modal verb; a statement of fact about the system.
    Declarative,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlockKind {
    Paragraph,
    ListItem,
    TableRow,
    CodeBlock,
    BlockQuote,
}

/// A statement about intended or documented behavior.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Claim {
    pub id: ClaimId,
    pub document: DocumentId,
    pub section: SectionId,
    pub heading_path: Vec<String>,
    /// The statement, whitespace-normalized, Markdown inline syntax removed except code spans
    /// (kept in backticks so references stay visible).
    pub text: String,
    pub kind: ClaimKind,
    pub modality: Modality,
    /// Effective authority: document authority, downgraded to Informative by section role.
    pub authority: Authority,
    pub block: BlockKind,
    pub range: SourceRange,
    /// Hash of `text` plus `heading_path`; changes when the claim's meaning may have changed.
    pub content_hash: ContentHash,
    pub references: Vec<Reference>,
    pub bounds: Vec<Bound>,
    pub ambiguity: Vec<AmbiguityNote>,
    /// For API contracts taken from fenced code: the documented item signature, verbatim.
    pub documented_signature: Option<String>,
    /// Language plugin whose fence the documented signature came from.
    #[serde(default)]
    pub signature_language: Option<String>,
    /// Deterministically checkable form, when the sentence matches a known pattern.
    pub constraint: Option<ConstraintForm>,
    /// Requirement this claim belongs to (`LANG-006.3`, `PSS-003`): from a bold ID that
    /// opens its paragraph or list item, or from the nearest enclosing heading that is an ID.
    #[serde(default)]
    pub requirement: Option<RequirementRef>,
}

/// How a claim was tied to a requirement ID.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RequirementRef {
    pub id: String,
    /// `inline`: the ID opens this claim's own paragraph (one requirement, one statement).
    /// `heading`: the ID is a section heading; every claim under it shares the ID.
    pub form: RequirementForm,
    /// Attribute list items directly after a bold opener (`Verify:`, `Boundary:`, ...).
    /// Empty for heading-form requirements.
    #[serde(default, skip_serializing_if = "RequirementAttributes::is_empty")]
    pub attributes: RequirementAttributes,
}

/// The attribute bullets of a requirement, verbatim after their `Name:` label. They belong to
/// the requirement and are never extracted as claims of their own.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RequirementAttributes {
    pub verify: Option<String>,
    pub boundary: Option<String>,
    pub traces: Option<String>,
    pub rationale: Option<String>,
    pub depends: Option<String>,
    pub status: Option<String>,
    pub cases: Option<String>,
    pub milestone: Option<String>,
    pub seam: Option<String>,
}

impl RequirementAttributes {
    pub const NAMES: [&'static str; 9] = [
        "Verify",
        "Boundary",
        "Traces",
        "Rationale",
        "Depends",
        "Status",
        "Cases",
        "Milestone",
        "Seam",
    ];

    pub fn is_empty(&self) -> bool {
        self.verify.is_none()
            && self.boundary.is_none()
            && self.traces.is_none()
            && self.rationale.is_none()
            && self.depends.is_none()
            && self.status.is_none()
            && self.cases.is_none()
            && self.milestone.is_none()
            && self.seam.is_none()
    }

    /// The milestone as a positive integer; `None` when absent or not one.
    pub fn milestone_number(&self) -> Option<u32> {
        parse_positive(self.milestone.as_deref()?)
    }

    /// The seam as a positive integer; `None` when absent or not one.
    pub fn seam_number(&self) -> Option<u32> {
        parse_positive(self.seam.as_deref()?)
    }

    /// The value of attribute `name` (one of [`Self::NAMES`]).
    pub fn get(&self, name: &str) -> Option<&str> {
        match name {
            "Verify" => self.verify.as_deref(),
            "Boundary" => self.boundary.as_deref(),
            "Traces" => self.traces.as_deref(),
            "Rationale" => self.rationale.as_deref(),
            "Depends" => self.depends.as_deref(),
            "Status" => self.status.as_deref(),
            "Cases" => self.cases.as_deref(),
            "Milestone" => self.milestone.as_deref(),
            "Seam" => self.seam.as_deref(),
            _ => None,
        }
    }

    /// Set attribute `name`; the first occurrence wins. Returns whether `name` is known.
    pub fn set(&mut self, name: &str, value: String) -> bool {
        let slot = match name {
            "Verify" => &mut self.verify,
            "Boundary" => &mut self.boundary,
            "Traces" => &mut self.traces,
            "Rationale" => &mut self.rationale,
            "Depends" => &mut self.depends,
            "Status" => &mut self.status,
            "Cases" => &mut self.cases,
            "Milestone" => &mut self.milestone,
            "Seam" => &mut self.seam,
            _ => return false,
        };
        if slot.is_none() {
            *slot = Some(value);
        }
        true
    }
}

/// A positive decimal integer (`3`, not `0`, `-1`, `3.5` or `three`); surrounding space and
/// backticks are ignored.
pub fn parse_positive(s: &str) -> Option<u32> {
    let t = s.trim().trim_matches('`').trim();
    if t.is_empty() || !t.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    t.parse::<u32>().ok().filter(|n| *n > 0)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequirementForm {
    Inline,
    Heading,
}

/// Whether `s` is a requirement ID: an upper-case prefix, optional upper-case/digit
/// segments, then a number with an optional dotted sub-number: `PSS-003`, `LANG-006.3`,
/// `UI-12`, `WEB-SEC-004`, `B-5`. The number may carry a one- or two-letter tag
/// (`PII-T1`, `AI-R3`). Issue-tracker keys (`PROJ-2147`) also match the shape, which
/// is why IDs are only recognized where they are *defined* (bold paragraph opener or
/// heading) and where a test *declares* `Verifies:`.
pub fn is_requirement_id(s: &str) -> bool {
    let mut parts: Vec<&str> = s.split('-').collect();
    if parts.len() < 2 || s.len() > 40 {
        return false;
    }
    let last = parts.pop().unwrap_or_default();
    let (num, sub) = match last.split_once('.') {
        Some((a, b)) => (a, Some(b)),
        None => (last, None),
    };
    let digits = |x: &str| !x.is_empty() && x.len() <= 4 && x.bytes().all(|b| b.is_ascii_digit());
    let tag = num.len()
        - num
            .trim_start_matches(|c: char| c.is_ascii_uppercase())
            .len();
    if tag > 2 || !digits(&num[tag..]) || sub.is_some_and(|b| !digits(b)) {
        return false;
    }
    let first = parts[0];
    !first.is_empty()
        && first.as_bytes()[0].is_ascii_uppercase()
        && parts.iter().all(|p| {
            !p.is_empty()
                && p.len() <= 10
                && p.bytes()
                    .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
        })
}

impl Claim {
    /// Claims that impose obligations or make checkable statements. Informative claims
    /// (examples, background) are indexed but never produce compliance findings.
    pub fn is_obligation(&self) -> bool {
        !matches!(self.authority, Authority::Informative) && !matches!(self.modality, Modality::May)
    }
}

/// A reference written in documentation.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Reference {
    /// Exactly as written (`Foo::bar()`, `../api.md#limits`, `src/lib.rs`).
    pub text: String,
    pub target: ReferenceTarget,
    pub strength: ReferenceStrength,
    pub range: Option<SourceRange>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReferenceTarget {
    /// Rust path segments with `()`/generics stripped: `crate::a::Foo::bar` -> `["crate","a","Foo","bar"]`.
    Symbol { path: Vec<String> },
    /// A repository file, resolved to a repository path.
    File { path: RepoPath, line: Option<u32> },
    /// A Markdown section (or whole document when `anchor` is `None`).
    Section {
        document: RepoPath,
        anchor: Option<String>,
    },
    /// A relative link that escapes the repository root. Not checked.
    OutsideRepo { text: String },
    /// http(s)/mailto etc. Not checked.
    External { url: String },
    /// `[ALIAS:path:lines]`: a file or directory in another git repository, at the
    /// revision configured for `alias` under `[repos]`. Aliases with no configured
    /// repository are not checked.
    Citation {
        /// As written (`APP`, `app`); mapped to a configured repository at reconcile time.
        alias: String,
        /// Repository-relative path, `/`-separated, as written.
        path: String,
        /// Cited line ranges, in order; empty for a whole file or directory.
        lines: Vec<LineSpan>,
        /// `"Section"` or `#anchor`: a heading in the cited Markdown file.
        section: Option<String>,
    },
}

/// An inclusive, 1-based line range.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct LineSpan {
    pub start: u32,
    pub end: u32,
}

impl std::fmt::Display for LineSpan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.start == self.end {
            write!(f, "{}", self.start)
        } else {
            write!(f, "{}-{}", self.start, self.end)
        }
    }
}

/// How sure the extractor is that a code span is meant as a code reference.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReferenceStrength {
    /// Snake-case identifier without `::` or `()` (`max_retries`). Never produces a dangling finding.
    Weak,
    /// CamelCase identifier in a code span (`RateLimiter`).
    Probable,
    /// Link, path with `::`, call syntax `name()`, or `fn name` / `struct Name`.
    Explicit,
}

/// A numeric bound stated in a claim: "must never exceed 100 requests".
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Bound {
    /// Best-effort noun phrase the bound applies to ("requests per window").
    pub subject: String,
    pub op: BoundOp,
    pub value: f64,
    pub unit: Option<String>,
    /// The matched phrase, verbatim.
    pub text: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BoundOp {
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
}

impl BoundOp {
    /// Does `x` satisfy `x <op> bound`?
    pub fn admits(self, x: f64, bound: f64) -> bool {
        match self {
            BoundOp::Lt => x < bound,
            BoundOp::Le => x <= bound,
            BoundOp::Gt => x > bound,
            BoundOp::Ge => x >= bound,
            BoundOp::Eq => x == bound,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AmbiguityNote {
    pub kind: AmbiguityKind,
    pub detail: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AmbiguityKind {
    /// "appropriate", "fast", "reasonable", "as needed", "etc." ...
    VagueTerm,
    /// Both a strong and a weak modal in one statement ("must ... if possible").
    MixedModality,
    /// Statement starts with a pronoun whose antecedent is in another block.
    UnresolvedSubject,
    /// Another normative claim states an incompatible requirement.
    ConflictingStatement,
    /// Reported by semantic analysis (validated, but still an interpretation).
    ModelReported,
}

/// Claim shapes that structural analysis can check deterministically.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "form", rename_all = "snake_case")]
pub enum ConstraintForm {
    /// "`a` must not depend on `b`" / "`a` must not import `b`". Names as written.
    NoDependency { from: String, to: String },
    /// "`scope` must not panic" / "must never panic". Empty scope = whole source roots.
    NoPanic { scope: Vec<String> },
    /// "`scope` must not use unsafe code". Empty scope = whole source roots.
    NoUnsafe { scope: Vec<String> },
}

/// A reference found outside any claim (explanatory prose, links). Still checked for
/// dangling targets, because descriptive prose that names removed APIs is drift.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ProseReference {
    pub section: SectionId,
    pub reference: Reference,
}

/// Everything extracted from one Markdown file. Produced by `concordance-markdown`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DocumentIndex {
    pub document: Document,
    pub sections: Vec<Section>,
    /// Claims with ids derived from content (`ClaimId::from_key([path, heading_path.., normalized text, occurrence])`).
    /// The engine may replace an id with a previous one when it matches an edited claim.
    pub claims: Vec<Claim>,
    pub prose_references: Vec<ProseReference>,
}
