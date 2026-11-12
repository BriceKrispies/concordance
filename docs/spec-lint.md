# Specification lint

`concordance lint [--strict] [--json]` checks that a specification follows the requirement format of
the spec-driven workflow (`docs/agents/spec-format.md` and `verification.md` in that template). The
lint reads the Markdown specification, the white paper and the indexed code. It never runs code.

```
concordance lint [common] [--strict] [--json]
```

Exit status: 0 when there is no error, 1 when there is at least one error, 2 and 3 as for the other
commands. `--strict` (or `[lint] strict = true`) turns every warning into an error. `--json` prints
one object: `strict`, `errors`, `warnings` and `findings[]`, each with `rule`, `severity`
(`error` or `warning`), `message`, `path` (repository-relative) and `line` (1-based). Human output is
one `path:line: error[R1] message` line per finding.

## What is linted

- **Specification files:** the Markdown files under the spec roots whose authority is `normative`
  (set `[authority] normative = ["docs/spec/**"]`). Design documents are not linted. `[lint] spec`
  restricts this to the normative files matching its globs (repository-relative; default: all of them).
  When no file is left the lint reports rule `R0`.
- **White paper:** `[lint] whitepaper`, default `docs/whitepaper.md`. Its IDs `G-n`, `NG-n`, `B-n`, `K-n` (constraints),
  `C-n`, `D-n`, `Q-n` are bold openers (`- **G-1** ...`) or first table cells (`| B-1 | ... |`).
  Code fences and HTML comments are skipped. A missing white paper is one `R5` finding.
- **Spec index:** the shallowest normative `index.md`. Its table (columns `File` and `Prefix`) says
  which ID prefixes each file owns.
- **Code:** the indexed source roots, for rule R3.

## A requirement

```markdown
**CLI-004** `tool build` SHALL exit with status 2 and write nothing when the manifest names a missing file.
- Verify: scenario
- Boundary: B-1 `tool build`
- Traces: G-2
- Rationale: A partial build that looks successful is worse than none.
```

The list items directly after the opener that start with `Verify:`, `Boundary:`, `Traces:`,
`Rationale:`, `Depends:`, `Status:`, `Cases:`, `Milestone:` or `Seam:` are the requirement's attributes. They are stored on the
requirement (`RequirementRef.attributes` on its claims) and are not extracted as claims. Other list
items under a requirement are ordinary prose and fall under R7. A requirement with `Status: retired`
is only checked by R8.

## Rules

| Rule | Severity | Reports |
|---|---|---|
| R1 atomic | error | no SHALL/SHALL NOT/MUST in the statement; more than one; a semicolon; "and" joining two modal clauses (one finding per statement, naming every reason) |
| R2 vague | error | a banned word in the statement: correctly, properly, efficient(ly), robust(ly), appropriate(ly), as expected, user-friendly, fast, secure(ly) |
| R3 internals | error / warning | see below |
| R4 provable | error | `Verify:` missing or not one of `example`, `property`, `scenario`, `budget`, `proof`, `conformance`; `Verify: conformance` without a `Cases:` list, or with a file that does not exist or has no `## Cases` table; `Cases:` on any method other than `conformance` and `proof` (on `proof` it is optional and checked the same way); never/always/for every/for all/for each with `example` or `scenario`; a time, size or count limit (`5 ms`, `10MB`, `90%`, `at most 3`, `no more than`, `fewer than`, `up to`, ...) with a method other than `budget`; with `[lint] require_milestone = true`, a missing `Milestone:`, a `Milestone:` or `Seam:` that is not a positive integer, or a `Seam:` greater than the `Milestone:` |
| R5 traceable | error / warning | `Traces:` missing, naming no white-paper ID or an unknown one; `Boundary:` missing, without a B- ID or naming an unknown one; a goal `G-n` or constraint `K-n` that no (non-retired) requirement traces to, reported at the white paper line (errors). `Boundary:` with no backticked surface, and `Depends:` naming an undefined requirement (warnings) |
| R6 said once | warning | two statements whose token sets (lower-cased, minus `the a an of to be is are shall must`) have Jaccard similarity of 0.8 or more; reported on the later one |
| R7 normative text | error | `must`, `shall`, `never`, `always` or `required` in a paragraph, list item or quote that is not a requirement or one of its attributes. Headings, tables, code blocks and code spans are exempt |
| R8 IDs | error | malformed ID (not `PREFIX-NNN`: upper-case prefix, dash, three digits); duplicate ID (also in the white paper); a prefix not owned by the file in the spec index, or a file the index does not list |

**R3 in detail.** A backticked span in the statement is checked when it looks like a code reference
(an identifier, a `::` path, a call, a source file).

- Error: it resolves to an indexed symbol that is not public API (with several candidates, all of
  them non-public). The existing reference resolution is used, so the answer is syntactic.
- Warning: it is an identifier that does not appear in the `Boundary:` surface (the backticked text
  in the `Boundary:` bullet), or a source file of the project.
- Warning: the statement contains an internal marker word: cache(s), algorithm, hashmap, hash map,
  btreemap, mutex, thread pool, internal(s), plus `[lint] internal_markers`.

Spans such as `tool build` or `--out` are not code references and are never flagged.

## Configuration

```toml
[lint]
whitepaper = "docs/whitepaper.md"   # relative to the repository
strict = true                        # warnings fail the lint
require_milestone = true             # optional: every requirement needs a valid Milestone:/Seam:
vague_words = ["fast", "secure"]     # optional: replaces the built-in banned words
internal_markers = ["shard"]         # optional: added to the built-in markers
spec = ["docs/spec/**"]              # optional: lint only the normative files matching these globs
```

## Milestones

```markdown
- Milestone: 3
- Seam: 1
```

`Milestone:` is the milestone at which the requirement is first verified; `Seam:` is the milestone by
which its data shape or contract must exist. Both are positive integers (`1`, `2`, ...), stored on the
requirement like the other attributes. With `[lint] require_milestone = true`, R4 reports, as errors on
the requirement: a missing `Milestone:`; a `Milestone:` or `Seam:` that is not a positive integer; a
`Seam:` greater than the `Milestone:`. Without the option the attributes are stored and not checked.
`[gate] started_milestones` makes scan use the milestone (see [requirements.md](requirements.md#milestones)).

## Conformance requirements

A requirement whose behaviour is a table of cases rather than one example uses `Verify: conformance`
and names its case tables:

```markdown
**LANG-001** A scan of a repository in a supported language SHALL report every finding listed in that language's conformance cases.
- Verify: conformance
- Boundary: B-5 `concordance_engine::start`
- Cases: fixtures/typescript/EXPECTED.md, fixtures/lean/EXPECTED.md
- Traces: G-1
```

`Verify: proof` may also list `Cases:` (for example the theorem statements of a formal development); there it
is optional and the files get the same shape checks. `Cases:` is a comma-separated list of repository-relative files. Each must exist and hold a `## Cases`
heading followed by a Markdown table whose first column is `Case` (unique, non-empty names, one row at
least, every row as wide as the header). Anything else is an R4 error on the requirement's `Cases:` line.
The columns after `Case` belong to the proof that reads the table (see
[requirements.md](requirements.md#case-tables)); the lint checks only the shape. A row added to
a table is a case added, with no change to the requirement.

## Proof checks in scan

`concordance scan` (and the UI) report, as findings on the proof or the requirement:

- A `Verifies:` naming a requirement no specification defines: a violated finding on the test
  (existing behaviour, see [requirements.md](requirements.md)).
- A requirement declared by more than one proof (`Verifies: ID` on two items): an ambiguous,
  low-severity finding with `subject.kind = proof_rule`, `rule = multiple_proofs`, listing every
  proof.
- A declaring proof whose resolved calls are all non-public symbols: a violated finding with
  `rule = bypasses_boundary`. Calls are followed through test scaffolding (tests, `cfg(test)` items,
  anything under `tests/`) so a helper that drives the public API counts as the proof. A proof with
  no resolved call at all (a scenario that runs the command as a process) is not reported.
  Resolution is syntactic: calls through trait objects and closures are not followed.
