//! `--exegra-results`: the per-requirement results of a scan, in the file Exegra reads
//! (see `docs/cli.md`).
//!
//! `claims` has an entry only for a requirement whose status is `verified` (`pass`) or
//! `violated` (`fail`); every other status is left out, because absence means "not established".
//! The status and the proofs are the ones reconciliation derived (`concordance-reconcile`), not
//! re-derived here. Keys are sorted and the only time in the file is `generatedAt`, which can be
//! left out, so the same state gives the same bytes.

use concordance_core::config::Config;
use concordance_core::doc::RequirementForm;
use concordance_core::evidence::{Evidence, Method};
use concordance_core::finding::{Finding, FindingStatus};
use concordance_core::EvidenceId;
use concordance_engine::virtuals::VirtualSet;
use concordance_graph::Model;
use std::collections::{BTreeMap, BTreeSet};

/// The result for one requirement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClaimResult {
    /// `pass` or `fail`.
    pub outcome: &'static str,
    /// The Concordance status: `verified` or `violated`.
    pub status: &'static str,
    pub detail: String,
    pub proofs: Vec<String>,
}

/// One lint finding (or source problem) located in the source file.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct LintEntry {
    pub line: u32,
    pub rule: String,
    pub message: String,
    pub claim: Option<String>,
    pub entity: Option<String>,
    pub severity: &'static str,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Results {
    /// The structured source file(s), comma separated.
    pub source: String,
    /// By the id of the claim entity the requirement was rendered from.
    pub claims: BTreeMap<String, ClaimResult>,
    pub lint: Vec<LintEntry>,
}

/// The first sentence of `s`, on one line, ending in a full stop.
fn sentence(s: &str) -> String {
    let one: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    let cut = one.find(". ").map(|i| i + 1).unwrap_or(one.len());
    let t = one[..cut].trim_end();
    if t.ends_with('.') {
        t.to_string()
    } else {
        format!("{t}.")
    }
}

fn valid_evidence<'a>(
    model: &'a Model,
    ids: &'a [EvidenceId],
) -> impl Iterator<Item = &'a Evidence> {
    ids.iter()
        .filter_map(|i| model.evidence.get(i))
        .filter(|e| e.is_valid())
}

/// `verified` -> `pass`, `violated` -> `fail`; anything else is not established.
fn result_of(model: &Model, f: &Finding) -> Option<ClaimResult> {
    let (outcome, status, ids, wanted) = match f.status {
        FindingStatus::Verified => ("pass", "verified", &f.supporting, "passed"),
        FindingStatus::Violated => ("fail", "violated", &f.contradicting, "failed"),
        _ => return None,
    };
    let mut proofs: Vec<String> = valid_evidence(model, ids)
        .filter_map(|e| match &e.method {
            Method::Test {
                test, outcome: o, ..
            } if o.as_str() == wanted => Some(test.clone()),
            _ => None,
        })
        .collect();
    proofs.sort();
    proofs.dedup();
    let detail = if proofs.is_empty() {
        match valid_evidence(model, ids).max_by_key(|e| e.strength) {
            Some(e) => sentence(&e.summary),
            None => sentence(&f.title),
        }
    } else {
        let names = proofs
            .iter()
            .map(|p| format!("`{p}`"))
            .collect::<Vec<_>>()
            .join(", ");
        let many = if proofs.len() == 1 { "" } else { "s" };
        if outcome == "pass" {
            format!("Proved by passing test{many} {names}.")
        } else {
            format!("Contradicted by failing test{many} {names}.")
        }
    };
    Some(ClaimResult {
        outcome,
        status,
        detail,
        proofs,
    })
}

/// The results for the structured sources in `set`, from the scanned `model` (whose virtual
/// documents are those of `set`).
pub fn build(model: &Model, set: &VirtualSet, config: &Config) -> Results {
    let mut out = Results {
        source: set
            .sources
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join(", "),
        ..Default::default()
    };
    let claim_entities: BTreeSet<&str> = set.requirements.values().map(String::as_str).collect();
    for (requirement, entity) in &set.requirements {
        let mut claims = model.claims_of_requirement(requirement).filter(|c| {
            c.requirement
                .as_ref()
                .is_some_and(|r| r.form == RequirementForm::Inline)
        });
        let (Some(claim), None) = (claims.next(), claims.next()) else {
            continue; // not defined, or defined twice: not established
        };
        if !model.virtual_docs.contains_key(&claim.range.path) {
            continue; // defined by a Markdown file, not by the source
        }
        let Some(finding) = model.claim_finding(&claim.id).filter(|f| f.is_open()) else {
            continue;
        };
        if let Some(r) = result_of(model, finding) {
            out.claims.insert(entity.clone(), r);
        }
    }

    let report = concordance_lint::lint_repo(model, config, false);
    for f in &report.findings {
        let Some(origin) = &f.origin else { continue };
        out.lint.push(LintEntry {
            line: origin.line,
            rule: f.rule.to_string(),
            message: f.message.clone(),
            claim: claim_entities
                .contains(origin.entity.as_str())
                .then(|| origin.entity.clone()),
            entity: Some(origin.entity.clone()),
            severity: f.severity.as_str(),
        });
    }
    for d in &model.virtual_diagnostics {
        out.lint.push(LintEntry {
            line: d.line,
            rule: "R0".to_string(),
            message: d.message.clone(),
            claim: claim_entities
                .contains(d.entity.as_str())
                .then(|| d.entity.clone()),
            entity: (!d.entity.is_empty()).then(|| d.entity.clone()),
            severity: if d.error { "error" } else { "warning" },
        });
    }
    out.lint.sort();
    out.lint.dedup();
    out
}

fn js(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".to_string())
}

fn opt(s: &Option<String>) -> String {
    s.as_deref().map_or("null".to_string(), js)
}

/// `secs` since the Unix epoch as `YYYY-MM-DDTHH:MM:SSZ`.
pub fn rfc3339(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    // Days since 1970-01-01 to a civil date (proleptic Gregorian).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3_600,
        rem % 3_600 / 60,
        rem % 60
    )
}

/// The file's text: sorted keys, two-space indent, one trailing newline. `generated_at` is the
/// only time in it (`None` leaves it out).
pub fn render(r: &Results, version: &str, generated_at: Option<&str>) -> String {
    let mut o = String::from("{\n  \"claims\": ");
    if r.claims.is_empty() {
        o.push_str("{}");
    } else {
        o.push_str("{\n");
        let items: Vec<String> = r
            .claims
            .iter()
            .map(|(id, c)| {
                let proofs = c.proofs.iter().map(|p| js(p)).collect::<Vec<_>>().join(", ");
                format!(
                    "    {}: {{\n      \"detail\": {},\n      \"outcome\": {},\n      \"proofs\": [{}],\n      \"status\": {}\n    }}",
                    js(id),
                    js(&c.detail),
                    js(c.outcome),
                    proofs,
                    js(c.status)
                )
            })
            .collect();
        o.push_str(&items.join(",\n"));
        o.push_str("\n  }");
    }
    o.push_str(",\n");
    if let Some(t) = generated_at {
        o.push_str(&format!("  \"generatedAt\": {},\n", js(t)));
    }
    o.push_str("  \"lint\": ");
    if r.lint.is_empty() {
        o.push_str("[]");
    } else {
        o.push_str("[\n");
        let items: Vec<String> = r
            .lint
            .iter()
            .map(|l| {
                format!(
                    "    {{\"claim\": {}, \"entity\": {}, \"line\": {}, \"message\": {}, \"rule\": {}, \"severity\": {}}}",
                    opt(&l.claim),
                    opt(&l.entity),
                    l.line,
                    js(&l.message),
                    js(&l.rule),
                    js(l.severity)
                )
            })
            .collect();
        o.push_str(&items.join(",\n"));
        o.push_str("\n  ]");
    }
    o.push_str(&format!(
        ",\n  \"source\": {},\n  \"tool\": \"concordance\",\n  \"version\": {}\n}}\n",
        js(&r.source),
        js(version)
    ));
    o
}
