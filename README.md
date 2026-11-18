# Concordance

Concordance checks that a project's Markdown specifications and its source code agree.

It reads the claims in your docs (requirements, MUST/SHALL statements, documented signatures, references to code), indexes the source, and reconciles the two. Each claim is reported as verified, violated, ambiguous or unverified, and every finding points at the exact lines in the spec and the code. It also reports code that no document mentions.

- Specifications are Markdown, or Exegra JSONC files (see `docs/requirements.md`).
- Requirement IDs in the spec (`**LANG-006.3**`) link to tests that declare `Verifies: LANG-006.3`. Ingested test results (libtest, nextest, JUnit, vitest/jest, TRX, Lake build logs) then verify or violate them, once `concordance record` has bound the results file to the sources and requirement text it was run on; a results file with no manifest verifies nothing.
- `concordance lint --strict` enforces the spec-driven requirement format (atomic, observable, at the boundary, provable, traceable to a white paper): see `docs/spec-lint.md`.
- A specification can cite code in other git repositories (`[APP:apps/api/src/catalog.ts:67-132]`), configured under `[repos]`; citations are checked at the revision they name and drift is reported as `stale`. Requirements can be grouped in milestones, and `[gate] started_milestones` gates only the started ones (the others report `planned`). See `docs/cli.md` and `docs/requirements.md`.
- A specification kept as one structured JSONC file in the Exegra format works the same way: Concordance reads its requirement and prose properties, reports locations in terms of the JSONC, and can write per-requirement results back for Exegra (`--exegra-results`). See `docs/requirements.md`.
- Languages are plugins. Rust, TypeScript, C# and Lean 4 are built in.
- It runs locally and watches the repository, updating findings as files change. It never modifies the repository it analyzes.
- Use it headless (`scan`, `watch`, `report`, with JSON output and CI exit codes) or through a local web UI (`serve`).
- Optional semantic checks of prose claims use the Claude Code CLI. The model searches the repository read-only and every citation it gives is checked against the real lines. They are off by default.

## Usage

```sh
cargo build --release
concordance serve --repo /path/to/repo --spec-root docs --src-root src   # web UI on http://127.0.0.1:4173/
concordance scan  --repo /path/to/repo --spec-root docs --src-root src --json --fail-on proven
```

```sh
cargo test 2>&1 | tee target/test-results/cargo-test.txt
concordance record --repo /path/to/repo      # right after the tests, before any edit
```

What `scan --fail-on` guarantees, by mode:

- `none` guarantees nothing: the exit code is 0 whatever the findings.
- `proven` guarantees only that no structural contradiction and no failing recorded test of a started milestone is open, and says nothing about requirements that have no proof.
- `violated` guarantees what `proven` does and that no semantic contradiction that survived an adversarial challenge is open.
- `unverified` guarantees what `violated` does and that every requirement of a started milestone has the status `verified`, which means a declared test passed in a run recorded against the current sources and requirement text (see `docs/requirements.md` for what that does and does not show).

See `examples/concordance.toml` for configuration and `docs/` for details.

## License

MIT
