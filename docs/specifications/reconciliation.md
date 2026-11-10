# Reconciliation rules

This document defines how evidence becomes findings. It is normative for
`concordance-reconcile` and for anything that produces evidence.

## Evidence

Every evidence record carries its method, stance, strength, exact citations and
the content hashes it depends on.

- Evidence MUST list every node it was computed from in its dependencies.
- Structural evidence of strength `Proven` must only be produced from facts that are certain in the indexed state (an absent symbol, differing signatures, a missing file or heading).
- Existence of a referenced symbol must never be recorded as supporting evidence; it is recorded as inconclusive linkage.
- Semantic evidence MUST only be created from results whose citations passed validation in `concordance_semantic::analyze()`.

## Finding status

Status is decided in one place, `derive::claim_finding()`.

- A claim must not be reported as `violated` unless valid contradicting evidence of strength `Strong` or higher exists, or a semantic contradiction survived an adversarial challenge.
- A behavioral claim must never be reported as `verified` by structural analysis alone.
- When semantic evidence has been invalidated and not re-established, the claim's finding MUST be `stale`.
- Findings must keep their identity across scans: `FindingSubject::finding_id()` derives the id only from what the finding is about.
- Resolved findings MUST be retained with `Lifecycle::Resolved` rather than deleted.

## Severity

Severity follows authority: violations of normative documents are `high`, violations
of descriptive or unknown-authority documents are `medium`.

## Authority

Normative documents cannot be rewritten automatically because the implementation
differs. Proposals that change normative intent must be marked as requiring an
explicit decision (`Proposal::requires_decision`).

- Concordance must not modify files in the analyzed repository.

## Stable identities

```rust
pub fn short_hash(parts: &[&str]) -> String
pub fn carry_claim_ids(old: &[Claim], new: &mut [Claim])
```

`carry_claim_ids()` must keep a claim's identity across an edit only when the match is unambiguous.

## Example

A specification stating "the volume must never exceed 100" against code that calls
`v.min(150)` produces a suspected semantic contradiction, not a proven one.
