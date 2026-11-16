use super::*;
use concordance_core::config::ForbiddenTerm;

fn req(id: &str, statement: &str, extra: &str) -> String {
    let rf = id.to_uppercase();
    format!(
        "{{\"id\":\"{id}\",\"kind\":\"requirement\",\"ref\":\"{rf}\",\"statement\":\"{statement}\",\"evaluate\":{{\"method\":\"example\"}},\"area\":\"01-a\",\"lifecycle\":\"active\",\"milestone\":1{extra}}}"
    )
}

fn table(id: &str, path: &str, body: &str) -> String {
    format!(
        "{{\"id\":\"{id}\",\"path\":\"{path}\",\"body\":\"# T\\n\\n## Cases\\n\\n{body}\",\"columns\":[],\"rowCount\":0}}"
    )
}

fn source(claims: &[String], tables: &[String], rest: &str) -> String {
    format!(
        "{{\n\"exegra\": \"0.1\",\n\"claims\": [\n{}\n],\n\"caseTables\": [\n{}\n]{}\n}}\n",
        claims.join(",\n"),
        tables.join(",\n"),
        rest
    )
}

fn codes(text: &str, rules: &BTreeMap<String, RuleFile>) -> Vec<(String, String)> {
    check_source("spec.jsonc", text, rules)
        .into_iter()
        .map(|f| (f.finding.rule.to_string(), f.finding.message))
        .collect()
}

fn only(rule: &str, text: &str, rules: &BTreeMap<String, RuleFile>) -> Vec<String> {
    codes(text, rules)
        .into_iter()
        .filter(|(r, _)| r == rule)
        .map(|(_, m)| m)
        .collect()
}

fn none() -> BTreeMap<String, RuleFile> {
    BTreeMap::new()
}

fn with(key: &str, f: RuleFile) -> BTreeMap<String, RuleFile> {
    BTreeMap::from([(key.to_string(), f)])
}

#[test]
fn s1_depends_and_statement_citation_on_a_later_milestone() {
    let a = req("a-001", "A SHALL work.", ",\"dependsOn\":[\"b-001\"]");
    let b = req("b-001", "B SHALL work.", "").replace("\"milestone\":1", "\"milestone\":3");
    let c = req("c-001", "C SHALL follow B-001.", "");
    let ok = req("d-001", "D SHALL follow A-001.", "");
    let got = only("S1", &source(&[a, b, c, ok], &[], ""), &none());
    assert_eq!(got.len(), 2, "{got:?}");
    assert!(got[0].contains("A-001") && got[0].contains("depends on B-001"));
    assert!(got[1].contains("C-001") && got[1].contains("cites B-001"));
}

#[test]
fn s1_cited_case_row_of_a_later_milestone() {
    let a = req("a-001", "A SHALL cover `late` rows.", ",\"cases\":[\"t1\"]");
    let t = table(
        "t1",
        "x/01-a/cases/t.md",
        "| Case | Milestone |\\n|---|---|\\n| early | 1 |\\n| late | 4 |",
    );
    let got = only("S1", &source(&[a], &[t], ""), &none());
    assert_eq!(got.len(), 1, "{got:?}");
    assert!(got[0].contains("case row `late`"));
}

#[test]
fn s2_budget_without_a_measurement_row() {
    let mut a = req("a-001", "A SHALL answer within 5 ms.", "");
    a = a.replace("\"example\"", "\"budget\"");
    let b = a.replace("a-001", "b-001").replace("A-001", "B-001");
    let t = table(
        "t1",
        "x/21-v/cases/budget-measurement.md",
        "| Case | Requirement |\\n|---|---|\\n| a-001 | A-001 |",
    );
    let got = only("S2", &source(&[a, b], &[t], ""), &none());
    assert_eq!(got.len(), 1, "{got:?}");
    assert!(got[0].contains("B-001"));
}

#[test]
fn s3_sub_budgets_over_the_parent() {
    let budget =
        |id: &str, st: &str, extra: &str| req(id, st, extra).replace("\"example\"", "\"budget\"");
    let parent = budget(
        "p-001",
        "The move SHALL complete end to end within 2 seconds at the 99th percentile.",
        ",\"dependsOn\":[\"c-001\",\"c-002\"]",
    );
    let c1 = budget(
        "c-001",
        "The hop SHALL take at most 1 second at the 99th percentile.",
        "",
    );
    let c2 = budget(
        "c-002",
        "The write SHALL take at most 1500 ms at the 99th percentile.",
        "",
    );
    let got = only(
        "S3",
        &source(&[parent.clone(), c1.clone(), c2], &[], ""),
        &none(),
    );
    assert_eq!(got.len(), 1, "{got:?}");
    assert!(got[0].contains("2500 ms") && got[0].contains("2000 ms"));
    // A sum within the parent, or a child that cannot be parsed, is silent.
    let c2 = budget(
        "c-002",
        "The write SHALL take at most 500 ms at the 99th percentile.",
        "",
    );
    assert!(only(
        "S3",
        &source(&[parent.clone(), c1.clone(), c2], &[], ""),
        &none()
    )
    .is_empty());
    let c2 = budget(
        "c-002",
        "The write SHALL take at most 1500 ms at the 95th percentile.",
        "",
    );
    assert!(only("S3", &source(&[parent, c1, c2], &[], ""), &none()).is_empty());
}

#[test]
fn s4_constant_valued_in_a_rationale_but_not_in_the_table() {
    let a = req(
        "a-001",
        "A SHALL expire a link after the link expiry.",
        ",\"rationale\":\"The link expiry is 7 days.\"",
    );
    let t = table(
        "t1",
        "x/01-a/cases/a-constants.md",
        "| Case | Constant | Value |\\n|---|---|---|\\n| lag | The projection lag | 60 s |",
    );
    let rules = with(
        "constants",
        RuleFile {
            names: vec!["link expiry".into(), "projection lag".into()],
            ..Default::default()
        },
    );
    let got = only("S4", &source(&[a], &[t], ""), &rules);
    assert_eq!(got.len(), 2, "{got:?}");
    assert!(got[0].contains("rationale") && got[0].contains("link expiry"));
    assert!(got[1].contains("names the constant"));
    // No configured names, no findings.
    let a = req(
        "a-001",
        "A SHALL expire.",
        ",\"rationale\":\"The link expiry is 7 days.\"",
    );
    assert!(only("S4", &source(&[a], &[], ""), &none()).is_empty());
}

#[test]
fn s5_forbidden_phrase_is_reported_in_active_text_only() {
    let a = req("a-001", "A browser session SHALL end.", "");
    let b = req("b-001", "A browser session SHALL end.", "").replace("active", "retired");
    let rules = with(
        "forbidden_terms",
        RuleFile {
            forbidden: vec![ForbiddenTerm {
                phrase: "browser session".into(),
                replacement: "sign-in session".into(),
            }],
            ..Default::default()
        },
    );
    let got = only("S5", &source(&[a, b], &[], ""), &rules);
    assert_eq!(got.len(), 1, "{got:?}");
    assert!(got[0].contains("sign-in session"));
}

#[test]
fn s6_term_defined_twice() {
    let rest = ",\n\"terms\": [\n{\"id\":\"t1\",\"term\":\"Page tier\",\"definition\":\"x\"},\n{\"id\":\"t2\",\"term\":\"Fragment\",\"definition\":\"y\"},\n{\"id\":\"t3\",\"term\":\"page tiers\",\"definition\":\"z\"}\n]";
    let got = only("S6", &source(&[], &[], rest), &none());
    assert_eq!(got.len(), 1, "{got:?}");
    assert!(got[0].contains("page tiers"));
}

#[test]
fn s7_retired_and_missing_ids_in_active_text() {
    let a = req("a-001", "A SHALL follow B-001 and A-009 and A-002.", "");
    let b = req("b-001", "B SHALL work.", "").replace("active", "retired");
    let c = req("a-002", "A2 SHALL work. It replaces B-001.", "");
    let got = only("S7", &source(&[a, b, c], &[], ""), &none());
    assert_eq!(got.len(), 2, "{got:?}");
    assert!(got
        .iter()
        .any(|m| m.contains("B-001") && m.contains("retired")));
    assert!(got
        .iter()
        .any(|m| m.contains("A-009") && m.contains("does not exist")));
}

#[test]
fn s8_near_duplicate_statements_are_a_hint() {
    let a = req(
        "a-001",
        "The page tier SHALL refuse a request whose token has expired with status 401.",
        "",
    );
    let b = req(
        "b-001",
        "The page tier SHALL refuse a request whose token has expired with status 401 and a retry hint.",
        "",
    );
    let c = req(
        "c-001",
        "Hosts SHALL stop a guest call after its deadline passes.",
        "",
    );
    let found = check_source("spec.jsonc", &source(&[a, b, c], &[], ""), &none());
    let s8: Vec<_> = found.iter().filter(|f| f.finding.rule == "S8").collect();
    assert_eq!(s8.len(), 1);
    assert_eq!(s8[0].finding.severity, Severity::Info);
    assert!(s8[0].finding.message.contains("B-001"));
}

#[test]
fn s9_replacement_character_literal_or_escaped() {
    let a = req("a-001", "A SHALL keep caf\u{FFFD}.", "");
    let b = req("b-001", "B SHALL keep caf\\ufffd.", "");
    let got = only("S9", &source(&[a, b], &[], ""), &none());
    assert_eq!(got.len(), 2, "{got:?}");
}

#[test]
fn a_rule_can_be_disabled_and_its_severity_set() {
    let a = req("a-001", "A SHALL keep caf\u{FFFD}.", "");
    let off = RuleFile {
        enabled: Some(false),
        ..Default::default()
    };
    assert!(codes(
        &source(std::slice::from_ref(&a), &[], ""),
        &with("replacement_char", off)
    )
    .is_empty());
    let sev = RuleFile {
        severity: Some("info".into()),
        ..Default::default()
    };
    let got = check_source(
        "s.jsonc",
        &source(&[a], &[], ""),
        &with("replacement_char", sev),
    );
    assert_eq!(got[0].finding.severity, Severity::Info);
}
