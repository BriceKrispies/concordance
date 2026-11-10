# Test results (B-4)

**RES-001** A scan with recorded test results SHALL report each requirement verified, violated or unverified as the result conformance cases list.
- Verify: conformance
- Boundary: B-4 `--test-results`
- Cases: fixtures/requirements/EXPECTED.md, fixtures/typescript/EXPECTED.md, fixtures/csharp/EXPECTED.md, fixtures/lean/EXPECTED.md
- Traces: G-2
- Rationale: one table per fixture covers libtest and nextest, vitest JUnit, TRX and Lake build logs.
