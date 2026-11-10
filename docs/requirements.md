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
`Traces:`, `Rationale:`, `Depends:`, `Status:`, `Cases:`). They are stored on the requirement, not extracted as
claims, and `concordance lint` checks them: see [spec-lint.md](spec-lint.md).

An ID is an upper-case prefix, optional upper-case/digit segments and a number with an optional
dotted sub-number: `PSS-003`, `LANG-006.3`, `WEB-SEC-004`. IDs are recognised only where they are
defined (bold opener or heading) and where a test declares them, so similar-looking keys in prose
(`PROJ-2147`) are not requirements.

## Case tables

A requirement with `Verify: conformance` states a capability once and lists the detail in case tables:
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
reported as a violated finding on the test.

Beyond the unknown ID, `scan` reports two more breaches of the proof rules as findings
(see [spec-lint.md](spec-lint.md#proof-checks-in-scan)): a requirement declared by more than one
proof, and a proof whose resolved calls all go to non-public symbols.

TypeScript/JavaScript tests (`it(...)` / `test(...)`, inside any `describe(...)`) take the
`Verifies:` line from the comment directly above the call:

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
