//! `cargo run --release -p concordance-exegra --example compare -- <spec.jsonc> <rendered.md>`
//!
//! Indexes an Exegra specification and the Markdown rendered from it (for example with
//! `import-register.js --render`), then compares the claims: counts by kind, what the
//! JSONC adds (authored entities, rows that define requirements), and whether every
//! location lies inside the `.jsonc`.

use concordance_core::doc::{Authority, AuthorityAssessment, AuthoritySource, Claim};
use concordance_core::lang::Languages;
use concordance_core::RepoPath;
use std::collections::BTreeMap;
use std::time::Instant;

fn by_kind<'a>(claims: impl Iterator<Item = &'a Claim>) -> BTreeMap<String, usize> {
    let mut m = BTreeMap::new();
    for c in claims {
        *m.entry(format!("{:?}", c.kind)).or_insert(0) += 1;
    }
    m
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() != 2 {
        eprintln!("usage: compare <spec.jsonc> <rendered.md>");
        std::process::exit(2);
    }
    let jsonc = std::fs::read_to_string(&args[0]).expect("read jsonc");
    let md = std::fs::read_to_string(&args[1]).expect("read md");
    let langs = Languages::new(vec![]);
    let normative = AuthorityAssessment {
        level: Authority::Normative,
        source: AuthoritySource::Configured,
        rationale: "compare".into(),
    };

    let t = Instant::now();
    let rendered = concordance_exegra::render_markdown(&jsonc).expect("render");
    println!(
        "render: {:?}, byte-identical to {}: {}",
        t.elapsed(),
        args[1],
        rendered == md
    );

    let t = Instant::now();
    let path = RepoPath::from_relative_str("system/spec.jsonc").unwrap();
    let x = concordance_exegra::index_exegra(
        &path,
        &jsonc,
        Some(normative.clone()),
        &langs,
        &Default::default(),
    )
    .expect("index jsonc");
    let tx = t.elapsed();
    let t = Instant::now();
    let mdp = RepoPath::from_relative_str("system/spec.md").unwrap();
    let m = concordance_markdown::index_document(&mdp, &md, Some(normative), &langs);
    println!("index: jsonc {tx:?}, markdown {:?}", t.elapsed());

    // Authored sections are the top-level headings the rendered Markdown does not have.
    let md_top: std::collections::HashSet<_> = m
        .sections
        .iter()
        .filter(|s| s.heading_path.len() == 1)
        .map(|s| s.heading.clone())
        .collect();
    let authored = |c: &Claim| !md_top.contains(&c.heading_path[0]);
    let md_keys: std::collections::HashSet<_> =
        m.claims.iter().map(|c| (c.heading_path.clone(), c.text.clone(), c.kind)).collect();
    let converted: Vec<&Claim> = x.claims.iter().filter(|c| !authored(c)).collect();
    let same: Vec<&Claim> = converted
        .iter()
        .copied()
        .filter(|c| md_keys.contains(&(c.heading_path.clone(), c.text.clone(), c.kind)))
        .collect();
    let added_rows: Vec<&Claim> = converted
        .iter()
        .copied()
        .filter(|c| !md_keys.contains(&(c.heading_path.clone(), c.text.clone(), c.kind)))
        .collect();
    println!(
        "markdown: {} claims {:?}",
        m.claims.len(),
        by_kind(m.claims.iter())
    );
    println!(
        "jsonc converted: {} claims, {} identical to markdown claims, {} added by rows that define requirements {:?}",
        converted.len(),
        same.len(),
        added_rows.len(),
        by_kind(added_rows.iter().copied())
    );
    let missing = m.claims.len() - same.len();
    println!("markdown claims missing from the jsonc: {missing}");
    let a: Vec<&Claim> = x.claims.iter().filter(|c| authored(c)).collect();
    println!(
        "jsonc authored: {} claims {:?}, {} obligations, {} define a requirement",
        a.len(),
        by_kind(a.iter().copied()),
        a.iter().filter(|c| c.is_obligation()).count(),
        a.iter().filter(|c| c.requirement.is_some()).count()
    );
    let reqs: std::collections::BTreeSet<_> = x
        .claims
        .iter()
        .filter_map(|c| c.requirement.as_ref().map(|r| r.id.clone()))
        .collect();
    let md_reqs = m.claims.iter().filter(|c| c.requirement.is_some()).count();
    println!(
        "requirement ids defined: jsonc {} (markdown {md_reqs} tagged claims)",
        reqs.len()
    );

    let lines = x.document.line_count;
    let outside = x
        .claims
        .iter()
        .flat_map(|c| {
            std::iter::once(&c.range).chain(c.references.iter().filter_map(|r| r.range.as_ref()))
        })
        .chain(x.sections.iter().map(|s| &s.range))
        .chain(
            x.prose_references
                .iter()
                .filter_map(|p| p.reference.range.as_ref()),
        )
        .filter(|r| {
            r.path != path || r.start_line < 1 || r.end_line > lines || r.end_line < r.start_line
        })
        .count();
    println!(
        "jsonc: {lines} lines, {} sections, {} prose references; ranges outside the file: {outside}",
        x.sections.len(),
        x.prose_references.len()
    );
    let src: Vec<&str> = jsonc.lines().collect();
    for id in ["TEN-1", "B-5", "CAP-01", "PII-T1", "AI-R3", "FR-AGT-1"] {
        match x
            .claims
            .iter()
            .find(|c| c.requirement.as_ref().is_some_and(|r| r.id == id))
        {
            Some(c) => {
                let l = src[c.range.start_line as usize - 1];
                println!(
                    "  {id}: line {}-{} {:?} {:?}: {}",
                    c.range.start_line,
                    c.range.end_line,
                    c.kind,
                    c.modality,
                    l.trim().chars().take(70).collect::<String>()
                );
            }
            None => println!("  {id}: not defined"),
        }
    }
}
