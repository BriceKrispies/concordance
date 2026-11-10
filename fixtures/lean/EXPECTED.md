# Expected: lean

A Lake package (`lakefile.lean`, `lean-toolchain`) checked through the Lean plugin. Lake
build logs are ingested as test results (`logs/*.log`, copied to the configured results file).

| Spec statement | Expected finding |
|---|---|
| `resetAll()` MUST restore every channel | violated: `resetAll` does not exist in `Volume/` (exact md location, no `.lean` target) |
| documented `def clampLevel (level : Nat) (floor : Nat) : Nat` | violated: actual signature has one parameter (`Volume/Mixer.lean`, line 7) |
| **VOL-001** `Volume.clampLevel` SHALL never exceed `Volume.maxLevel` | unverified until a log is ingested; verified when `clamp_le_max` (doc comment `Verifies: VOL-001`) is in a built module with no error or `sorry` inside it; violated (test failure) when the module failed to build |
| **VOL-002** `clampLevel(n)` SHALL be idempotent | proven by `clamp_idempotent` (`-- Verifies: VOL-002` above it), which uses `sorry`: violated when the log carries `declaration uses 'sorry'` at line 20 |

Exercised by `tests/tests/lean.rs`.
