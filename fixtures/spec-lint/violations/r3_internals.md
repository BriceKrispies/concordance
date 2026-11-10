# Internals

**INTR-001** `tool build` SHALL call `hash_manifest` before it writes output.
- Verify: scenario
- Boundary: B-1 `tool build`
- Traces: G-1

**INTR-002** `tool build` SHALL keep the manifest in a cache between runs.
- Verify: scenario
- Boundary: B-1 `tool build`
- Traces: G-1

**INTR-003** `tool build` SHALL return the same targets as `tool_lib::build_project`.
- Verify: example
- Boundary: B-1 `tool build`
- Traces: G-1
