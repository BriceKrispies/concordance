//! Assembly of a [`DocumentIndex`] from the parsed blocks.

use crate::authority;
use crate::classify::{self, Sentence};
use crate::inline::{Inline, Item};
use crate::lines::LineIndex;
use crate::parse::{self, Raw};
use crate::refs;
use crate::sections::{role_from_heading, AnchorAllocator};
use crate::text::{mask_ranges, split_sentences};
use concordance_core::doc::*;
use concordance_core::lang::Languages;
use concordance_core::{ClaimId, ContentHash, DocumentId, RepoPath, SectionId, SourceRange};
use std::collections::HashMap;

const SEP: &str = "\u{1f}";

struct Builder<'a> {
    path: &'a RepoPath,
    languages: &'a Languages,
    lines: &'a LineIndex<'a>,
    doc_id: DocumentId,
    doc_authority: Authority,
    sections: Vec<Section>,
    claims: Vec<Claim>,
    prose: Vec<ProseReference>,
    claim_occ: HashMap<(String, String), usize>,
    /// Requirement ID defined by each section's heading (or inherited from an ancestor).
    sec_req: Vec<Option<String>>,
    /// Requirement ID opening the text block being processed.
    block_req: Option<String>,
}

/// The requirement ID a heading defines: its first word, if that word is an ID
/// (`PSS-003`, `LANG-006.3 Comments`, `UI-004: Focus`).
fn heading_requirement(plain: &str) -> Option<String> {
    let first = plain.split_whitespace().next()?;
    let first = first
        .trim_end_matches([':', '.', ',', ')'])
        .trim_start_matches('(');
    is_requirement_id(first).then(|| first.to_string())
}

/// A bold requirement ID opening a block: `**LANG-006.3** Block comments ...`.
/// Returns the ID and the number of plain-text bytes to skip (ID plus separator).
fn inline_requirement(source: &str, inline: &Inline) -> Option<(String, usize)> {
    let text = inline.text.as_str();
    let end = text.find(|c: char| c.is_whitespace()).unwrap_or(text.len());
    let token = text[..end].trim_end_matches([':', '.', ',']);
    let token = token.trim_start_matches('[').trim_end_matches(']');
    if !is_requirement_id(token) {
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

fn code_ranges(inline: &Inline) -> Vec<(usize, usize)> {
    inline
        .items
        .iter()
        .filter_map(|i| match i {
            Item::Code { ts, te, .. } => Some((*ts, *te)),
            _ => None,
        })
        .collect()
}

impl<'a> Builder<'a> {
    fn reference(&self, item: &Item) -> Option<Reference> {
        match item {
            Item::Code { code, bytes, .. } => {
                let (target, strength) = refs::classify_code(code)?;
                Some(Reference {
                    text: code.clone(),
                    target,
                    strength,
                    range: Some(self.lines.range(self.path, bytes.0, bytes.1)),
                })
            }
            Item::Link { dest, bytes, .. } => {
                let target = refs::classify_link(self.path, dest)?;
                Some(Reference {
                    text: dest.clone(),
                    target,
                    strength: ReferenceStrength::Explicit,
                    range: Some(self.lines.range(self.path, bytes.0, bytes.1)),
                })
            }
        }
    }

    fn add_prose(&mut self, section: &SectionId, refs: Vec<Reference>) {
        for r in refs {
            let weak_symbol = matches!(r.target, ReferenceTarget::Symbol { .. })
                && r.strength == ReferenceStrength::Weak;
            if !weak_symbol {
                self.prose.push(ProseReference {
                    section: section.clone(),
                    reference: r,
                });
            }
        }
    }

    fn effective_authority(&self, role: SectionRole) -> Authority {
        if role != SectionRole::Body {
            Authority::Informative
        } else {
            self.doc_authority
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn push_claim(
        &mut self,
        sec: usize,
        text: String,
        kind: ClaimKind,
        modality: Modality,
        block: BlockKind,
        range: SourceRange,
        references: Vec<Reference>,
        extra: (
            Vec<Bound>,
            Vec<AmbiguityNote>,
            Option<String>,
            Option<ConstraintForm>,
        ),
    ) {
        let section = &self.sections[sec];
        let hp = section.heading_path.join(SEP);
        let lower = text.to_lowercase();
        let occ = self
            .claim_occ
            .entry((hp.clone(), lower.clone()))
            .or_insert(0);
        let occ_s = occ.to_string();
        *occ += 1;
        let id = ClaimId::from_key(&[self.path.as_str(), &hp, &lower, &occ_s]);
        let content_hash = ContentHash::of_str(&format!("{hp}{SEP}{text}"));
        let authority = self.effective_authority(section.role);
        let (bounds, ambiguity, documented_signature, constraint) = extra;
        let requirement = match (&self.block_req, &self.sec_req[sec]) {
            (Some(id), _) => Some(RequirementRef {
                id: id.clone(),
                form: RequirementForm::Inline,
            }),
            (None, Some(id)) => Some(RequirementRef {
                id: id.clone(),
                form: RequirementForm::Heading,
            }),
            (None, None) => None,
        };
        // The requirement is part of the claim's meaning: re-tagging it is a change.
        let content_hash = match &requirement {
            Some(r) => ContentHash::of_str(&format!("{hp}{SEP}{}{SEP}{text}", r.id)),
            None => content_hash,
        };
        self.claims.push(Claim {
            id,
            document: self.doc_id.clone(),
            section: section.id.clone(),
            heading_path: section.heading_path.clone(),
            text,
            kind,
            modality,
            authority,
            block,
            range,
            content_hash,
            references,
            bounds,
            ambiguity,
            documented_signature,
            signature_language: None,
            constraint,
            requirement,
        });
    }

    fn text_block(
        &mut self,
        sec: usize,
        kind: BlockKind,
        inline: &Inline,
        whole_row: Option<(usize, usize)>,
    ) {
        let text = inline.text.as_str();
        let cranges = code_ranges(inline);
        let sentences = match whole_row {
            Some(_) => vec![(0, text.len())],
            None => split_sentences(text, &cranges),
        };
        let section_id = self.sections[sec].id.clone();
        let opener = inline_requirement(self.lines.source(), inline);
        self.block_req = opener.as_ref().map(|(id, _)| id.clone());
        let skip = opener.map(|(_, n)| n).unwrap_or(0);
        for (s, e) in sentences {
            // The opening ID is a label, not part of the statement.
            let s = if s < skip { skip.min(e) } else { s };
            if s >= e {
                continue;
            }
            let sent = &text[s..e];
            let mut refs = Vec::new();
            let mut codes = Vec::new();
            let mut rel_ranges = Vec::new();
            for item in &inline.items {
                let ts = item.start();
                if ts < s || ts >= e {
                    continue;
                }
                if let Item::Code { te, code, .. } = item {
                    codes.push(code.clone());
                    rel_ranges.push((ts - s, te.min(&e) - s));
                }
                if let Some(r) = self.reference(item) {
                    refs.push(r);
                }
            }
            let classified = classify::classify(&Sentence {
                text: sent,
                code_ranges: &rel_ranges,
                refs: &refs,
                codes: &codes,
            });
            match classified {
                Some(c) => {
                    let range = match whole_row {
                        Some((a, b)) => self.lines.line_range(self.path, a, b),
                        None => {
                            let a = inline.byte_at(s);
                            let b = inline.byte_at(e.saturating_sub(1)) + 1;
                            self.lines.line_range(self.path, a, b)
                        }
                    };
                    self.push_claim(
                        sec,
                        sent.to_string(),
                        c.kind,
                        c.modality,
                        kind,
                        range,
                        refs,
                        (c.bounds, c.ambiguity, None, c.constraint),
                    );
                }
                None => self.add_prose(&section_id, refs),
            }
        }
    }

    fn code_block(&mut self, sec: usize, lang: &str, content: &str, first_byte: usize) {
        let Some(plugin) = self.languages.for_fence(lang) else {
            return;
        };
        let language = plugin.id().to_string();
        let first_line = self.lines.line(first_byte);
        for sig in plugin.documented_signatures(content) {
            let start = first_line + sig.start as u32;
            let end = (first_line + sig.end as u32)
                .min(self.lines.max_line())
                .max(start);
            let range = SourceRange::lines(self.path.clone(), start, end);
            let collapsed = sig.text.split_whitespace().collect::<Vec<_>>().join(" ");
            let reference = Reference {
                text: sig.name.clone(),
                target: ReferenceTarget::Symbol {
                    path: vec![sig.name.clone()],
                },
                strength: ReferenceStrength::Explicit,
                range: Some(SourceRange {
                    path: self.path.clone(),
                    start_line: start,
                    start_col: sig.name_col as u32 + 1,
                    end_line: start,
                    end_col: (sig.name_col + sig.name.chars().count()) as u32,
                }),
            };
            self.push_claim(
                sec,
                collapsed,
                ClaimKind::ApiContract,
                Modality::Declarative,
                BlockKind::CodeBlock,
                range,
                vec![reference],
                (Vec::new(), Vec::new(), Some(sig.text.clone()), None),
            );
            if let Some(c) = self.claims.last_mut() {
                c.signature_language = Some(language.clone());
            }
        }
    }
}

/// RFC 2119 keyword count over prose (code spans and code blocks excluded).
fn rfc_keyword_count(raws: &[Raw]) -> usize {
    raws.iter()
        .map(|r| match r {
            Raw::Text { inline, .. } => {
                authority::count_rfc_keywords(&mask_ranges(&inline.text, &code_ranges(inline)))
            }
            _ => 0,
        })
        .sum()
}

pub fn build(
    path: &RepoPath,
    text: &str,
    configured: Option<AuthorityAssessment>,
    languages: &Languages,
) -> DocumentIndex {
    let front = authority::front_matter(text);
    let parse_text = match &front {
        Some(f) => authority::blank_prefix(text, f.end),
        None => text.to_string(),
    };
    let lines = LineIndex::new(text);
    let raws = parse::walk(&parse_text);

    let title = raws.iter().find_map(|r| match r {
        Raw::Heading {
            level: 1, plain, ..
        } if !plain.is_empty() => Some(plain.clone()),
        _ => None,
    });
    let doc_id = DocumentId::from_key(&[path.as_str()]);
    let assessment = authority::assess(
        path,
        configured,
        front.and_then(|f| f.authority),
        rfc_keyword_count(&raws),
    );
    let document = Document {
        id: doc_id.clone(),
        path: path.clone(),
        title: title.clone(),
        authority: assessment.clone(),
        content_hash: ContentHash::of_str(text),
        line_count: lines.line_count(),
    };

    let mut b = Builder {
        path,
        languages,
        lines: &lines,
        doc_id,
        doc_authority: assessment.level,
        sections: Vec::new(),
        claims: Vec::new(),
        prose: Vec::new(),
        claim_occ: HashMap::new(),
        sec_req: vec![None],
        block_req: None,
    };

    // Section bookkeeping.
    let mut sec_occ: HashMap<String, usize> = HashMap::new();
    let mut anchors = AnchorAllocator::default();
    let mut start_lines: Vec<u32> = Vec::new();
    let mut stack: Vec<(u8, usize)> = vec![(0, 0)];

    let root_heading = title
        .clone()
        .unwrap_or_else(|| path.file_name().to_string());
    sec_occ.insert(String::new(), 1);
    b.sections.push(Section {
        id: SectionId::from_key(&[path.as_str(), "", "0"]),
        document: b.doc_id.clone(),
        parent: None,
        heading: root_heading,
        level: 0,
        heading_path: Vec::new(),
        anchor: String::new(),
        role: SectionRole::Body,
        range: SourceRange::lines(path.clone(), 1, 1),
    });
    start_lines.push(1);
    let mut current = 0usize;

    for raw in &raws {
        match raw {
            Raw::Heading {
                level,
                plain,
                inline,
                start,
            } => {
                while stack.len() > 1 && stack.last().is_some_and(|t| t.0 >= *level) {
                    stack.pop();
                }
                let parent_idx = stack.last().map(|t| t.1).unwrap_or(0);
                let parent = &b.sections[parent_idx];
                let mut hp = parent.heading_path.clone();
                hp.push(plain.clone());
                let role = role_from_heading(plain).unwrap_or(parent.role);
                let parent_id = parent.id.clone();
                let joined = hp.join(SEP);
                let occ = sec_occ.entry(joined.clone()).or_insert(0);
                let occ_s = occ.to_string();
                *occ += 1;
                let start_line = lines.line(*start);
                let idx = b.sections.len();
                b.sections.push(Section {
                    id: SectionId::from_key(&[path.as_str(), &joined, &occ_s]),
                    document: b.doc_id.clone(),
                    parent: Some(parent_id),
                    heading: plain.clone(),
                    level: *level,
                    heading_path: hp,
                    anchor: anchors.allocate(plain),
                    role,
                    range: SourceRange::lines(path.clone(), start_line, start_line),
                });
                start_lines.push(start_line);
                let inherited = b.sec_req[parent_idx].clone();
                b.sec_req.push(heading_requirement(plain).or(inherited));
                stack.push((*level, idx));
                current = idx;
                let sid = b.sections[idx].id.clone();
                let hrefs: Vec<Reference> =
                    inline.items.iter().filter_map(|i| b.reference(i)).collect();
                b.add_prose(&sid, hrefs);
            }
            Raw::Text {
                kind,
                inline,
                whole_row,
            } => {
                b.text_block(current, *kind, inline, *whole_row);
                b.block_req = None;
            }
            Raw::Code {
                lang,
                content,
                first_byte,
            } => b.code_block(current, lang, content, *first_byte),
        }
    }

    // Section ranges: heading line to the line before the next heading of the same or higher level.
    let max_line = lines.max_line();
    for k in 0..b.sections.len() {
        let level = b.sections[k].level;
        let start = start_lines[k];
        let end = match (k + 1..b.sections.len()).find(|&j| b.sections[j].level <= level) {
            Some(j) => start_lines[j].saturating_sub(1),
            None => max_line,
        };
        b.sections[k].range = SourceRange::lines(path.clone(), start, end.max(start));
    }

    DocumentIndex {
        document,
        sections: b.sections,
        claims: b.claims,
        prose_references: b.prose,
    }
}
