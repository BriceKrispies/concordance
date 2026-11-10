//! Recorded test runs: libtest / cargo-nextest output ingested as evidence.
//!
//! Concordance never runs the analyzed repository's tests (that would execute its
//! code); it reads outputs the user or CI produced. Three formats are accepted, mixed
//! freely in one file:
//!
//! * libtest text: `test suite::case ... ok` / `FAILED` / `ignored`
//! * libtest JSON lines: `{"type":"test","event":"ok","name":"suite::case"}`
//! * cargo-nextest: `PASS [ 0.01s] crate::binary suite::case` (also `FAIL`, `SKIP`, ...)
//! * JUnit XML (vitest `--reporter=junit`, most CI reporters): `<testcase name="a > b">`
//!   with `<failure>`/`<error>`/`<skipped>` children
//! * vitest / jest text: `✓ src/x.test.ts > describe > case`, `× ...` / `✗ ...`
//!
//! Names are normalized to `::`-separated segments: `describe > case` -> `describe::case`.
//!
//! A run is evidence about the code as it was when the run was recorded. The engine
//! compares the output file's modification time with the analyzed sources: results that
//! predate a source change are reported as outdated, never as current.

use crate::{ContentHash, RepoPath, Timestamp};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TestOutcome {
    Passed,
    Ignored,
    Failed,
}

impl TestOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            TestOutcome::Passed => "passed",
            TestOutcome::Ignored => "ignored",
            TestOutcome::Failed => "failed",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestRun {
    /// The output file, as configured (absolute).
    pub path: String,
    /// Modification time of the output file: when the run was recorded.
    pub modified: Timestamp,
    pub content_hash: ContentHash,
    /// Test name (`::`-separated, as the harness printed it) -> outcome. A name reported
    /// more than once keeps its worst outcome.
    pub results: BTreeMap<String, TestOutcome>,
    /// Indexed source files modified after the run was recorded.
    pub newer_sources: usize,
    /// One of them, for explanations.
    pub newest_source: Option<RepoPath>,
}

impl TestRun {
    pub fn is_outdated(&self) -> bool {
        self.newer_sources > 0
    }
}

/// Parse any mix of the accepted formats. Unrecognized lines are ignored.
pub fn parse(text: &str) -> BTreeMap<String, TestOutcome> {
    let mut out: BTreeMap<String, TestOutcome> = BTreeMap::new();
    let mut put = |name: String, o: TestOutcome| {
        let e = out.entry(name).or_insert(o);
        *e = (*e).max(o);
    };
    if text.contains("<testcase") {
        for (name, o) in parse_junit(text) {
            put(name, o);
        }
    }
    for line in text.lines() {
        let l = line.trim();
        if let Some((name, o)) = parse_json(l)
            .or_else(|| parse_libtest(l))
            .or_else(|| parse_nextest(l))
            .or_else(|| parse_vitest(l))
        {
            put(name, o);
        }
    }
    out
}

fn unescape(s: &str) -> String {
    s.replace("&gt;", ">")
        .replace("&lt;", "<")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

fn attr(tag: &str, name: &str) -> Option<String> {
    let key = format!(" {name}=");
    let i = tag.find(&key)? + key.len();
    let q = tag[i..].chars().next()?;
    let rest = &tag[i + 1..];
    let end = rest.find(q)?;
    Some(unescape(&rest[..end]))
}

/// `describe > case` (vitest, mocha) -> `describe::case`.
fn segments(name: &str) -> String {
    name.split(" > ")
        .map(str::trim)
        .collect::<Vec<_>>()
        .join("::")
}

fn parse_junit(text: &str) -> Vec<(String, TestOutcome)> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(i) = rest.find("<testcase") {
        rest = &rest[i..];
        let Some(tag_end) = rest.find('>') else { break };
        let tag = &rest[..tag_end];
        let body = if tag.ends_with('/') {
            ""
        } else {
            let close = rest.find("</testcase>").unwrap_or(rest.len());
            &rest[tag_end..close]
        };
        if let Some(name) = attr(tag, "name") {
            let o = if body.contains("<failure") || body.contains("<error") {
                TestOutcome::Failed
            } else if body.contains("<skipped") {
                TestOutcome::Ignored
            } else {
                TestOutcome::Passed
            };
            out.push((segments(&name), o));
        }
        rest = &rest[tag_end..];
    }
    out
}

fn parse_vitest(l: &str) -> Option<(String, TestOutcome)> {
    let mut chars = l.chars();
    let o = match chars.next()? {
        '\u{2713}' | '\u{221a}' => TestOutcome::Passed,
        '\u{d7}' | '\u{2717}' | '\u{2715}' => TestOutcome::Failed,
        '\u{2193}' => TestOutcome::Ignored,
        _ => return None,
    };
    let rest = chars.as_str().trim();
    // Drop a trailing duration (`12ms`, `1.2s`).
    let rest = match rest.rsplit_once(' ') {
        Some((head, tail))
            if tail.ends_with("ms")
                || (tail.ends_with('s') && tail[..tail.len() - 1].parse::<f64>().is_ok()) =>
        {
            head
        }
        _ => rest,
    };
    let parts: Vec<&str> = rest.split(" > ").collect();
    // The first part is the test file when it looks like a path.
    let parts = if parts.len() > 1 && parts[0].contains('.') && !parts[0].contains(' ') {
        &parts[1..]
    } else {
        &parts[..]
    };
    (!parts.is_empty()).then(|| {
        (
            parts
                .iter()
                .map(|p| p.trim())
                .collect::<Vec<_>>()
                .join("::"),
            o,
        )
    })
}

fn parse_libtest(l: &str) -> Option<(String, TestOutcome)> {
    let rest = l.strip_prefix("test ")?;
    let (name, status) = rest.rsplit_once(" ... ")?;
    let status = status.trim();
    let o = if status == "ok" {
        TestOutcome::Passed
    } else if status == "FAILED" {
        TestOutcome::Failed
    } else if status.starts_with("ignored") {
        TestOutcome::Ignored
    } else {
        return None; // bench lines and anything else
    };
    let name = name.trim();
    (!name.is_empty() && !name.contains(' ')).then(|| (name.to_string(), o))
}

fn parse_json(l: &str) -> Option<(String, TestOutcome)> {
    if !l.starts_with('{') {
        return None;
    }
    let v: serde_json::Value = serde_json::from_str(l).ok()?;
    if v.get("type")?.as_str()? != "test" {
        return None;
    }
    let o = match v.get("event")?.as_str()? {
        "ok" => TestOutcome::Passed,
        "failed" | "timeout" => TestOutcome::Failed,
        "ignored" => TestOutcome::Ignored,
        _ => return None,
    };
    Some((v.get("name")?.as_str()?.to_string(), o))
}

fn parse_nextest(l: &str) -> Option<(String, TestOutcome)> {
    let (status, rest) = l.split_once(char::is_whitespace)?;
    let o = match status {
        "PASS" | "FLAKY" => TestOutcome::Passed,
        "FAIL" | "SIGSEGV" | "SIGABRT" | "TIMEOUT" | "LEAK-FAIL" => TestOutcome::Failed,
        "SKIP" => TestOutcome::Ignored,
        _ => return None,
    };
    let rest = rest.trim_start().strip_prefix('[')?;
    let (_, rest) = rest.split_once(']')?;
    let mut parts = rest.split_whitespace();
    let binary = parts.next()?;
    let name = parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    // `crate::binary` -> prefix the binary (integration-test target) so the name keeps
    // the module path that appears in the source tree.
    let full = match binary.split_once("::") {
        Some((_, bin)) => format!("{bin}::{name}"),
        None => name.to_string(),
    };
    Some((full, o))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_all_three_formats_and_keeps_the_worst_outcome() {
        let text = "\
running 3 tests
test lexing::nested_comments ... ok
test lexing::unterminated ... FAILED
test slow::case ... ignored, needs gcc
test bench_x ... bench:  1,234 ns/iter (+/- 5)
{ \"type\": \"test\", \"event\": \"started\", \"name\": \"j::a\" }
{ \"type\": \"test\", \"event\": \"ok\", \"name\": \"j::a\" }
{ \"type\": \"test\", \"event\": \"failed\", \"name\": \"j::b\" }
        PASS [   0.004s] demo::suite_1a lexical::nests
        FAIL [   0.010s] demo lexing::nested_comments
test result: FAILED. 1 passed; 1 failed
";
        let r = parse(text);
        assert_eq!(
            r.get("lexing::nested_comments"),
            Some(&TestOutcome::Failed),
            "worst of ok + FAIL"
        );
        assert_eq!(r.get("lexing::unterminated"), Some(&TestOutcome::Failed));
        assert_eq!(r.get("slow::case"), Some(&TestOutcome::Ignored));
        assert_eq!(r.get("j::a"), Some(&TestOutcome::Passed));
        assert_eq!(r.get("j::b"), Some(&TestOutcome::Failed));
        assert_eq!(
            r.get("suite_1a::lexical::nests"),
            Some(&TestOutcome::Passed)
        );
        assert!(!r.contains_key("bench_x"));
        assert_eq!(r.len(), 6);
    }

    #[test]
    fn parses_junit_and_vitest_text() {
        let junit = r#"<?xml version="1.0"?>
<testsuites><testsuite name="src/mixer.test.ts">
  <testcase classname="src/mixer.test.ts" name="Mixer &gt; rejects levels above 100" time="0.002"></testcase>
  <testcase classname="src/mixer.test.ts" name="Mixer &gt; clamps" time="0.001">
    <failure message="expected 100">AssertionError</failure>
  </testcase>
  <testcase classname="src/mixer.test.ts" name="skipped one"><skipped/></testcase>
  <testcase classname="x" name="self closing" />
</testsuite></testsuites>"#;
        let r = parse(junit);
        assert_eq!(
            r.get("Mixer::rejects levels above 100"),
            Some(&TestOutcome::Passed)
        );
        assert_eq!(r.get("Mixer::clamps"), Some(&TestOutcome::Failed));
        assert_eq!(r.get("skipped one"), Some(&TestOutcome::Ignored));
        assert_eq!(r.get("self closing"), Some(&TestOutcome::Passed));

        let text = " \u{2713} src/mixer.test.ts > Mixer > rejects levels above 100 3ms\n \u{d7} src/mixer.test.ts > Mixer > clamps 1ms\n";
        let r = parse(text);
        assert_eq!(
            r.get("Mixer::rejects levels above 100"),
            Some(&TestOutcome::Passed)
        );
        assert_eq!(r.get("Mixer::clamps"), Some(&TestOutcome::Failed));
    }
}
