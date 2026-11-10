//! Public types of the semantic layer. The engine depends only on these.

use concordance_core::code::{Fact, SymbolKind};
use concordance_core::doc::{Authority, Claim};
use concordance_core::evidence::{CitationRole, Dependency};
use concordance_core::{AnalysisId, ContentHash, RepoPath, SourceRange, SymbolId};
use serde::{Deserialize, Serialize};
use std::future::Future;
use std::pin::Pin;

/// Bump whenever prompts, schemas or citation-validation rules change; part of every
/// cache key and of the engine's persisted analysis version.
pub const PROMPT_VERSION: &str = "concordance-semantic-v1";
pub const MAX_ROUNDS: u32 = 2;
pub const MAX_EXPANSIONS_PER_ROUND: usize = 4;
/// Upper bound on excerpt text sent per request (bytes, all excerpts together).
pub const CONTEXT_BUDGET_BYTES: usize = 48_000;

pub type CancelRx = tokio::sync::watch::Receiver<bool>;
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// The claim under analysis plus the passage it came from.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ClaimContext {
    pub claim: Claim,
    pub document_path: RepoPath,
    pub document_authority: Authority,
    /// Exact Markdown lines around the claim (the claim's block, plus a few lines).
    pub passage_start_line: u32,
    pub passage: Vec<String>,
}

/// One symbol's compact representation plus its exact current source lines.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SymbolContext {
    pub symbol: SymbolId,
    pub qualified_name: String,
    pub kind: SymbolKind,
    pub file: RepoPath,
    /// Hash of the file content the excerpt was read from (must equal the indexed hash).
    pub file_hash: ContentHash,
    /// The symbol's own content hash (an evidence dependency).
    pub content_hash: ContentHash,
    pub signature: Option<String>,
    pub doc: Option<String>,
    pub facts: Vec<Fact>,
    /// 1-based line number of `lines[0]`.
    pub start_line: u32,
    pub lines: Vec<String>,
    /// Why this symbol was included ("explicit reference `Foo::bar`", "lexical match", "callee of ...").
    pub reason: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AnalysisMode {
    /// Layer B.
    Assess,
    /// Layer C: try to refute `prior`.
    Challenge { prior: Assessment },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AnalysisRequest {
    pub id: AnalysisId,
    pub claim: ClaimContext,
    pub symbols: Vec<SymbolContext>,
    pub mode: AnalysisMode,
    pub model: String,
}

/// Additional context the model asked for.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ContextWanted {
    /// A symbol by (partial) path, e.g. `Limiter::refill`.
    Symbol { path: String },
}

/// Supplied by the engine: resolves context requests against the current model,
/// reading current file contents (verifying hashes).
pub trait ContextProvider: Send + Sync {
    fn expand(&self, wanted: &ContextWanted) -> Option<SymbolContext>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Agrees,
    Disagrees,
    Ambiguous,
    InsufficientEvidence,
}

impl Verdict {
    pub fn as_str(self) -> &'static str {
        match self {
            Verdict::Agrees => "agrees",
            Verdict::Disagrees => "disagrees",
            Verdict::Ambiguous => "ambiguous",
            Verdict::InsufficientEvidence => "insufficient_evidence",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChallengeOutcome {
    /// The challenger could not refute the prior assessment.
    Upheld,
    /// The challenger found a validated counterexample or missing evidence.
    Refuted,
    Inconclusive,
}

/// A citation that passed validation against the sent excerpts.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ValidatedCitation {
    pub role: CitationRole,
    /// `None` for specification citations.
    pub symbol: Option<SymbolId>,
    pub range: SourceRange,
    /// The exact lines (joined with `\n`) from the excerpt, not the model's quote.
    pub excerpt: String,
    /// The model's short note on why this location matters (bounded, <= 300 chars).
    pub note: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Assessment {
    pub verdict: Verdict,
    /// Concise explanation (<= 1200 chars, truncated if longer).
    pub explanation: String,
    pub supporting: Vec<ValidatedCitation>,
    pub contradicting: Vec<ValidatedCitation>,
    pub ambiguities: Vec<String>,
    /// Citations dropped by validation, with reasons.
    pub rejected: Vec<String>,
    /// Set when the verdict was downgraded by validation.
    pub downgraded: Option<String>,
    /// Set in `Challenge` mode.
    pub challenge: Option<ChallengeOutcome>,
}

/// Metadata about the model invocation(s). Never contains raw model reasoning.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InvocationRecord {
    pub backend: String,
    pub requested_model: String,
    /// Model reported by the CLI (`modelUsage` key), if any.
    pub reported_model: Option<String>,
    pub rounds: u32,
    pub duration_ms: u64,
    pub cost_usd: Option<f64>,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub session_ids: Vec<String>,
    /// Hash of the final prompt text.
    pub prompt_hash: ContentHash,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AnalysisResult {
    pub id: AnalysisId,
    pub assessment: Assessment,
    pub invocation: InvocationRecord,
    /// Content the result depends on: the claim and every symbol whose excerpt was sent
    /// (including expansions), with the hashes they had.
    pub dependencies: Vec<Dependency>,
}

#[derive(Debug, Clone)]
pub enum AnalysisError {
    /// CLI missing or not authenticated.
    Unavailable(String),
    Cancelled,
    Timeout,
    Invocation(String),
    InvalidResponse(String),
}

impl std::fmt::Display for AnalysisError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AnalysisError::Unavailable(e) => write!(f, "semantic backend unavailable: {e}"),
            AnalysisError::Cancelled => write!(f, "cancelled"),
            AnalysisError::Timeout => write!(f, "timed out"),
            AnalysisError::Invocation(e) => write!(f, "invocation failed: {e}"),
            AnalysisError::InvalidResponse(e) => write!(f, "invalid response: {e}"),
        }
    }
}

/// One structured-output request to a model.
pub struct BackendRequest<'a> {
    pub system: &'a str,
    pub prompt: &'a str,
    pub schema: &'a serde_json::Value,
    pub model: &'a str,
}

pub struct BackendResponse {
    pub structured: serde_json::Value,
    pub reported_model: Option<String>,
    pub duration_ms: u64,
    pub cost_usd: Option<f64>,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub session_id: Option<String>,
}

/// A structured-output model backend. `ClaudeCli` in production; tests script responses.
pub trait Backend: Send + Sync {
    fn name(&self) -> &str;
    fn invoke<'a>(
        &'a self,
        req: BackendRequest<'a>,
        cancel: CancelRx,
    ) -> BoxFuture<'a, Result<BackendResponse, AnalysisError>>;
}
