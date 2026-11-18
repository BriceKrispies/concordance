//! Requirement blocks: a bold ID opening a paragraph or list item, the attribute bullets
//! directly after it (`Verify:`, `Boundary:`, `Traces:`, `Rationale:`, `Depends:`, `Status:`),
//! and every other piece of prose outside requirements. Shared by claim extraction (the
//! attributes belong to the requirement, not to claims) and by the specification lint.

use crate::inline::{Inline, Item};
use crate::lines::LineIndex;
use crate::parse::{self, Raw};
use concordance_core::doc::{is_requirement_id, BlockKind, RequirementAttributes};
use std::collections::HashMap;

/// A backticked span with the line it is on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CodeSpan {
    pub text: String,
    pub line: u32,
}

/// One attribute bullet of a requirement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttributeItem {
    /// One of [`RequirementAttributes::NAMES`].
    pub name: &'static str,
    /// Text after `Name:`, with code spans kept in backticks.
    pub value: String,
    pub line: u32,
    pub codes: Vec<CodeSpan>,
}

/// A requirement as written: ID, statement and attributes, with exact lines.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequirementBlock {
    /// The ID as written (it may be malformed: see [`RequirementBlock::is_recognized`]).
    pub id: String,
    /// Line of the opener.
    pub line: u32,
    /// The statement: the paragraph after the ID, code spans in backticks.
    pub statement: String,
    /// Statement text offset -> source line, ascending by offset.
    line_map: Vec<(usize, u32)>,
    /// Backticked spans in the statement.
    pub codes: Vec<CodeSpan>,
    pub attributes: Vec<AttributeItem>,
}

impl RequirementBlock {
    /// Source line of byte `offset` into [`Self::statement`].
    pub fn line_at(&self, offset: usize) -> u32 {
        let i = self.line_map.partition_point(|a| a.0 <= offset);
        if i == 0 {
            self.line
        } else {
            self.line_map[i - 1].1
        }
    }

    /// The first attribute named `name`.
    pub fn attribute(&self, name: &str) -> Option<&AttributeItem> {
        self.attributes.iter().find(|a| a.name == name)
    }

    /// Whether the ID has the shape of a requirement ID (see [`concordance_core::doc::is_requirement_id`],
    /// the one grammar `scan` and `lint` both use).
    pub fn is_recognized(&self) -> bool {
        is_requirement_id(&self.id)
    }

    /// The attributes as stored on a claim's requirement reference.
    pub fn attribute_set(&self) -> RequirementAttributes {
        let mut set = RequirementAttributes::default();
        for a in &self.attributes {
            set.set(a.name, a.value.clone());
        }
        set
    }
}

/// Prose outside any requirement (and not a heading, table or code block).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LooseText {
    pub line: u32,
    /// The text with code spans blanked out (same byte length as the original).
    pub text: String,
    pub block: BlockKind,
    line_map: Vec<(usize, u32)>,
}

impl LooseText {
    /// Source line of byte `offset` into [`Self::text`].
    pub fn line_at(&self, offset: usize) -> u32 {
        let i = self.line_map.partition_point(|a| a.0 <= offset);
        if i == 0 {
            self.line
        } else {
            self.line_map[i - 1].1
        }
    }
}

/// `text` with the byte ranges replaced by spaces of the same length.
fn blank_ranges(text: &str, ranges: &[(usize, usize)]) -> String {
    let mut out = String::with_capacity(text.len());
    for (i, c) in text.char_indices() {
        if ranges.iter().any(|&(s, e)| i >= s && i < e) {
            out.extend(std::iter::repeat_n(' ', c.len_utf8()));
        } else {
            out.push(c);
        }
    }
    out
}

/// The requirement structure of one Markdown file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SpecStructure {
    pub requirements: Vec<RequirementBlock>,
    pub loose: Vec<LooseText>,
}

/// Something that looks like an ID but is not necessarily a valid one: `cli-004`, `CLI-04`,
/// `CLI_004`. Used for lint only.
fn looks_like_id(tok: &str) -> bool {
    if tok.len() > 40 || tok.is_empty() {
        return false;
    }
    let Some(sep) = tok.rfind(['-', '_']) else {
        return false;
    };
    let (prefix, num) = (&tok[..sep], &tok[sep + 1..]);
    let alnum = |c: char| c.is_ascii_alphanumeric();
    !prefix.is_empty()
        && prefix
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic())
        && prefix.chars().all(|c| alnum(c) || c == '-' || c == '_')
        && num.chars().next().is_some_and(|c| c.is_ascii_digit())
        && num.chars().all(|c| alnum(c) || c == '.')
}

/// A bold ID opening a block: `**LANG-006.3** Block comments ...`. Returns the ID and the
/// number of plain-text bytes to skip (ID plus separator). With `lenient`, IDs that only
/// look like IDs are returned too.
pub(crate) fn opener(source: &str, inline: &Inline, lenient: bool) -> Option<(String, usize)> {
    let text = inline.text.as_str();
    let end = text.find(|c: char| c.is_whitespace()).unwrap_or(text.len());
    let token = text[..end].trim_end_matches([':', '.', ',']);
    let token = token.trim_start_matches('[').trim_end_matches(']');
    if !(is_requirement_id(token) || (lenient && looks_like_id(token))) {
        return None;
    }
    let at = inline.byte_at(0);
    let bold = source
        .get(..at)
        .is_some_and(|p| p.ends_with("**") || p.ends_with("__") || p.ends_with("**["));
    if !bold {
        return None;
    }
    let rest = &text[end..];
    let skip = end
        + (rest.len()
            - rest
                .trim_start_matches(|c: char| c.is_whitespace() || matches!(c, ':' | '—' | '-'))
                .len());
    Some((token.to_string(), skip))
}

fn attribute_of(text: &str) -> Option<(&'static str, usize)> {
    for name in RequirementAttributes::NAMES {
        if let Some(rest) = text.strip_prefix(name) {
            let trimmed = rest.trim_start();
            if let Some(after) = trimmed.strip_prefix(':') {
                let value = after.trim_start();
                return Some((name, text.len() - value.len()));
            }
        }
    }
    None
}

fn codes_from(inline: &Inline, from: usize, lines: &LineIndex) -> Vec<CodeSpan> {
    inline
        .items
        .iter()
        .filter_map(|i| match i {
            Item::Code { ts, code, .. } if *ts >= from => Some(CodeSpan {
                text: code.clone(),
                line: lines.line(inline.byte_at(*ts)),
            }),
            _ => None,
        })
        .collect()
}

pub(crate) struct Blocks {
    pub blocks: Vec<RequirementBlock>,
    /// Raw index of an opener -> index into `blocks`.
    pub openers: HashMap<usize, usize>,
    /// Raw indexes of consumed attribute bullets.
    pub attribute_raws: std::collections::HashSet<usize>,
    pub loose: Vec<LooseText>,
}

/// Find requirement blocks and loose prose in the flat raw walk.
pub(crate) fn find(raws: &[Raw], source: &str, lines: &LineIndex) -> Blocks {
    let mut out = Blocks {
        blocks: Vec::new(),
        openers: HashMap::new(),
        attribute_raws: Default::default(),
        loose: Vec::new(),
    };
    let mut i = 0;
    while i < raws.len() {
        let Raw::Text { kind, inline, .. } = &raws[i] else {
            i += 1;
            continue;
        };
        let found = if matches!(kind, BlockKind::Paragraph | BlockKind::ListItem) {
            opener(source, inline, true)
        } else {
            None
        };
        let Some((id, skip)) = found else {
            if !matches!(kind, BlockKind::TableRow) && !out.attribute_raws.contains(&i) {
                let ranges: Vec<(usize, usize)> = inline
                    .items
                    .iter()
                    .filter_map(|it| match it {
                        Item::Code { ts, te, .. } => Some((*ts, *te)),
                        _ => None,
                    })
                    .collect();
                out.loose.push(LooseText {
                    line: lines.line(inline.byte_at(0)),
                    text: blank_ranges(&inline.text, &ranges),
                    block: *kind,
                    line_map: inline
                        .anchors
                        .iter()
                        .map(|a| (a.0, lines.line(a.1)))
                        .collect(),
                });
            }
            i += 1;
            continue;
        };
        let line_map = inline
            .anchors
            .iter()
            .map(|a| (a.0.saturating_sub(skip), lines.line(a.1)))
            .collect();
        let mut block = RequirementBlock {
            id,
            line: lines.line(inline.byte_at(0)),
            statement: inline.text[skip.min(inline.text.len())..].to_string(),
            line_map,
            codes: codes_from(inline, skip, lines),
            attributes: Vec::new(),
        };
        let idx = out.blocks.len();
        out.openers.insert(i, idx);
        let mut j = i + 1;
        while let Some(Raw::Text {
            kind: BlockKind::ListItem,
            inline: item,
            ..
        }) = raws.get(j)
        {
            if opener(source, item, true).is_some() {
                break;
            }
            if let Some((name, value_at)) = attribute_of(&item.text) {
                block.attributes.push(AttributeItem {
                    name,
                    value: item.text[value_at..].trim().to_string(),
                    line: lines.line(item.byte_at(0)),
                    codes: codes_from(item, value_at, lines),
                });
                out.attribute_raws.insert(j);
            }
            j += 1;
        }
        out.blocks.push(block);
        i += 1;
    }
    out
}

/// Parse `text` and return its requirement blocks and loose prose, with exact lines.
pub fn structure(text: &str) -> SpecStructure {
    let front = crate::authority::front_matter(text);
    let parse_text = match &front {
        Some(f) => crate::authority::blank_prefix(text, f.end),
        None => text.to_string(),
    };
    let lines = LineIndex::new(text);
    let raws = parse::walk(&parse_text);
    let found = find(&raws, text, &lines);
    SpecStructure {
        requirements: found.blocks,
        loose: found.loose,
    }
}
