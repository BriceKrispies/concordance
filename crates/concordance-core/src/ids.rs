//! Strongly typed, stable identities.
//!
//! Every id is `<prefix>_<16 hex chars>`, derived from a structural key with blake3.
//! The prefix makes ids self-describing, so a bare id string can be turned back into a
//! [`crate::graph::NodeRef`] without extra tagging.

use serde::{Deserialize, Serialize};
use std::fmt;

/// 64-bit blake3 digest of `parts`, separated by NUL, as 16 lowercase hex chars.
pub fn short_hash(parts: &[&str]) -> String {
    let mut h = blake3::Hasher::new();
    for (i, p) in parts.iter().enumerate() {
        if i > 0 {
            h.update(&[0]);
        }
        h.update(p.as_bytes());
    }
    h.finalize().to_hex()[..16].to_string()
}

macro_rules! id_type {
    ($(#[$m:meta])* $name:ident, $prefix:literal) => {
        $(#[$m])*
        #[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub const PREFIX: &'static str = $prefix;

            /// Derive the id from a structural key.
            pub fn from_key(parts: &[&str]) -> Self {
                Self(format!("{}_{}", $prefix, short_hash(parts)))
            }

            /// Accept an id string only if it carries this type's prefix and shape.
            pub fn parse(s: &str) -> Option<Self> {
                let rest = s.strip_prefix(concat!($prefix, "_"))?;
                (rest.len() == 16 && rest.bytes().all(|b| b.is_ascii_hexdigit()))
                    .then(|| Self(s.to_owned()))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

id_type!(
    /// A Markdown document, keyed by repository path.
    DocumentId, "doc");
id_type!(
    /// A heading-delimited section of a document.
    SectionId, "sec");
id_type!(
    /// A claim. Assigned at first extraction and carried across edits by the engine's
    /// claim matcher when the match is unambiguous.
    ClaimId, "clm");
id_type!(
    /// A Rust source file, keyed by repository path.
    FileId, "src");
id_type!(
    /// A Rust symbol, keyed by kind + crate-qualified path + disambiguator (never by line).
    SymbolId, "sym");
id_type!(
    /// A typed relationship edge.
    EdgeId, "rel");
id_type!(
    /// An evidence record, keyed by subject + method + target.
    EvidenceId, "evd");
id_type!(
    /// A finding, keyed by what it is about, so it is stable across scans.
    FindingId, "fnd");
id_type!(
    /// One semantic-analysis invocation (or cached reuse).
    AnalysisId, "ana");

/// blake3 content hash (32 hex chars = 128 bits) of normalized content.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ContentHash(String);

impl ContentHash {
    pub fn of(bytes: &[u8]) -> Self {
        ContentHash(blake3::hash(bytes).to_hex()[..32].to_string())
    }
    pub fn of_str(s: &str) -> Self {
        Self::of(s.as_bytes())
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ContentHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_stable_prefixed_and_parse_back() {
        let a = ClaimId::from_key(&["docs/a.md", "Intro", "must be fast"]);
        assert_eq!(
            a,
            ClaimId::from_key(&["docs/a.md", "Intro", "must be fast"])
        );
        assert!(a.as_str().starts_with("clm_"));
        assert_eq!(ClaimId::parse(a.as_str()), Some(a.clone()));
        assert_eq!(SymbolId::parse(a.as_str()), None);
        // Part boundaries matter: ["ab","c"] != ["a","bc"].
        assert_ne!(
            ClaimId::from_key(&["ab", "c"]),
            ClaimId::from_key(&["a", "bc"])
        );
    }
}
