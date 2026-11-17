# Expected: exegra

An Exegra JSONC specification (`docs/spec.jsonc`) whose sections render to Markdown, with a
TypeScript package whose tests declare `Verifies:` for ids the JSONC defines.

| Spec entity | Expected |
|---|---|
| rule rows `TEN-1`, `TEN-2` | define requirements; claims located at the row's JSONC line |
| authored claims `FR-ISO-1` (invariant), `FR-AGT-1` (structural) | claims at their JSONC entity, defining their ids |
| seam `S-01` | informative text, no requirement |
| `src/tenant.test.ts` `Verifies: TEN-1`, `Verifies: FR-ISO-1` | linked; no unknown-requirement finding |
| `docs/data.json` | ignored: not an Exegra specification |

Exercised by `crates/concordance-exegra/src/tests.rs` and `tests/tests/exegra.rs`.
