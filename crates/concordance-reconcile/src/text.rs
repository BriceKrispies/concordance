//! Small text helpers.

use concordance_core::doc::Claim;
use concordance_core::evidence::{
    Citation, CitationRole, Dependency, Evidence, Method, Stance, Strength, Validity,
};
use concordance_core::graph::NodeRef;
use concordance_core::{EvidenceId, SourceRange};

use crate::BatchContext;

pub const MAX_EXCERPT: usize = 2000;

pub fn bounded(s: &str) -> String {
    concordance_graph::truncate(s, MAX_EXCERPT)
}

/// Levenshtein distance on chars (inputs are short identifiers).
pub fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0; b.len() + 1];
    for i in 1..=a.len() {
        cur[0] = i;
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

/// Are two identifiers plausibly a rename of each other?
pub fn similar_name(a: &str, b: &str) -> bool {
    if a == b {
        return false;
    }
    let (la, lb) = (a.to_lowercase(), b.to_lowercase());
    let d = edit_distance(&la, &lb);
    let max = la.len().max(lb.len());
    let min = la.len().min(lb.len());
    let contained = (la.contains(&lb) || lb.contains(&la)) && min * 10 >= max * 7;
    d <= 2 || (max >= 6 && (d as f32 / max as f32) <= 0.3) || contained
}

pub fn spec_citation(claim: &Claim, range: Option<&SourceRange>) -> Citation {
    Citation {
        role: CitationRole::Specification,
        range: range.cloned().unwrap_or_else(|| claim.range.clone()),
        excerpt: bounded(&claim.text),
        node: Some(NodeRef::Claim(claim.id.clone())),
    }
}

pub struct EvidenceBuilder<'a> {
    pub ctx: &'a BatchContext,
    pub subject: NodeRef,
    pub method: Method,
}

impl EvidenceBuilder<'_> {
    pub fn build(
        &self,
        key: &str,
        stance: Stance,
        strength: Strength,
        summary: String,
        citations: Vec<Citation>,
        dependencies: Vec<Dependency>,
    ) -> Evidence {
        let method_key = serde_json::to_string(&self.method).unwrap_or_default();
        Evidence {
            id: EvidenceId::from_key(&[self.subject.as_str(), &method_key, key]),
            subject: self.subject.clone(),
            stance,
            strength,
            method: self.method.clone(),
            summary,
            citations,
            dependencies,
            validity: Validity::Valid,
            revision: self.ctx.revision,
            created_at: self.ctx.now,
        }
    }
}
