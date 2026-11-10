//! JSON schemas sent to the model and the strict parse of its structured output.

use crate::model::{AnalysisError, ChallengeOutcome, Verdict};
use serde::Deserialize;
use serde_json::{json, Value};

fn code_citation_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["source", "path", "start_line", "end_line", "quote", "note"],
        "properties": {
            "source": { "type": "string", "enum": ["code", "passage"] },
            "path": { "type": "string" },
            "start_line": { "type": "integer" },
            "end_line": { "type": "integer" },
            "quote": { "type": "string" },
            "note": { "type": "string" }
        }
    })
}

fn citation_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["source", "excerpt_id", "start_line", "end_line", "quote", "note"],
        "properties": {
            "source": { "type": "string", "enum": ["excerpt", "passage"] },
            "excerpt_id": { "type": "string" },
            "start_line": { "type": "integer" },
            "end_line": { "type": "integer" },
            "quote": { "type": "string" },
            "note": { "type": "string" }
        }
    })
}

fn base_properties() -> serde_json::Map<String, Value> {
    let v = json!({
        "verdict": { "type": "string", "enum": ["agrees", "disagrees", "ambiguous", "insufficient_evidence"] },
        "explanation": { "type": "string" },
        "supporting": { "type": "array", "items": citation_schema() },
        "contradicting": { "type": "array", "items": citation_schema() },
        "ambiguities": { "type": "array", "items": { "type": "string" } },
        "needed_context": {
            "type": "array",
            "items": {
                "type": "object",
                "additionalProperties": false,
                "required": ["symbol_path", "reason"],
                "properties": {
                    "symbol_path": { "type": "string" },
                    "reason": { "type": "string" }
                }
            }
        }
    });
    match v {
        Value::Object(m) => m,
        _ => unreachable!(),
    }
}

const BASE_REQUIRED: [&str; 6] = [
    "verdict",
    "explanation",
    "supporting",
    "contradicting",
    "ambiguities",
    "needed_context",
];

/// Explore mode: citations name files, and there is no `needed_context` (the model
/// reads what it needs).
fn explore_properties() -> serde_json::Map<String, Value> {
    let mut m = base_properties();
    m.remove("needed_context");
    m.insert(
        "supporting".into(),
        json!({ "type": "array", "items": code_citation_schema() }),
    );
    m.insert(
        "contradicting".into(),
        json!({ "type": "array", "items": code_citation_schema() }),
    );
    m
}

fn schema_with(explore: bool, challenge: bool) -> Value {
    let mut props = if explore {
        explore_properties()
    } else {
        base_properties()
    };
    let mut required: Vec<&str> = BASE_REQUIRED
        .iter()
        .copied()
        .filter(|r| !(explore && *r == "needed_context"))
        .collect();
    if challenge {
        props.insert(
            "challenge_outcome".into(),
            json!({ "type": "string", "enum": ["upheld", "refuted", "inconclusive"] }),
        );
        required.push("challenge_outcome");
    }
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": required,
        "properties": props
    })
}

/// The schema for one request.
pub fn schema_for(explore: bool, challenge: bool) -> Value {
    schema_with(explore, challenge)
}

/// Schema for Layer B.
pub fn assess_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": BASE_REQUIRED,
        "properties": base_properties()
    })
}

/// Schema for Layer C: the assess schema plus `challenge_outcome`.
pub fn challenge_schema() -> Value {
    let mut props = base_properties();
    props.insert(
        "challenge_outcome".into(),
        json!({ "type": "string", "enum": ["upheld", "refuted", "inconclusive"] }),
    );
    let mut required: Vec<&str> = BASE_REQUIRED.to_vec();
    required.push("challenge_outcome");
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": required,
        "properties": props
    })
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CitationSource {
    Excerpt,
    Passage,
    /// Explore mode: a repository file by path.
    Code,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawCitation {
    pub source: CitationSource,
    #[serde(default)]
    pub excerpt_id: String,
    /// Explore mode: the cited file.
    #[serde(default)]
    pub path: String,
    pub start_line: i64,
    pub end_line: i64,
    pub quote: String,
    pub note: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(dead_code)]
pub struct RawNeeded {
    pub symbol_path: String,
    pub reason: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AssessWire {
    verdict: Verdict,
    explanation: String,
    supporting: Vec<RawCitation>,
    contradicting: Vec<RawCitation>,
    ambiguities: Vec<String>,
    #[serde(default)]
    needed_context: Vec<RawNeeded>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ChallengeWire {
    verdict: Verdict,
    explanation: String,
    supporting: Vec<RawCitation>,
    contradicting: Vec<RawCitation>,
    ambiguities: Vec<String>,
    #[serde(default)]
    needed_context: Vec<RawNeeded>,
    challenge_outcome: ChallengeOutcome,
}

/// Parsed (but not yet validated) model output.
#[derive(Clone, Debug)]
pub struct RawAssessment {
    pub verdict: Verdict,
    pub explanation: String,
    pub supporting: Vec<RawCitation>,
    pub contradicting: Vec<RawCitation>,
    pub ambiguities: Vec<String>,
    pub needed_context: Vec<RawNeeded>,
    pub challenge_outcome: Option<ChallengeOutcome>,
}

fn invalid(e: serde_json::Error) -> AnalysisError {
    AnalysisError::InvalidResponse(e.to_string())
}

pub fn parse_assess(v: &Value) -> Result<RawAssessment, AnalysisError> {
    let w = AssessWire::deserialize(v).map_err(invalid)?;
    Ok(RawAssessment {
        verdict: w.verdict,
        explanation: w.explanation,
        supporting: w.supporting,
        contradicting: w.contradicting,
        ambiguities: w.ambiguities,
        needed_context: w.needed_context,
        challenge_outcome: None,
    })
}

pub fn parse_challenge(v: &Value) -> Result<RawAssessment, AnalysisError> {
    let w = ChallengeWire::deserialize(v).map_err(invalid)?;
    Ok(RawAssessment {
        verdict: w.verdict,
        explanation: w.explanation,
        supporting: w.supporting,
        contradicting: w.contradicting,
        ambiguities: w.ambiguities,
        needed_context: w.needed_context,
        challenge_outcome: Some(w.challenge_outcome),
    })
}
