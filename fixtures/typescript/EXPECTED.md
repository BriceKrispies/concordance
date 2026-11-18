# Expected: typescript

A TypeScript package (vitest tests) checked through the TypeScript plugin.

| Spec statement | Expected finding |
|---|---|
| `resetAll()` MUST restore every channel | violated: `resetAll` does not exist in `src/` (exact md + no .ts target) |
| documented `function parseLevel(text: string): number` | violated: actual signature has `radix = 10` (`src/mixer.ts`) |
| `parseLevel()` MUST NOT throw | violated: `throw new TypeError(...)` in `src/mixer.ts` |
| **VOL-001** `Mixer.setVolume()` SHALL reject a level above 100 | unverified until a run is ingested; verified when the vitest JUnit report passes `Mixer > rejects levels above 100`; violated when it fails |
| `Mixer.volume` | resolves to the getter (no dangling reference) |

## Cases

Checked by the LANG-001 and RES-001 conformance proofs (see `docs/requirements.md`, "Case tables"). Adding a row adds a case.
A row passes when some open finding whose title or claim text contains `Claim contains` has the given status,
category and location. `absent` and `absent:status` mean no such finding exists; `!x` negates a cell.
Rows that name a results file install it as the test-results file and record it (`concordance record`) before the scan.

| Case | Requirement | Results | Claim contains | Status | Category | Location |
|---|---|---|---|---|---|---|
| removed-function | LANG-001 | none | resetAll() | violated | structural_contradiction | docs/spec.md |
| outdated-signature | LANG-001 | none | function parseLevel(text: string) | violated |  | src/mixer.ts:21 |
| throwing-function | LANG-001 | none | MUST NOT throw | violated |  | src/mixer.ts:24 |
| dotted-path-resolves | LANG-001 | none | returns the current level | !violated |  |  |
| no-run | RES-001 | none | SHALL reject a level above 100 | unverified |  |  |
| run-passed | RES-001 | results/junit-pass.xml | SHALL reject a level above 100 | verified |  |  |
| run-failed | RES-001 | results/junit-fail.xml | SHALL reject a level above 100 | violated | test_failure |  |
