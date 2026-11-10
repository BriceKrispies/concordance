# Expected: requirements

Requirement IDs tie specification statements to the tests that verify them.

| Requirement / item | Test-run state | Expected finding |
|---|---|---|
| LEX-001 (`nested_comment_ends_at_the_balancing_close`) | no run ingested | unverified: no ingested run reports the test |
| LEX-001 | run recorded `ok` after the last source change | verified (`Supported`, test method) |
| LEX-001 | run recorded `FAILED` | violated (`TestFailure`) |
| LEX-001 | a source file changed after the run | unverified: the result is outdated |
| LEX-002 | any | unverified: no test declares `Verifies: LEX-002` |
| `verifies_a_requirement_that_does_not_exist` | any | violated: `Verifies: LEX-404` names an undefined requirement |

Exercised by `tests/tests/requirements.rs`.
