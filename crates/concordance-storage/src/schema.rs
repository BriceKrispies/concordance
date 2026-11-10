//! Schema migrations. Each entry in [`MIGRATIONS`] is applied exactly once, in its own
//! transaction, together with the `user_version` bump that records it.

use crate::StoreError;
use rusqlite::Connection;

/// Ordered migration batches. `user_version` equals the number of applied batches.
pub(crate) const MIGRATIONS: &[&str] = &[r#"
CREATE TABLE meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE documents (
    id   TEXT PRIMARY KEY,
    path TEXT NOT NULL,
    data TEXT NOT NULL
);
CREATE INDEX documents_path ON documents(path);

CREATE TABLE sections (
    id          TEXT PRIMARY KEY,
    document_id TEXT NOT NULL,
    data        TEXT NOT NULL
);
CREATE INDEX sections_document ON sections(document_id);

CREATE TABLE claims (
    id          TEXT PRIMARY KEY,
    document_id TEXT NOT NULL,
    data        TEXT NOT NULL
);
CREATE INDEX claims_document ON claims(document_id);

CREATE TABLE prose_refs (
    document_id TEXT PRIMARY KEY,
    data        TEXT NOT NULL
);

CREATE TABLE files (
    id   TEXT PRIMARY KEY,
    path TEXT NOT NULL,
    data TEXT NOT NULL
);
CREATE INDEX files_path ON files(path);

CREATE TABLE symbols (
    id      TEXT PRIMARY KEY,
    file_id TEXT NOT NULL,
    data    TEXT NOT NULL
);
CREATE INDEX symbols_file ON symbols(file_id);

CREATE TABLE imports (
    file_id TEXT PRIMARY KEY,
    data    TEXT NOT NULL
);

CREATE TABLE edges (
    id      TEXT PRIMARY KEY,
    from_id TEXT NOT NULL,
    to_id   TEXT,
    data    TEXT NOT NULL
);
CREATE INDEX edges_from ON edges(from_id);
CREATE INDEX edges_to ON edges(to_id);

CREATE TABLE evidence (
    id      TEXT PRIMARY KEY,
    subject TEXT NOT NULL,
    data    TEXT NOT NULL
);
CREATE INDEX evidence_subject ON evidence(subject);

CREATE TABLE findings (
    id     TEXT PRIMARY KEY,
    status TEXT NOT NULL,
    data   TEXT NOT NULL
);
CREATE INDEX findings_status ON findings(status);

CREATE TABLE finding_transitions (
    finding_id TEXT NOT NULL,
    revision   INTEGER NOT NULL,
    at         INTEGER NOT NULL,
    data       TEXT NOT NULL
);
CREATE INDEX finding_transitions_finding ON finding_transitions(finding_id);

CREATE TABLE revisions (
    revision INTEGER PRIMARY KEY,
    at       INTEGER NOT NULL,
    data     TEXT NOT NULL
);

CREATE TABLE events (
    seq  INTEGER PRIMARY KEY,
    data TEXT NOT NULL
);

CREATE TABLE analyses (
    key         TEXT PRIMARY KEY,
    analysis_id TEXT NOT NULL,
    claim_id    TEXT NOT NULL,
    revision    INTEGER NOT NULL,
    created_at  INTEGER NOT NULL,
    data        TEXT NOT NULL
);
CREATE INDEX analyses_claim ON analyses(claim_id);
"#];

/// Bring the database up to the newest known schema.
pub(crate) fn migrate(conn: &mut Connection) -> Result<(), StoreError> {
    let found: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    let supported = MIGRATIONS.len() as i64;
    if found > supported {
        return Err(StoreError::SchemaTooNew { found, supported });
    }
    let start = usize::try_from(found).unwrap_or(0);
    for (index, sql) in MIGRATIONS.iter().enumerate().skip(start) {
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        // `index + 1` is a trusted integer, so inlining it into the pragma is safe.
        tx.execute_batch(&format!("PRAGMA user_version = {}", index + 1))?;
        tx.commit()?;
    }
    Ok(())
}
