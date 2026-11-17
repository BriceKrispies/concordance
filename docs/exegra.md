# Exegra specifications

A specification can be an Exegra JSONC file instead of Markdown. Concordance
reads any `.jsonc` or `.json` file under a spec root whose top-level object has an `"exegra"` key;
other JSON (`package.json`, data files) is ignored. Findings point into the `.jsonc` file.

## What is read

**Converted sections.** A specification converted from Markdown (Exegra's
`tools/import-register.js`) keeps the Markdown in `sections`: each section's `title` and `level`
give its heading, and its `body` holds the prose verbatim, with each table header and table row
replaced by a `{{entity:<id>}}` marker line. Concordance renders the sections to Markdown in memory
exactly as `import-register.js --render` does (sections by `order`; tables from `columns` and
`separator`; rows from `cells`, or from their table's `columnProperties`, honouring a row's
`layout`), indexes that Markdown like any Markdown specification, and maps each rendered line back
to the JSONC:

| Rendered line | Located at |
|---|---|
| heading | the section's `title` |
| prose | the section's `body` (with the column inside the string, so code references are exact) |
| table header or separator | the table entity |
| table row | the row entity |

So converted prose yields the same claims as the rendered Markdown, located in the `.jsonc`.

**Rows that define requirements.** A row of a configured kind that carries a `ref` defines that
`ref` as a requirement ID, so a test can declare `Verifies: TEN-1`. The row becomes one claim even
when its text has no modal verb (a register row states a rule in the present tense). The kind of a
row is its table's `rowKind`, or else the name of the list that holds it.

```toml
[exegra]
requirement_kinds = ["rule", "boundaryRule", "obligation", "requirement", "claim"]  # the default
```

Singular and plural spellings are the same kind (`rule` = `rules`).

**Authored entities.**

| List | Read as |
|---|---|
| `claims` | One normative claim per entry: the `statement`, located at its line. `kind` sets the claim kind (`invariant`; `structural`, `calls`, `reads`, `writes`, `data-flow` -> architectural constraint; `interface` -> API contract); the modality comes from the text. A `ref` defines a requirement ID (when `claim` is in `requirement_kinds`). |
| `designAreas`, `variationPoints`, `seams`, `assumptions` | Informative text (`title`, `intent`, `missing`; `description`, `tenantNeed`, `provisionalDefault`; `question`, `provisionalChoice`, `interface`, `revisitWhen`; `statement`), one section per entity. They describe choices and open decisions, not obligations, so they never produce compliance findings; their references are still checked. |

They are indexed in their own top-level sections (`Claims`, `Design areas`, ...), which do not
change the rendered Markdown.

## Authority

An Exegra specification is normative by default (`source: spec_format`). `[authority]` globs
override it like any other document.

## Identity and incremental behaviour

A change to the file re-derives its document like a Markdown edit. Claim identity does not depend
on line numbers (it comes from the heading path and the text), so edits elsewhere in the file keep
every other claim's id; a reworded claim keeps its id under the usual carry-over rule.

## Checking the rendering

```
cargo run --release -p concordance-exegra --example render -- spec.jsonc out.md
cargo run --release -p concordance-exegra --example compare -- spec.jsonc rendered.md
```

`render` prints the Markdown Concordance indexes; it should be byte-identical to
`node tools/import-register.js --render <spec.jsonc> --out rendered.md`. `compare` indexes both
and reports claim counts by kind, what the JSONC adds, and any range outside the file.
