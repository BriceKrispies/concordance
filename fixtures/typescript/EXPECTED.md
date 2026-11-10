# Expected: typescript

A TypeScript package (vitest tests) checked through the TypeScript plugin.

| Spec statement | Expected finding |
|---|---|
| `resetAll()` MUST restore every channel | violated: `resetAll` does not exist in `src/` (exact md + no .ts target) |
| documented `function parseLevel(text: string): number` | violated: actual signature has `radix = 10` (`src/mixer.ts`) |
| `parseLevel()` MUST NOT throw | violated: `throw new TypeError(...)` in `src/mixer.ts` |
| **VOL-001** `Mixer.setVolume()` SHALL reject a level above 100 | unverified until a run is ingested; verified when the vitest JUnit report passes `Mixer > rejects levels above 100`; violated when it fails |
| `Mixer.volume` | resolves to the getter (no dangling reference) |

Exercised by `tests/tests/typescript.rs`.
