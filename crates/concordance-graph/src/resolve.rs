//! Name resolution of documentation references against indexed symbols.
//!
//! This is syntactic: it matches path segments, not Rust's name-resolution rules.
//! Results say how they were obtained (`Resolved::Exact` vs `UniqueSuffix` vs
//! `Ambiguous`), and edges built from them carry that label.

use crate::Model;
use concordance_core::code::{Symbol, SymbolKind, Visibility};
use concordance_core::SymbolId;
use std::collections::BTreeMap;

/// Path roots that are never local symbols.
pub const STD_ROOTS: &[&str] = &["std", "core", "alloc", "proc_macro", "test"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Resolved {
    /// The full crate-qualified path matched one symbol.
    Exact(SymbolId),
    /// A path suffix matched exactly one symbol (after preferring the context crate).
    UniqueSuffix(SymbolId),
    /// Several symbols matched; best candidate first.
    Ambiguous(Vec<SymbolId>),
    /// The reference names a workspace crate.
    Crate(String),
    /// The reference names something outside the analyzed code (std or a dependency).
    External(String),
    Unresolved,
}

impl Resolved {
    pub fn best(&self) -> Option<&SymbolId> {
        match self {
            Resolved::Exact(s) | Resolved::UniqueSuffix(s) => Some(s),
            Resolved::Ambiguous(v) => v.first(),
            _ => None,
        }
    }
}

fn normalize_crate(s: &str) -> String {
    s.replace('-', "_")
}

impl Model {
    pub fn is_workspace_crate(&self, name: &str) -> bool {
        let n = normalize_crate(name);
        self.workspace.crates.iter().any(|c| c.name == n)
    }

    fn is_dependency_crate(&self, name: &str) -> bool {
        let n = normalize_crate(name);
        self.workspace
            .crates
            .iter()
            .any(|c| c.dependencies.contains(&n))
    }

    /// Resolve written path segments (e.g. `["Limiter","refill"]`) to symbols.
    /// `context_crate` is used for `crate::` paths and to break ties.
    pub fn resolve_path(&self, segments: &[String], context_crate: Option<&str>) -> Resolved {
        let mut segs: Vec<String> = segments.iter().filter(|s| !s.is_empty()).cloned().collect();
        let mut anchored = false;
        while let Some(first) = segs.first() {
            match first.as_str() {
                "crate" => {
                    segs.remove(0);
                    if let Some(c) = context_crate {
                        segs.insert(0, c.to_string());
                        anchored = true;
                        break;
                    }
                }
                "self" | "super" | "Self" => {
                    segs.remove(0);
                }
                _ => break,
            }
        }
        let Some(first) = segs.first().cloned() else {
            return Resolved::Unresolved;
        };
        if segs.len() == 1 && self.is_workspace_crate(&first) {
            return Resolved::Crate(normalize_crate(&first));
        }
        let first_is_crate = self.is_workspace_crate(&first);
        if first_is_crate {
            segs[0] = normalize_crate(&first);
            anchored = true;
        } else if STD_ROOTS.contains(&first.as_str())
            || (segs.len() > 1 && self.is_dependency_crate(&first))
        {
            return Resolved::External(first);
        }
        let last = segs.last().expect("non-empty").clone();

        let candidates: Vec<&Symbol> = self
            .symbols_named(&last)
            .filter(|s| s.kind != SymbolKind::Impl)
            .collect();
        if candidates.is_empty() {
            return Resolved::Unresolved;
        }
        // 1. exact full path
        let exact: Vec<&Symbol> = candidates
            .iter()
            .copied()
            .filter(|s| s.path == segs)
            .collect();
        if exact.len() == 1 {
            return Resolved::Exact(exact[0].id.clone());
        }
        // 2. suffix match; for crate-anchored paths also allow re-exports
        //    (`krate::Type` matching `krate::module::Type`).
        let mut matched: Vec<&Symbol> = candidates
            .iter()
            .copied()
            .filter(|s| s.path.ends_with(&segs))
            .collect();
        if matched.is_empty() && anchored && segs.len() >= 2 {
            matched = candidates
                .iter()
                .copied()
                .filter(|s| s.path.first() == segs.first() && s.path.ends_with(&segs[1..]))
                .collect();
        }
        if matched.is_empty() {
            return Resolved::Unresolved;
        }
        if matched.len() == 1 {
            return Resolved::UniqueSuffix(matched[0].id.clone());
        }
        // Rank: context crate, non-test, public, shorter path.
        let ctx = context_crate.map(normalize_crate);
        matched.sort_by_key(|s| {
            let in_ctx = ctx
                .as_deref()
                .is_some_and(|c| s.path.first().map(String::as_str) == Some(c));
            let is_test = s.path.iter().any(|p| p == "tests" || p == "test")
                || s.cfg.iter().any(|c| c == "test");
            (
                !in_ctx,
                is_test,
                s.visibility != Visibility::Public,
                s.path.len(),
                s.id.clone(),
            )
        });
        Resolved::Ambiguous(matched.into_iter().map(|s| s.id.clone()).collect())
    }

    /// All symbols under a scope path (crate, module, type or item), including the scope
    /// symbol itself. Empty scope = every symbol. `None` if the scope does not resolve.
    pub fn symbols_in_scope(&self, scope: &[String]) -> Option<Vec<&Symbol>> {
        if scope.is_empty() {
            return Some(self.symbols.values().collect());
        }
        let prefixes: Vec<Vec<String>> = match self.resolve_path(scope, None) {
            Resolved::Crate(c) => vec![vec![c]],
            Resolved::Exact(id) | Resolved::UniqueSuffix(id) => {
                vec![self.symbols.get(&id)?.path.clone()]
            }
            // Ambiguous scope: check every candidate, never just the first one.
            Resolved::Ambiguous(ids) => ids
                .iter()
                .filter_map(|i| self.symbols.get(i))
                .map(|s| s.path.clone())
                .collect(),
            _ => {
                // A module that only exists as files (no `mod` item indexed): match by path prefix.
                let hits: Vec<&Symbol> = self
                    .symbols
                    .values()
                    .filter(|s| s.path.windows(scope.len()).any(|w| w == scope))
                    .collect();
                return (!hits.is_empty()).then_some(hits);
            }
        };
        Some(
            self.symbols
                .values()
                .filter(|s| prefixes.iter().any(|p| s.path.starts_with(p)))
                .collect(),
        )
    }
}

const STOPWORDS: &[&str] = &[
    "the",
    "a",
    "an",
    "and",
    "or",
    "of",
    "to",
    "in",
    "on",
    "for",
    "with",
    "by",
    "is",
    "are",
    "be",
    "must",
    "not",
    "never",
    "shall",
    "should",
    "may",
    "it",
    "its",
    "this",
    "that",
    "these",
    "those",
    "when",
    "if",
    "then",
    "than",
    "at",
    "as",
    "from",
    "into",
    "all",
    "any",
    "each",
    "every",
    "no",
    "more",
    "most",
    "least",
    "only",
    "will",
    "can",
    "cannot",
    "always",
    "has",
    "have",
    "been",
    "which",
    "who",
    "what",
    "there",
    "their",
    "value",
    "values",
    "data",
    "new",
    "get",
    "set",
    "use",
    "used",
    "using",
    "type",
    "types",
    "function",
    "functions",
    "method",
    "methods",
    "returns",
    "return",
    "error",
    "errors",
    "result",
    "number",
    "time",
];

/// Lexical retrieval candidates for a claim's prose: symbols whose names match word
/// n-grams (`rate limiter` -> `RateLimiter` / `rate_limiter`). Retrieval only; edges
/// built from these are `Resolution::Lexical` and never count as evidence.
pub fn lexical_candidates(model: &Model, text: &str, limit: usize) -> Vec<(SymbolId, f32, String)> {
    let words: Vec<String> = text
        .split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .filter(|w| !w.is_empty())
        .map(|w| w.to_string())
        .collect();
    let mut scores: BTreeMap<SymbolId, (f32, String)> = BTreeMap::new();
    let consider = |name: &str, score: f32, scores: &mut BTreeMap<SymbolId, (f32, String)>| {
        for s in model.symbols_named(name) {
            if matches!(
                s.kind,
                SymbolKind::Impl
                    | SymbolKind::Module
                    | SymbolKind::Variant
                    | SymbolKind::Field
                    | SymbolKind::Property
                    | SymbolKind::Namespace
            ) {
                continue;
            }
            let e = scores
                .entry(s.id.clone())
                .or_insert((0.0, name.to_string()));
            if score > e.0 {
                *e = (score, name.to_string());
            }
        }
    };
    let lower: Vec<String> = words.iter().map(|w| w.to_lowercase()).collect();
    for n in (1..=3).rev() {
        for i in 0..lower.len().saturating_sub(n - 1) {
            let gram = &lower[i..i + n];
            if gram
                .iter()
                .any(|w| STOPWORDS.contains(&w.as_str()) || w.chars().all(|c| c.is_ascii_digit()))
            {
                continue;
            }
            let camel: String = gram
                .iter()
                .map(|w| {
                    let mut c = w.chars();
                    c.next()
                        .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
                        .unwrap_or_default()
                })
                .collect();
            let snake = gram.join("_");
            let base = match n {
                3 => 0.4,
                2 => 0.35,
                _ => 0.25,
            };
            consider(&camel, base, &mut scores);
            if n > 1 || snake.len() >= 5 {
                consider(&snake, base - 0.05, &mut scores);
            }
        }
    }
    // Identifiers written verbatim in prose (outside code spans): `RateLimiter`, `max_volume`.
    for w in &words {
        if w.contains('_') || w.chars().skip(1).any(|c| c.is_uppercase()) {
            consider(w, 0.4, &mut scores);
        }
    }
    let mut out: Vec<(SymbolId, f32, String)> =
        scores.into_iter().map(|(id, (s, n))| (id, s, n)).collect();
    out.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.0.cmp(&b.0))
    });
    out.truncate(limit);
    out
}
