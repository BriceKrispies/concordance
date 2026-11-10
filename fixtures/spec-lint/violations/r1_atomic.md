# Atomic

**ATOM-001** `tool build` SHALL exit with status 2 and SHALL write nothing when the manifest is missing.
- Verify: scenario
- Boundary: B-1 `tool build`
- Traces: G-1

**ATOM-002** `tool build` SHALL print the targets; the summary follows.
- Verify: scenario
- Boundary: B-1 `tool build`
- Traces: G-1

**ATOM-003** `tool build` SHALL print the targets and never print the summary.
- Verify: property
- Boundary: B-1 `tool build`
- Traces: G-1

**ATOM-004** `tool build` prints the targets in order.
- Verify: example
- Boundary: B-1 `tool build`
- Traces: G-1
