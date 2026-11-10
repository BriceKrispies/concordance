//! Section roles and anchor generation.

use crate::text::{has_phrase, has_word, slugify, words_lower};
use concordance_core::doc::SectionRole;
use std::collections::{HashMap, HashSet};

/// Role implied by the heading text alone (`None` = inherit).
pub fn role_from_heading(heading: &str) -> Option<SectionRole> {
    let w = words_lower(heading);
    let any = |ws: &[&str]| ws.iter().any(|x| has_word(&w, x));
    if any(&["example", "examples"]) {
        return Some(SectionRole::Example);
    }
    if any(&["history", "changelog"])
        || has_phrase(&w, &["alternatives", "considered"])
        || has_phrase(&w, &["prior", "art"])
    {
        return Some(SectionRole::History);
    }
    if any(&["rationale", "why"]) {
        return Some(SectionRole::Rationale);
    }
    if any(&["background", "context", "motivation"]) {
        return Some(SectionRole::Background);
    }
    if any(&["notes"]) {
        return Some(SectionRole::ImplementationNotes);
    }
    if has_phrase(&w, &["non", "goals"])
        || has_word(&w, "nongoals")
        || has_phrase(&w, &["out", "of", "scope"])
    {
        return Some(SectionRole::NonGoals);
    }
    if has_phrase(&w, &["open", "questions"])
        || any(&["todo"])
        || has_phrase(&w, &["future", "work"])
    {
        return Some(SectionRole::OpenQuestions);
    }
    None
}

/// Hands out GitHub-style unique anchors (`foo`, `foo-1`, `foo-2`).
#[derive(Default)]
pub struct AnchorAllocator {
    seen: HashSet<String>,
    next: HashMap<String, usize>,
}

impl AnchorAllocator {
    pub fn allocate(&mut self, heading: &str) -> String {
        let base = slugify(heading);
        if self.seen.insert(base.clone()) {
            return base;
        }
        let n = self.next.entry(base.clone()).or_insert(1);
        loop {
            let candidate = format!("{base}-{n}");
            *n += 1;
            if self.seen.insert(candidate.clone()) {
                return candidate;
            }
        }
    }
}
