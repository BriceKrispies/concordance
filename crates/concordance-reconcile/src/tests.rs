//! Call resolution and proof reach over indexed Rust source (WG-4819).

use crate::link::symbol_edges;
use crate::verification::proof_reach;
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
