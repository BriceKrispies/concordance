# Expected: csharp

A C# project (xUnit tests) checked through the C# plugin.

| Spec statement | Expected finding |
|---|---|
| `ResetAll()` MUST restore every channel | violated: `ResetAll` does not exist in `src/` (exact md + no .cs target) |
| documented `public static int ParseLevel(string text);` | violated: actual signature has `int radix = 10` (`src/Mixer.cs`, line 25) |
| `ParseLevel()` MUST NOT throw | violated: `throw new FormatException(...)` in `src/Mixer.cs`, line 29 |
| **VOL-001** `Mixer.SetVolume()` SHALL reject a level above 100 | unverified until a run is ingested; verified when the TRX report passes `Volume.MixerTests.Rejects_level_above_100`; violated when it fails |
| `Mixer.Volume` | resolves to the property (no dangling reference) |

## Cases

Checked by the LANG-001 and RES-001 conformance proofs (see `docs/requirements.md`, "Case tables"). Adding a row adds a case.
A row passes when some open finding whose title or claim text contains `Claim contains` has the given status,
category and location. `absent` and `absent:status` mean no such finding exists; `!x` negates a cell.
Rows that name a results file install it as the test-results file before the scan.

| Case | Requirement | Results | Claim contains | Status | Category | Location |
|---|---|---|---|---|---|---|
| removed-function | LANG-001 | none | ResetAll() | violated | structural_contradiction | docs/spec.md |
| outdated-signature | LANG-001 | none | int ParseLevel(string text) | violated |  | src/Mixer.cs:25 |
| throwing-function | LANG-001 | none | MUST NOT throw | violated |  | src/Mixer.cs:29 |
| dotted-path-resolves | LANG-001 | none | returns the current level | !violated |  |  |
| no-run | RES-001 | none | SHALL reject a level above 100 | unverified |  |  |
| run-passed | RES-001 | results/pass.trx | SHALL reject a level above 100 | verified |  |  |
| run-failed | RES-001 | results/fail.trx | SHALL reject a level above 100 | violated | test_failure |  |
