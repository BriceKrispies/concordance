//! Call resolution and proof reach over indexed Rust source (WG-4819).

use crate::link::symbol_edges;
use crate::verification::{lean_statement, proof_flaws, proof_reach};
use concordance_core::code::{CrateInfo, Symbol, SymbolKind, WorkspaceInfo};
use concordance_core::graph::{NodeRef, Resolution};
use concordance_core::{RepoPath, SymbolId};
use concordance_graph::Model;

const SUBSTRATE_SYNTAX: &str = r#"
pub struct Parser;

impl Parser {
    fn next(&mut self) -> Option<u8> {
        None
    }
}
"#;

const SIMULATION_PROJECT: &str = r#"
pub struct Scenario;
pub struct Proposal;

pub fn project(_scenario: &Scenario, _proposal: &Proposal, _keep: &mut dyn FnMut(u8, u8, u8) -> bool) {}

fn hidden_projection() {}
"#;

/// Integration test of the simulation crate. It imports `project` by a braced `use` list and
/// calls `.next()` on a std iterator. It does not import the substrate crate.
const SIMULATION_BOUNDARY_TEST: &str = r#"
use std::collections::HashMap;
use strata_simulation::project::{project, Proposal, Scenario};

/// Verifies: SIM-001
#[test]
fn projection_boundary() {
    let map: HashMap<u8, u8> = HashMap::new();
    let _first = map.values().next();
    let scenario = Scenario;
    let proposal = Proposal;
    project(&scenario, &proposal, &mut |_, _, _| true);
}
"#;

/// A proof that reaches a private function through an explicit path.
const SIMULATION_PRIVATE_PATH_TEST: &str = r#"
/// Verifies: SIM-002
#[test]
fn bypasses_boundary() {
    strata_simulation::project::hidden_projection();
}
"#;

fn krate(name: &str, dir: &str) -> CrateInfo {
    CrateInfo {
        name: name.into(),
        language: "rust".into(),
        manifest: RepoPath::from_relative_str(&format!("{dir}/Cargo.toml")).unwrap(),
        root_dir: Some(RepoPath::from_relative_str(dir).unwrap()),
        targets: vec![],
        dependencies: vec![],
        features: vec![],
    }
}

/// A model holding the indexed files, their imports, and the call edges of every symbol.
fn model_of(files: &[(&str, &str)]) -> Model {
    let workspace = WorkspaceInfo {
        crates: vec![
            krate("strata_simulation", "crates/strata-simulation"),
            krate("strata_substrate", "crates/strata-substrate"),
        ],
        ..WorkspaceInfo::default()
    };
    let mut model = Model::default();
    model.workspace = workspace.clone();
    for (path, text) in files {
        let repo_path = RepoPath::from_relative_str(path).unwrap();
        let idx = concordance_rust::index_source(&repo_path, text, &workspace);
        let file_id = idx.file.id.clone();
        model.upsert_file(idx.file);
        model.set_imports(&file_id, idx.imports);
        for sym in idx.symbols {
            model.upsert_symbol(sym);
        }
    }
    let symbols: Vec<Symbol> = model.symbols.values().cloned().collect();
    for sym in &symbols {
        for edge in symbol_edges(&model, sym) {
            model.upsert_edge(edge);
        }
    }
    model
}

fn standard_model() -> Model {
    model_of(&[
        ("crates/strata-substrate/src/syntax.rs", SUBSTRATE_SYNTAX),
        (
            "crates/strata-simulation/src/project.rs",
            SIMULATION_PROJECT,
        ),
        (
            "crates/strata-simulation/tests/boundary.rs",
            SIMULATION_BOUNDARY_TEST,
        ),
        (
            "crates/strata-simulation/tests/private_path.rs",
            SIMULATION_PRIVATE_PATH_TEST,
        ),
    ])
}

fn named<'a>(model: &'a Model, name: &str, kind: SymbolKind) -> &'a Symbol {
    model
        .symbols
        .values()
        .find(|s| s.name == name && s.kind == kind)
        .unwrap_or_else(|| panic!("no {kind:?} named `{name}` in the model"))
}

#[test]
fn method_call_matched_by_name_is_not_proof_reach() {
    let model = standard_model();
    let proof = named(&model, "projection_boundary", SymbolKind::Function);
    let parser_next = named(&model, "next", SymbolKind::Method);

    // `map.values().next()` matches the unrelated private `Parser::next` by name only. A private
    // item of another crate is never a call target, so no edge of the proof leads to it.
    assert!(model
        .edges_from(&NodeRef::Symbol(proof.id.clone()))
        .all(|e| e.to != Some(NodeRef::Symbol(parser_next.id.clone()))));

    // It is not counted as an internal the proof reaches; the public `project` is.
    let reach = proof_reach(&model, proof);
    assert!(reach.reaches_public);
    assert!(!reach.internals.contains(&parser_next.id));
}

#[test]
fn imported_function_call_resolves_to_the_public_function() {
    let model = standard_model();
    let proof = named(&model, "projection_boundary", SymbolKind::Function);
    let project_fn = named(&model, "project", SymbolKind::Function);

    let project_edge = model
        .edges_from(&NodeRef::Symbol(proof.id.clone()))
        .find(|e| e.target_text == "project")
        .expect("the imported `project(..)` call has an edge");
    assert_eq!(project_edge.resolution, Resolution::Exact);
    assert_eq!(
        project_edge.to,
        Some(NodeRef::Symbol(project_fn.id.clone()))
    );
    assert!(project_fn.is_public_api);
}

#[test]
fn explicit_path_to_private_function_still_bypasses_the_boundary() {
    let model = standard_model();
    let proof = named(&model, "bypasses_boundary", SymbolKind::Function);
    let hidden: &SymbolId = &named(&model, "hidden_projection", SymbolKind::Function).id;

    let reach = proof_reach(&model, proof);
    assert!(!reach.reaches_public);
    assert_eq!(reach.internals, vec![hidden.clone()]);
}

const SUBSTRATE_PRIVATE: &str = r#"
fn lint() {}

fn only_here() {}
"#;

/// A proof file with a helper of its own named like a private function of another crate.
const SHADOWING_TEST: &str = r#"
fn lint() -> bool {
    true
}

/// Verifies: SIM-003
#[test]
fn local_helper_shadows() {
    assert!(lint());
}

/// Verifies: SIM-004
#[test]
fn private_item_of_another_crate() {
    assert!(only_here());
}
"#;

const BINARY_TEST: &str = r#"
use std::process::Command;

fn run() -> bool {
    Command::new(env!("CARGO_BIN_EXE_strata")).status().is_ok()
}

/// Verifies: SIM-005
#[test]
fn runs_the_binary() {
    assert!(run());
}
"#;

fn shadowing_model() -> Model {
    model_of(&[
        ("crates/strata-substrate/src/lint.rs", SUBSTRATE_PRIVATE),
        ("crates/strata-simulation/tests/shadow.rs", SHADOWING_TEST),
        ("crates/strata-simulation/tests/binary.rs", BINARY_TEST),
    ])
}

#[test]
fn a_local_helper_shadows_a_same_named_private_function_elsewhere() {
    let model = shadowing_model();
    let proof = named(&model, "local_helper_shadows", SymbolKind::Function);
    let local = model
        .symbols
        .values()
        .find(|s| s.name == "lint" && s.file == proof.file)
        .expect("the helper is indexed");
    let edge = model
        .edges_from(&NodeRef::Symbol(proof.id.clone()))
        .find(|e| e.target_text == "lint")
        .expect("the call has an edge");
    assert_eq!(edge.to, Some(NodeRef::Symbol(local.id.clone())));
    assert_eq!(edge.resolution, Resolution::Exact);
}

#[test]
fn a_private_function_of_another_crate_is_not_reachable() {
    let model = shadowing_model();
    let proof = named(
        &model,
        "private_item_of_another_crate",
        SymbolKind::Function,
    );
    assert!(model
        .edges_from(&NodeRef::Symbol(proof.id.clone()))
        .all(|e| e.target_text != "only_here"));
    // With nothing it calls resolving to indexed code, the proof is reported.
    let flaws = proof_flaws(&model, proof);
    assert_eq!(
        flaws.iter().map(|f| f.rule).collect::<Vec<_>>(),
        vec!["calls_nothing"]
    );
}

#[test]
fn a_proof_that_runs_the_packages_binary_is_not_reported() {
    let model = shadowing_model();
    let proof = named(&model, "runs_the_binary", SymbolKind::Function);
    assert!(proof_flaws(&model, proof).is_empty());
}

const INSPECTION_TEST: &str = r#"
/// Verifies: SIM-006
/// Inspects: crates/strata-substrate/src
#[test]
fn reads_the_sources() {
    assert!(std::fs::read_to_string("x").is_ok());
}

/// Verifies: SIM-007
/// Inspects: crates/strata-nowhere/src
#[test]
fn names_no_indexed_code() {
    assert!(std::fs::read_to_string("x").is_ok());
}

/// Verifies: SIM-008
/// Inspects: crates/strata-simulation/tests
#[test]
fn names_only_tests() {
    assert!(std::fs::read_to_string("x").is_ok());
}
"#;

#[test]
fn a_declared_source_inspection_is_not_reported_when_it_names_indexed_code() {
    let model = model_of(&[
        ("crates/strata-substrate/src/lint.rs", SUBSTRATE_PRIVATE),
        ("crates/strata-simulation/tests/inspect.rs", INSPECTION_TEST),
    ]);
    let rules = |name: &str| {
        proof_flaws(&model, named(&model, name, SymbolKind::Function))
            .iter()
            .map(|f| f.rule)
            .collect::<Vec<_>>()
    };
    assert!(rules("reads_the_sources").is_empty());
    // A path that holds no indexed code, or only test scaffolding, declares nothing.
    assert_eq!(rules("names_no_indexed_code"), vec!["calls_nothing"]);
    assert_eq!(rules("names_only_tests"), vec!["calls_nothing"]);
}

#[test]
fn a_proof_that_calls_only_private_code_bypasses_the_boundary() {
    let model = standard_model();
    let proof = named(&model, "bypasses_boundary", SymbolKind::Function);
    assert_eq!(
        proof_flaws(&model, proof)
            .iter()
            .map(|f| f.rule)
            .collect::<Vec<_>>(),
        vec!["bypasses_boundary"]
    );
    let boundary = named(&model, "projection_boundary", SymbolKind::Function);
    assert!(proof_flaws(&model, boundary).is_empty());
}

#[test]
fn the_statement_of_a_lean_header_follows_its_binders() {
    assert_eq!(
        lean_statement("theorem t (n : Nat) : n = n").as_deref(),
        Some("n = n")
    );
    assert_eq!(lean_statement("theorem c : True").as_deref(), Some("True"));
    assert_eq!(
        lean_statement("theorem f {a : Nat} : ∀ x : Nat, x = x").as_deref(),
        Some("∀ x : Nat, x = x")
    );
    assert_eq!(lean_statement("theorem none"), None);
}

/// Model of Markdown specification files (each `(path, text)`), indexed as the engine does.
fn model_of_specs(files: &[(&str, &str)]) -> (Model, concordance_core::lang::Languages) {
    let langs = concordance_core::lang::Languages::new(vec![]);
    let mut model = Model::default();
    for (path, text) in files {
        let rel = RepoPath::from_relative_str(path).unwrap();
        let idx = concordance_markdown::index_document(&rel, text, None, &langs);
        let doc = idx.document.id.clone();
        model.upsert_document(idx.document);
        model.replace_sections(&doc, idx.sections);
        for c in idx.claims {
            model.upsert_claim(c);
        }
    }
    (model, langs)
}

fn duplicate_id_findings(
    model: &Model,
    langs: concordance_core::lang::Languages,
) -> Vec<concordance_core::finding::Finding> {
    let ctx = crate::BatchContext {
        revision: concordance_core::Revision(1),
        now: concordance_core::Timestamp(0),
        changed_files: Vec::new(),
        semantic_enabled: false,
        languages: langs,
        gate: Default::default(),
    };
    crate::verification::proof_rule_findings(model, &ctx)
        .into_iter()
        .filter(|f| {
            matches!(&f.subject, concordance_core::finding::FindingSubject::ProofRule { rule, .. } if rule == "duplicate_id")
        })
        .collect()
}

#[test]
fn a_requirement_id_opened_in_two_documents_is_a_finding() {
    let (model, langs) = model_of_specs(&[
        (
            "docs/spec/a.md",
            "**KRN-007** The kernel SHALL boot.\n- Verify: example\n",
        ),
        (
            "docs/spec/b.md",
            "**KRN-007** The kernel SHALL halt.\n- Verify: example\n",
        ),
    ]);
    let found = duplicate_id_findings(&model, langs);
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].title.contains("KRN-007"), "{}", found[0].title);
    assert!(found[0].title.contains("2 documents"), "{}", found[0].title);
}

#[test]
fn one_requirement_id_in_one_document_is_not_a_duplicate() {
    let (model, langs) = model_of_specs(&[(
        "docs/spec/a.md",
        "**KRN-007** The kernel SHALL boot.\n- Verify: example\n\n**KRN-008** The kernel SHALL halt.\n- Verify: example\n",
    )]);
    assert!(duplicate_id_findings(&model, langs).is_empty());
}

// ---- requirement realisation and holes (WG-4842) ----

use crate::derive::claim_finding;
use crate::verification::{claim_test_evidence, proof_rule_findings};
use concordance_core::config::GateConfig;
use concordance_core::evidence::Evidence;
use concordance_core::finding::{Finding, FindingStatus, FindingSubject};
use concordance_core::testrun::{RunBinding, TestOutcome, TestRun};
use concordance_core::{ContentHash, Timestamp};

fn ctx_with(gate: GateConfig) -> crate::BatchContext {
    crate::BatchContext {
        revision: concordance_core::Revision(1),
        now: Timestamp(0),
        changed_files: Vec::new(),
        semantic_enabled: false,
        languages: concordance_core::lang::Languages::new(vec![]),
        gate,
    }
}

/// A model of Rust `sources` and one Markdown specification `spec`.
fn realisation_model(spec: &str, sources: &[(&str, &str)]) -> Model {
    let mut model = model_of(sources);
    let langs = concordance_core::lang::Languages::new(vec![]);
    let rel = RepoPath::from_relative_str("docs/spec/r.md").unwrap();
    let idx = concordance_markdown::index_document(&rel, spec, None, &langs);
    let doc = idx.document.id.clone();
    model.upsert_document(idx.document);
    model.replace_sections(&doc, idx.sections);
    for c in idx.claims {
        model.upsert_claim(c);
    }
    model
}

fn passing(test: &str) -> TestRun {
    TestRun {
        path: "run.txt".into(),
        modified: Timestamp(0),
        content_hash: ContentHash::of_str(""),
        results: [(test.to_string(), TestOutcome::Passed)]
            .into_iter()
            .collect(),
        crates: Default::default(),
        binding: RunBinding::Recorded,
        changed_sources: 0,
        changed_example: None,
        changed_statements: Default::default(),
    }
}

fn finding_of(model: &Model, req: &str, gate: GateConfig) -> Finding {
    let claim = model.claims_of_requirement(req).next().expect("claim");
    let doc = model.documents.get(&claim.document).expect("document");
    let ctx = ctx_with(gate);
    let evs: Vec<Evidence> = claim_test_evidence(model, claim, &ctx);
    let refs: Vec<&Evidence> = evs.iter().collect();
    claim_finding(model, claim, doc, &refs, &[], &ctx)
}

fn hole_rules(model: &Model, gate: GateConfig) -> Vec<String> {
    proof_rule_findings(model, &ctx_with(gate))
        .into_iter()
        .filter_map(|f| match f.subject {
            FindingSubject::ProofRule { rule, .. } if rule.starts_with("hole_") => Some(rule),
            _ => None,
        })
        .collect()
}

const HOLE_SPEC: &str = "**H-012** The item implementing the projection SHALL satisfy its postconditions: it keeps every proposal.\n- Realisation: hole\n- Component: C-1\n";

const HOLE_IMPL: &str = r#"
pub struct Scenario;
pub struct Proposal;

/// Implements: H-012
pub fn project(_scenario: &Scenario, _proposal: &Proposal, _keep: &mut dyn FnMut(u8, u8, u8) -> bool) {}
"#;

const HOLE_IMPL_TWICE: &str = r#"
/// Implements: H-012
pub fn project() {}

/// Implements: H-012
pub fn project_again() {}
"#;

const HOLE_TEST: &str = r#"
use strata_simulation::project::{project, Proposal, Scenario};

/// Verifies: H-012
#[test]
fn hole_postcondition() {
    project(&Scenario, &Proposal, &mut |_, _, _| true);
}
"#;

#[test]
fn an_operational_requirement_without_code_is_operational_not_unverified() {
    let spec = "**OPS-001** The deployment SHALL answer within 200 ms.\n- Realisation: operational\n- Milestone: 3\n";
    let model = realisation_model(spec, &[]);
    // Milestone 3 is not started: a checked requirement would be `planned`.
    let gate = GateConfig {
        started_milestones: Some(vec![1]),
        ..Default::default()
    };
    let f = finding_of(&model, "OPS-001", gate.clone());
    assert_eq!(f.status, FindingStatus::Operational, "{}", f.explanation);

    let checked = "**OPS-002** The deployment SHALL answer within 200 ms.\n";
    let f = finding_of(
        &realisation_model(checked, &[]),
        "OPS-002",
        GateConfig::default(),
    );
    assert_eq!(f.status, FindingStatus::Unverified);
}

#[test]
fn a_construction_requirement_is_verified_only_when_every_modelling_claim_is() {
    let spec = "**CON-001** Generated code SHALL reject a stale write.\n- Realisation: construction\n- Modelled by: c-1, c-2\n";
    let mut model = realisation_model(spec, &[]);
    model
        .exegra_verdicts
        .insert("c-1".into(), "verified".into());
    model
        .exegra_verdicts
        .insert("c-2".into(), "violated".into());
    let f = finding_of(&model, "CON-001", GateConfig::default());
    assert_eq!(f.status, FindingStatus::Unverified);
    assert!(
        f.explanation.contains("`c-2` is violated"),
        "{}",
        f.explanation
    );
    assert!(!f.explanation.contains("`c-1` is"), "{}", f.explanation);

    model
        .exegra_verdicts
        .insert("c-2".into(), "verified".into());
    let f = finding_of(&model, "CON-001", GateConfig::default());
    assert_eq!(f.status, FindingStatus::Verified, "{}", f.explanation);

    // A claim with no result in the Exegra results does not establish it either.
    model.exegra_verdicts.remove("c-1");
    let f = finding_of(&model, "CON-001", GateConfig::default());
    assert_eq!(f.status, FindingStatus::Unverified);
    assert!(
        f.explanation.contains("`c-1` has no result"),
        "{}",
        f.explanation
    );
}

#[test]
fn a_hole_with_no_implementing_item_is_unimplemented() {
    let model = realisation_model(HOLE_SPEC, &[]);
    assert_eq!(
        hole_rules(&model, GateConfig::default()),
        vec!["hole_unimplemented"]
    );
    // Only the components that are started are expected to have code.
    let gate = GateConfig {
        started_components: Some(vec!["C-2".into()]),
        ..Default::default()
    };
    assert!(hole_rules(&model, gate.clone()).is_empty());
    assert_eq!(
        finding_of(&model, "H-012", gate).status,
        FindingStatus::Planned
    );
}

#[test]
fn a_hole_with_two_implementing_items_is_a_duplicate() {
    let model = realisation_model(
        HOLE_SPEC,
        &[("crates/strata-simulation/src/project.rs", HOLE_IMPL_TWICE)],
    );
    assert_eq!(
        hole_rules(&model, GateConfig::default()),
        vec!["hole_duplicate"]
    );
}

#[test]
fn a_hole_with_one_item_and_a_passing_test_is_verified() {
    let mut model = realisation_model(
        HOLE_SPEC,
        &[
            ("crates/strata-simulation/src/project.rs", HOLE_IMPL),
            ("crates/strata-simulation/tests/hole.rs", HOLE_TEST),
        ],
    );
    assert!(hole_rules(&model, GateConfig::default()).is_empty());
    model.test_runs = vec![passing("hole_postcondition")];
    let f = finding_of(&model, "H-012", GateConfig::default());
    assert_eq!(f.status, FindingStatus::Verified, "{}", f.explanation);
}

#[test]
fn a_hole_whose_tests_pass_but_has_no_implementation_is_not_verified() {
    let mut model = realisation_model(
        HOLE_SPEC,
        &[
            ("crates/strata-simulation/src/project.rs", "pub struct Scenario;\npub struct Proposal;\npub fn project(_s: &Scenario, _p: &Proposal, _k: &mut dyn FnMut(u8, u8, u8) -> bool) {}\n"),
            ("crates/strata-simulation/tests/hole.rs", HOLE_TEST),
        ],
    );
    model.test_runs = vec![passing("hole_postcondition")];
    let f = finding_of(&model, "H-012", GateConfig::default());
    assert_eq!(f.status, FindingStatus::Unverified, "{}", f.explanation);
}
