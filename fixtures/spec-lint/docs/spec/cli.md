# Build

The build command reads a manifest and writes one artifact per target.

A table of words is exempt from the modal-word rule, and so is a code block:

| Word | Use |
|---|---|
| SHALL | Marks the one obligation of a requirement; it never appears twice. |

```text
The tool MUST always print its name.
```

**CLI-001** `tool build` SHALL exit with status 2 and write nothing when the manifest names a missing file.
- Verify: scenario
- Boundary: B-1 `tool build`
- Traces: G-2
- Rationale: A partial build that looks successful is worse than none.

**CLI-002** `tool build` SHALL print the name of each built target to standard output.
- Verify: example
- Boundary: B-1 `tool build`
- Traces: G-1

**CLI-003** `tool build` SHALL finish a build of the sample project within 2 seconds.
- Verify: budget
- Boundary: B-1 `tool build`
- Traces: G-1
- Depends: CLI-002

**CLI-004** `tool_lib::build_project` SHALL return the same targets for every manifest that lists the same names.
- Verify: property
- Boundary: B-2 `tool_lib::build_project`
- Traces: G-1

**CLI-005** `tool build` SHALL print a progress bar.
- Verify: example
- Boundary: B-1 `tool build`
- Traces: G-1
- Status: retired
- Rationale: Replaced by CLI-002.
