//! JSON API handlers.

use crate::views::*;
use crate::AppState;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use concordance_core::evidence::{CitationRole, Evidence, MethodKind};
use concordance_core::finding::{Finding, FindingStatus, FindingSubject, Severity};
use concordance_core::graph::NodeRef;
use concordance_core::{ClaimId, DocumentId, FindingId, SymbolId};
use concordance_engine::source::{excerpt, SourceExcerpt};
use serde::Deserialize;
use serde_json::json;
use std::collections::{BTreeSet, HashMap};

type Q = Query<HashMap<String, String>>;

pub struct ApiError(StatusCode, &'static str, String);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.0,
            Json(json!({ "error": { "code": self.1, "message": self.2 } })),
        )
            .into_response()
    }
}

fn not_found_err(what: &str) -> ApiError {
    ApiError(
        StatusCode::NOT_FOUND,
        "not_found",
        format!("{what} not found"),
    )
}

fn bad(msg: impl Into<String>) -> ApiError {
    ApiError(StatusCode::BAD_REQUEST, "bad_request", msg.into())
}

pub async fn not_found() -> ApiError {
    not_found_err("endpoint")
}

fn paging(q: &HashMap<String, String>) -> (usize, usize) {
    let limit = q
        .get("limit")
        .and_then(|v| v.parse().ok())
        .unwrap_or(50usize)
        .clamp(1, 500);
    let offset = q
        .get("offset")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0usize);
    (offset, limit)
}

fn list(q: &HashMap<String, String>, key: &str) -> Vec<String> {
    q.get(key)
        .map(|v| {
            v.split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

fn text_q(q: &HashMap<String, String>) -> Option<String> {
    q.get("q")
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
}

fn flag(q: &HashMap<String, String>, key: &str) -> bool {
    matches!(q.get(key).map(String::as_str), Some("true" | "1" | "yes"))
}

// ------------------------------------------------------------------ status / repository

pub async fn status(State(st): State<AppState>) -> Json<serde_json::Value> {
    let shared = &st.engine.shared;
    let ((revision, counts), seq) = shared.read(|m| (m.revision, m.counts()));
    let s = shared.status();
    let c = &shared.config;
    Json(json!({
        "seq": seq,
        "revision": revision,
        "repository": {
            "root": c.repo_root.display().to_string(),
            "spec_roots": c.spec_roots.iter().map(|p| p.display().to_string()).collect::<Vec<_>>(),
            "src_roots": c.src_roots.iter().map(|p| p.display().to_string()).collect::<Vec<_>>(),
        },
        "scan": s.scan,
        "last_scan": s.last_scan,
        "semantic": s.semantic,
        "watching": s.watching,
        "counts": counts,
    }))
}

pub async fn repository(State(st): State<AppState>) -> Json<serde_json::Value> {
    let shared = &st.engine.shared;
    let (ws, seq) = shared.read(|m| m.workspace.clone());
    let c = &shared.config;
    Json(json!({
        "seq": seq,
        "root": c.repo_root.display().to_string(),
        "spec_roots": c.spec_roots.iter().map(|p| p.display().to_string()).collect::<Vec<_>>(),
        "src_roots": c.src_roots.iter().map(|p| p.display().to_string()).collect::<Vec<_>>(),
        "state_dir": c.state_dir.display().to_string(),
        "crates": ws.crates,
        "from_cargo_metadata": ws.from_cargo_metadata,
        "warnings": ws.warnings,
        "config": c,
    }))
}

// ------------------------------------------------------------------ documents / claims

pub async fn documents(State(st): State<AppState>, Query(q): Q) -> impl IntoResponse {
    let (offset, limit) = paging(&q);
    let text = text_q(&q);
    let (all, seq) = st.engine.shared.read(|m| {
        m.documents
            .values()
            .filter(|d| {
                text.as_ref().is_none_or(|t| {
                    d.path.as_str().to_lowercase().contains(t)
                        || d.title.as_deref().unwrap_or("").to_lowercase().contains(t)
                })
            })
            .map(|d| document_summary(m, d))
            .collect::<Vec<_>>()
    });
    Json(page(all, offset, limit, seq))
}

pub async fn document(
    State(st): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let id = DocumentId::parse(&id).ok_or_else(|| bad("invalid document id"))?;
    let (v, seq) = st.engine.shared.read(|m| {
        let d = m.documents.get(&id)?;
        let mut sections: Vec<_> = m.sections_of(&id).cloned().collect();
        sections.sort_by_key(|s| s.range.start_line);
        let mut claims: Vec<_> = m.claims_of(&id).map(|c| claim_view(m, c)).collect();
        claims.sort_by_key(|c| c.claim.range.start_line);
        Some(json!({
            "document": d,
            "sections": sections,
            "claims": claims,
            "prose_references": m.prose_refs.get(&id).cloned().unwrap_or_default(),
        }))
    });
    let mut v = v.ok_or_else(|| not_found_err("document"))?;
    v["seq"] = json!(seq);
    Ok(Json(v))
}

pub async fn claims(
    State(st): State<AppState>,
    Query(q): Q,
) -> Result<impl IntoResponse, ApiError> {
    let (offset, limit) = paging(&q);
    let doc = q
        .get("document")
        .map(|d| DocumentId::parse(d).ok_or_else(|| bad("invalid document id")))
        .transpose()?;
    let statuses: Vec<FindingStatus> = list(&q, "status")
        .iter()
        .filter_map(|s| FindingStatus::parse(s))
        .collect();
    let kinds = list(&q, "kind");
    let text = text_q(&q);
    let (all, seq) = st.engine.shared.read(|m| {
        let mut v: Vec<ClaimView> = m
            .claims
            .values()
            .filter(|c| doc.as_ref().is_none_or(|d| &c.document == d))
            .filter(|c| {
                kinds.is_empty()
                    || kinds.iter().any(|k| {
                        serde_json::to_value(c.kind)
                            .ok()
                            .and_then(|v| v.as_str().map(|s| s == k))
                            .unwrap_or(false)
                    })
            })
            .filter(|c| {
                text.as_ref()
                    .is_none_or(|t| c.text.to_lowercase().contains(t))
            })
            .map(|c| claim_view(m, c))
            .filter(|cv| statuses.is_empty() || cv.status.is_some_and(|s| statuses.contains(&s)))
            .collect();
        v.sort_by(|a, b| {
            a.document_path
                .cmp(&b.document_path)
                .then(a.claim.range.start_line.cmp(&b.claim.range.start_line))
        });
        v
    });
    Ok(Json(page(all, offset, limit, seq)))
}

pub async fn claim(
    State(st): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let id = ClaimId::parse(&id).ok_or_else(|| bad("invalid claim id"))?;
    let (v, seq) = st.engine.shared.read(|m| {
        let c = m.claims.get(&id)?;
        let node = NodeRef::Claim(id.clone());
        let f = m.claim_finding(&id);
        Some(json!({
            "claim": c,
            "document": m.documents.get(&c.document),
            "status": f.filter(|f| f.is_open()).map(|f| f.status),
            "finding": f,
            "edges": m.edges_from(&node).map(|e| edge_view(m, e)).collect::<Vec<_>>(),
            "evidence": m.evidence_for(&node).collect::<Vec<_>>(),
        }))
    });
    let mut v = v.ok_or_else(|| not_found_err("claim"))?;
    v["seq"] = json!(seq);
    Ok(Json(v))
}

// ------------------------------------------------------------------ code

pub async fn crates(State(st): State<AppState>) -> Json<serde_json::Value> {
    let (items, seq) = st.engine.shared.read(crate_summaries);
    Json(json!({ "seq": seq, "items": items }))
}

pub async fn symbols(State(st): State<AppState>, Query(q): Q) -> impl IntoResponse {
    let (offset, limit) = paging(&q);
    let krate = q.get("crate").cloned().filter(|s| !s.is_empty());
    let module = q.get("module").cloned().filter(|s| !s.is_empty());
    let kinds = list(&q, "kind");
    let text = text_q(&q);
    let public = flag(&q, "public");
    let unconstrained = flag(&q, "unconstrained");
    let (all, seq) = st.engine.shared.read(|m| {
        let mut v: Vec<SymbolView> = m
            .symbols
            .values()
            .filter(|s| krate.as_ref().is_none_or(|k| s.path.first() == Some(k)))
            .filter(|s| {
                module.as_ref().is_none_or(|md| {
                    m.files.get(&s.file).is_some_and(|f| {
                        let mp = f.module_path.join("::");
                        mp == *md || mp.starts_with(&format!("{md}::"))
                    })
                })
            })
            .filter(|s| {
                kinds.is_empty()
                    || kinds.iter().any(|k| {
                        serde_json::to_value(s.kind)
                            .ok()
                            .and_then(|v| v.as_str().map(|x| x == k))
                            .unwrap_or(false)
                    })
            })
            .filter(|s| !public || s.is_public_api)
            .filter(|s| {
                text.as_ref()
                    .is_none_or(|t| s.qualified_name().to_lowercase().contains(t))
            })
            .map(|s| symbol_view(m, s))
            .filter(|v| !unconstrained || v.constrained_by == 0)
            .collect();
        v.sort_by(|a, b| a.qualified_name.cmp(&b.qualified_name));
        v
    });
    Json(page(all, offset, limit, seq))
}

pub async fn symbol(
    State(st): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let id = SymbolId::parse(&id).ok_or_else(|| bad("invalid symbol id"))?;
    let root = st.engine.shared.config.repo_root.clone();
    let (v, seq) = st.engine.shared.read(|m| {
        let s = m.symbols.get(&id)?;
        let file = m.files.get(&s.file)?;
        let constrained: Vec<_> = constraining_edges(m, &id)
            .filter_map(|e| match &e.from {
                NodeRef::Claim(c) => m.claims.get(c).map(|c| json!({ "claim": claim_view(m, c), "edge": e })),
                _ => None,
            })
            .collect();
        let mut children: Vec<_> = m.children_of(&id).map(|c| symbol_view(m, c)).collect();
        children.sort_by_key(|c| c.range.start_line);
        Some(json!({
            "symbol": s,
            "file": file,
            "constrained_by": constrained,
            "findings": m.findings_for_symbol(&id).filter(|f| f.is_open()).collect::<Vec<_>>(),
            "children": children,
            "excerpt": excerpt(m, &root, &file.path, s.range.start_line, s.range.end_line.min(s.range.start_line + 120), 0),
        }))
    });
    let mut v = v.ok_or_else(|| not_found_err("symbol"))?;
    v["seq"] = json!(seq);
    Ok(Json(v))
}

// ------------------------------------------------------------------ findings

fn matches_finding(
    m: &concordance_graph::Model,
    f: &Finding,
    q: &HashMap<String, String>,
    newest: Option<concordance_core::Revision>,
) -> bool {
    let statuses: Vec<FindingStatus> = list(q, "status")
        .iter()
        .filter_map(|s| FindingStatus::parse(s))
        .collect();
    if !statuses.is_empty() && !statuses.contains(&f.status) {
        return false;
    }
    let sev: Vec<Severity> = list(q, "severity")
        .iter()
        .filter_map(|s| Severity::parse(s))
        .collect();
    if !sev.is_empty() && !sev.contains(&f.severity) {
        return false;
    }
    match q.get("lifecycle").map(String::as_str).unwrap_or("open") {
        "all" => {}
        "resolved" => {
            if f.is_open() {
                return false;
            }
        }
        _ => {
            if !f.is_open() {
                return false;
            }
        }
    }
    if let Some(d) = q.get("document").filter(|s| !s.is_empty()) {
        let in_doc = f.documents.iter().any(|x| x.as_str() == d)
            || f.claims
                .iter()
                .any(|c| m.claims.get(c).is_some_and(|c| c.document.as_str() == d));
        if !in_doc {
            return false;
        }
    }
    if let Some(k) = q.get("crate").filter(|s| !s.is_empty()) {
        if f.crate_name.as_deref() != Some(k.as_str()) {
            return false;
        }
    }
    if let Some(md) = q.get("module").filter(|s| !s.is_empty()) {
        if !f
            .module
            .as_deref()
            .is_some_and(|x| x == md || x.starts_with(&format!("{md}::")))
        {
            return false;
        }
    }
    if let Some(meth) = q.get("method").filter(|s| !s.is_empty()) {
        let want = match meth.as_str() {
            "structural" => MethodKind::Structural,
            "semantic" => MethodKind::Semantic,
            "adversarial" => MethodKind::Adversarial,
            "test" => MethodKind::Test,
            _ => return false,
        };
        if !f.methods.contains(&want) {
            return false;
        }
    }
    if flag(q, "new") && (f.introduced_by.is_empty() || Some(f.introduced_in) != newest) {
        return false;
    }
    if let Some(t) = text_q(q) {
        let hay = format!(
            "{} {} {}",
            f.title,
            f.explanation,
            f.symbols
                .iter()
                .filter_map(|s| m.symbols.get(s))
                .map(|s| s.qualified_name())
                .collect::<Vec<_>>()
                .join(" ")
        )
        .to_lowercase();
        let claim_hit = f.claims.iter().any(|c| {
            m.claims
                .get(c)
                .is_some_and(|c| c.text.to_lowercase().contains(&t))
        });
        if !hay.contains(&t) && !claim_hit {
            return false;
        }
    }
    true
}

pub async fn findings(State(st): State<AppState>, Query(q): Q) -> impl IntoResponse {
    let (offset, limit) = paging(&q);
    let sort = q.get("sort").cloned().unwrap_or_else(|| "severity".into());
    let (all, seq) = st.engine.shared.read(|m| {
        let newest = m
            .findings
            .values()
            .filter(|f| !f.introduced_by.is_empty())
            .map(|f| f.introduced_in)
            .max();
        let mut v: Vec<Finding> = m
            .findings
            .values()
            .filter(|f| matches_finding(m, f, &q, newest))
            .cloned()
            .collect();
        match sort.as_str() {
            "updated" => {
                v.sort_by(|a, b| b.last_updated.cmp(&a.last_updated).then(a.id.cmp(&b.id)))
            }
            "status" => v.sort_by(|a, b| {
                a.status
                    .cmp(&b.status)
                    .then(a.severity.cmp(&b.severity))
                    .then(a.id.cmp(&b.id))
            }),
            "title" => v.sort_by(|a, b| a.title.cmp(&b.title)),
            _ => v.sort_by(|a, b| {
                a.severity
                    .cmp(&b.severity)
                    .then(a.status.cmp(&b.status))
                    .then(b.last_updated.cmp(&a.last_updated))
                    .then(a.id.cmp(&b.id))
            }),
        }
        v
    });
    Json(page(all, offset, limit, seq))
}

pub async fn finding(
    State(st): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let id = FindingId::parse(&id).ok_or_else(|| bad("invalid finding id"))?;
    let shared = st.engine.shared.clone();
    let root = shared.config.repo_root.clone();
    let (v, seq) = shared.read(|m| {
        let f = m.findings.get(&id)?;
        let claim_ids = finding_claims(&f.subject);
        let claims: Vec<ClaimView> = claim_ids
            .iter()
            .filter_map(|c| m.claims.get(c))
            .map(|c| claim_view(m, c))
            .collect();
        let symbols: Vec<SymbolView> = f
            .symbols
            .iter()
            .filter_map(|s| m.symbols.get(s))
            .map(|s| symbol_view(m, s))
            .collect();
        // Evidence: everything about the subject (including invalidated history).
        let subjects: Vec<NodeRef> = match &f.subject {
            FindingSubject::Claim { claim } => vec![NodeRef::Claim(claim.clone())],
            FindingSubject::ClaimConflict { claims } => {
                claims.iter().map(|c| NodeRef::Claim(c.clone())).collect()
            }
            FindingSubject::Undocumented { symbol, .. }
            | FindingSubject::Verification { symbol, .. } => vec![NodeRef::Symbol(symbol.clone())],
            FindingSubject::ProseReference { document, .. } => {
                vec![NodeRef::Document(document.clone())]
            }
        };
        let wanted: BTreeSet<_> = f
            .supporting
            .iter()
            .chain(&f.contradicting)
            .cloned()
            .collect();
        let mut evidence: Vec<&Evidence> = match &f.subject {
            FindingSubject::Claim { .. } => {
                subjects.iter().flat_map(|s| m.evidence_for(s)).collect()
            }
            _ => wanted.iter().filter_map(|e| m.evidence.get(e)).collect(),
        };
        evidence.sort_by_key(|e| (!wanted.contains(&e.id), !e.is_valid(), e.id.clone()));
        let mut spec_excerpts: Vec<SourceExcerpt> = Vec::new();
        for c in claim_ids.iter().filter_map(|c| m.claims.get(c)) {
            if let Some(x) = excerpt(
                m,
                &root,
                &c.range.path,
                c.range.start_line,
                c.range.end_line,
                3,
            ) {
                spec_excerpts.push(x);
            }
        }
        let mut seen = BTreeSet::new();
        let mut code_excerpts: Vec<SourceExcerpt> = Vec::new();
        for e in evidence.iter().filter(|e| e.is_valid()) {
            for c in &e.citations {
                let key = (c.range.path.clone(), c.range.start_line, c.range.end_line);
                if !seen.insert(key) {
                    continue;
                }
                let target = match c.role {
                    CitationRole::Implementation => &mut code_excerpts,
                    CitationRole::Specification if claim_ids.is_empty() => &mut spec_excerpts,
                    _ => continue,
                };
                if target.len() >= 6 {
                    continue;
                }
                if let Some(x) = excerpt(
                    m,
                    &root,
                    &c.range.path,
                    c.range.start_line,
                    c.range.end_line.min(c.range.start_line + 80),
                    2,
                ) {
                    target.push(x);
                }
            }
        }
        if code_excerpts.is_empty() {
            for s in f.symbols.iter().filter_map(|s| m.symbols.get(s)).take(3) {
                if let Some(x) = excerpt(
                    m,
                    &root,
                    &s.range.path,
                    s.range.start_line,
                    s.range.end_line.min(s.range.start_line + 60),
                    0,
                ) {
                    code_excerpts.push(x);
                }
            }
        }
        Some(json!({
            "finding": f,
            "claims": claims,
            "symbols": symbols,
            "evidence": evidence,
            "spec_excerpts": spec_excerpts,
            "code_excerpts": code_excerpts,
        }))
    });
    let mut v = v.ok_or_else(|| not_found_err("finding"))?;
    let history = {
        let s2 = shared.clone();
        let id2 = id.clone();
        tokio::task::spawn_blocking(move || {
            s2.store
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .finding_transitions(Some(&id2), 200)
        })
        .await
        .ok()
        .and_then(|r| r.ok())
        .unwrap_or_default()
    };
    v["history"] = json!(history);
    v["seq"] = json!(seq);
    Ok(Json(v))
}

// ------------------------------------------------------------------ graph, history, source, events

pub async fn neighborhood(
    State(st): State<AppState>,
    Query(q): Q,
) -> Result<Json<serde_json::Value>, ApiError> {
    let node = q
        .get("node")
        .and_then(|n| NodeRef::parse(n))
        .ok_or_else(|| bad("`node` must be a document, section, claim, file or symbol id"))?;
    let depth = q
        .get("depth")
        .and_then(|d| d.parse().ok())
        .unwrap_or(1u8)
        .clamp(1, 2);
    let limit = q
        .get("limit")
        .and_then(|d| d.parse().ok())
        .unwrap_or(60usize)
        .clamp(5, 150);
    let (n, seq) = st
        .engine
        .shared
        .read(|m| m.neighborhood(&node, depth, limit));
    let n = n.ok_or_else(|| not_found_err("node"))?;
    let mut v = serde_json::to_value(n).unwrap_or_default();
    v["seq"] = json!(seq);
    Ok(Json(v))
}

pub async fn history(State(st): State<AppState>, Query(q): Q) -> Json<serde_json::Value> {
    let limit = q
        .get("limit")
        .and_then(|d| d.parse().ok())
        .unwrap_or(50usize)
        .clamp(1, 500);
    let shared = st.engine.shared.clone();
    let seq = shared.bus.last_seq();
    let (revisions, transitions) = tokio::task::spawn_blocking(move || {
        let store = shared.store.lock().unwrap_or_else(|e| e.into_inner());
        (
            store.revisions(limit, None).unwrap_or_default(),
            store
                .finding_transitions(None, limit * 4)
                .unwrap_or_default(),
        )
    })
    .await
    .unwrap_or_default();
    Json(json!({ "seq": seq, "revisions": revisions, "transitions": transitions }))
}

pub async fn source(
    State(st): State<AppState>,
    Path(id): Path<String>,
    Query(q): Q,
) -> Result<Json<SourceExcerpt>, ApiError> {
    let node = NodeRef::parse(&id).ok_or_else(|| bad("invalid id"))?;
    let start = q.get("start").and_then(|v| v.parse().ok()).unwrap_or(1u32);
    let end = q
        .get("end")
        .and_then(|v| v.parse().ok())
        .unwrap_or(start.saturating_add(200));
    if start == 0 || end < start || end - start > 2000 {
        return Err(bad("invalid line range (max 2000 lines)"));
    }
    let root = st.engine.shared.config.repo_root.clone();
    let (x, _) = st.engine.shared.read(|m| {
        let path = match &node {
            NodeRef::Document(d) => m.documents.get(d).map(|d| d.path.clone()),
            NodeRef::File(f) => m.files.get(f).map(|f| f.path.clone()),
            _ => None,
        }?;
        excerpt(m, &root, &path, start, end, 0)
    });
    x.map(Json).ok_or_else(|| not_found_err("source"))
}

pub async fn recent_events(State(st): State<AppState>, Query(q): Q) -> Json<serde_json::Value> {
    let after = q.get("after").and_then(|v| v.parse().ok()).unwrap_or(0u64);
    let limit = q
        .get("limit")
        .and_then(|v| v.parse().ok())
        .unwrap_or(200usize)
        .clamp(1, 1000);
    let shared = st.engine.shared.clone();
    let seq = shared.bus.last_seq();
    let mut items: Vec<serde_json::Value> = shared
        .bus
        .recent(after, limit)
        .iter()
        .map(|e| serde_json::to_value(&**e).unwrap_or_default())
        .collect();
    if items.is_empty() {
        let s2 = shared.clone();
        let stored = tokio::task::spawn_blocking(move || {
            let store = s2.store.lock().unwrap_or_else(|e| e.into_inner());
            let floor = seq.saturating_sub(limit as u64).max(after);
            store.events_after(floor, limit).unwrap_or_default()
        })
        .await
        .unwrap_or_default();
        items = stored
            .iter()
            .map(|e| serde_json::to_value(e).unwrap_or_default())
            .collect();
    }
    Json(json!({ "seq": seq, "items": items }))
}

#[derive(Deserialize, Default)]
pub struct ScanBody {
    #[serde(default)]
    force: bool,
}

pub async fn post_scan(
    State(st): State<AppState>,
    body: Option<Json<ScanBody>>,
) -> impl IntoResponse {
    let force = body.map(|b| b.0.force).unwrap_or(false);
    let engine = st.engine.clone();
    tokio::spawn(async move { engine.request_scan(force).await });
    (StatusCode::ACCEPTED, Json(json!({ "accepted": true })))
}

#[derive(Deserialize, Default)]
pub struct AnalysisBody {
    #[serde(default)]
    claims: Vec<String>,
    #[serde(default)]
    all: bool,
}

pub async fn post_analysis(
    State(st): State<AppState>,
    body: Option<Json<AnalysisBody>>,
) -> Result<impl IntoResponse, ApiError> {
    let body = body.map(|b| b.0).unwrap_or_default();
    let mut claims = Vec::new();
    for c in &body.claims {
        claims.push(ClaimId::parse(c).ok_or_else(|| bad(format!("invalid claim id `{c}`")))?);
    }
    if claims.is_empty() && !body.all {
        return Err(bad("provide `claims` or `all: true`"));
    }
    match st.engine.request_analysis(claims, body.all).await {
        Ok(n) => Ok((StatusCode::ACCEPTED, Json(json!({ "queued": n })))),
        Err(e) => Err(ApiError(StatusCode::CONFLICT, "conflict", e)),
    }
}
