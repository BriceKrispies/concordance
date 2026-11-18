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
`--spec-root`/`--src-root` replaces the file's whole list. When neither the command line nor the file
names a root, the repository root is used for both spec and source roots. `examples/concordance.toml`
lists every key of the file and says what each one does. `test_results` in the file and
`--test-results` on the command line are the same setting; the command line replaces the file's list.
`[gate] started_milestones = [1, 2]` marks the milestones that count; `[lint] require_milestone = true`
requires `Milestone:` on every requirement.

`repo` may be a plain parent directory whose spec and source roots are separate git repositories
(a workspace). Each root's own `.gitignore` applies, and every `package.json` under the source
roots is a TypeScript package.

## Exit codes

| Code | Meaning |
|---|---|
| 0 | Success. With `--fail-on`, the gate passed. |
| 1 | `scan` and `report`: the `--fail-on` gate failed. `proven` means at least one open structural contradiction or failing requirement test; `violated` also counts semantic contradictions that survived an adversarial challenge. `lint` has no `--fail-on`: it exits 1 when it has at least one error, and with `--strict` every warning counts as an error (info-level hints never do). |
| 2 | Usage or configuration error (bad flags, missing roots, invalid TOML, no persisted state for `report`, a `[repos]` entry that is not a git repository or whose `cited` revision does not exist). |
| 3 | Runtime error (state database, I/O, cannot bind the port). |

`unverified`, `planned`, `ambiguous`, `stale` and `undocumented` findings never fail the gate: they are not
violations. With `[gate] started_milestones`, findings of requirements whose `Milestone:` is not started are
left out of the gate entirely (`gate.ignored_unstarted` counts them); see [requirements.md](requirements.md#milestones).
Missing Claude is never treated as successful semantic verification. The report's
`semantic.coverage` field states exactly what was and was not assessed.

## Statuses

| Status | Meaning |
|---|---|
| `verified` | Evidence establishes the claim: a matching documented signature, a satisfied deterministic constraint, a passing test in a run recorded after the last source change, or a semantic agreement that a challenge upheld. |
| `violated` | Contradicted: a structural contradiction (a dangling reference, a differing signature, a constant outside a documented bound, an unsafe or panic site where the claim forbids one, a forbidden dependency), a failing test, a proof-rule breach, or a semantic contradiction that survived a challenge. Its `category` says which. |
| `unverified` | Nothing establishes the claim either way. This includes a test result recorded before a source change (re-run the tests), a requirement no test declares `Verifies:` for, and a suspected contradiction that has not been confirmed. |
| `planned` | An `unverified` requirement whose milestone is not in `[gate] started_milestones`. A requirement with evidence keeps its own status. |
| `stale` | Evidence the status rests on was invalidated and not re-established (semantic evidence after a change), or cited code changed at the `compare` revision (`citation_drift`). Never a gate failure. |
| `ambiguous` | The claim cannot be read one way: vague or self-contradictory wording, conflicting bounds, a requirement declared by more than one proof, or a semantic `ambiguous` verdict. |
| `undocumented` | Implementation behaviour (panics, error returns, environment reads, missing contract on public API) that no claim covers. Reported as `missing_documentation`. |

A **resolved** finding is not a status but a lifecycle: its subject no longer shows the issue or no longer
exists. It is kept in the state, hidden unless `--include-resolved` is given, and not counted by the gate.
Human output lists every `violated`, `stale` and `ambiguous` finding and the first ten of each other status;
`-v`/`--verbose` lists them all.
`--force` re-parses every file even when its content hash is unchanged.

## Output

- `scan` (default) prints a human summary on stdout and progress on stderr.
- `scan --json` prints one JSON report: `tool`, `version`, `repository`, `revision`, `semantic`,
  `counts`, `gate`, `citations` (only with `[repos]`), and `findings[]`. Each finding includes its `claim_refs` and `symbol_refs`, with
  exact ranges for downstream tooling, and its requirement's `milestone` when it has one.
- `scan --jsonl` prints every event envelope as one line, then the report as the last line.
- `watch` streams events until Ctrl+C; with `--jsonl` the stream is machine-readable.
- `report` reads persisted state without scanning or probing Claude.
- `serve` listens on `127.0.0.1:4173` unless `--host` or `[server]` says otherwise (`--port` likewise). A
  non-loopback address exposes the UI to the network.
- `lint` (`--json`) prints `tool`, `command`, `strict`, `errors`, `warnings`, `infos` and `findings[]`; see
  [spec-lint.md](spec-lint.md).

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
## Citations

A specification can cite files in other git repositories:

```
[APP:apps/api/src/catalog.ts:67-132]   lines 67 to 132
[APP:docs/adr/0002:20,374-384]                    several ranges; `0002` abbreviates `0002-naming.md`
[APP:docs/adr/0003-...:21]                        an explicit abbreviation
[APP:AGENTS.md "Data and Migrations"]             a heading (also `#data-and-migrations`)
[WEB:services/worker/]                       a directory (a bare top-level name needs `/` or lines)
[APP:apps/api/src (search)]                       free text after a space is a note
[app:a.ts:1-2; app:b.md:3]                        several citations in one bracket (`;`, or `,` before an alias)
```

Citations are found in paragraphs, list items, table cells and headings, never inside code spans.
The path must contain `/` or `.`, so Jira keys and page ids (`[PROJ-351]`, `[Wiki:1031]`) are not
citations. Each alias is configured once:

```toml
[repos.APP]
path = "../platform"   # relative to the config file
cited = "27e5fb9"                     # the revision the citations describe
compare = "origin/main"               # optional
spellings = ["app"]                   # optional; aliases also match regardless of case
```

The repository is read with `git ls-tree`, `git cat-file` and `git diff` at those revisions only;
its working tree and checked-out branch do not matter, and nothing is written. A `compare` revision
that does not exist is a warning (drift is then not checked).

| Citation | Finding |
|---|---|
| Path missing at `cited` | `violated`, structural contradiction (fails `--fail-on proven`) |
| Line range past the end of the file, a range on a directory, a missing heading | `violated`, structural contradiction, naming the file length |
| Path abbreviates exactly one entry (`docs/adr/0005`, `0003-...`) | none; recorded as `Strong` (not `Proven`) support with a note |
| Path abbreviates several entries | `ambiguous` |
| Cited lines (or the whole cited file) differ at `compare`, or the file was deleted | `stale`, category `citation_drift` (never fails the gate) |
| Same text at other lines or under a renamed path at `compare` | none; recorded as support |
| Glob path (`services/*/src`) | `violated` only if nothing matches; lines are not checked |
| Alias without a `[repos]` entry | none; counted in `citations.note` |

Drift is `stale`, not `violated`: the citation is still true of the revision it names, so nothing in
the specification is contradicted. What changed is that the evidence the statement rests on may no
longer describe current code, which is what `stale` means for invalidated evidence. A citation in a
claim makes that claim's finding `stale` unless something stronger (a contradiction, a verification)
decides it; a citation in plain prose gets its own finding keyed by the document and the citation text.

`citations` in the JSON report counts distinct citation texts per alias: `citations`,
`occurrences`, `resolved`, `prefix_resolved`, `ambiguous`, `missing`, `bad_lines`, `drifted`,
`moved`. The resolved revisions are part of the persisted state version: when `compare` moves (after
`git fetch`) or the configuration changes, the next start re-derives every citation.

## Semantic analysis

`--semantic` (or `[semantic] enabled = true`) uses the local Claude Code CLI. The binary is
`claude`, or whatever `claude_path` names. With `[semantic] context = "excerpts"` it is invoked as:

```
claude -p --output-format json --json-schema <schema> --tools "" --no-session-persistence \
       --setting-sources "" --system-prompt <...> --model <model>
```

The default, `context = "explore"`, adds read-only repository tools and a turn limit (`max_turns`):
`--tools Read,Grep,Glob --allowedTools Read,Grep,Glob --add-dir <repo> --max-turns <n> --strict-mcp-config`.

The prompt goes on stdin, and the working directory is a neutral folder inside the state directory.
If the CLI is missing, a `system.warning` is emitted, the status reports `available: false`, and
behavioral claims stay `unverified`.
