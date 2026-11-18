# Expected: requirements

Requirement IDs tie specification statements to the tests that verify them.

| Requirement / item | Test-run state | Expected finding |
|---|---|---|
| LEX-001 (`nested_comment_ends_at_the_balancing_close`) | no run ingested | unverified: no ingested run reports the test |
| LEX-001 | run `ok`, recorded against the current sources and requirement text | verified (`Supported`, test method) |
| LEX-001 | run recorded `FAILED` | violated (`TestFailure`) |
| LEX-001 | a source file changed after the run was recorded | stale: the result is outdated |
| LEX-002 | any | unverified: no test declares `Verifies: LEX-002` |
| `verifies_a_requirement_that_does_not_exist` | any | violated: `Verifies: LEX-404` names an undefined requirement |

The `LEX-001` result going stale after a source change or a change of the requirement text, a results file with no manifest, and the other ways a recorded pass stops verifying are not rows (they need a second scan after an edit); `tests/tests/requirements.rs` and `tests/tests/verified.rs` keep them.

## Cases

Checked by the RES-001 conformance proof (see `docs/requirements.md`, "Case tables"). Adding a row adds a case.
A row passes when some open finding whose title or claim text contains `Claim contains` has the given status,
category and location. `absent` and `absent:status` mean no such finding exists; `!x` negates a cell.
Rows that name a results file install it as the test-results file and record it (`concordance record`) before the scan.

| Case | Requirement | Results | Claim contains | Status | Category | Location |
|---|---|---|---|---|---|---|
| no-run-linked | RES-001 | none | balances | unverified |  |  |
| no-test-declares | RES-001 | none | MUST reject a block comment | unverified |  |  |
| undefined-requirement | RES-001 | none | unknown requirement `LEX-404` | violated |  |  |
| run-passed | RES-001 | results/passed.txt | balances | verified | supported |  |
| unrelated-pass | RES-001 | results/passed.txt | MUST reject a block comment | unverified |  |  |
| run-failed | RES-001 | results/failed.txt | balances | violated | test_failure |  |
| line-of-another-crate | RES-001 | results/other-crate.txt | balances | unverified |  |  |
