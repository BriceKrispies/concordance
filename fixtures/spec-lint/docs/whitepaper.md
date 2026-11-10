Status: approved 2026-11-10 by the author

# Tool

## Problem

Builds that fail halfway leave users with output they cannot trust.

## Goals

- **G-1** A user can build a project from its manifest.
- **G-2** A failed build leaves no partial output.

## Non-goals

- **NG-1** Remote builds.

## Boundaries

| ID | Boundary | Kind | Who uses it |
|---|---|---|---|
| B-1 | `tool build` | CLI | users |
| B-2 | `tool_lib::build_project` | API | embedders |

## Architecture

| ID | Component | Owns | Talks to |
|---|---|---|---|
| C-1 | Builder | manifests and targets | the file system |

## Decisions

- **D-1** The builder is a library with a thin command on top, so embedders get the same behaviour.

## Open questions

- **Q-1** Should builds run in parallel? Recommendation: not in the first release.
