//! SQL behind [`crate::Store`]: atomic commit, full load and the query methods.

use crate::{
    AnalysisRecord, Delta, FindingTransition, RevisionRecord, Snapshot, StoreError,
    EVENT_HISTORY_CAP,
};
use concordance_core::code::{Import, SourceFile, Symbol, WorkspaceInfo};
use concordance_core::doc::{Claim, Document, ProseReference, Section};
use concordance_core::event::Envelope;
use concordance_core::evidence::Evidence;
use concordance_core::finding::Finding;
use concordance_core::graph::Edge;
use concordance_core::{ClaimId, DocumentId, FileId, FindingId, Revision};
use rusqlite::{params, Connection, OptionalExtension, Params, Transaction};
use serde::de::DeserializeOwned;
use serde::Serialize;

fn js<T: Serialize + ?Sized>(value: &T) -> Result<String, StoreError> {
    Ok(serde_json::to_string(value)?)
}

fn decode<T: DeserializeOwned>(raw: &str) -> Result<T, StoreError> {
    Ok(serde_json::from_str(raw)?)
}

/// Turn a stored key column back into an id type (all id types are transparent strings).
fn key<T: DeserializeOwned>(raw: String) -> Result<T, StoreError> {
    Ok(serde_json::from_value(serde_json::Value::String(raw))?)
}

/// Clamp a `usize` limit to the SQLite integer range.
fn lim(limit: usize) -> i64 {
    i64::try_from(limit).unwrap_or(i64::MAX)
}

fn put_meta(tx: &Transaction<'_>, name: &str, value: &str) -> Result<(), StoreError> {
    let mut stmt = tx.prepare_cached(
        "INSERT INTO meta(key, value) VALUES(?1, ?2) \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )?;
    stmt.execute(params![name, value])?;
    Ok(())
}

/// Apply one delta inside `tx`. The caller commits.
pub(crate) fn commit(tx: &Transaction<'_>, d: &Delta) -> Result<(), StoreError> {
    // 1. Deletes first.
    for id in &d.delete_documents {
        tx.prepare_cached("DELETE FROM documents WHERE id = ?1")?
            .execute([id.as_str()])?;
        tx.prepare_cached("DELETE FROM prose_refs WHERE document_id = ?1")?
            .execute([id.as_str()])?;
    }
    for id in &d.delete_sections {
        tx.prepare_cached("DELETE FROM sections WHERE id = ?1")?
            .execute([id.as_str()])?;
    }
    for id in &d.delete_claims {
        tx.prepare_cached("DELETE FROM claims WHERE id = ?1")?
            .execute([id.as_str()])?;
    }
    for id in &d.delete_files {
        tx.prepare_cached("DELETE FROM files WHERE id = ?1")?
            .execute([id.as_str()])?;
        tx.prepare_cached("DELETE FROM imports WHERE file_id = ?1")?
            .execute([id.as_str()])?;
    }
    for id in &d.delete_symbols {
        tx.prepare_cached("DELETE FROM symbols WHERE id = ?1")?
            .execute([id.as_str()])?;
    }
    for id in &d.delete_edges {
        tx.prepare_cached("DELETE FROM edges WHERE id = ?1")?
            .execute([id.as_str()])?;
    }
    for id in &d.delete_evidence {
        tx.prepare_cached("DELETE FROM evidence WHERE id = ?1")?
            .execute([id.as_str()])?;
    }

    // 2. Upserts.
    let mut stmt = tx.prepare_cached(
        "INSERT INTO documents(id, path, data) VALUES(?1, ?2, ?3) \
         ON CONFLICT(id) DO UPDATE SET path = excluded.path, data = excluded.data",
    )?;
    for doc in &d.upsert_documents {
        stmt.execute(params![doc.id.as_str(), doc.path.as_str(), js(doc)?])?;
    }
    drop(stmt);

    let mut stmt = tx.prepare_cached(
        "INSERT INTO sections(id, document_id, data) VALUES(?1, ?2, ?3) \
         ON CONFLICT(id) DO UPDATE SET document_id = excluded.document_id, data = excluded.data",
    )?;
    for section in &d.upsert_sections {
        stmt.execute(params![
            section.id.as_str(),
            section.document.as_str(),
            js(section)?
        ])?;
    }
    drop(stmt);

    let mut stmt = tx.prepare_cached(
        "INSERT INTO claims(id, document_id, data) VALUES(?1, ?2, ?3) \
         ON CONFLICT(id) DO UPDATE SET document_id = excluded.document_id, data = excluded.data",
    )?;
    for claim in &d.upsert_claims {
        stmt.execute(params![
            claim.id.as_str(),
            claim.document.as_str(),
            js(claim)?
        ])?;
    }
    drop(stmt);

    let mut stmt = tx.prepare_cached(
        "INSERT INTO files(id, path, data) VALUES(?1, ?2, ?3) \
         ON CONFLICT(id) DO UPDATE SET path = excluded.path, data = excluded.data",
    )?;
    for file in &d.upsert_files {
        stmt.execute(params![file.id.as_str(), file.path.as_str(), js(file)?])?;
    }
    drop(stmt);

    let mut stmt = tx.prepare_cached(
        "INSERT INTO symbols(id, file_id, data) VALUES(?1, ?2, ?3) \
         ON CONFLICT(id) DO UPDATE SET file_id = excluded.file_id, data = excluded.data",
    )?;
    for symbol in &d.upsert_symbols {
        stmt.execute(params![
            symbol.id.as_str(),
            symbol.file.as_str(),
            js(symbol)?
        ])?;
    }
    drop(stmt);

    let mut stmt = tx.prepare_cached(
        "INSERT INTO edges(id, from_id, to_id, data) VALUES(?1, ?2, ?3, ?4) \
         ON CONFLICT(id) DO UPDATE SET from_id = excluded.from_id, to_id = excluded.to_id, data = excluded.data",
    )?;
    for edge in &d.upsert_edges {
        let to = edge.to.as_ref().map(|n| n.as_str());
        stmt.execute(params![edge.id.as_str(), edge.from.as_str(), to, js(edge)?])?;
    }
    drop(stmt);

    let mut stmt = tx.prepare_cached(
        "INSERT INTO evidence(id, subject, data) VALUES(?1, ?2, ?3) \
         ON CONFLICT(id) DO UPDATE SET subject = excluded.subject, data = excluded.data",
    )?;
    for ev in &d.upsert_evidence {
        stmt.execute(params![ev.id.as_str(), ev.subject.as_str(), js(ev)?])?;
    }
    drop(stmt);

    let mut stmt = tx.prepare_cached(
        "INSERT INTO findings(id, status, data) VALUES(?1, ?2, ?3) \
         ON CONFLICT(id) DO UPDATE SET status = excluded.status, data = excluded.data",
    )?;
    for finding in &d.upsert_findings {
        stmt.execute(params![
            finding.id.as_str(),
            finding.status.as_str(),
            js(finding)?
        ])?;
    }
    drop(stmt);

    // 3. Replace-per-key lists, transitions, revision record, events, meta.
    let mut stmt = tx.prepare_cached(
        "INSERT INTO prose_refs(document_id, data) VALUES(?1, ?2) \
         ON CONFLICT(document_id) DO UPDATE SET data = excluded.data",
    )?;
    for (document, refs) in &d.prose_references {
        stmt.execute(params![document.as_str(), js(refs)?])?;
    }
    drop(stmt);

    let mut stmt = tx.prepare_cached(
        "INSERT INTO imports(file_id, data) VALUES(?1, ?2) \
         ON CONFLICT(file_id) DO UPDATE SET data = excluded.data",
    )?;
    for (file, imports) in &d.imports {
        stmt.execute(params![file.as_str(), js(imports)?])?;
    }
    drop(stmt);

    let mut stmt = tx.prepare_cached(
        "INSERT INTO finding_transitions(finding_id, revision, at, data) VALUES(?1, ?2, ?3, ?4)",
    )?;
    for t in &d.finding_transitions {
        stmt.execute(params![
            t.finding.as_str(),
            t.revision.0 as i64,
            t.at.0 as i64,
            js(t)?
        ])?;
    }
    drop(stmt);

    if let Some(record) = &d.revision_record {
        let mut stmt = tx.prepare_cached(
            "INSERT INTO revisions(revision, at, data) VALUES(?1, ?2, ?3) \
             ON CONFLICT(revision) DO UPDATE SET at = excluded.at, data = excluded.data",
        )?;
        stmt.execute(params![
            record.revision.0 as i64,
            record.at.0 as i64,
            js(record)?
        ])?;
    }

    let mut stmt = tx.prepare_cached("INSERT OR IGNORE INTO events(seq, data) VALUES(?1, ?2)")?;
    for event in &d.events {
        stmt.execute(params![event.seq as i64, js(event)?])?;
    }
    drop(stmt);

    put_meta(tx, "revision", &js(&d.revision)?)?;
    put_meta(tx, "last_seq", &js(&d.last_seq)?)?;
    if let Some(workspace) = &d.workspace {
        put_meta(tx, "workspace", &js(workspace)?)?;
    }
    if let Some(v) = &d.extractor_version {
        put_meta(tx, "extractor_version", &js(v)?)?;
    }

    // 4. Cap event history to the newest EVENT_HISTORY_CAP rows.
    tx.execute(
        "DELETE FROM events WHERE seq <= (SELECT seq FROM events ORDER BY seq DESC LIMIT 1 OFFSET ?1)",
        [EVENT_HISTORY_CAP as i64],
    )?;
    Ok(())
}

/// Rows of one JSON `data` column, decoded.
fn json_rows<T: DeserializeOwned, P: Params>(
    conn: &Connection,
    sql: &str,
    params: P,
) -> Result<Vec<T>, StoreError> {
    let mut stmt = conn.prepare_cached(sql)?;
    let rows = stmt.query_map(params, |r| r.get::<_, String>(0))?;
    let mut out = Vec::new();
    for raw in rows {
        out.push(decode(&raw?)?);
    }
    Ok(out)
}

/// Rows of (key column, JSON `data` column), with the key decoded as an id type.
fn keyed_rows<K: DeserializeOwned, V: DeserializeOwned>(
    conn: &Connection,
    sql: &str,
) -> Result<Vec<(K, V)>, StoreError> {
    let mut stmt = conn.prepare_cached(sql)?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
    let mut out = Vec::new();
    for row in rows {
        let (k, v) = row?;
        out.push((key(k)?, decode(&v)?));
    }
    Ok(out)
}

fn meta_get<T: DeserializeOwned>(conn: &Connection, name: &str) -> Result<Option<T>, StoreError> {
    let raw: Option<String> = conn
        .query_row("SELECT value FROM meta WHERE key = ?1", [name], |r| {
            r.get(0)
        })
        .optional()?;
    raw.map(|s| decode(&s)).transpose()
}

pub(crate) fn load(conn: &Connection) -> Result<Snapshot, StoreError> {
    Ok(Snapshot {
        revision: meta_get::<Revision>(conn, "revision")?.unwrap_or_default(),
        last_seq: meta_get::<u64>(conn, "last_seq")?.unwrap_or(0),
        workspace: meta_get::<WorkspaceInfo>(conn, "workspace")?,
        extractor_version: meta_get::<String>(conn, "extractor_version")?,
        documents: json_rows::<Document, _>(conn, "SELECT data FROM documents ORDER BY id", [])?,
        sections: json_rows::<Section, _>(conn, "SELECT data FROM sections ORDER BY id", [])?,
        claims: json_rows::<Claim, _>(conn, "SELECT data FROM claims ORDER BY id", [])?,
        prose_references: keyed_rows::<DocumentId, Vec<ProseReference>>(
            conn,
            "SELECT document_id, data FROM prose_refs ORDER BY document_id",
        )?,
        files: json_rows::<SourceFile, _>(conn, "SELECT data FROM files ORDER BY id", [])?,
        symbols: json_rows::<Symbol, _>(conn, "SELECT data FROM symbols ORDER BY id", [])?,
        imports: keyed_rows::<FileId, Vec<Import>>(
            conn,
            "SELECT file_id, data FROM imports ORDER BY file_id",
        )?,
        edges: json_rows::<Edge, _>(conn, "SELECT data FROM edges ORDER BY id", [])?,
        evidence: json_rows::<Evidence, _>(conn, "SELECT data FROM evidence ORDER BY id", [])?,
        findings: json_rows::<Finding, _>(conn, "SELECT data FROM findings ORDER BY id", [])?,
    })
}

pub(crate) fn events_after(
    conn: &Connection,
    after: u64,
    limit: usize,
) -> Result<Vec<Envelope>, StoreError> {
    json_rows(
        conn,
        "SELECT data FROM events WHERE seq > ?1 ORDER BY seq ASC LIMIT ?2",
        params![after as i64, lim(limit)],
    )
}

pub(crate) fn revisions(
    conn: &Connection,
    limit: usize,
    before: Option<Revision>,
) -> Result<Vec<RevisionRecord>, StoreError> {
    json_rows(
        conn,
        "SELECT data FROM revisions WHERE (?1 IS NULL OR revision < ?1) ORDER BY revision DESC LIMIT ?2",
        params![before.map(|r| r.0 as i64), lim(limit)],
    )
}

pub(crate) fn finding_transitions(
    conn: &Connection,
    finding: Option<&FindingId>,
    limit: usize,
) -> Result<Vec<FindingTransition>, StoreError> {
    match finding {
        Some(id) => json_rows(
            conn,
            "SELECT data FROM finding_transitions WHERE finding_id = ?1 ORDER BY rowid ASC LIMIT ?2",
            params![id.as_str(), lim(limit)],
        ),
        None => json_rows(
            conn,
            "SELECT data FROM finding_transitions ORDER BY rowid DESC LIMIT ?1",
            params![lim(limit)],
        ),
    }
}

pub(crate) fn cached_analysis(
    conn: &Connection,
    key_value: &str,
) -> Result<Option<AnalysisRecord>, StoreError> {
    let raw: Option<String> = conn
        .query_row(
            "SELECT data FROM analyses WHERE key = ?1",
            [key_value],
            |r| r.get(0),
        )
        .optional()?;
    raw.map(|s| decode(&s)).transpose()
}

pub(crate) fn put_analysis(conn: &Connection, record: &AnalysisRecord) -> Result<(), StoreError> {
    let mut stmt = conn.prepare_cached(
        "INSERT INTO analyses(key, analysis_id, claim_id, revision, created_at, data) \
         VALUES(?1, ?2, ?3, ?4, ?5, ?6) \
         ON CONFLICT(key) DO UPDATE SET analysis_id = excluded.analysis_id, claim_id = excluded.claim_id, \
         revision = excluded.revision, created_at = excluded.created_at, data = excluded.data",
    )?;
    stmt.execute(params![
        record.key.as_str(),
        record.analysis.as_str(),
        record.claim.as_str(),
        record.revision.0 as i64,
        record.created_at.0 as i64,
        js(record)?,
    ])?;
    Ok(())
}

pub(crate) fn analyses_for_claim(
    conn: &Connection,
    claim: &ClaimId,
    limit: usize,
) -> Result<Vec<AnalysisRecord>, StoreError> {
    json_rows(
        conn,
        "SELECT data FROM analyses WHERE claim_id = ?1 ORDER BY created_at DESC, revision DESC LIMIT ?2",
        params![claim.as_str(), lim(limit)],
    )
}
