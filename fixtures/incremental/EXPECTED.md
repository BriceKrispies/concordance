# Expected Concordance findings: `incremental`

Two independent features in separate modules. Every claim is correct. The point
of this fixture is the evidence scope: a finding about one module should depend
only on that module's code, so an edit to one module must not invalidate the
other module's evidence.

## Expected findings

- `math::add()` must return the sum of its two arguments.
  Status: **verified**. `a + b`.
- Signature block `pub fn add(a: i32, b: i32) -> i32` in `docs/math.md`.
  Status: **verified**. Matches `src/math.rs`.
- `incremental::math::clamp_percent()` must never return a value greater than 100.
  Status: **verified**. `v.min(100)`.
- `text::shout()` must return the input with every letter converted to uppercase.
  Status: **verified**. `s.to_uppercase()`.
- `text::word_count()` must count the whitespace-separated words in its input.
  Status: **verified**. `s.split_whitespace().count()`.
- Signature block `pub fn word_count(s: &str) -> usize` in `docs/text.md`.
  Status: **verified**. Matches `src/text.rs`.
- Example assertions in both docs use only public items and match the behaviour above.
  Status: **verified** (consistent with the code; not executed).

## Evidence scope (required behaviour)

- Editing only `src/text.rs` must not invalidate evidence for the math claims
  (`math::add()`, `incremental::math::clamp_percent()`, and the `add` signature block).
  Concordance should keep the math verdicts cached or unchanged and re-check only the text claims
  (`text::shout()`, `text::word_count()`, and the `word_count` signature block).
- Editing only `src/math.rs` must not invalidate evidence for the text claims, by the same argument.
- Editing `src/lib.rs` (the module declarations) may affect both sides and is allowed to invalidate both.

## Must NOT be reported

- No `stale`, `violated`, `ambiguous` or `undocumented` findings.
- The "Overview" paragraphs must not produce findings.
- The `use incremental::math;` and `use incremental::text;` lines in the examples must not be reported as broken references.
- No finding should be reported for `clamp_percent()` or `shout()` under `docs/` since neither is named in a signature block; they are covered by claims.
