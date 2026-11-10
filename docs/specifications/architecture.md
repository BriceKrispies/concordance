# Architecture constraints

Concordance is a single-machine Rust workspace with explicit typed boundaries.

## Layering

The domain model in `concordance_core` is the contract every subsystem speaks.

- `concordance_core` must not depend on `rusqlite`.
- `concordance_core` must not depend on `axum`.
- `concordance_graph` must not depend on `concordance_storage`.
- `concordance_reconcile` must not depend on `concordance_semantic`.
- `concordance_markdown` must not depend on `syn`.

## Safety

- The `concordance_core` crate must not use unsafe code.
- The `concordance_reconcile` crate must not use unsafe code.
- The `concordance_graph` crate must not use unsafe code.

## Parsing and indexing

Markdown is parsed with a real parser; Rust with `syn`.

```rust
pub fn index_document(path: &RepoPath, text: &str, configured: Option<AuthorityAssessment>) -> DocumentIndex
pub fn index_source(path: &RepoPath, text: &str, ws: &WorkspaceInfo) -> SourceIndex
pub fn normalize_signature(text: &str) -> Option<(String, String)>
```

- `concordance_rust::index_source()` must report a parse failure in `SourceFile::parse_error` instead of failing.
- Symbol identities MUST NOT depend on line numbers.

## File access

The HTTP API addresses files only by ids the model knows.

- `concordance_engine::source::read_text()` must refuse paths that resolve outside the repository root.
- The server must listen on `127.0.0.1` by default.

## Persistence

- `Store::commit()` must apply a whole `Delta` in a single transaction.
- Source file contents must not be stored in the state database; only hashes, metadata and bounded excerpts.

```rust
pub fn commit(&mut self, delta: &Delta) -> Result<(), StoreError>
```

## Rationale

Keeping the model free of storage and HTTP types lets the CLI and the server share
one engine and one event stream instead of two inconsistent execution paths.
