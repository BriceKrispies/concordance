# HTTP API (v1)

All endpoints are under `/api/v1`, return JSON (`application/json; charset=utf-8`), and use the
serialization of the types in `crates/concordance-core` (snake_case enums, ids as prefixed strings:
`doc_`, `sec_`, `clm_`, `src_`, `sym_`, `rel_`, `evd_`, `fnd_`, `ana_`). The server listens on
`127.0.0.1` by default.

Errors from the handlers: non-2xx with `{"error": {"code": "not_found" | "bad_request" | "conflict", "message": "..."}}`.
The guard in front of every route answers `403 Forbidden` with a plain-text body (`host not allowed`,
`cross-origin request refused`, `cross-site request refused`). On a loopback bind, a `Host` other than
`localhost`, `127.0.0.1` or `::1` is refused. For any bind, a non-GET request whose `Origin` names another
host, or whose `Sec-Fetch-Site` is `cross-site`, is refused.

Collections are paginated: query `limit` (default 50, max 500) and `offset` (default 0); response
`Page<T>` = `{"items": [T], "total": n, "offset": n, "limit": n}`.

## Snapshot and consistency

Every snapshot response carries `"seq"`: the event sequence number the snapshot is consistent
with. A client loads snapshots, then opens the event stream with `cursor = seq` and applies only
events with `seq > cursor`.

## Endpoints

| Method | Path | Returns |
|---|---|---|
| GET | `/status` | `Status` |
| GET | `/repository` | `Repository` |
| GET | `/documents?q=` | `Page<DocumentSummary>` |
| GET | `/documents/{doc_id}` | `DocumentDetail` |
| GET | `/claims?document=&status=&kind=&q=` | `Page<ClaimView>` |
| GET | `/claims/{clm_id}` | `ClaimDetail` |
| GET | `/crates` | `{seq, items: [CrateSummary]}` |
| GET | `/symbols?crate=&module=&kind=&q=&public=true&unconstrained=true` | `Page<SymbolView>` |
| GET | `/symbols/{sym_id}` | `SymbolDetail` |
| GET | `/findings?status=&severity=&document=&crate=&module=&method=&lifecycle=&new=&q=&sort=` | `Page<Finding>` |
| GET | `/findings/{fnd_id}` | `FindingDetail` |
| GET | `/graph/neighborhood?node={id}&depth=1&limit=60` | `Neighborhood` |
| GET | `/history?limit=` | `History` |
| GET | `/source/{doc_id or src_id}?start=&end=` | `SourceExcerpt` |
| GET | `/events?cursor=` | SSE stream (see below) |
| GET | `/events/recent?after=&limit=` | `{seq, items: [Envelope]}` |
| POST | `/scans` body `{"force": false}` | `202 {"accepted": true}` |
| POST | `/analyses` body `{"claims": [clm_id], "all": false}` | `202 {"queued": n}` or `409` when semantic analysis is disabled/unavailable |

Filter values: `status` is a comma list of `verified,violated,unverified,planned,operational,stale,ambiguous,undocumented`;
`severity` comma list of `critical,high,medium,low,info`; `method` one of `structural,semantic,adversarial`;
`lifecycle` one of `open` (default), `resolved`, `all`; `new=true` keeps findings introduced in the latest
revision that had file changes; `sort` one of `severity` (default), `updated`, `status`, `title`;
`q` is a case-insensitive substring match on title/explanation/claim text/symbol path.

## Response types

```text
Status = {
  seq, revision,
  repository: { root, spec_roots: [string], src_roots: [string] },
  scan: null | { id, reason, phase, done, total, started_at },
  last_scan: null | { revision, at, duration_ms, changed_files },
  semantic: { enabled: bool, available: bool|null, detail: string|null, model, queued, running, completed, failed },
  watching: bool,
  counts: Counts            // core::finding::Counts
}
Repository = { seq, root, spec_roots, src_roots, state_dir, crates: [CrateInfo], from_cargo_metadata, warnings, config: Config }

DocumentSummary = { id, path, title, authority: AuthorityAssessment, sections, claims, obligations,
                    status_counts: { verified, violated, unverified, planned, operational, stale, ambiguous },
                    source?: path }       // virtual document: the structured source it is rendered from
DocumentDetail = { seq, document: Document, sections: [Section], claims: [ClaimView], prose_references: [ProseReference] }

ClaimView = { claim: Claim, status: FindingStatus|null, finding: fnd_id|null, document_path,
              origin?: Origin }
              // status null for informative (non-obligation) claims
ClaimDetail = { seq, claim: Claim, document: Document, status, finding: Finding|null,
                edges: [EdgeView], evidence: [Evidence], origin?: Origin }
Origin = { file: path, line: n, entity: string }
              // where a line of a virtual document was rendered from in its structured
              // source file (see requirements.md, "Structured sources"); `line` is the entity's
              // line in that file. Every `origin` field is absent for ordinary Markdown.
EdgeView = { edge: Edge, target_label: string|null, target_kind: string|null }

CrateSummary = { name, root_dir, files, symbols, public_api, unconstrained_public_api, modules: [string] }
SymbolView = { id, name, qualified_name, kind, visibility, is_public_api, has_doc, file_path, range,
               constrained_by: n, finding_status: FindingStatus|null }
SymbolDetail = { seq, symbol: Symbol, file: SourceFile, constrained_by: [{ claim: ClaimView, edge: Edge }],
                 findings: [Finding], children: [SymbolView], excerpt: SourceExcerpt|null }

FindingDetail = { seq, finding: Finding, claims: [ClaimView], symbols: [SymbolView], evidence: [Evidence],
                  history: [FindingTransition], spec_excerpts: [SourceExcerpt], code_excerpts: [SourceExcerpt],
                  location_origins?: [Origin|null] }   // parallel to finding.locations
SourceExcerpt = { path, file_id, language: "markdown"|"rust", start_line, lines: [string],
                  highlight: { start_line, end_line } | null, origin?: Origin, current: bool }
                // origin: of the first highlighted line, for a virtual document. Its lines are
                // those of the rendered text, which is what `Document`, `Claim` and `Section`
                // ranges refer to.
                // current=false when the file changed since indexing (lines may not match ranges)

Neighborhood = { seq, center: id, nodes: [{ id, kind, label, detail, status }],
                 edges: [{ id, kind, from, to, resolution, confidence }], truncated: bool }
History = { seq, revisions: [RevisionRecord], transitions: [FindingTransition] }
```

## Event stream

`GET /api/v1/events?cursor=N` (or header `Last-Event-ID: N`) opens a `text/event-stream`.

* Each message: `id: <seq>`, `event: <type>` (e.g. `finding.created`), `data: <Envelope JSON>`.
* If the cursor is replayable, every event with `seq > N` is sent once, in order.
* Without a cursor the stream starts at the current head.
* If the cursor is older than the replay window, or the client falls too far behind while
  connected, the server sends `event: resync` with `data: {"latest": seq}`. The client must reload
  snapshots and continue; events up to `latest` are not sent.
* The first message is always `event: hello`, `data: {"seq": head, "replayed_from": N|null}`.
* Keep-alive comments are sent every 15 s.
