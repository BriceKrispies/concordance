# Incremental engine

The engine (`concordance_engine`) owns all mutation. It processes scans, change
batches, analysis requests and analysis results serially.

## Change detection

- Files whose content hash is unchanged MUST NOT be re-parsed.
- A change batch must reparse only the files that changed.
- The watcher must debounce bursts of filesystem events before sending a batch to the engine.

## Invalidation

- Evidence must be invalidated when any node it depends on changes its content hash.
- Evidence whose dependencies did not change must remain valid and must not be recomputed.
- When a referenced symbol is removed, the relationship from the claim MUST be reported with a `relationship.invalidated` event.

## Semantic analyses

- An analysis result MUST NOT be applied unless every dependency still has the hash it was computed from.
- A superseded result must be reported with `AnalysisOutcome::Superseded` and never published as current.
- The engine must run at most `max_concurrent` semantic analyses at a time.
- Semantic analysis must never send the whole repository to the model; `build_request()` includes at most four symbol excerpts initially.

## Restart

- After a restart, persisted claims, findings and evidence must be restored from the state database.
- Unchanged files must not be re-parsed after a restart, and valid semantic evidence must not be re-analyzed.

## Notes

The per-scan analysis budget (`max_analyses_per_scan`) bounds model cost; claims
beyond the budget stay `unverified` until the next batch or an explicit request.
