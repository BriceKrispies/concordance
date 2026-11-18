# Specification lint

`concordance lint [--strict] [--json]` checks that a specification follows the requirement format of
the spec-driven workflow. That workflow's template is not part of this repository; this document is the
reference for what the lint checks. The lint reads the Markdown specification, the white paper and the
indexed code. It never runs code.

```
concordance lint [common] [--strict] [--json]
```

Exit status: 0 when there is no error, 1 when there is at least one error, 2 and 3 as for the other
commands. `--strict` (or `[lint] strict = true`) turns every warning into an error. `--json` prints
one object: `strict`, `errors`, `warnings`, `infos` and `findings[]`, each with `rule`, `severity`
(`error`, `warning` or `info`; `info` is a review hint, see S8), `message`, `path` (repository-relative), `line` (1-based) and, only for a
finding in a virtual document, `origin` (`file`, `line`, `entity`: where that line was rendered from in
the structured source). Human output is one `path:line: error[R1] message` line per finding; in a virtual
document it is `path:line (source.jsonc:1320 krn-002): error[R1] message`.

## What is linted

- **Specification files:** the Markdown files under the spec roots whose authority is `normative`
  (set `[authority] normative = ["docs/spec/**"]`). Design documents are not linted. `[lint] spec`
  restricts this to the normative files matching its globs (repository-relative; default: all of them).
  When no file is left the lint reports rule `R0`. Virtual documents of a structured source count as
  files here.
- **White paper:** `[lint] whitepaper`, default `docs/whitepaper.md`. Its IDs `G-n`, `NG-n`, `B-n`, `K-n` (constraints),
  `C-n`, `D-n`, `Q-n` are bold openers (`- **G-1** ...`) or first table cells (`| B-1 | ... |`).
  Code fences and HTML comments are skipped. A missing white paper is one `R5` finding.
- **Spec index:** the shallowest normative `index.md`. Its table (columns `File` and `Prefix`) says
  which ID prefixes each file owns.
- **Code:** the indexed source roots, for rule R3.

## Structured sources

The lint reads the Markdown of a specification kept in an Exegra JSONC file (see
[requirements.md](requirements.md#structured-sources-exegra-jsonc)) from the virtual documents the
engine rendered. Rules, `[authority]` globs, `[lint] whitepaper` and `[lint] spec` apply to a virtual
document under its `path` exactly as to a file, because the rendered text is in the formats below:

- a requirement is `**<ref>** <statement>` followed by `Status:`, `Verify:`, `Cases:`, `Boundary:`,
  `Traces:`, `Depends:`, `Rationale:`, `Milestone:`, `Seam:` bullets for the properties present;
- goals, non-goals, constraints, decisions and open questions are `- **G-1** ...` bullets, and boundaries
  and components are table rows `| B-1 | ... |` (so the white-paper reader finds them);
- a `Cases:` file that is a virtual document (a case table of the source) is read from its rendered
  text, never from the file system;
- a finding in a virtual document carries its `origin`, and problems in the source itself (an unknown
  marker, a `casesPath` that matched no case table, ...) are reported as rule `R0` at the source file:
  an error for a missing or contradictory entity, a warning for a reference or path that does not
  resolve.

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

The lint takes only a bold ID opening a paragraph or list item as a requirement. A requirement given
by a heading (`## PSS-003`, see [requirements.md](requirements.md#in-the-specification)) is not one to
this lint: its statement is read as prose, so R7 reports its `shall`/`must` words. `concordance scan`
does treat heading IDs as requirements.

Identifiers: R8 accepts exactly `PREFIX-NNN` (an upper-case prefix, a dash, three digits). `concordance
scan` accepts what [requirements.md](requirements.md#in-the-specification) describes: one to four digits
and a dotted sub-number (`P-1`, `LANG-006.3`). The two disagree, so `lint` reports as R8 the IDs that
`scan` accepts. Concordance's own specification uses three digits throughout.

## Rules

| Rule | Severity | Reports |
|---|---|---|
| R0 sources | error / warning | no normative specification file found; a problem in a structured source (see above), reported at the source file |
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

## Semantic rules over a structured source

Rules R1..R8 read rendered Markdown. The rules S1..S9 read the entities of an Exegra JSONC source
directly (`claims`, `caseTables`, `terms`, `decisions`) and check the specification against itself.
They run for every structured source of the repository and report at the source file, on the line of
the entity (with its `origin`). A repository without a structured source has no S findings. "Active"
means a requirement whose `lifecycle` is not `retired`.

| Rule | Key | Default | Reports |
|---|---|---|---|
| S1 milestone order | `milestone_order` | warning | an active requirement whose `dependsOn`, or whose statement (an id such as `KRN-048`), names an active requirement of a later milestone; or whose statement names, in backticks, a row of one of its own case tables whose `Milestone` column is later |
| S2 budget row | `budget_row` | error | an active requirement verified as `budget` that has no row in the budget-measurement table (the case table whose path contains `budget-measurement`; its `Requirement` column holds the id). Silent when the source has no such table |
| S3 sub-budget | `sub_budget` | warning | a parent budget worded as a decomposition (`together`, `in total`, `end to end`, `included`, ...) whose single `within`/`at most`/`under` time bound is smaller than the sum of the bounds of the budgets it depends on. Only exact parses are compared: one time bound per statement and the same percentile on both; anything else is skipped silently |
| S4 constants | `constants` | warning | needs `names`. A rationale that gives a number with a unit in a sentence naming a configured constant, when the constants table of the requirement's area has no row for it; with `statements = true` (default), a statement that names a configured constant that no constants table defines. A constants table is a case table whose path contains `constants` (`tables` to change); its `Constant` column (else its second column) and case name are matched against the name, ignoring case and punctuation |
| S5 forbidden terms | `forbidden_terms` | warning | needs `forbidden = [{ phrase, replacement }]`. A phrase in an active requirement (statement, rationale), a decision, a glossary definition, a case table or a section. Matching is by whole words, ignoring case and whitespace runs, and keeps hyphens: `sign-in session` and `sign in session` are different phrases. A sentence that also names the replacement and says `formerly`, `replaces`, `replaced`, `instead of` or `deprecated` is skipped |
| S6 duplicate terms | `duplicate_terms` | error | a glossary term (`terms`) defined twice, ignoring case, punctuation and a trailing plural `s` |
| S7 dead reference | `dead_references` | warning | text of an active requirement statement (and its rationale with `rationales = true`), of a decision, or of a case row of a table that an active requirement cites, that names a retired requirement, or an id whose prefix is known but that does not exist. A sentence containing a history word (`retired`, `removed`, `deleted`, `replace`, `supersed`, `merged`, `dropped`, `withdrawn`, `formerly`, `no longer`, `renumbered`, `relaxes`, `obsolete`; `markers` replaces the list) is a history note and is skipped, as is the text of retired requirements |
| S8 said once | `said_once` | info | an active statement whose stemmed content words have Jaccard similarity of at least `threshold_percent` (default 75) with an earlier one, among statements of at least `min_tokens` (default 6) words. Pairs R6 already reports, pairs that differ by one substituted word on each side, and pairs that differ by different numbers are skipped. One finding per statement, naming its best match |
| S9 replacement character | `replacement_char` | error | U+FFFD, as the character or as the escape `�`, anywhere in the source file. It runs even when the file does not parse |

Severity `info` is a review hint: it never fails the lint and `--strict` does not raise it. The JSON
report has an `infos` count and the human summary adds `N hint(s)`.

Every rule is configured under `[lint.rules.<key>]`. All keys are optional; an unknown rule name is a
configuration error.

```toml
[lint.rules.milestone_order]
enabled = true                      # default true for every rule
severity = "warning"                # "error", "warning" or "info"

[lint.rules.budget_row]
tables = ["budget-measurement"]     # path substrings of the budget-measurement table

[lint.rules.sub_budget]
markers = ["together", "end to end"]  # words that mark a parent budget

[lint.rules.constants]
names = ["projection lag", "replay window"]   # the configured constant names; none, no findings
tables = ["constants"]
statements = true

[lint.rules.forbidden_terms]
forbidden = [{ phrase = "browser session", replacement = "sign-in session" }]

[lint.rules.dead_references]
rationales = false                  # also read rationales
markers = ["retired", "removed"]    # replaces the history words

[lint.rules.said_once]
threshold_percent = 75
min_tokens = 6
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
  anything under `tests/`) so a helper that drives the public API counts as the proof.
- A Rust proof with no call that resolves to indexed code at all (an empty test, one that only calls other
  crates): an ambiguous, low-severity finding with `rule = calls_nothing`. Calls in the arguments of
  `assert!`, `assert_eq!`, `assert_ne!` and their `debug_` forms are read. A proof that runs the package's own
  binary (`env!("CARGO_BIN_EXE_<name>")`, in the proof or in a test helper it calls) reaches the command-line
  boundary and is not reported. TypeScript, C# and Lean proofs are not judged by their calls.
- A Lean theorem that states `True`: a violated finding with `rule = trivial_statement`; one that mentions,
  directly or through the definitions and theorems it names, an `axiom` declared in the repository: an
  ambiguous, low-severity finding with `rule = uses_axiom`.
- A passing result of a proof reported by any of these rules does not verify its requirement: the requirement
  stays `unverified` and says why.

Resolution is syntactic: calls through trait objects and closures are not followed, a method call
  matched by name alone (`x.name()` where the receiver's type is not known, `Resolution::NameOnly` in the
  graph) does not count as reaching that symbol, a bare name resolves first to a function of the caller's own
  file, and a private item of another crate is never a call target.
