# Command line

```
concordance serve  [common] [--port N] [--host ADDR] [--no-watch]
concordance scan   [common] [--json | --jsonl] [--force] [--fail-on none|proven|violated] [-v] [--include-resolved]
concordance watch  [common] [--jsonl]
concordance report [common] [--format text|json] [--fail-on ...] [-v] [--include-resolved]

common: --repo PATH  --spec-root PATH (repeatable)  --src-root PATH (repeatable)
        --config FILE  --state-dir DIR  --semantic | --no-semantic  --model ALIAS
        --test-results FILE (repeatable; see docs/requirements.md)
```

Roots may be absolute or relative to the repository. Without `--config`, `concordance.toml` in
the repository is used if it exists. Command-line values override the file; a repeated
`--spec-root`/`--src-root` replaces the file's whole list. See `examples/concordance.toml` for the
schema (it is intentionally small: roots, excludes, state dir, authority globs, semantic, server).

## Exit codes

| Code | Meaning |
|---|---|
| 0 | Success. With `--fail-on`, the gate passed. |
| 1 | The `--fail-on` gate failed: `proven` means at least one open structural contradiction or failing requirement test; `violated` also counts semantic contradictions that survived an adversarial challenge. |
| 2 | Usage or configuration error (bad flags, missing roots, invalid TOML, no persisted state for `report`). |
| 3 | Runtime error (state database, I/O, cannot bind the port). |

`unverified`, `ambiguous`, `stale` and `undocumented` findings never fail the gate: they are not
violations. Missing Claude is never treated as successful semantic verification. The report's
`semantic.coverage` field states exactly what was and was not assessed.

## Output

- `scan` (default) prints a human summary on stdout and progress on stderr.
- `scan --json` prints one JSON report: `tool`, `version`, `repository`, `revision`, `semantic`,
  `counts`, `gate`, and `findings[]`. Each finding includes its `claim_refs` and `symbol_refs`, with
  exact ranges for downstream tooling.
- `scan --jsonl` prints every event envelope as one line, then the report as the last line.
- `watch` streams events until Ctrl+C; with `--jsonl` the stream is machine-readable.
- `report` reads persisted state without scanning or probing Claude.

None of the commands need a TTY. They work with piped stdin and stdout.

## Semantic analysis

`--semantic` (or `[semantic] enabled = true`) uses the local Claude Code CLI. The binary is
`claude`, or whatever `claude_path` names. It is invoked as:

```
claude -p --output-format json --json-schema <schema> --tools "" --no-session-persistence \
       --setting-sources "" --system-prompt <...> --model <model>
```

The prompt goes on stdin, and the working directory is a neutral folder inside the state directory.
If the CLI is missing, a `system.warning` is emitted, the status reports `available: false`, and
behavioral claims stay `unverified`.
