# Provable

**PROV-001** `tool build` SHALL list the targets in manifest order.
- Boundary: B-1 `tool build`
- Traces: G-1

**PROV-002** `tool build` SHALL list the sources of each target.
- Verify: guess
- Boundary: B-1 `tool build`
- Traces: G-1

**PROV-003** `tool build` SHALL never overwrite an existing artifact.
- Verify: example
- Boundary: B-1 `tool build`
- Traces: G-2

**PROV-004** `tool build` SHALL finish within 5 seconds.
- Verify: example
- Boundary: B-1 `tool build`
- Traces: G-1

**PROV-005** `tool build` SHALL print at most 3 warnings.
- Verify: scenario
- Boundary: B-1 `tool build`
- Traces: G-1
