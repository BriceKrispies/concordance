# Command line

```
concordance serve  [common] [--port N] [--host ADDR] [--no-watch]
concordance scan   [common] [--json | --jsonl] [--force] [--fail-on none|proven|violated] [-v] [--include-resolved]
                   [--exegra-results FILE [--no-generated-at]]
concordance watch  [common] [--jsonl]
concordance report [common] [--format text|json] [--fail-on ...] [-v] [--include-resolved]
                   [--exegra-results FILE [--no-generated-at]]
concordance lint   [common] [--strict] [--json]     # requirement format, see docs/spec-lint.md

common: --repo PATH  --spec-root PATH (repeatable)  --src-root PATH (repeatable)
        --config FILE  --state-dir DIR  --semantic | --no-semantic  --model ALIAS
        --test-results FILE (repeatable; see docs/requirements.md)
```

Roots may be absolute or relative to the repository. A `--spec-root` may name a single file, for
example the structured specification (`--spec-root specs/spec.jsonc`, see
[requirements.md](requirements.md#structured-sources-exegra-jsonc)). Without `--config`, `concordance.toml` in
the repository is used if it exists. Command-line values override the file; a repeated
`--spec-root`/`--src-root` replaces the file's whole list. See `examples/concordance.toml` for the
schema (it is intentionally small: roots, excludes, state dir, authority globs, semantic, server). `[gate] started_milestones = [1, 2]` marks the milestones that
count; `[lint] require_milestone = true` requires `Milestone:` on every requirement.

## Exit codes

| Code | Meaning |
|---|---|
| 0 | Success. With `--fail-on`, the gate passed. |
| 1 | The `--fail-on` gate failed (for `lint`: at least one error, or with `--strict` any finding): `proven` means at least one open structural contradiction or failing requirement test; `violated` also counts semantic contradictions that survived an adversarial challenge. |
| 2 | Usage or configuration error (bad flags, missing roots, invalid TOML, no persisted state for `report`). |
| 3 | Runtime error (state database, I/O, cannot bind the port). |

`unverified`, `planned`, `ambiguous`, `stale` and `undocumented` findings never fail the gate: they are not
violations. With `[gate] started_milestones`, findings of requirements whose `Milestone:` is not started are
left out of the gate entirely (`gate.ignored_unstarted` counts them); see [requirements.md](requirements.md#milestones). Missing Claude is never treated as successful semantic verification. The report's
`semantic.coverage` field states exactly what was and was not assessed.

## Output

- `scan` (default) prints a human summary on stdout and progress on stderr.
- `scan --json` prints one JSON report: `tool`, `version`, `repository`, `revision`, `semantic`,
  `counts`, `gate`, and `findings[]`. Each finding includes its `claim_refs` and `symbol_refs`, with
  exact ranges for downstream tooling, and its requirement's `milestone` when it has one.
- `scan --jsonl` prints every event envelope as one line, then the report as the last line.
- `watch` streams events until Ctrl+C; with `--jsonl` the stream is machine-readable.
- `report` reads persisted state without scanning or probing Claude.

- In a virtual document (rendered from a structured source) every location is followed by its origin:
  `specifications/kernel.md:23 (specifications/spec.jsonc:1320 krn-002)`. `scan --json` and
  `report --format json` add `location_origins` to a finding (parallel to `locations`, `null` where a
  location is not in a virtual document) and `origin` to each of its `claim_refs`. Both are absent when
  nothing is in a virtual document. `lint --json` adds `origin` to a finding.
- `report` renders the structured sources again, because persisted state holds no virtual documents. If
  the state is older than the source it warns on stderr: run `scan` first.

None of the commands need a TTY. They work with piped stdin and stdout.

## Exegra results

`concordance scan --exegra-results FILE` and `concordance report --exegra-results FILE` write the
results of a structured specification source (see [requirements.md](requirements.md#structured-sources-exegra-jsonc))
to `FILE`, for Exegra to read. The normal output is unchanged. `--no-generated-at` leaves out the only
time in the file, so identical state gives identical bytes. Exit code 2 when there is no structured
source under the spec roots, when `FILE` cannot be written, or (for `report`) when the persisted state is
older than the source. The file has sorted keys:

```json
{
  "claims": {
    "krn-002": { "detail": "Proved by passing test `krn_002_halts`.", "outcome": "pass", "proofs": ["krn_002_halts"], "status": "verified" }
  },
  "generatedAt": "2026-11-14T12:00:00Z",
  "lint": [ { "claim": "krn-001", "entity": "krn-001", "line": 31, "message": "...", "rule": "R1", "severity": "error" } ],
  "source": "specifications/spec.jsonc",
  "tool": "concordance",
  "version": "0.1.0"
}
```

- `claims` is keyed by the id of the Exegra claim entity the requirement was rendered from, and has an
  entry **only** for a requirement whose Concordance status is `verified` (`outcome` `pass`) or
  `violated` (`fail`). Every other status (unverified, planned, stale, ambiguous) is omitted: absence
  means "not established". `status` and `proofs` come from reconciliation, not from a second derivation:
  `proofs` are the tests whose recorded outcome supports (or, for `fail`, contradicts) the requirement;
  `detail` is one sentence naming them or the contradiction. A requirement defined twice is omitted.
- `lint` has every lint finding located in the source (rule, severity, message as in
  [spec-lint.md](spec-lint.md)) and every problem in the source (rule `R0`). `claim` is the entity id when
  the entity is a requirement, else `null`; `entity` is the entity id or `null`; `line` is the line of the
  entity in the JSONC file. With several structured sources `source` lists them comma separated.

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
