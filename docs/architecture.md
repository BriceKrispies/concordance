# Architecture

Concordance keeps four things apart: **intent** (claims from Markdown), **implementation**
(symbols and syntactic facts from Rust), **evidence** (what can currently be established about
their relationship, with provenance and validity) and **uncertainty** (findings whose status never
claims more than their evidence supports).

## Crates

```
concordance-core        domain model: ids, claims, symbols, edges, evidence, findings, events, config
concordance-markdown    pulldown-cmark -> DocumentIndex (sections, claims, references, authority)
concordance-rust        Rust language plugin: syn + cargo metadata -> SourceIndex
concordance-typescript  TypeScript/JavaScript language plugin: oxc + package.json -> SourceIndex
concordance-graph       in-memory Model + reverse indexes, name resolution, neighborhoods
concordance-reconcile   linking, Layer A structural checks, Layer D gaps, status derivation, lifecycle
concordance-semantic    Layer B/C through the Claude Code CLI: scoped prompts, strict schemas, citation validation
concordance-storage     SQLite: versioned schema, one transaction per change batch
concordance-events      bounded ring + cursor subscriptions (replay, lag -> resync)
concordance-engine      the incremental engine: discovery, parsing, change batches, invalidation, scheduling, watching
concordance-server      axum: /api/v1 JSON, SSE, embedded web/ assets, Host guard
concordance-cli         `concordance` binary: serve, scan, watch, report
tests/                  end-to-end tests over fixtures/
```

The prompt suggested a crate named `concordance-incremental`. Its role is filled by
`concordance-engine`, because the incremental dependency engine *is* the engine: it is the
single writer of the model.

Source languages are plugins (`core::lang::LanguagePlugin`): each owns file extensions, Markdown
fence tags, package discovery, per-file indexing into the language-neutral `SourceIndex`, and the
extraction and normalisation of documented signatures. The engine builds the registry
(`concordance_engine::languages()`); nothing else names a language. Symbol kinds and fact kinds are
shared across languages (a TypeScript `throw` and a Rust `panic!` are both failure sites), so
constraint claims, requirement verification and status rules work the same for every language.

Dependencies point downward only. `core` knows no HTTP, SQL, Markdown AST or `syn`. The self-scan
(`docs/specifications/architecture.md`) checks part of this layering against Cargo metadata and
`use` items.

## Data flow

```
 .md ──markdown──> DocumentIndex ─┐                      ┌─> Evidence (structural)
                                  ├─> engine (apply) ─> Model ─> reconcile ─┤
 .rs ──rust──────> SourceIndex ───┘        │                     │          └─> Findings
                                           │                     └─> semantic jobs ─> Evidence (semantic/adversarial)
                                           └─> EventBus (typed events) ─> SSE / JSONL / CLI
                                           └─> Store (SQLite delta per batch)
```

## Identity

| Entity | Id derived from | Survives |
|---|---|---|
| Document, File | repository path | content edits |
| Section | path + heading path + occurrence | edits elsewhere |
| Claim | path + heading path + normalized text + occurrence; **carried** by the engine to an edited claim when the textual match is unique (Jaccard >= 0.5, runner-up at least 0.1 behind) | light rewording |
| Symbol | kind + crate-qualified path + disambiguator (trait for trait-impl items, cfg, occurrence), never lines | moves, body edits |
| Edge | kind + source + reference text + target | |
| Evidence | subject + method + check key | re-derivation |
| Finding | its subject (claim, prose reference, symbol+aspect, claim pair) | every scan; resolved findings are kept |

## The incremental engine

A change batch (initial scan, watcher batch, manual scan) works in these steps:

1. **Discover and hash.** Files whose blake3 hash and configured authority are unchanged are skipped.
   A change in the extractor or rules version (persisted in the database) forces a full re-derivation.
2. **Parse.** Changed files are parsed on the blocking pool with bounded parallelism. Results are applied
   in chunks under the write lock, so indexing events stream while parsing continues.
3. **Diff.** Each document or file is diffed against its previous version: claims (with identity
   carry-over) and symbols are classified as created, updated, moved or removed. The diff records:
   - changed nodes,
   - claims that reference affected names (`refs_by_name`) or link into the changed document,
   - callers of changed symbols,
   - constraint claims, when the code structure changed.
4. **Reconcile.** Under one write lock:
   - Evidence whose dependency hashes changed is **invalidated**, with a reason.
   - Edges are re-resolved for affected claims and documents, producing relationship events.
   - Structural evidence is recomputed for exactly the affected claims, documents and symbols.
   - Findings are re-derived and merged with their previous versions, which keeps identity, history
     and `introduced_in`.
   - Conflicts between claims are re-checked when bounds changed.
   - All events are published while the lock is held, then the delta is committed in one transaction.
5. **Schedule semantic analysis.** Candidates are affected obligation claims that structural analysis
   cannot decide and that lack valid semantic evidence. Running analyses whose inputs were invalidated
   are cancelled.

**Freshness.** A semantic result is applied only if, under the write lock, the claim and every symbol
it saw still have the hashes it was computed from. Otherwise it is reported as `superseded` and
re-queued. Results are cached by prompt version, mode, model and input hashes, and a cached result
is reused only after the same freshness check.

## Status rules

`concordance-reconcile/src/derive.rs` is the single place where status is decided. The precedence is
documented at the top of that file, and `docs/specifications/reconciliation.md` states it normatively.
In short:

| Status | Requires |
|---|---|
| `violated` (structural contradiction) | Valid structural contradicting evidence of strength >= Strong: a dangling explicit reference, a differing signature, a missing file or heading, an unsafe or panic site in a constrained scope, a declared forbidden dependency, or a constant outside a documented bound. |
| `violated` (suspected semantic contradiction) | Semantic `disagrees` with validated code citations, **and** an adversarial challenge that upheld it. |
| `stale` | Semantic evidence was invalidated by a change and has not been re-established. |
| `ambiguous` | Semantic `ambiguous`, heuristic vague wording, or two normative bounds that conflict. |
| `verified` | One of:<br>• a matching documented signature;<br>• a satisfied deterministic constraint (strength >= Strong);<br>• semantic `agrees` upheld by a challenge;<br>• semantic `agrees` in a single pass, for low-impact claims only. |
| `unverified` | Everything else. Suspected contradictions that were not confirmed stay here, labelled. |
| `undocumented` | Public API behavior (panics, error returns, environment reads, missing contract) that no claim covers. |

Symbol existence is recorded as *inconclusive linkage*. It never supports a behavioral claim.

## Authority

Document authority comes from, in order:

1. configured globs,
2. front matter (`concordance: normative`),
3. upper-case RFC 2119 keywords,
4. path convention (`spec/`, `specifications/`, `rfcs/`, ...),
5. otherwise `unknown`, which is never assumed normative.

Section headings such as Example, Background, Rationale and History make claims informative, so
they produce no compliance findings.

Proposals follow authority:

| Document | Proposals |
|---|---|
| Normative | Change the implementation, or accept the behavior change (*requires decision*). |
| Descriptive | Update the documentation. |
| Unknown | Clarify which side is authoritative. |

Concordance never edits the analyzed repository.

## Streaming

- `EventBus` keeps a ring of recent envelopes (default 8192) and a `watch` of the head seq. Each
  subscriber is just a cursor, so publishers never block and memory is bounded.
- A subscriber that falls out of the ring gets `Lagged` and must resync.
- SSE sends `hello`, then replays from the cursor (or sends `resync`), then streams live with
  15-second keep-alives.
- `watch --jsonl` and `scan --jsonl` print the same envelopes.

## Concurrency

There is one engine task (the single writer), which receives commands from an mpsc channel. Around it:

- parsing runs on the blocking pool, at most 8 concurrent chunks;
- semantic jobs run as tokio tasks, bounded by `max_concurrent` and the per-batch budget;
- the watcher runs on its own thread and debounces events (300 ms quiet, 2 s maximum);
- HTTP handlers take the model read lock briefly and never across an `.await`.

On shutdown, the engine stops, running analyses are cancelled (the Claude subprocess is killed),
SSE streams close via `EventBus::close` and axum shuts down gracefully.

## Persistence

SQLite (bundled) lives at `<state_dir>/state.db`, outside the repository by default
(`%LOCALAPPDATA%\concordance\<repo>-<hash>`).

- Entities are stored as JSON with key columns alongside.
- Each change batch is one transaction, in WAL mode.
- The schema version is kept in `PRAGMA user_version`.
- Event history is capped at 20,000 events.
- No file contents are stored; only hashes and bounded excerpts inside evidence citations.
