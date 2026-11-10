# Expected Concordance findings: `ambiguous`

The spec is deliberately vague and self-contradictory. Concordance must not
guess at intent. It should report the vague and conflicting claims as
`ambiguous` and must not force them into `verified` or `violated`.

## Expected findings

- `Cache::get()` must return `None` for missing keys.
  Status: **verified**. `self.entries.get(key).cloned()` returns `None` when the key is absent.
  This is the one clear claim.
- `Cache::insert()` must be fast.
  Status: **ambiguous**. "Fast" has no measurable bound. Report the ambiguity;
  do not mark it violated or verified.
- The cache must hold at most 100 entries.
  Status: **ambiguous**. Conflicts with the later "at most 500 entries" claim.
  Report both claims together as a contradiction in the spec. Neither should be
  marked violated: the code has no capacity limit, so it cannot confirm either number.
- The cache must hold at most 500 entries.
  Status: **ambiguous**. Conflicts with the earlier "at most 100 entries" claim.
  The code has no bound, so neither claim can be verified.
- The cache should evict entries in a timely manner when appropriate.
  Status: **ambiguous**. "Timely" and "when appropriate" have no defined meaning.
- Entries must expire after some time, if possible.
  Status: **ambiguous**. The hedge "if possible" and the unspecified "some time" make
  this non-testable. The code has no expiry; do not report it as violated.
- `Cache::evict()`, named in the eviction section.
  Status: **unverified** for the timing claim; the function exists and clears all entries.
  It is not a finding on its own.

## Must NOT be reported

- No `violated` finding for the capacity claims (the code has no capacity, but the spec is contradictory, so the tool must say ambiguous).
- No `violated` finding for "must expire" or for "must be fast".
- No `undocumented` finding for `Cache::get()`, `Cache::insert()` or `Cache::evict()`; all are named in the spec.
- The `## Example` block must not be reported as a requirement.
- The "Overview" paragraph must not be reported.
- The `ambiguous` crate name in `use ambiguous::Cache;` must not be reported as a broken reference.
