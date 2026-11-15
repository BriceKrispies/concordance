# Library

## Requirements

**LIB-001** `tool_lib::build_project` SHALL return the targets a manifest lists, in order.
- Verify: conformance
- Cases: cases/flows.md
- Boundary: B-2 `tool_lib::build_project`
- Traces: G-1
- Milestone: 1

**LIB-002** `tool_lib::build_project` SHALL return no target for an empty manifest.
- Verify: example
- Boundary: B-2 `tool_lib::build_project`
- Traces: G-1
- Milestone: 1
- Seam: 1
