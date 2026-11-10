# Specification lint (B-2)

**LINT-001** `concordance lint --strict` SHALL report every finding listed in the lint conformance cases, with its rule code and line.
- Verify: conformance
- Boundary: B-2 `concordance lint`
- Cases: fixtures/spec-lint/EXPECTED.md
- Traces: G-3

**LINT-002** `concordance lint --strict` SHALL report no finding for the conforming specification fixture.
- Verify: example
- Boundary: B-2 `concordance lint`
- Traces: G-3
