//! `lake build` output -> per-theorem outcomes.
//!
//! Lake reports per-module progress (`✔ [3/5] Built Volume.Mixer`, `✖ [4/5] Building ...`)
//! and compiler diagnostics (`warning: ./Volume/Mixer.lean:12:8: declaration uses 'sorry'`,
//! `error: ./Volume/Mixer.lean:20:2: ...`). A test (a tagged theorem) then
//!
//! * fails when an error diagnostic or a `sorry` warning lies inside its line range,
//! * fails when its module failed to build (a `✖` line, or any error in the file),
//! * passes only when the log shows its own module built (`Built Volume.Mixer`, the module
//!   named in full, so a module of the same leaf name in another package does not match),
//! * has no outcome otherwise: a log that merely says the build completed, or never mentions
//!   the module, is no evidence about it (Lake prints only the modules it built, so build from
//!   clean or with `-R` when the log must cover every module).

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
        // `ℹ [4/13] Replayed Volume.Mixer`: Lake's mark for a module taken from its cache.
        '\u{2139}' => ('n', chars.as_str()),
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

/// Whether the log's dotted `module` is the module of the test's file. The module is the
/// file's path inside its package (`Volume.Mixer`); with no package root known, the repository
/// path stands in for it.
fn same_module(site: &TestSite, module: &str) -> bool {
    let named = site.module.join(".");
    if named.is_empty() {
        module_of(site.file.as_str()) == module
    } else {
        named == module
    }
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
        let module_failed = b.failed.iter().any(|m| same_module(t, m))
            || b.errors.iter().any(|(f, _)| same_file(repo, f));
        let o = if inside(&b.sorries) || inside(&b.errors) || module_failed {
            TestOutcome::Failed
        } else if b.built.iter().any(|m| same_module(t, m)) {
            TestOutcome::Passed
        } else {
            continue;
        };
        let e = out.entry(t.name.clone()).or_insert(o);
        *e = (*e).max(o);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use concordance_core::{RepoPath, SourceRange};

    fn site(module: &[&str]) -> TestSite {
        let file = format!("{}.lean", module.join("/"));
        let path = RepoPath::from_relative_str(&file).unwrap();
        TestSite {
            name: "pkg::Volume::clamp_le_max".into(),
            range: SourceRange::lines(path.clone(), 10, 14),
            file: path,
            module: module.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn a_completed_build_says_nothing_about_a_module_it_did_not_report() {
        let log = "✔ [1/2] Built Volume\nBuild completed successfully.\n";
        let got = outcomes(log, &[site(&["Volume", "Mixer"])]);
        assert!(got.is_empty(), "{got:?}");
    }

    #[test]
    fn the_module_must_be_named_in_full() {
        // `Mixer` of another package is not `Volume.Mixer`.
        let log = "✔ [1/1] Built Mixer\nBuild completed successfully.\n";
        let got = outcomes(log, &[site(&["Volume", "Mixer"])]);
        assert!(got.is_empty(), "{got:?}");
    }

    #[test]
    fn a_module_replayed_from_the_cache_counts_as_built() {
        let log = "\u{2139} [4/13] Replayed Volume.Mixer\n\u{2714} [5/13] Ran Volume.Mixer:irArts\nBuild completed successfully (13 jobs).\n";
        let b = parse(log);
        assert!(b.built.contains("Volume.Mixer"));
        assert_eq!(b.built.len(), 1);
    }

    #[test]
    fn a_reported_build_of_its_own_module_passes() {
        let log = "✔ [1/1] Built Volume.Mixer\nBuild completed successfully.\n";
        let got = outcomes(log, &[site(&["Volume", "Mixer"])]);
        assert_eq!(
            got.get("pkg::Volume::clamp_le_max"),
            Some(&TestOutcome::Passed)
        );
    }
}
