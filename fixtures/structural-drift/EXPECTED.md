# Expected Concordance findings: `structural-drift`

The code has been renamed and changed since the spec was written. The spec
has structural drift (dead names, outdated signatures, broken links, missing
files) but its behavioural claims about the code that does exist are correct.

## Expected findings

- `parse_headers()` must return `None` for any line that has no colon.
  Status: **stale**. The function was renamed to `parse_header()`. Report the
  old name as missing and suggest `parse_header()` as the closest match.
  Do not report this as a violation of the behaviour.
- `Header::from_bytes()` must never panic on malformed input.
  Status: **stale**. No `from_bytes` item exists on `Header` (it was removed).
- Signature block `pub fn encode(h: &Header, buf: &mut String)`.
  Status: **stale**. The real signature is `pub fn encode(h: &Header) -> String`.
  The block is outdated, not the code.
- `[wire format](wire-format.md#framing)`.
  Status: **stale**. `docs/wire-format.md` exists but has no "Framing" heading;
  only "Header lines" and "Versioning" exist. The file link resolves; the anchor does not.
- `[old notes](notes.md)`.
  Status: **stale**. `docs/notes.md` does not exist.
- `src/codec.rs`, named as where the byte-level encoding is implemented.
  Status: **stale**. The file does not exist.
- `parse_header()` must return `None` when the input contains no `:`.
  Status: **verified**. `input.split_once(':')?` returns `None`.
- `encode()` must produce a single `name: value` line with no trailing newline.
  Status: **verified**. `format!("{}: {}", ...)` with no newline.
- `parse_header()` returns the first header found in the line.
  Status: **verified** in the sense that `split_once` splits at the first colon.

## Must NOT be reported

- No `violated` finding for `parse_headers()` or `Header::from_bytes()`. The
  behaviour is not being contradicted; the names are dead.
- The "Overview" paragraphs (background, no modal words) must not produce findings.
- The `use structural_drift::{encode, parse_header};` line in the example must
  not be reported as a broken reference, since both items exist.
- `docs/wire-format.md` must not be reported as missing. Only its `#framing` anchor is broken.
- No `undocumented` findings for `Header`, `parse_header()` or `encode()`; all are covered by the spec.
