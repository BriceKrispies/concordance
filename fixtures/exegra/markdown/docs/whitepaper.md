# Tool

## Goals

- **G-1** A user can build a project from its manifest.
- **G-2** A failed build leaves no partial output.

## Non-goals

- **NG-1** Remote builds.

## Boundaries

| ID | Boundary | Interface | Used by |
|---|---|---|---|
| B-1 | `tool build` | CLI | users |
| B-2 | `tool_lib::build_project` | API | embedders |
