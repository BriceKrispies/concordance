# Expected: spec-lint

A conforming specification (`docs/spec/cli.md`, `docs/spec/index.md`), a white paper and one violating
specification per rule under `violations/`. The conforming fixture gives no finding. It is the proof of
LINT-002.

## Cases

Checked by the LINT-001 conformance proof through the real `concordance` binary. For each case stem (the part
of `Case` before the dot) the driver copies the fixture and applies the violation:

- `violations/<stem>.md` is installed as `docs/spec/<stem>.md` and gets a row in the spec index. Its owning
  prefix is the upper-cased letters before the first dash of the first bold ID in the file.
- `cases/` holds the case files that `r4_cases` names: `good.md` (a valid table) and `broken.md` (no table).
- `whitepaper_uncovered` replaces `docs/whitepaper.md` instead.
- `missing_whitepaper` has no file: the driver deletes `docs/whitepaper.md`.

It then runs `concordance lint --strict --json` and expects exactly the rows of that stem, as `(File, Rule, Line)`,
every one an error, and exit status 1. It also runs `concordance lint --json` (not strict) and expects the same
findings with the `Severity` column, the matching error and warning counts, and exit status 1 only when a row is an error.

| Case | File | Rule | Line | Severity |
|---|---|---|---|---|
| r1_atomic.1 | docs/spec/r1_atomic.md | R1 | 3 | error |
| r1_atomic.2 | docs/spec/r1_atomic.md | R1 | 8 | error |
| r1_atomic.3 | docs/spec/r1_atomic.md | R1 | 13 | error |
| r1_atomic.4 | docs/spec/r1_atomic.md | R1 | 18 | error |
| r2_vague.1 | docs/spec/r2_vague.md | R2 | 3 | error |
| r2_vague.2 | docs/spec/r2_vague.md | R2 | 8 | error |
| r3_internals.1 | docs/spec/r3_internals.md | R3 | 3 | error |
| r3_internals.2 | docs/spec/r3_internals.md | R3 | 8 | warning |
| r3_internals.3 | docs/spec/r3_internals.md | R3 | 13 | warning |
| r4_provable.1 | docs/spec/r4_provable.md | R4 | 3 | error |
| r4_provable.2 | docs/spec/r4_provable.md | R4 | 8 | error |
| r4_provable.3 | docs/spec/r4_provable.md | R4 | 13 | error |
| r4_provable.4 | docs/spec/r4_provable.md | R4 | 18 | error |
| r4_provable.5 | docs/spec/r4_provable.md | R4 | 23 | error |
| r4_cases.1 | docs/spec/r4_cases.md | R4 | 3 | error |
| r4_cases.2 | docs/spec/r4_cases.md | R4 | 12 | error |
| r4_cases.3 | docs/spec/r4_cases.md | R4 | 18 | error |
| r4_cases.4 | docs/spec/r4_cases.md | R4 | 24 | error |
| r5_traceable.1 | docs/spec/r5_traceable.md | R5 | 3 | error |
| r5_traceable.2 | docs/spec/r5_traceable.md | R5 | 10 | error |
| r5_traceable.3 | docs/spec/r5_traceable.md | R5 | 12 | error |
| r5_traceable.4 | docs/spec/r5_traceable.md | R5 | 18 | error |
| r6_duplicate.1 | docs/spec/r6_duplicate.md | R6 | 3 | warning |
| r7_modal.1 | docs/spec/r7_modal.md | R7 | 3 | error |
| r7_modal.2 | docs/spec/r7_modal.md | R7 | 5 | error |
| r7_modal.3 | docs/spec/r7_modal.md | R7 | 8 | error |
| r8_ids.1 | docs/spec/r8_ids.md | R8 | 3 | error |
| r8_ids.2 | docs/spec/r8_ids.md | R8 | 8 | error |
| r8_ids.3 | docs/spec/r8_ids.md | R8 | 18 | error |
| r8_ids.4 | docs/spec/r8_ids.md | R8 | 23 | error |
| whitepaper_uncovered.1 | docs/whitepaper.md | R5 | 9 | error |
| missing_whitepaper.1 | docs/whitepaper.md | R5 | 1 | error |
