Status: draft

# Concordance

This white paper covers what Concordance promises to the people and agents who run it: the languages it
understands, the test results it accepts, and the specification lint. The engine's internal design (graph,
incremental reconciliation, events) is described in `docs/specifications/` and `docs/architecture.md`, which
are descriptive.

## Problem

Specifications and code drift apart silently. A reviewer cannot tell, for a given requirement, whether the
code does what it says, whether anything proves it, or whether the requirement still means anything. Coding
agents make this worse, because they produce code faster than anyone can check it against intent.

## Goals

- **G-1** For a repository in a supported language, `concordance scan` reports which spec statements the code
  contradicts, with the exact spec and code locations.
- **G-2** A requirement is reported verified or violated from the recorded results of the tests that declare
  it, in the test-result formats of the supported languages.
- **G-3** `concordance lint` tells a spec author, line by line, where a specification breaks the requirement
  format.
- **G-4** Rust, TypeScript, C# and Lean 4 repositories are supported.

## Non-goals

- **NG-1** Concordance does not run the analyzed repository's tests or builds. It reads their recorded results.
- **NG-2** Concordance does not edit the analyzed repository.
- **NG-3** Languages beyond the four listed, until a plugin is added.

## Boundaries

| ID | Boundary | Kind | Who uses it |
|---|---|---|---|
| B-1 | `concordance scan` and its report (`--json`) | CLI | people, CI, agents |
| B-2 | `concordance lint` | CLI | spec authors, agents |
| B-3 | `concordance.toml` | file format | everyone who configures it |
| B-4 | The test-result files given with `--test-results` or `test_results` | file formats | CI, test runners |
| B-5 | The embedding API: `concordance_engine::start` and the model it publishes | library API | embedders and Concordance's own tests |

## Architecture

| ID | Component | Owns | Talks to |
|---|---|---|---|
| C-1 | Markdown indexer | claims, requirements and their attributes | C-3 |
| C-2 | Language plugins (Rust, TypeScript, C#, Lean 4) | symbols, facts, tests and `Verifies:` per language | C-3 |
| C-3 | Engine | linking, reconciliation, findings, test-result ingestion | C-1, C-2, C-4 |
| C-4 | Lint | the requirement-format rules | C-1 |

## Decisions

- **D-1** Detailed behaviour per language is stated as conformance cases: tables of spec statement and
  expected finding next to each fixture. The specification states the capability once, and supporting a new
  construct adds a case, not a requirement.
- **D-2** Plugins parse without the language's own toolchain: `syn` for Rust, `oxc` for TypeScript,
  tree-sitter for C#, and a lexical scanner for Lean 4.

## Constraints

- **K-1** A scan never modifies any file under the analyzed repository.
- **K-2** Scanning the same repository content twice reports the same findings.

## Open questions

- **Q-1** Should `docs/specifications/` be rewritten into requirements over time, or stay descriptive?
  Recommendation: stay descriptive, and grow `docs/spec/` umbrella by umbrella as behaviour is touched.

## Glossary

- **conformance case**: one row of a fixture's case table: a spec statement, the expected finding, and where
  it must point.
- **supported language**: a language with a registered plugin.
