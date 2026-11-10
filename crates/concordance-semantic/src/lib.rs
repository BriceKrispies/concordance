//! Layer B (semantic assessment) and Layer C (adversarial challenge) via the local
//! Claude Code CLI.
//!
//! CONTRACT:
//! * The model receives one claim plus scoped symbol excerpts (never the repository).
//!   Repository text is wrapped as untrusted data; the system prompt says so.
//! * Responses are requested as structured output (`claude -p --output-format json
//!   --json-schema ...`) and parsed strictly; anything not matching the schema is an
//!   `InvalidResponse`.
//! * Every citation is validated against the excerpts that were actually sent: the
//!   cited excerpt id must exist, lines must lie inside that excerpt, and the quoted
//!   text must occur (whitespace-normalized) in those lines. Invalid citations are
//!   dropped and listed in `Assessment::rejected`.
//! * A `Disagrees` verdict with no surviving contradicting citation is downgraded to
//!   `InsufficientEvidence` (reason in `Assessment::downgraded`). Likewise `Agrees`
//!   with no surviving supporting citation.
//! * Context expansion: the model may list `needed_context`; up to `MAX_ROUNDS` rounds,
//!   each adding at most `MAX_EXPANSIONS_PER_ROUND` symbols via the `ContextProvider`.
//! * Cancellation (via the watch channel) kills the subprocess promptly.

pub mod model;
pub use model::*;

mod analyze;
mod cli;
mod prompt;
mod schema;
mod scripted;
mod validate;

#[cfg(test)]
mod tests;

pub use analyze::analyze;
pub use cli::ClaudeCli;
pub use prompt::SYSTEM_PROMPT;
pub use schema::{assess_schema, challenge_schema};
pub use scripted::ScriptedBackend;
