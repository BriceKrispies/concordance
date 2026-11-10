//! `lake build` output -> per-theorem outcomes.
//!
//! Lake reports per-module progress (`✔ [3/5] Built Volume.Mixer`, `✖ [4/5] Building ...`)
//! and compiler diagnostics (`warning: ./Volume/Mixer.lean:12:8: declaration uses 'sorry'`,
//! `error: ./Volume/Mixer.lean:20:2: ...`). A test (a tagged theorem) then
//!
//! * fails when an error diagnostic or a `sorry` warning lies inside its line range,
//! * fails when its module failed to build (a `✖` line, or any error in the file),
//! * passes when its module was reported built, or the log says the whole build
//!   completed successfully,
//! * has no outcome otherwise (the module is not mentioned: no evidence).

use concordance_core::testrun::{TestOutcome, TestSite};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Default, PartialEq, Eq)]
pub struct BuildLog {
    /// Dotted module names reported built (`Volume.Mixer`).
    pub built: BTreeSet<String>,
    /// Dotted module names whose build failed.
    pub failed: BTreeSet<String>,
    /// `(file, line)` of error diagnostics; file normalized (`Volume/Mixer.lean`).
    pub errors: Vec<(String, u32)>,
    /// `(file, line)` of `declaration uses 'sorry'` warnings.
    pub sorries: Vec<(String, u32)>,
    /// `Build completed successfully` appeared.
    pub completed: bool,
}

fn norm_file(p: &str) -> String {
    let p = p.trim().replace('\\', "/");
    p.trim_start_matches("./").to_string()
}

/// `(file, line, rest)` of a `path.lean:LINE:COL: rest` diagnostic body.
fn location(s: &str) -> Option<(String, u32, &str)> {
    let i = s.find(".lean:")?;
    let file = norm_file(&s[..i + 5]);
    let rest = &s[i + 6..];
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    let line: u32 = digits.parse().ok()?;
    let after = &rest[digits.len()..];
    let msg = after
        .trim_start_matches(|c: char| c == ':' || c.is_ascii_digit())
        .trim_start();
    Some((file, line, msg))
}

/// The module in a progress line, if it is one: `✔ [3/5] Built Volume.Mixer`.
fn progress(line: &str) -> Option<(char, String)> {
    let mut chars = line.chars();
    let first = chars.next()?;
    let (mark, rest) = match first {
        '\u{2714}' => ('b', chars.as_str()),
        '\u{26a0}' => ('w', chars.as_str()),
        '\u{2716}' => ('f', chars.as_str()),
        _ => ('n', line),
    };
    let rest = rest.trim_start();
    let rest = match rest.strip_prefix('[') {
        Some(r) => r.split_once(']')?.1.trim_start(),
        None => rest,
    };
    let (verb, module) = rest.split_once(char::is_whitespace)?;
    let module = module.trim();
    let module = module.split([':', ' ']).next().unwrap_or(module);
    if module.is_empty()
        || !module
            .chars()
            .next()
            .is_some_and(|c| c.is_alphabetic() || c == '«')
    {
        return None;
    }
    let state = match (mark, verb) {
        ('f', "Built" | "Building" | "Compiling" | "Replayed") => 'f',
        ('b' | 'w', "Built" | "Replayed") => 'b',
        ('n', "Built" | "Replayed") => 'b',
        _ => return None,
    };
    Some((state, module.replace(['«', '»'], "")))
}

pub fn parse(log: &str) -> BuildLog {
    let mut out = BuildLog::default();
    for raw in log.lines() {
        let line = raw.trim();
        if line.contains("Build completed successfully") {
            out.completed = true;
        }
        let diag = line
            .strip_prefix("error: ")
            .map(|r| (true, r))
            .or_else(|| line.strip_prefix("warning: ").map(|r| (false, r)));
        if let Some((is_error, rest)) = diag {
            if let Some((file, ln, msg)) = location(rest) {
                if is_error {
                    out.errors.push((file, ln));
                } else if msg.contains("declaration uses 'sorry'")
                    || msg.contains("declaration uses `sorry`")
                {
                    out.sorries.push((file, ln));
                }
            }
            continue;
        }
        // `A/B.lean:12:8: error: ...` straight from the compiler.
        if let Some((file, ln, msg)) = location(line) {
            if line.starts_with(&file) || line.starts_with("./") {
                if msg.starts_with("error") {
                    out.errors.push((file, ln));
                } else if msg.contains("declaration uses 'sorry'") {
                    out.sorries.push((file, ln));
                }
                continue;
            }
        }
        if let Some((state, module)) = progress(line) {
            if state == 'f' {
                out.failed.insert(module);
            } else {
                out.built.insert(module);
            }
        }
    }
    out
}

/// `Volume/Mixer.lean` -> `Volume.Mixer`.
fn module_of(path: &str) -> String {
    path.trim_end_matches(".lean").replace('/', ".")
}

/// Whether the repository path `repo` (`pkg/Volume/Mixer.lean`) is the log's `file`.
fn same_file(repo: &str, file: &str) -> bool {
    repo == file || repo.ends_with(&format!("/{file}")) || file.ends_with(&format!("/{repo}"))
}

/// Whether the log's dotted `module` names the module of `repo`.
fn same_module(repo: &str, module: &str) -> bool {
    let m = module_of(repo);
    m == module || m.ends_with(&format!(".{module}"))
}

pub fn outcomes(log: &str, tests: &[TestSite]) -> BTreeMap<String, TestOutcome> {
    let b = parse(log);
    let recognized = b.completed
        || !b.built.is_empty()
        || !b.failed.is_empty()
        || !b.errors.is_empty()
        || !b.sorries.is_empty();
    let mut out = BTreeMap::new();
    if !recognized {
        return out;
    }
    for t in tests {
        let repo = t.file.as_str();
        let (lo, hi) = (t.range.start_line, t.range.end_line);
        let inside = |v: &[(String, u32)]| {
            v.iter()
                .any(|(f, l)| same_file(repo, f) && (lo..=hi).contains(l))
        };
        let module_failed = b.failed.iter().any(|m| same_module(repo, m))
            || b.errors.iter().any(|(f, _)| same_file(repo, f));
        let o = if inside(&b.sorries) || inside(&b.errors) || module_failed {
            TestOutcome::Failed
        } else if b.completed || b.built.iter().any(|m| same_module(repo, m)) {
            TestOutcome::Passed
        } else {
            continue;
        };
        let e = out.entry(t.name.clone()).or_insert(o);
        *e = (*e).max(o);
    }
    out
}
