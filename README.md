# Concordance

Concordance checks that a project's Markdown specifications and its source code agree.

It reads the claims in your docs (requirements, MUST/SHALL statements, documented signatures, references to code), indexes the source, and reconciles the two. Each claim is reported as verified, violated, ambiguous or unverified, and every finding points at the exact lines in the spec and the code. It also reports code that no document mentions.

- Requirement IDs in the spec (`**LANG-006.3**`) link to tests that declare `Verifies: LANG-006.3`. Ingested test results (libtest, nextest, JUnit, vitest/jest) then verify or violate them.
- Languages are plugins. Rust and TypeScript are built in.
- It runs locally and watches the repository, updating findings as files change. It never modifies the repository it analyzes.
- Use it headless (`scan`, `watch`, `report`, with JSON output and CI exit codes) or through a local web UI (`serve`).
- Optional semantic checks of prose claims can use an LLM. They are off by default.

## Usage

```sh
cargo build --release
concordance serve --repo /path/to/repo --spec-root docs --src-root src   # web UI on http://127.0.0.1:4173/
concordance scan  --repo /path/to/repo --spec-root docs --src-root src --json --fail-on proven
```

See `examples/concordance.toml` for configuration and `docs/` for details.

## License

MIT
