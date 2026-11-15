//! Documents that do not exist as files: Markdown rendered from a structured specification
//! source (an Exegra JSONC file, see `concordance-exegra`).
//!
//! A virtual document is keyed by the path it would have as a file. Its text is what the
//! Markdown front end indexed, so every line, range and hash refers to that text. [`Origin`]
//! carries a location back to the entity of the source it was rendered from.

use crate::RepoPath;
use serde::{Deserialize, Serialize};
use std::fmt;

/// Where a line of a virtual document came from in its source file.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Origin {
    /// The source file (the JSONC), repository-relative.
    pub file: RepoPath,
    /// 1-based line of the entity in the source file.
    pub line: u32,
    /// Id of the entity in the source file.
    pub entity: String,
}

impl fmt::Display for Origin {
    /// `specifications/spec.jsonc:1320 krn-002`
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{} {}", self.file, self.line, self.entity)
    }
}

impl Origin {
    /// `location` followed by the origin in parentheses, when there is one:
    /// `specifications/kernel.md:23 (specifications/spec.jsonc:1320 krn-002)`.
    pub fn annotate(location: &str, origin: Option<&Origin>) -> String {
        match origin {
            Some(o) => format!("{location} ({o})"),
            None => location.to_string(),
        }
    }
}

/// One rendered document and the map from its lines back to its source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VirtualDoc {
    /// The file the document was rendered from.
    pub source: RepoPath,
    /// The rendered Markdown. Its content hash is the indexed document's hash.
    pub text: String,
    entities: Vec<String>,
    /// Per line: (index into `entities`, line in the source file).
    lines: Vec<(u32, u32)>,
}

impl VirtualDoc {
    /// `lines[i]` is `(entity id, source line)` for line `i + 1` of `text`.
    pub fn new(
        source: RepoPath,
        text: String,
        lines: impl IntoIterator<Item = (String, u32)>,
    ) -> Self {
        let mut entities: Vec<String> = Vec::new();
        let mut table = Vec::new();
        for (entity, line) in lines {
            let idx = match entities.iter().rposition(|e| *e == entity) {
                Some(i) => i,
                None => {
                    entities.push(entity);
                    entities.len() - 1
                }
            };
            table.push((idx as u32, line));
        }
        VirtualDoc {
            source,
            text,
            entities,
            lines: table,
        }
    }

    /// The origin of `line` (1-based) of the text.
    pub fn origin_of(&self, line: u32) -> Option<Origin> {
        let (e, src) = *self
            .lines
            .get(usize::try_from(line.checked_sub(1)?).ok()?)?;
        Some(Origin {
            file: self.source.clone(),
            line: src,
            entity: self.entities[e as usize].clone(),
        })
    }
}

/// A problem found while rendering a source file, located in the source.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceDiagnostic {
    pub source: RepoPath,
    pub entity: String,
    pub line: u32,
    /// An error makes `lint` fail; a warning is reported only.
    pub error: bool,
    pub message: String,
}

impl fmt::Display for SourceDiagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}:{} ({}): {}",
            self.source, self.line, self.entity, self.message
        )
    }
}
