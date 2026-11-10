# Dependency rationale

Every external crate has a concrete job that would otherwise be wasteful or risky to rewrite.

| Crate | Purpose | Why not std / hand-written |
|---|---|---|
| `serde`, `serde_json` | Typed serialization of every persisted and streamed entity | De-facto standard; hand-rolled JSON would be fragile |
| `blake3` | Content hashes and stable ids | No hash in std suitable for content addressing |
| `pulldown-cmark` | Markdown parsing with source offsets | Parsing Markdown with regexes is explicitly out of scope |
| `syn` (+ `proc-macro2` span-locations, `quote`) | Rust parsing with line/column spans; rendering signatures | Writing a Rust parser is out of scope |
| `oxc_parser`, `oxc_ast`, `oxc_ast_visit`, `oxc_allocator`, `oxc_span` (pinned `=0.150.0`) | TypeScript/JavaScript parsing with byte spans and an AST visitor | A TypeScript parser is out of scope; oxc is pure Rust (no Node, no C toolchain) and fast. Pinned because its API changes between minor versions |
| `rusqlite` (bundled) | SQLite persistence without a system install | No database engine in std |
| `tokio` | Async runtime, process spawning with timeouts and kill, sync primitives | Needed by axum; process cancellation |
| `axum` (+ its `hyper`/`tower` stack) | HTTP/1.1 server, routing, SSE | A custom HTTP framework is out of scope |
| `futures-util` | Stream combinator for the SSE body (`stream::unfold`) | Already in axum's tree |
| `notify` | Cross-platform file watching (ReadDirectoryChangesW on Windows) | Platform APIs are intricate |
| `ignore` | Directory walking that respects `.gitignore` | Correct gitignore semantics are subtle |
| `globset` | `[authority]` and `exclude` globs | Already a dependency of `ignore` |
| `clap` | CLI argument parsing with help and validation | Repeatable flags, enums, and help text |
| `toml` | Configuration file parsing | No TOML parser in std |

Things deliberately done without a crate:

- **Cargo metadata.** We parse `cargo metadata --format-version 1` JSON with `serde_json` instead of
  using `cargo_metadata`.
- **Static asset embedding.** A `build.rs` generates a table of `include_bytes!` instead of using
  `rust-embed`.
- **Debouncing.** A small thread replaces `notify-debouncer-*`.
- **Event bus.** A ring buffer plus `tokio::sync::watch` replaces broadcast crates.
- **Temp dirs and HTTP clients in tests.** These are written with std and tokio.
- **Levenshtein and Jaccard.** Both are a few lines.
