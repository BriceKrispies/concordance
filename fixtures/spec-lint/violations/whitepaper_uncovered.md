Status: approved 2026-11-10 by the author

# Tool

## Goals

- **G-1** A user can build a project from its manifest.
- **G-2** A failed build leaves no partial output.
- **G-3** A user can list the targets of a project.

## Boundaries

| ID | Boundary | Kind | Who uses it |
|---|---|---|---|
| B-1 | `tool build` | CLI | users |
| B-2 | `tool_lib::build_project` | API | embedders |
