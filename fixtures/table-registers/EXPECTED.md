# Expected: table-registers

A register-style specification (tables of tickets, roles and schedules) over a small
TypeScript package. Each row is one claim.

| Spec statement | Expected finding |
|---|---|
| T-1 row: "The client never throws." | not violated: "the client" names no resolvable scope, so no failure-site constraint is checked against the whole source |
| T-1 row: Summary "Some widgets are blank" | not ambiguous: the Summary column quotes the ticket title |
| T-2 row: "must load ... quickly" | ambiguous: vague term in the author's own column |
| Roles row: "May do" column label | not ambiguous: a column label is not a weak qualifier |
| `rate(1 hour)` | no reference: a schedule expression, not a call |
| `store.find_widget_key(text)` | not violated: a call on a value (here a SQL routine) cannot prove absence |
| `RemoteWidgetTable` | unverified, `suspected_structural_contradiction`: unresolved name, moderate evidence |
| `loadWidget()` MUST NOT throw | violated: `throw new Error(...)` in `src/client.ts` |
| `renderAll()` MUST draw every widget | violated: `renderAll` does not exist |
| `src/__tests__/support.ts`, `src/client.test.ts` exports | no undocumented finding: test code is not public API |
| `loadWidget` (throws) | undocumented panics: production export |

Exercised by `tests/tests/table_registers.rs`.
