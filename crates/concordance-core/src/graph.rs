//! Typed graph vocabulary: node references and relationship edges.
//!
//! Containment (document > section > claim, file > symbol, symbol > child symbol) is
//! implied by the entities' own fields and is not stored as edges. Edges carry the
//! relationships that need resolution and provenance.

use crate::{ClaimId, DocumentId, EdgeId, FileId, SectionId, SourceRange, SymbolId};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;

/// Reference to any graph node. Serialized as the bare id string (ids are prefixed).
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum NodeRef {
    Document(DocumentId),
    Section(SectionId),
    Claim(ClaimId),
    File(FileId),
    Symbol(SymbolId),
}

impl NodeRef {
    pub fn parse(s: &str) -> Option<Self> {
        let prefix = s.split('_').next()?;
        Some(match prefix {
            DocumentId::PREFIX => NodeRef::Document(DocumentId::parse(s)?),
            SectionId::PREFIX => NodeRef::Section(SectionId::parse(s)?),
            ClaimId::PREFIX => NodeRef::Claim(ClaimId::parse(s)?),
            FileId::PREFIX => NodeRef::File(FileId::parse(s)?),
            SymbolId::PREFIX => NodeRef::Symbol(SymbolId::parse(s)?),
            _ => return None,
        })
    }

    pub fn as_str(&self) -> &str {
        match self {
            NodeRef::Document(i) => i.as_str(),
            NodeRef::Section(i) => i.as_str(),
            NodeRef::Claim(i) => i.as_str(),
            NodeRef::File(i) => i.as_str(),
            NodeRef::Symbol(i) => i.as_str(),
        }
    }

    pub fn kind(&self) -> &'static str {
        match self {
            NodeRef::Document(_) => "document",
            NodeRef::Section(_) => "section",
            NodeRef::Claim(_) => "claim",
            NodeRef::File(_) => "file",
            NodeRef::Symbol(_) => "symbol",
        }
    }
}

impl fmt::Display for NodeRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Serialize for NodeRef {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for NodeRef {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        NodeRef::parse(&s).ok_or_else(|| serde::de::Error::custom(format!("not a node id: {s}")))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgeKind {
    /// Claim/prose explicitly names a Rust symbol (code span or link).
    ReferencesSymbol,
    /// Claim/prose names a repository file.
    ReferencesFile,
    /// Claim/prose links to a Markdown section or document.
    ReferencesSection,
    /// Claim prose lexically matches a symbol name (retrieval candidate only, never proof).
    MentionsSymbol,
    /// Architectural claim applies to every symbol under a module/crate scope.
    ConstrainsScope,
    /// Symbol calls another symbol (syntactic, name-resolved).
    Calls,
    /// Impl block implements a trait.
    Implements,
    /// File imports a symbol (`use`).
    Imports,
    /// Requirement claim -> test function that declares `Verifies: <requirement>`.
    VerifiedBy,
}

/// How an edge's target was determined. Never upgraded beyond what was done.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Resolution {
    /// Full crate-qualified path matched exactly one node.
    Exact,
    /// Path suffix matched exactly one node (`Foo::new` -> `my_crate::foo::Foo::new`).
    UniqueSuffix,
    /// Several nodes matched; the edge points at the best candidate, `candidates` counts all.
    Ambiguous { candidates: u32 },
    /// Lexical name similarity only.
    Lexical,
    /// Nothing matched. `to` is `None`.
    Unresolved,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Edge {
    pub id: EdgeId,
    pub kind: EdgeKind,
    pub from: NodeRef,
    /// `None` only for `Resolution::Unresolved`.
    pub to: Option<NodeRef>,
    /// The reference text that produced this edge (`Foo::bar()`, a link target, a callee path).
    pub target_text: String,
    pub resolution: Resolution,
    /// 0..=1. Exact = 1.0, unique suffix 0.9, ambiguous 1/candidates, lexical <= 0.4.
    pub confidence: f32,
    /// Build configuration the target depends on (target symbol's `cfg` predicates).
    pub cfg: Vec<String>,
    /// Where the reference was written.
    pub origin: Option<SourceRange>,
}

impl Edge {
    /// Stable edge identity: kind + source + reference text (+ resolved target).
    pub fn make_id(
        kind: EdgeKind,
        from: &NodeRef,
        target_text: &str,
        to: Option<&NodeRef>,
    ) -> EdgeId {
        let kind = serde_json::to_string(&kind).unwrap_or_default();
        EdgeId::from_key(&[
            &kind,
            from.as_str(),
            target_text,
            to.map_or("", |t| t.as_str()),
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_refs_round_trip_through_bare_id_strings() {
        let n = NodeRef::Symbol(SymbolId::from_key(&["fn", "a::b"]));
        let json = serde_json::to_string(&n).unwrap();
        assert!(json.starts_with("\"sym_"));
        let back: NodeRef = serde_json::from_str(&json).unwrap();
        assert_eq!(back, n);
        assert!(NodeRef::parse("xyz_0011223344556677").is_none());
    }
}
