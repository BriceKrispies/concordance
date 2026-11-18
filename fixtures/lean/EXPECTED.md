# Expected: lean

A Lake package (`lakefile.lean`, `lean-toolchain`) checked through the Lean plugin. The `lake-real-*.log` files are
real `lake build` output (Lean v4.9.0 on Windows); the other logs are hand-written edge cases. Lake
build logs are ingested as test results (`logs/*.log`, copied to the configured results file).

| Spec statement | Expected finding |
|---|---|
| `resetAll()` MUST restore every channel | violated: `resetAll` does not exist in `Volume/` (exact md location, no `.lean` target) |
| documented `def clampLevel (level : Nat) (floor : Nat) : Nat` | violated: actual signature has one parameter (`Volume/Mixer.lean`, line 7) |
| **VOL-001** `Volume.clampLevel` SHALL never exceed `Volume.maxLevel` | unverified until a log is ingested; verified when `clamp_le_max` (doc comment `Verifies: VOL-001`) is in a module the log shows built, with no error or `sorry` inside it (a log that only says the build completed shows nothing); violated (test failure) when the module failed to build |
| **VOL-002** `clampLevel(n)` SHALL be idempotent | proven by `clamp_idempotent` (`-- Verifies: VOL-002` above it), which uses `sorry`: violated when the log carries `declaration uses 'sorry'` at line 20 |

## Cases

Checked by the LANG-001 and RES-001 conformance proofs (see `docs/requirements.md`, "Case tables"). Adding a row adds a case.
A row passes when some open finding whose title or claim text contains `Claim contains` has the given status,
category and location. `absent` and `absent:status` mean no such finding exists; `!x` negates a cell.
Rows that name a results file install it as the test-results file and record it (`concordance record`) before the scan.

| Case | Requirement | Results | Claim contains | Status | Category | Location |
|---|---|---|---|---|---|---|
| removed-definition | LANG-001 | none | resetAll() | violated | structural_contradiction | docs/spec.md |
| outdated-signature | LANG-001 | none | def clampLevel (level : Nat) (floor : Nat) | violated |  | Volume/Mixer.lean:7 |
| no-log-max | RES-001 | none | SHALL never return a level above | unverified |  |  |
| no-log-idempotent | RES-001 | none | SHALL be idempotent | unverified |  |  |
| sorry-leaves-max-verified | RES-001 | logs/lake-sorry.log | SHALL never return a level above | verified |  |  |
| sorry-violates-idempotent | RES-001 | logs/lake-sorry.log | SHALL be idempotent | violated | test_failure |  |
| failed-build-violates-max | RES-001 | logs/lake-failed.log | SHALL never return a level above | violated | test_failure |  |
| real-sorry-verifies-max | RES-001 | logs/lake-real-sorry.log | SHALL never return a level above | verified |  |  |
| real-sorry-violates-idempotent | RES-001 | logs/lake-real-sorry.log | SHALL be idempotent | violated | test_failure |  |
| real-failed-violates-max | RES-001 | logs/lake-real-failed.log | SHALL never return a level above | violated | test_failure |  |
| real-failed-still-violates-idempotent | RES-001 | logs/lake-real-failed.log | SHALL be idempotent | violated | test_failure |  |
| completed-build-without-the-module | RES-001 | logs/lake-completed-only.log | SHALL never return a level above | unverified |  |  |
