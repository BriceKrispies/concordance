# Build

The build command reads a manifest.

## Requirements

**CLI-001** `tool build` SHALL exit with status 2 and write nothing when the manifest names a missing file.
- Verify: scenario
- Boundary: B-1 `tool build`
- Traces: G-2
- Rationale: A partial build that looks successful is worse than none.

**CLI-002** `tool build` SHALL print the name of each built target and SHALL exit with status 0.
- Verify: example
- Boundary: B-1 `tool build`
- Traces: G-1

**CLI-003** `tool build` SHALL report a failed manifest correctly.
- Verify: example
- Boundary: B-1 `tool build`
- Traces: G-2

**CLI-004** `tool build` SHALL print a progress bar.
- Status: retired
- Verify: example
- Boundary: B-1 `tool build`
- Traces: G-1
- Rationale: Replaced by CLI-002.
