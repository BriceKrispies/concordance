# Expected Concordance findings: `undocumented`

The spec gives only a general statement about startup configuration. The public
API has important behaviour (an environment dependency, several error variants,
an `unwrap` that can panic, and a `panic!` on shutdown) that the spec never
states. Concordance should report the gaps as `undocumented`, not invent claims.

## Expected findings

- `load_config(path: &str) -> Result<Config, ConfigError>`.
  Status: **undocumented**. The spec says the application loads configuration at
  startup, which is not a contract for this function: no statement of its
  inputs, outputs, errors, or the `APP_MODE` dependency.
- `load_config()` reads the `APP_MODE` environment variable.
  Status: **undocumented**. Not in the spec at all.
- `ConfigError::Missing` and `ConfigError::Invalid(String)` as the error variants of `load_config()`.
  Status: **undocumented**. The variants have no documented meaning.
- `load_config()` calls `.unwrap()` on the first line of the file, so it panics on an empty file.
  Status: **undocumented**. This is observable behaviour the spec does not mention.
  Report it as undocumented panic behaviour; do not call it a violation, because the spec makes no claim.
- `shutdown(handle: &mut Server)` panics with `"server not started"` when the server has not been started.
  Status: **undocumented**. No contract at all for `shutdown()`.
- `Config` and `Server` as public types.
  Status: **undocumented** in their fields and lifecycle. `Config` is loosely mentioned
  ("settings are held in a `Config` value"), but its fields are not described.

## Must NOT be reported

- No `violated` or `verified` findings. The spec contains no testable contract about these functions.
- No `stale` finding for `Config`, `Server`, `load_config` or `shutdown`: all exist with the names used.
- The "Overview" and "Operations" paragraphs must not be reported as requirements, and they do not
  create contracts for the code.
- The `## Example` block must not be reported as a requirement. Its `Server::default()` call is
  a reference to a type that has no `Default` impl; this is an example error, and Concordance should
  not report it as a finding about the code's behaviour.
- The phrase "The application loads configuration at startup" should be classed as an ordinary statement,
  not as a verified claim about `load_config()`.
