# Conformance

**CASE-001** `tool build` SHALL print the target count.
- Verify: conformance
- Boundary: B-1 `tool build`
- Traces: G-1

**CASE-002** `tool build` SHALL print the skipped targets.
- Verify: scenario
- Boundary: B-1 `tool build`
- Traces: G-1
- Cases: cases/good.md

**CASE-003** `tool build` SHALL print the failed targets.
- Verify: conformance
- Boundary: B-1 `tool build`
- Traces: G-1
- Cases: cases/missing.md

**CASE-004** `tool build` SHALL print the sources read.
- Verify: conformance
- Boundary: B-1 `tool build`
- Traces: G-1
- Cases: cases/broken.md

**CASE-005** `tool build` SHALL print the manifest path.
- Verify: conformance
- Boundary: B-1 `tool build`
- Traces: G-1
- Cases: cases/good.md
