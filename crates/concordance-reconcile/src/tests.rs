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

    // `map.values().next()` is a name-only edge to the unrelated private `Parser::next`.
    let next_edge = model
        .edges_from(&NodeRef::Symbol(proof.id.clone()))
        .find(|e| e.target_text == ".next()")
        .expect("the method call has an edge");
    assert_eq!(next_edge.resolution, Resolution::NameOnly);
    assert_eq!(next_edge.to, Some(NodeRef::Symbol(parser_next.id.clone())));

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
