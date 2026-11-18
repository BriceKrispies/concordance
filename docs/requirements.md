# Requirements verified by tests

Structural checks cover claims a parser can check (references, signatures, limits). Behavioural
prose needs either a model (semantic analysis) or a test. This convention makes the test route
deterministic: a specification statement carries an ID, a test declares that it verifies that ID,
and a recorded run says whether the test passed.

## In the specification

Give each requirement an ID in one of two ways:

```markdown
**LANG-006.3** A block comment SHALL end at the `*/` that balances its opening `/*`.
```

A bold ID opening a paragraph or list item makes that paragraph one requirement. This is the
form to prefer: one ID, one statement.

```markdown
## PSS-003
...
```

A heading that is (or starts with) an ID tags every claim in its section and subsections. A test
cannot say which of several bundled obligations it exercised, so for a heading that holds more than
one obligation, test results are recorded but never verify or violate a claim on their own; the
finding says the requirement should be split.

A bold-ID requirement may carry attribute bullets directly after it (`Verify:`, `Boundary:`,
`Traces:`, `Rationale:`, `Depends:`, `Status:`, `Cases:`, `Milestone:`, `Seam:`). They are stored on the requirement, not extracted as
claims, and `concordance lint` checks them: see [spec-lint.md](spec-lint.md).

An ID is an upper-case prefix, optional upper-case/digit segments and a number with an optional
dotted sub-number: `PSS-003`, `LANG-006.3`, `WEB-SEC-004`, `B-5`. The number may carry a one- or
two-letter tag (`PII-T1`, `AI-R3`). IDs are recognised only where they are
defined (bold opener or heading) and where a test declares them, so similar-looking keys in prose
(`PROJ-2147`) are not requirements. `concordance lint` is stricter than this: its rule R8 accepts only
`PREFIX-NNN` with three digits, so an ID that `scan` accepts (`P-1`, `LANG-006.3`) is an R8 error under
`lint`. See [spec-lint.md](spec-lint.md#a-requirement).

## Structured sources (Exegra JSONC)

A project may keep its whole specification as **one JSONC file in the Exegra format** (JSON with `//`
and `/* */` comments and trailing commas), edited as structured data, instead of Markdown files. Name
the file in `spec_roots` (`--spec-root specs/spec.jsonc`), or leave it under a spec root: a `.jsonc` or
`.json` file with a top-level `"exegra"` key is a structured source; any other `.json` file is ignored.
Markdown input is unchanged and may be used alongside.

Concordance reads the file's **properties**, renders each document to Markdown in the formats this
documentation describes, and indexes that text exactly as it indexes a file. So linking, verification,
lint, authority globs (`[authority]`, `[lint] whitepaper`, `[lint] spec`) and `Cases:` files work on
the structured source with the same paths they had as Markdown. The rendered texts are *virtual
documents*; nothing is written to disk.

| Source | Becomes |
|---|---|
| `documents[]` `{id, path, title, role}` | one virtual document at `path` (a repository-relative name; no file need exist). `role` is not interpreted, except that `cases` documents take their text from their case table |
| `sections[]` `{id, title, level, document, order, body}` | the heading line (`#` x `level` + `title`; none for level 0) and `body`, in `order` |
| marker line `{{entity:<id>}}` in a `body` | the rendering of that entity (below). A marker that cannot be rendered stays in the text and is reported |
| `caseTables[]` `{path, document, body}` | the text of the `cases` document `document` |

**Rendering of entities** (line breaks inside a property become spaces):

| Entity | Rendered |
|---|---|
| requirement (`claims[]`, `"kind":"requirement"`) | `**<ref>** <statement>` and attribute bullets, below |
| goal, non-goal, constraint, decision, open question | `- **<ref>** <statement>`. A non-goal's `horizon` is written in italics before the statement; a goal's `stage` in italics after it. A decision with no `statement` uses its `title` |
| boundary | a table row `\| <ref> \| <name> \| <interfaceKind> \| <usedBy> \|`, preceded by a header and separator row when the previous line is not a row of the same table |
| component | a table row `\| <ref> \| <name> \| <purpose> \| <talksTo> \|`, with the same header rule |
| release stage | a table row `\| <ref> \| <name> \| <purpose> \| <milestones> \| <exitEvidence> \|`, the stage's name in bold in the first column |
| milestone | its `detail` lines, as they are (with no `detail`, one paragraph) |
| term | a list item `- **<term>**: <definition>` (`- **<term>**` alone with no definition) |
| any other entity | one paragraph of its main text properties: `<name or title>: <purpose, statement, definition, proves, detail, exitEvidence, description>` |

**The Exegra requirement profile.** A requirement is a `claims[]` entity with `"kind":"requirement"`.
These properties are read, and an attribute bullet is written only when its property is present, in
this order:

| Property | Rendered |
|---|---|
| `ref` (required) | the ID in `**<ref>**`, as written (`KRN-002`) |
| `statement` (required) | the statement after the ID |
| `lifecycle` = `retired` | `- Status: retired` |
| `evaluate.method` | `- Verify: <method>` |
| `cases[]` (case table ids), `casesPath` | `- Cases: <path of each case table>, <casesPath>`. A `casesPath` matches no case table; it is reported and written as is |
| `boundary` (id of an entity with a `ref`), `boundaryDetail` | `- Boundary: <ref of the boundary> <boundaryDetail>` |
| `traces[]` (entity ids) | `- Traces: <ref>, <ref>` |
| `dependsOn[]` (claim ids) | `- Depends: <ref>, <ref>` |
| `rationale` | `- Rationale: <rationale>` |
| `milestone` | `- Milestone: <n>` |
| `seam` | `- Seam: <n>` |

A requirement without `ref` or `statement` is an error: its marker stays in the text. A reference to an
entity that does not exist (or has no `ref`) is a warning, and the id is written instead.

**`raw` is never read.** Exegra entities may carry `raw`, the Markdown they were first imported from.
It goes stale as soon as a property is edited, so Concordance renders from properties only. A section's
`body` is a property like any other.

**Virtual documents.** A virtual document is keyed by its `path` like a file, and hashed by its rendered
text. Rescanning re-renders every source, re-indexes only the documents whose text changed and
removes the ones that no longer render. The Markdown and the structured form of one document must not
both be present: a path that is both a Markdown file and a virtual document, or the `path` of two
sources, is reported as an error and **neither is indexed**.

**Locations.** Lines and ranges are those of the rendered text, so a finding is at
`specifications/kernel.md:23`. Where a location is in a virtual document, outputs also say where it was
rendered from: `specifications/kernel.md:23 (specifications/spec.jsonc:1320 krn-002)` is the JSONC
file, the line of the entity and the entity id. See [cli.md](cli.md) and [api.md](api.md).

**Problems in the source** (a marker naming an unknown entity, a section naming an unknown document,
duplicate document paths, a `cases` document without a case table, a requirement without `ref` or
`statement`, a duplicate entity id, a line that is more than one marker, a reference that does not
resolve, a `casesPath`) are reported on every scan as system events and by `concordance lint` as rule
`R0`: never silently dropped.

**Results for Exegra.** `concordance scan --exegra-results FILE` and `concordance report
--exegra-results FILE` write each requirement's `pass`/`fail` result and the lint findings by entity
id, in a file Exegra reads. See [cli.md](cli.md#exegra-results).

## Milestones

`Milestone: N` names the milestone at which a requirement is first verified; `Seam: M` (with `M <= N`)
the milestone by which its data shape or contract must exist. To gate only the requirements of started
milestones, list them in `concordance.toml`:

```toml
[gate]
started_milestones = [1, 2]
```

With `started_milestones` set:

- a requirement whose `Milestone:` is not listed is reported as `planned` instead of `unverified`
  (a requirement with evidence keeps its `verified`, `violated`, `stale` or `ambiguous` status);
- `scan --fail-on` and `report --fail-on` ignore every finding of such a requirement, including a
  failing test; the gate counts them as `ignored_unstarted`;
- a requirement with no `Milestone:` (or one that is not a positive integer) counts as started;
- without `started_milestones`, nothing is planned.

The scan JSON carries `counts.planned`, `findings[].status = "planned"` and `findings[].milestone`
(the requirement's milestone, when it has a valid one). The requirement's claims carry the raw
`milestone` and `seam` under `requirement.attributes`.

## Case tables

A requirement with `Verify: conformance` (or `Verify: proof` that links its table with an optional `Cases:`) states a capability once and lists the detail in case tables:
the `## Cases` section of each file named by its `Cases:` attribute (see [spec-lint.md](spec-lint.md)).
Concordance's own umbrella requirements (`docs/spec`) work this way. The prose above the table stays for
people; the table is what is checked.

| Column | Meaning |
|---|---|
| `Case` | a unique short name |
| `Requirement` | the umbrella ID the row belongs to |
| `Results` | `none`, or a results file relative to the fixture, installed as the configured test-results file before the scan |
| `Claim contains` | a substring of the finding's title or of one of its claims' text (empty: every finding) |
| `Status` | the expected status (`violated`, `verified`, `unverified`, `undocumented`, ...); `!status` for any other; `absent` or `absent:status` when no such finding may exist |
| `Category` | optional: `structural_contradiction`, `test_failure`, `supported`, `missing_documentation`, ...; `!category` negates |
| `Location` | optional `path[:line]`: one of the finding's locations is in that file and covers that line; `!path[:line]` negates |

The driver (`tests/src/conformance.rs`) reads the requirement's `Cases:` list through the Markdown
indexer, copies each fixture, groups the rows by `Results`, installs each results file, scans, and checks
every row. It reports all failing rows by case name, fixture and what was found, and fails once. Adding a
row is enough to add a case. A single `Verifies:` test per umbrella requirement runs the driver.

The lint requirements use a lint case table (`Case | File | Rule | Line | Severity`, see
`fixtures/spec-lint/EXPECTED.md`) run through the `concordance` binary.

## In the tests

```rust
/// Verifies: LANG-006.3
#[test]
fn nested_comment_ends_at_the_balancing_close() { ... }
```

A `Verifies:` line in a test function's doc comment links it to one or more requirements
(`Verifies: LANG-006.3, LANG-006.4`). A `Verifies:` naming an ID that no specification defines is
reported as a violated finding on the test. A `#[test]` declared inside a `proptest!` block is indexed
as a test like any other, with its own name and `Verifies:` line.

Beyond the unknown ID, `scan` reports two more breaches of the proof rules as findings
(see [spec-lint.md](spec-lint.md#proof-checks-in-scan)): a requirement declared by more than one
proof, and a proof whose resolved calls all go to non-public symbols.

TypeScript/JavaScript tests (`it(...)` / `test(...)`, at any depth, with or without an enclosing
`describe(...)`) take the `Verifies:` line from the comment directly above the call:

```ts
describe("Mixer", () => {
  // Verifies: VOL-001
  it("rejects levels above 100", () => { ... });
});
```

C# tests (methods marked `[Fact]`, `[Theory]`, `[Test]`, `[TestCase]` or `[TestMethod]`) take the
`Verifies:` line from their `///` XML doc comment or the `//` comment directly above:

```csharp
// Verifies: VOL-001
[Fact]
public void Rejects_level_above_100() { ... }
```

Lean theorems (`theorem` / `lemma`) are tests when they carry `Verifies:` in their `/-- -/` doc
comment or in the `--` comment lines directly above:

```lean
/-- Verifies: VOL-001 -/
theorem clamp_le_max (n : Nat) : clampLevel n ≤ maxLevel := by ...
```

## Test results

Concordance does not run the analysed repository's tests. Point it at their output:

```
cargo test 2>&1 | tee C:/ci/tests.txt
concordance serve --config concordance.toml --test-results C:/ci/tests.txt
```

or `test_results = ["C:/ci/tests.txt"]` in the config file. Accepted, mixed freely: libtest text,
libtest JSON lines (`-Z unstable-options --format json`), cargo-nextest output, JUnit XML
(`vitest run --reporter=junit --outputFile=...`, most CI reporters) and vitest/jest text output
(`✓ file > describe > case`). For .NET, `dotnet test --logger trx` (Visual Studio TRX) and
`--logger junit` output are read; results are named `Namespace::Class::Method`, and the rows of a
`[Theory]` count as one test whose worst outcome wins.
For Lean, a `lake build` log (`lake build 2>&1 | tee build.log`): a tagged
theorem passes when its module built and no error or `declaration uses 'sorry'` warning lies inside
the theorem's lines; it fails when one does, or when its module failed to build; a module the log
does not mention gives no result.
Result names are matched to test functions by their module path. The file is watched, so writing
a new run re-derives every requirement finding.

| Recorded outcome | Status of a one-statement requirement |
|---|---|
| passed, run recorded after the last change to any indexed source file | `verified` |
| failed | `violated` (`test_failure`; counts for `--fail-on proven`) |
| recorded before a source change | `unverified`: the result is outdated, re-run the tests |
| ignored, or no run reports the test | `unverified` |
| no test declares `Verifies:` for it | `unverified`: "has no test" |

The freshness rule is conservative: any change to an indexed source file after the run makes all
results outdated, since Concordance cannot know which code a test exercised.
