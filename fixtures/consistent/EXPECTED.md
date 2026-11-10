# Expected Concordance findings: `consistent`

The spec matches the code. Every checkable claim should be verified and
nothing should be reported as a problem.

## Expected findings

- `RateLimiter::try_acquire()` must return `false` when no tokens are available.
  Status: **verified**. The `if self.tokens == 0` guard returns `false`.
- The number of available tokens must never exceed the configured capacity.
  Status: **verified**. `new` starts at capacity and `refill` clamps with `.min(self.capacity)`.
  `try_acquire` only decreases the count.
- `This crate must not use unsafe code.`
  Status: **verified**. No `unsafe` blocks or items exist in `src/lib.rs`.
- `RateLimiter::new()` creates a limiter whose bucket starts full.
  Status: **verified**. `tokens: capacity` in the constructor.
- `RateLimiter::refill()` adds tokens and leaves the bucket at capacity when refilled past it.
  Status: **verified**. `saturating_add(n).min(self.capacity)`.
- `RateLimiter::available()` returns the number of tokens and does not change the bucket.
  Status: **verified**. Takes `&self` and returns `self.tokens`.
- Signature block: `pub fn new(capacity: u32) -> RateLimiter`, `pub fn try_acquire(&mut self) -> bool`,
  `pub fn refill(&mut self, n: u32)`, `pub fn available(&self) -> u32`.
  Status: **verified**. Each line matches the code exactly.
- The `## Example` snippet uses only public items and matches the behaviour above.
  Status: **verified** (it is consistent with the code; it is not executed).

## Must NOT be reported

- No `stale`, `violated`, `ambiguous` or `undocumented` findings at all.
- The explanatory paragraphs in "Overview" (no modal words, no contract) must not produce findings.
- The `## Example` block must not be flagged as a requirement or as a code reference that fails to resolve.
- The `consistent` crate name in the `use consistent::RateLimiter;` line must not be treated as a broken reference.
