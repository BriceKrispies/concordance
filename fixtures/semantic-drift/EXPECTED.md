# Expected Concordance findings: `semantic-drift`

The signatures and names all match, so a purely structural check passes.
The problem is semantic: `set_volume` clamps to 150, not to `MAX_VOLUME` (100),
and `MAX_VOLUME` is declared but never used.

## Expected findings

- The volume must never exceed 100.
  Status: **violated**. `set_volume(150)` stores `150` (`v.min(150)`), and `volume()`
  returns 150. Evidence: `src/lib.rs`, `set_volume`, `.min(150)`.
- `Mixer::set_volume()` must clamp the requested volume to `MAX_VOLUME`.
  Status: **violated**. The clamp bound is the literal `150`, not the constant
  `MAX_VOLUME` (100). `MAX_VOLUME` is referenced in the spec and declared in the
  code but is not used by `set_volume`. The report should name the unused constant.
- `Mixer::volume()` returns the current volume.
  Status: **verified**. Returns `self.volume`.
- The `MAX_VOLUME` constant itself.
  Status: **verified** as existing; it is not used by any function (see the violation above).
  Do not report it as undocumented or as missing.

## Must NOT be reported

- No `stale` finding for `Mixer::set_volume()`, `Mixer::volume()` or `MAX_VOLUME`: all exist with the spec's names.
- No `undocumented` finding for `MAX_VOLUME`; the spec mentions it.
- The "Overview" paragraph must not produce findings.
- The example's `assert_eq!(mixer.volume(), 42);` must not be reported as contradicting the code, since 42 is within both limits.
- Do not report a `violated` finding for the "volume returns the current volume" claim.
