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

An ID is an upper-case prefix, optional upper-case/digit segments and a number with an optional
dotted sub-number: `PSS-003`, `LANG-006.3`, `WEB-SEC-004`. IDs are recognised only where they are
defined (bold opener or heading) and where a test declares them, so similar-looking keys in prose
(`PROJ-2147`) are not requirements.

## In the tests

```rust
/// Verifies: LANG-006.3
#[test]
fn nested_comment_ends_at_the_balancing_close() { ... }
```

A `Verifies:` line in a test function's doc comment links it to one or more requirements
(`Verifies: LANG-006.3, LANG-006.4`). A `Verifies:` naming an ID that no specification defines is
reported as a violated finding on the test.

TypeScript/JavaScript tests (`it(...)` / `test(...)`, inside any `describe(...)`) take the
`Verifies:` line from the comment directly above the call:

```ts
describe("Mixer", () => {
  // Verifies: VOL-001
  it("rejects levels above 100", () => { ... });
});
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
(`✓ file > describe > case`).
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
