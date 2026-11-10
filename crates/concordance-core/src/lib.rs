//! Concordance domain model.
//!
//! This crate is the single architectural contract between subsystems. It has no
//! knowledge of HTTP, SQLite, Markdown syntax trees or `syn` ASTs: extractors
//! translate their inputs into these types, and everything downstream (graph,
//! reconciliation, storage, streaming, UI) speaks only these types.
//!
//! The model keeps four things apart (see `docs/architecture.md`):
//! * **intent**: [`doc::Claim`]s extracted from Markdown,
//! * **implementation**: [`code::Symbol`]s and [`code::Fact`]s extracted from Rust,
//! * **evidence**: [`evidence::Evidence`] linking the two, with provenance and validity,
//! * **uncertainty / drift**: [`finding::Finding`]s, whose status never claims more
//!   than their evidence supports.

pub mod code;
pub mod config;
pub mod doc;
pub mod event;
pub mod evidence;
pub mod finding;
pub mod graph;
pub mod ids;
pub mod lang;
pub mod location;
pub mod testrun;

pub use ids::*;
pub use location::*;

/// Milliseconds since the Unix epoch.
#[derive(
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Default,
    serde::Serialize,
    serde::Deserialize,
)]
#[serde(transparent)]
pub struct Timestamp(pub u64);

impl Timestamp {
    pub fn now() -> Self {
        let ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        Timestamp(ms)
    }
}

/// Monotonic repository-model revision. Bumped once per committed change batch
/// (initial scan, watch batch, applied analysis). Every evidence record and finding
/// states the revision it was computed at.
#[derive(
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Default,
    serde::Serialize,
    serde::Deserialize,
)]
#[serde(transparent)]
pub struct Revision(pub u64);

impl Revision {
    pub fn next(self) -> Self {
        Revision(self.0 + 1)
    }
}
