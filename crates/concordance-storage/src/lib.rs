//! SQLite persistence.
//!
//! CONTRACT:
//! * One database file per analyzed repository, outside the repository by default.
//! * Schema versioned in `PRAGMA user_version`; migrations are an ordered list applied in
//!   one transaction each; opening a database with a newer version than known is an error.
//! * `commit` applies a whole [`Delta`] in a single transaction: either all of it is
//!   visible after a crash or none of it is. WAL mode, `synchronous=NORMAL`.
//! * Entities are stored as JSON (`serde_json`) in a `data` column, with the key
//!   columns needed for lookups/joins duplicated as real columns.
//! * Source file contents are never stored; only hashes, metadata and bounded excerpts
//!   embedded in entities (evidence citations).
//! * Event history is capped (`EVENT_HISTORY_CAP` most recent state-change events).

use concordance_core::code::{Import, SourceFile, Symbol, WorkspaceInfo};
use concordance_core::doc::{Claim, Document, ProseReference, Section};
use concordance_core::event::{ChangeKind, Envelope, ScanReason};
use concordance_core::evidence::Evidence;
use concordance_core::finding::{Counts, Finding, FindingStatus};
use concordance_core::graph::Edge;
use concordance_core::{
    AnalysisId, ClaimId, DocumentId, EdgeId, EvidenceId, FileId, FindingId, RepoPath, Revision,
    SectionId, SymbolId, Timestamp,
};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::path::Path;

mod ops;
mod schema;

pub const EVENT_HISTORY_CAP: usize = 20_000;

#[derive(Debug)]
pub enum StoreError {
    Sqlite(String),
    Serde(String),
    /// Database was written by a newer Concordance.
    SchemaTooNew {
        found: i64,
        supported: i64,
    },
    Io(String),
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StoreError::Sqlite(e) => write!(f, "sqlite: {e}"),
            StoreError::Serde(e) => write!(f, "serialization: {e}"),
            StoreError::SchemaTooNew { found, supported } => {
                write!(
                    f,
                    "state database schema v{found} is newer than supported v{supported}"
                )
            }
            StoreError::Io(e) => write!(f, "io: {e}"),
        }
    }
}

impl std::error::Error for StoreError {}

impl From<rusqlite::Error> for StoreError {
    fn from(e: rusqlite::Error) -> Self {
        StoreError::Sqlite(e.to_string())
    }
}

impl From<serde_json::Error> for StoreError {
    fn from(e: serde_json::Error) -> Self {
        StoreError::Serde(e.to_string())
    }
}

impl From<std::io::Error> for StoreError {
    fn from(e: std::io::Error) -> Self {
        StoreError::Io(e.to_string())
    }
}

/// One committed revision: what changed and the resulting counts.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RevisionRecord {
    pub revision: Revision,
    pub at: Timestamp,
    pub reason: ScanReason,
    pub changed: Vec<(RepoPath, ChangeKind)>,
    pub counts: Counts,
}

/// A finding status change. `to = None` means resolved; `from = None` means created.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FindingTransition {
    pub finding: FindingId,
    pub revision: Revision,
    pub at: Timestamp,
    pub from: Option<FindingStatus>,
    pub to: Option<FindingStatus>,
    pub title: String,
}

/// A cached semantic analysis result. `payload` is the semantic crate's serialized
/// result; storage does not interpret it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AnalysisRecord {
    /// Cache key: hash of prompt version, mode, model and all input content hashes.
    pub key: String,
    pub analysis: AnalysisId,
    pub claim: ClaimId,
    pub revision: Revision,
    pub created_at: Timestamp,
    pub payload: serde_json::Value,
}

/// Everything changed by one engine commit. Deletes are applied before upserts.
#[derive(Clone, Debug, Default)]
pub struct Delta {
    pub revision: Revision,
    /// Highest event seq included in this commit (persisted as meta for restart).
    pub last_seq: u64,
    pub workspace: Option<WorkspaceInfo>,
    /// Version of the extractors that produced the indexed entities. When it differs
    /// from the running build, every file is re-parsed.
    pub extractor_version: Option<String>,

    pub delete_documents: Vec<DocumentId>,
    pub upsert_documents: Vec<Document>,
    pub delete_sections: Vec<SectionId>,
    pub upsert_sections: Vec<Section>,
    pub delete_claims: Vec<ClaimId>,
    pub upsert_claims: Vec<Claim>,
    /// Replaces the full prose-reference list of each listed document.
    pub prose_references: Vec<(DocumentId, Vec<ProseReference>)>,

    pub delete_files: Vec<FileId>,
    pub upsert_files: Vec<SourceFile>,
    pub delete_symbols: Vec<SymbolId>,
    pub upsert_symbols: Vec<Symbol>,
    /// Replaces the full import list of each listed file.
    pub imports: Vec<(FileId, Vec<Import>)>,

    pub delete_edges: Vec<EdgeId>,
    pub upsert_edges: Vec<Edge>,
    pub delete_evidence: Vec<EvidenceId>,
    pub upsert_evidence: Vec<Evidence>,
    /// Findings are never deleted; resolved findings are retained with `Lifecycle::Resolved`.
    pub upsert_findings: Vec<Finding>,
    pub finding_transitions: Vec<FindingTransition>,

    pub revision_record: Option<RevisionRecord>,
    /// State-change events published for this commit (persisted to capped history).
    pub events: Vec<Envelope>,
}

/// Complete persisted state, as loaded at startup.
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub revision: Revision,
    pub last_seq: u64,
    pub workspace: Option<WorkspaceInfo>,
    pub extractor_version: Option<String>,
    pub documents: Vec<Document>,
    pub sections: Vec<Section>,
    pub claims: Vec<Claim>,
    pub prose_references: Vec<(DocumentId, Vec<ProseReference>)>,
    pub files: Vec<SourceFile>,
    pub symbols: Vec<Symbol>,
    pub imports: Vec<(FileId, Vec<Import>)>,
    pub edges: Vec<Edge>,
    pub evidence: Vec<Evidence>,
    pub findings: Vec<Finding>,
}

pub struct Store {
    conn: Connection,
}

impl Store {
    /// Open (creating directories and the file if needed) and migrate.
    pub fn open(path: &Path) -> Result<Store, StoreError> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        let mut conn = Connection::open(path)?;
        conn.execute_batch(
            "PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL; PRAGMA foreign_keys = OFF;",
        )?;
        schema::migrate(&mut conn)?;
        Ok(Store { conn })
    }

    /// In-memory database (tests).
    pub fn open_in_memory() -> Result<Store, StoreError> {
        let mut conn = Connection::open_in_memory()?;
        conn.execute_batch("PRAGMA synchronous = NORMAL; PRAGMA foreign_keys = OFF;")?;
        schema::migrate(&mut conn)?;
        Ok(Store { conn })
    }

    pub fn load(&self) -> Result<Snapshot, StoreError> {
        ops::load(&self.conn)
    }

    /// Apply a delta atomically.
    pub fn commit(&mut self, delta: &Delta) -> Result<(), StoreError> {
        let tx = self.conn.transaction()?;
        ops::commit(&tx, delta)?;
        tx.commit()?;
        Ok(())
    }

    /// Persisted events with `seq > after`, oldest first, at most `limit`.
    pub fn events_after(&self, after: u64, limit: usize) -> Result<Vec<Envelope>, StoreError> {
        ops::events_after(&self.conn, after, limit)
    }

    /// Most recent revisions first.
    pub fn revisions(
        &self,
        limit: usize,
        before: Option<Revision>,
    ) -> Result<Vec<RevisionRecord>, StoreError> {
        ops::revisions(&self.conn, limit, before)
    }

    /// Transitions of one finding (oldest first), or of all findings (most recent first) when `None`.
    pub fn finding_transitions(
        &self,
        finding: Option<&FindingId>,
        limit: usize,
    ) -> Result<Vec<FindingTransition>, StoreError> {
        ops::finding_transitions(&self.conn, finding, limit)
    }

    pub fn cached_analysis(&self, key: &str) -> Result<Option<AnalysisRecord>, StoreError> {
        ops::cached_analysis(&self.conn, key)
    }

    pub fn put_analysis(&mut self, record: &AnalysisRecord) -> Result<(), StoreError> {
        ops::put_analysis(&self.conn, record)
    }

    /// All analysis records for a claim, newest first.
    pub fn analyses_for_claim(
        &self,
        claim: &ClaimId,
        limit: usize,
    ) -> Result<Vec<AnalysisRecord>, StoreError> {
        ops::analyses_for_claim(&self.conn, claim, limit)
    }
}
