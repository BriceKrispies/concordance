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

An ID is an upper-case prefix, optional upper-case/digit segments, and a number of one to four digits
with an optional dotted sub-number: `PSS-003`, `LANG-006.3`, `WEB-SEC-004`, `B-5`, `H-012`. The number may carry a
one- or two-letter tag (`PII-T1`, `AI-R3`). `concordance lint` uses this same grammar (R8 reports an ID
that does not match it). IDs are recognised only where they are defined (bold opener or heading) and
where a test declares them, so similar-looking keys in prose (`PROJ-2147`) are not requirements. The
same ID opened in two documents is a `duplicate_id` proof-rule finding. See
[spec-lint.md](spec-lint.md#identifiers).

## Structured sources (Exegra JSONC)

A project may keep its whole specification as **one JSONC file in the Exegra format** (JSON with `//`
and `/* */` comments and trailing commas), edited as structured data, instead of Markdown files. Name
the file in `spec_roots` (`--spec-root specs/spec.jsonc`), or leave it under a spec root: a `.jsonc` or
`.json` file with a top-level `"exegra"` key is a structured source; any other `.json` file is ignored.
Markdown input is unchanged and may be used alongside. The documents of a structured source are
normative (authority source `SpecFormat`) whatever their RFC keyword count, unless `[authority]` names
another level for their path.

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

**The Exegra requirement profile.** A requirement is a `claims[]` entity with `"kind":"requirement"`. Any other `kind` (`"Requirement"`,
`"requirment"`) is not a requirement: it renders as a paragraph of prose and is checked as prose, so
a statement with an upper-case modal gets an R7 error and one without gets no finding at all.
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
| `realisation` | `- Realisation: <value>` (see [Realisation](#realisation)) |
| claims whose `models` names this requirement (by entity id or `ref`) | `- Modelled by: <claim id>, <claim id>` |

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
`statement`, a requirement that no section marker places, a duplicate entity id, a line that is more
than one marker, a reference that does not resolve, a `casesPath`) are reported on every scan as system events and by `concordance lint` as rule
`R0`: never silently dropped.

**Results for Exegra.** `concordance scan --exegra-results FILE` and `concordance report
--exegra-results FILE` write each requirement's `pass`/`fail` result and the lint findings by entity
id, in a file Exegra reads. See [cli.md](cli.md#exegra-results).

## Realisation

A requirement says how it is realised, with `Realisation: construction | checked | operational` (in an
Exegra source, the claim's `realisation`). Absent or unrecognised means `checked`.

| Realisation | Established by | Status |
|---|---|---|
| `checked` | Hand-written code and tests that declare `Verifies:` (everything above). | As before. |
| `construction` | Generated code guarantees it. Every Exegra claim whose `models` names the requirement must be `verified` in the Exegra claim results. | `verified` when all of them are; otherwise `unverified`, and the finding names each claim that is not (`c-2` is violated, `c-1` has no result). A construction requirement no claim models is `unverified` and says so. Tests are not consulted; a recorded violation still gives `violated`. |
| `operational` | A measurement on a running deployment (budgets, deployment and operations requirements). Nothing in this repository is expected to establish it. | `operational`: never `unverified` or `planned` for lack of code or tests, whatever its milestone, and never counted by `--fail-on unverified`. A failing test or a structural contradiction is still `violated`. |

The Exegra claim results are the JSON that `exegra evaluate --json` prints
(`{"result": {"results": [{"claimId": "...", "status": "verified"}]}}`; a bare `{"results": [...]}`, a
top-level array and a `{"claims": {"<id>": {"status": ...}}}` map are read too). Name the files with
`exegra_verdicts = ["exegra-claims.json"]` in `concordance.toml` or `--exegra-verdicts FILE` (repeatable).
Only the status `verified` counts; `supported`, `inconclusive`, `not-evaluated` and a missing claim do not.
The files are re-read on every scan and watched like the test results. This is a different file from
`--exegra-results`, which Concordance writes for Exegra.

The gate treats the groups consistently: `--fail-on proven` and `violated` look only at violations
(`operational` is not one); `--fail-on unverified` counts every started requirement that is not `verified`,
`construction` ones included, and never an `operational` one. The scan JSON carries `counts.operational`, and
each finding of a `construction`, `operational` or `hole` requirement carries `findings[].realisation`; the
text report writes it in parentheses before the title.

## Holes

A contract with `realisation: "hand"` and a `ref` such as `H-012` (the ID grammar is the one above: prefix
letters, one to four digits, optional dotted sub-number) is a **hole**: hand-written code that fills a place
the generated code leaves open. In an Exegra source a hole is placed with a `{{entity:<contract id>}}` marker
like a requirement (a hole no section places is an error, as for a requirement) and rendered as

```markdown
**H-012** The item implementing <name> SHALL satisfy its postconditions: <postconditions>
- Realisation: hole
- Component: <ref of implementedBy>
- Milestone: <n>          (only when the contract has a `milestone`)
```

so it is a requirement for verification, and the Markdown form works without Exegra. Spec lint reports only R8
(identifiers) on a hole; the postcondition text is not held to R1 to R5.

- **One implementation.** Exactly one indexed Rust item must have the line `/// Implements: H-012` in its doc
  comment (several refs may follow one `Implements:`, comma separated). No item gives the `unverified` finding
  `hole_unimplemented` (subject `proof_rule`); two or more give the `ambiguous` finding `hole_duplicate`, listing
  them. Items of other languages are not counted.
- **Postconditions.** They are verified by tests carrying `/// Verifies: H-012`, exactly like a requirement
  (recording, freshness and proof rules included). A hole whose tests pass but which has no single
  implementing item is `unverified`, not `verified`.
- **When code is expected.** A contract carries no milestone in the Exegra format, so the milestone rule is used
  when the contract has one (`Milestone:`, gated by `started_milestones`), and otherwise the component named by
  `implementedBy` decides: with `[gate] started_components = ["C-1"]` only the holes of the listed components
  (their `ref`) are expected to have code. Holes of other components are `planned`, produce no
  `hole_unimplemented`, and are left out of the gate. Without `started_components`, every hole is expected.
  With `started_components` set, a hole with no `Component:` is not started.

## Target

`Realisation: target` marks a requirement or hole that describes the enterprise target, not code that ships. No
item, test or measurement is expected in this repository, so its finding has the status `target`: its own group
(`counts.target`, `findings[].realisation = "target"`), never `unverified`, `planned` or `hole_unimplemented`, and
left out of `--fail-on unverified`. A requirement with a failing test is still `violated`, and one with a
passing recorded test is still `verified`.

In an Exegra source a contract that is a hole renders as `Realisation: target` instead of `hole` when its
`deployment` is `target`, declared or, when it declares none, defaulted from its `implementedBy` component having
`implementation.status: "not-implemented"` (`deployment: "shipped"` overrides the default). The scan report also
lists the machines (implementation-level behaviors whose `realisation` is `target`, declared or defaulted from
the `subject` component the same way) and the contracts the specification marks as target in `targets`
(`targets.machines`, `targets.contracts`), and the text report prints their counts. They are never counted as
unimplemented or as holes.

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

An `operational` requirement is never `planned`; a hole follows the milestone rule when it has a `Milestone:`
and `[gate] started_components` otherwise (see [Holes](#holes)).

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

Beyond the unknown ID, `scan` reports more breaches of the proof rules as findings
(see [spec-lint.md](spec-lint.md#proof-checks-in-scan)): a requirement declared by more than one
proof, a proof that bypasses the boundary or calls no indexed code, and Lean proofs that state `True` or rest on
an axiom. A passing result of a reported proof does not verify its requirement (see
[Test results](#test-results)).

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

Concordance does not run the analysed repository's tests. Point it at their output, then record it:

```
cargo test 2>&1 | tee target/test-results/cargo-test.txt
concordance record --config concordance.toml     # right after the tests, before any edit
concordance scan --config concordance.toml --fail-on unverified
```

or `test_results = ["target/test-results/cargo-test.txt"]` in the config file. Accepted, mixed freely:
libtest text, libtest JSON lines (`-Z unstable-options --format json`), cargo-nextest output, JUnit XML
(`vitest run --reporter=junit --outputFile=...`, most CI reporters) and vitest/jest text output
(`✓ file > describe > case`). For .NET, `dotnet test --logger trx` (Visual Studio TRX) and
`--logger junit` output are read; results are named `Namespace::Class::Method`, and the rows of a
`[Theory]` count as one test whose worst outcome wins.
For Lean, a `lake build` log (`lake build 2>&1 | tee build.log`): a tagged
theorem passes when the log shows its own module built (`Built Volume.Mixer`, the module named in full) and no
error or `declaration uses 'sorry'` warning lies inside the theorem's lines; it fails when one does, or when
its module failed to build; a module the log does not show built gives no result, however the build
ended. Lake prints only the modules it built, so build from clean (or with `-R`) when the log must cover
every module.
Result names are matched to test functions by their module path. The file is watched, so writing
a new run re-derives every requirement finding.

### Recording a run

A results file says what happened, not what code it happened to. `concordance record` is the user saying
"these results describe the sources and the specification as they are now": it writes
`<results file>.manifest.json` beside each configured results file, holding the hash of every indexed
source file and of every requirement's statement text, and the hash of the results file itself. `scan`
reads the manifest and treats the results as current only while all of the following hold:

- the manifest exists, is readable and describes this results file (its hash matches);
- no indexed source file differs from the manifest: changed, added or removed (a file whose
  modification time moved but whose bytes did not is unchanged);
- the statement text of the requirement has not changed since.

Modification times are not used anywhere. Freshness is per run for sources (any change to any indexed
source file makes every result of that run stale, since Concordance cannot know which code a test
exercised) and per requirement for statements (editing one requirement's text makes only its results
stale). Files outside the source roots (manifests such as `Cargo.toml`, build scripts, data files, the
toolchain) are not covered.

What is still trusted: that `record` ran immediately after the tests with no edit between them, and
that the results file is real output of those tests. Concordance cannot see either.

| Recorded outcome | Status of a one-statement requirement |
|---|---|
| passed, in a run recorded against the current sources and requirement text, and its proof is not reported (below) | `verified` |
| failed, in such a run | `violated` (`test_failure`; counts for `--fail-on proven`) |
| a results file with no manifest, a manifest that does not match the file, a source file changed since `record`, or the requirement's text changed since `record` | `stale`: the result is not current, run the tests and `concordance record` again; never a gate failure |
| passed, but its proof is reported by a proof rule (bypasses the boundary, calls no indexed code, states `True`, rests on a declared axiom) | `unverified`: the pass is not accepted |
| the test is in a Rust file no `mod` declaration reaches from a crate target | `unverified`: it is never compiled, so no run executed it |
| ignored, or no run reports the test (in its crate) | `unverified` |
| no test declares `Verifies:` for it | `unverified`: "has no test" |

A results file with no manifest is `stale`. There is no weaker "verified" status for it and no setting that
accepts unrecorded results: the default is strict.

### Matching a result with its test

A result line is matched with a test by name, and by crate where the format says which crate printed it:
libtest text (the `Running unittests src/lib.rs (target/debug/deps/x-hash)` and `Doc-tests x` lines above
the results) and nextest (the first segment of the binary id, the package). A line printed by the test
binary of another crate does not apply to a test of this one, and a scope that names no crate of the
repository applies to no test. JUnit, TRX, vitest/jest text and JSON-lines results carry no crate and
match by name only. A test in a Rust file that no `mod` declaration reaches from a crate target (a library,
binary, test, example or bench root) is not compiled, so it has no result whatever a same-named line says.
`cfg` predicates are not evaluated and `#[path]` is followed only for a plain string literal.

### Proof checks

A passing result does not verify a requirement when its proof is reported by one of these rules (the
findings are listed in [spec-lint.md](spec-lint.md#proof-checks-in-scan)):

- `bypasses_boundary`: every call the proof makes that resolves to indexed code goes to a non-public symbol.
- `calls_nothing` (Rust proofs only): no call of the proof, or of the test helpers it calls, resolves to indexed
  code. Calls are read in function bodies and in the arguments of `assert!`, `assert_eq!`, `assert_ne!` and their
  `debug_` forms. A proof that runs the package's own binary (it reads `env!("CARGO_BIN_EXE_<name>")`) reaches the
  command-line boundary and is not reported. A proof whose doc comment carries `Inspects: <path>`, naming a file or directory of indexed code that is not test scaffolding, declares that it reads that source text instead of calling it, and is not reported either; nothing checks that the test really reads the path. TypeScript and C# calls (constructors, method calls) are not resolved
  reliably enough to judge a proof by them.
- `trivial_statement` (Lean): the theorem states `True`.
- `uses_axiom` (Lean): the theorem mentions, directly or through the definitions and theorems it names, an
  `axiom` declared in the repository.

## What "verified" establishes

`verified` means: a test whose `///` doc comment (Rust) or the comment the language section above names declares
`Verifies: ID` passed in a recorded run; that run is bound by `concordance record` to the current sources and the
current text of the requirement; the result line is matched with the test's crate where the format says it; the
test's file is compiled; and the proof is not reported by a proof rule. It does not establish any of the
following, and nothing else in Concordance checks them:

- **The test's body is not judged.** A test that calls a public function and asserts nothing about it, one
  that asserts `true` after calling an unrelated public function, or one whose assertion cannot fail, verifies its
  requirement if the run reports it passed. Only the calls are checked: a Rust proof that calls no indexed code, or
  only non-public code, is reported and not accepted; a proof in TypeScript or C# is not judged by its calls.
- **The declaration is taken as written.** `Verifies:` is the author's claim. A test may declare
  several requirements, and a declaration of an ID that the specification gives twice is not reported.
- **Name matching below the crate is by name.** Within a crate, libtest and nextest results are matched by module path
  or title, and JUnit, TRX, vitest/jest and JSON-lines results by name alone. A line printed inside another
  test's captured output (`test x ... ok`) is ignored for libtest text.
- **Recording is trusted.** The manifest proves that the user recorded content; it cannot show that the results
  were produced from it. If the sources were edited between the tests and `record`, or a results file was
  copied from another checkout, the run is accepted as describing the content that was recorded.
  A JUnit test cut off before its end is recorded as no result (not as passed); other truncated outputs
  are read only for the lines they contain.
- **Attributes are not evaluated.** `#[ignore]` is respected through its result line. `cfg`
  predicates are not evaluated, so a test disabled by `cfg` is still a declaring test.
- **A statement is compared as text.** A reworded requirement is stale, but a requirement that is edited and
  re-recorded is accepted for its new wording without anyone checking that the test still covers it.
- **Lean theorems are compile checks.** A tagged theorem is `verified` when the log shows its module built and no
  `sorry` or error lies in its lines, it does not state `True`, and it names no axiom declared in the repository.
  Axioms of Lean itself or of a dependency are not seen, the statement is not compared with the requirement, and a
  theorem about the wrong thing verifies.

A `verified` status therefore says that a declared test passed on the code and requirement text recorded with it.
Whether the test checks the requirement is a judgement the status does not make.

What the gate modes guarantee (`scan --fail-on`, see [cli.md](cli.md#exit-codes)): `proven` guarantees only that
no structural contradiction and no failing recorded test of a started milestone is open, and nothing about
requirements that have no proof; `violated` adds the semantic contradictions that survived a challenge; and
`unverified` additionally guarantees that every requirement of a started milestone has the status `verified`,
with the limits listed above.
