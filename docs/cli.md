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
schema (it is intentionally small: roots, excludes, state dir, authority globs, semantic, server,
exegra, cited repositories).

Spec roots hold Markdown (`.md`) and Exegra JSONC specifications (`.jsonc`/`.json` with a
top-level `"exegra"` key); see [exegra.md](exegra.md).

`repo` may be a plain parent directory whose spec and source roots are separate git repositories
(a workspace). Each root's own `.gitignore` applies, and every `package.json` under the source
roots is a TypeScript package.

## Exit codes

| Code | Meaning |
|---|---|
| 0 | Success. With `--fail-on`, the gate passed. |
| 1 | The `--fail-on` gate failed: `proven` means at least one open structural contradiction or failing requirement test; `violated` also counts semantic contradictions that survived an adversarial challenge. |
| 2 | Usage or configuration error (bad flags, missing roots, invalid TOML, no persisted state for `report`, a `[repos]` entry that is not a git repository or whose `cited` revision does not exist). |
| 3 | Runtime error (state database, I/O, cannot bind the port). |

`unverified`, `ambiguous`, `stale` and `undocumented` findings never fail the gate: they are not
violations. Missing Claude is never treated as successful semantic verification. The report's
`semantic.coverage` field states exactly what was and was not assessed.

## Output

- `scan` (default) prints a human summary on stdout and progress on stderr.
- `scan --json` prints one JSON report: `tool`, `version`, `repository`, `revision`, `semantic`,
  `counts`, `gate`, `citations` (only with `[repos]`), and `findings[]`. Each finding includes its
  `claim_refs` and `symbol_refs`, with exact ranges for downstream tooling.
- `scan --jsonl` prints every event envelope as one line, then the report as the last line.
- `watch` streams events until Ctrl+C; with `--jsonl` the stream is machine-readable.
- `report` reads persisted state without scanning or probing Claude.

None of the commands need a TTY. They work with piped stdin and stdout.

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
`claude`, or whatever `claude_path` names. It is invoked as:

```
claude -p --output-format json --json-schema <schema> --tools "" --no-session-persistence \
       --setting-sources "" --system-prompt <...> --model <model>
```

The prompt goes on stdin, and the working directory is a neutral folder inside the state directory.
If the CLI is missing, a `system.warning` is emitted, the status reports `available: false`, and
behavioral claims stay `unverified`.
