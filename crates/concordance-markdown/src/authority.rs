//! Document authority: configuration, front matter, RFC 2119 keywords, path conventions.

use concordance_core::doc::{Authority, AuthorityAssessment, AuthoritySource};
use concordance_core::RepoPath;

pub struct FrontMatter {
    /// Byte offset just past the closing delimiter line.
    pub end: usize,
    pub authority: Option<Authority>,
}

/// Detect a leading `---` ... `---` block and the `concordance:` key inside it.
pub fn front_matter(text: &str) -> Option<FrontMatter> {
    let body = text.strip_prefix('\u{feff}').unwrap_or(text);
    let offset = text.len() - body.len();
    let mut pos = 0usize;
    let mut first = true;
    let mut authority = None;
    for line in body.split_inclusive('\n') {
        let trimmed = line.trim_end();
        if first {
            if trimmed != "---" {
                return None;
            }
            first = false;
        } else if trimmed == "---" || trimmed == "..." {
            return Some(FrontMatter {
                end: offset + pos + line.len(),
                authority,
            });
        } else if let Some(v) = trimmed.trim_start().strip_prefix("concordance:") {
            authority = match v
                .trim()
                .trim_matches(['"', '\''])
                .to_ascii_lowercase()
                .as_str()
            {
                "normative" => Some(Authority::Normative),
                "descriptive" => Some(Authority::Descriptive),
                "informative" => Some(Authority::Informative),
                _ => authority,
            };
        }
        pos += line.len();
    }
    None
}

/// Replace every byte of `text[..end]` that is not a newline with a space, keeping offsets.
pub fn blank_prefix(text: &str, end: usize) -> String {
    let mut out = String::with_capacity(text.len());
    for (i, c) in text.char_indices() {
        if i < end && c != '\n' && c != '\r' {
            for _ in 0..c.len_utf8() {
                out.push(' ');
            }
        } else {
            out.push(c);
        }
    }
    out
}

const RFC_WORDS: &[&str] = &["MUST", "SHALL", "REQUIRED", "SHOULD", "RECOMMENDED"];

/// Count upper-case RFC 2119 keywords appearing as whole words.
pub fn count_rfc_keywords(masked_text: &str) -> usize {
    masked_text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| RFC_WORDS.contains(w))
        .count()
}

const NORMATIVE_DIRS: &[&str] = &[
    "spec",
    "specs",
    "specification",
    "specifications",
    "rfc",
    "rfcs",
    "requirements",
    "adr",
    "adrs",
];
const DESCRIPTIVE_DIRS: &[&str] = &["guide", "guides", "tutorial", "tutorials", "howto"];
const INFORMATIVE_DIRS: &[&str] = &["history", "archive", "archived"];

fn assessment(level: Authority, source: AuthoritySource, rationale: String) -> AuthorityAssessment {
    AuthorityAssessment {
        level,
        source,
        rationale,
    }
}

pub fn assess(
    path: &RepoPath,
    configured: Option<AuthorityAssessment>,
    front: Option<Authority>,
    rfc_keywords: usize,
) -> AuthorityAssessment {
    if let Some(c) = configured {
        return c;
    }
    if let Some(level) = front {
        let name = match level {
            Authority::Normative => "normative",
            Authority::Descriptive => "descriptive",
            _ => "informative",
        };
        return assessment(
            level,
            AuthoritySource::FrontMatter,
            format!("front matter declares `concordance: {name}`"),
        );
    }
    if rfc_keywords >= 2 {
        return assessment(
            Authority::Normative,
            AuthoritySource::Rfc2119Keywords,
            format!("{rfc_keywords} upper-case RFC 2119 keywords (MUST, SHALL, ...) appear outside code"),
        );
    }
    let name = path.file_name();
    let stem = name.rsplit_once('.').map(|(s, _)| s).unwrap_or(name);
    let comps: Vec<String> = path
        .as_str()
        .split('/')
        .map(|c| c.to_ascii_lowercase())
        .collect();
    let dirs = &comps[..comps.len().saturating_sub(1)];
    let stem_lower = stem.to_ascii_lowercase();
    let find = |set: &[&str]| -> Option<String> {
        dirs.iter()
            .find(|d| set.contains(&d.as_str()))
            .cloned()
            .or_else(|| {
                set.contains(&stem_lower.as_str())
                    .then(|| stem_lower.clone())
            })
    };
    if let Some(c) = find(NORMATIVE_DIRS) {
        return assessment(
            Authority::Normative,
            AuthoritySource::PathConvention,
            format!("path component `{c}` indicates normative specification"),
        );
    }
    let lname = name.to_ascii_lowercase();
    if lname == "readme.md" || lname == "changelog.md" {
        return assessment(
            Authority::Descriptive,
            AuthoritySource::PathConvention,
            format!("file name `{name}` is conventionally descriptive"),
        );
    }
    if let Some(c) = find(DESCRIPTIVE_DIRS) {
        return assessment(
            Authority::Descriptive,
            AuthoritySource::PathConvention,
            format!("path component `{c}` indicates descriptive documentation"),
        );
    }
    if let Some(c) = find(INFORMATIVE_DIRS) {
        return assessment(
            Authority::Informative,
            AuthoritySource::PathConvention,
            format!("path component `{c}` indicates historical material"),
        );
    }
    assessment(
        Authority::Unknown,
        AuthoritySource::Default,
        "no configuration, front matter, RFC 2119 keywords or path convention established authority".to_string(),
    )
}
