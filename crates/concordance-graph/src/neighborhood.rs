//! Scoped neighborhood queries: the relationship view loads one node's surroundings,
//! never the whole graph.

use crate::{truncate, Model};
use concordance_core::finding::FindingStatus;
use concordance_core::graph::{EdgeKind, NodeRef, Resolution};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

#[derive(Clone, Debug, Serialize)]
pub struct NeighborhoodNode {
    pub id: String,
    pub kind: String,
    pub label: String,
    pub detail: Option<String>,
    pub status: Option<FindingStatus>,
}

#[derive(Clone, Debug, Serialize)]
pub struct NeighborhoodEdge {
    pub id: String,
    /// An `EdgeKind` in snake_case, or `contains` for structural containment.
    pub kind: String,
    pub from: String,
    pub to: String,
    pub resolution: Option<Resolution>,
    pub confidence: f32,
}

#[derive(Clone, Debug, Serialize)]
pub struct Neighborhood {
    pub center: String,
    pub nodes: Vec<NeighborhoodNode>,
    pub edges: Vec<NeighborhoodEdge>,
    pub truncated: bool,
}

fn kind_str(k: EdgeKind) -> String {
    serde_json::to_value(k)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

impl Model {
    fn node_entry(&self, n: &NodeRef) -> Option<NeighborhoodNode> {
        let (detail, status) = match n {
            NodeRef::Claim(id) => {
                let c = self.claims.get(id)?;
                (
                    Some(format!("{:?} · {}", c.kind, c.range)),
                    self.claim_status(id),
                )
            }
            NodeRef::Symbol(id) => {
                let s = self.symbols.get(id)?;
                let worst = self
                    .findings_for_symbol(id)
                    .filter(|f| f.is_open())
                    .map(|f| f.status)
                    .min();
                (Some(format!("{:?} · {}", s.kind, s.range)), worst)
            }
            NodeRef::Section(id) => (Some(self.sections.get(id)?.range.to_string()), None),
            NodeRef::Document(id) => (
                Some(format!(
                    "{:?} authority",
                    self.documents.get(id)?.authority.level
                )),
                None,
            ),
            NodeRef::File(id) => (self.files.get(id)?.crate_name.clone(), None),
        };
        Some(NeighborhoodNode {
            id: n.to_string(),
            kind: n.kind().to_string(),
            label: truncate(&self.label(n)?, 100),
            detail,
            status,
        })
    }

    /// Direct neighbors of a node with the connecting edge (outgoing = node is `from`).
    fn neighbors(&self, n: &NodeRef) -> Vec<(NodeRef, NeighborhoodEdge)> {
        let mut out = Vec::new();
        let contains = |a: &NodeRef, b: &NodeRef| NeighborhoodEdge {
            id: format!("contains:{a}:{b}"),
            kind: "contains".into(),
            from: a.to_string(),
            to: b.to_string(),
            resolution: None,
            confidence: 1.0,
        };
        let push_edges = |out: &mut Vec<(NodeRef, NeighborhoodEdge)>, node: &NodeRef| {
            for e in self.edges_from(node) {
                if let Some(to) = &e.to {
                    out.push((
                        to.clone(),
                        NeighborhoodEdge {
                            id: e.id.to_string(),
                            kind: kind_str(e.kind),
                            from: e.from.to_string(),
                            to: to.to_string(),
                            resolution: Some(e.resolution.clone()),
                            confidence: e.confidence,
                        },
                    ));
                }
            }
            for e in self.edges_to(node) {
                out.push((
                    e.from.clone(),
                    NeighborhoodEdge {
                        id: e.id.to_string(),
                        kind: kind_str(e.kind),
                        from: e.from.to_string(),
                        to: node.to_string(),
                        resolution: Some(e.resolution.clone()),
                        confidence: e.confidence,
                    },
                ));
            }
        };
        match n {
            NodeRef::Claim(id) => {
                if let Some(c) = self.claims.get(id) {
                    let s = NodeRef::Section(c.section.clone());
                    out.push((s.clone(), contains(&s, n)));
                }
                push_edges(&mut out, n);
            }
            NodeRef::Section(id) => {
                if let Some(s) = self.sections.get(id) {
                    let parent = match &s.parent {
                        Some(p) => NodeRef::Section(p.clone()),
                        None => NodeRef::Document(s.document.clone()),
                    };
                    out.push((parent.clone(), contains(&parent, n)));
                    for c in self.claims_of(&s.document).filter(|c| &c.section == id) {
                        let cn = NodeRef::Claim(c.id.clone());
                        out.push((cn.clone(), contains(n, &cn)));
                    }
                    for child in self
                        .sections_of(&s.document)
                        .filter(|x| x.parent.as_ref() == Some(id))
                    {
                        let cn = NodeRef::Section(child.id.clone());
                        out.push((cn.clone(), contains(n, &cn)));
                    }
                }
            }
            NodeRef::Document(id) => {
                for s in self.sections_of(id).filter(|s| s.parent.is_none()) {
                    let sn = NodeRef::Section(s.id.clone());
                    out.push((sn.clone(), contains(n, &sn)));
                }
                push_edges(&mut out, n);
            }
            NodeRef::File(id) => {
                for s in self.symbols_of(id).filter(|s| s.parent.is_none()) {
                    let sn = NodeRef::Symbol(s.id.clone());
                    out.push((sn.clone(), contains(n, &sn)));
                }
                push_edges(&mut out, n);
            }
            NodeRef::Symbol(id) => {
                if let Some(s) = self.symbols.get(id) {
                    let parent = match &s.parent {
                        Some(p) => NodeRef::Symbol(p.clone()),
                        None => NodeRef::File(s.file.clone()),
                    };
                    out.push((parent.clone(), contains(&parent, n)));
                }
                for c in self.children_of(id) {
                    let cn = NodeRef::Symbol(c.id.clone());
                    out.push((cn.clone(), contains(n, &cn)));
                }
                push_edges(&mut out, n);
            }
        }
        out
    }

    /// Breadth-first neighborhood of `center` up to `depth` hops, at most `limit` nodes.
    pub fn neighborhood(&self, center: &NodeRef, depth: u8, limit: usize) -> Option<Neighborhood> {
        let center_entry = self.node_entry(center)?;
        let mut nodes: BTreeMap<String, NeighborhoodNode> = BTreeMap::new();
        nodes.insert(center_entry.id.clone(), center_entry);
        let mut edges: BTreeMap<String, NeighborhoodEdge> = BTreeMap::new();
        let mut seen: BTreeSet<NodeRef> = BTreeSet::from([center.clone()]);
        let mut queue = VecDeque::from([(center.clone(), 0u8)]);
        let mut truncated = false;
        while let Some((n, d)) = queue.pop_front() {
            if d >= depth {
                continue;
            }
            for (next, edge) in self.neighbors(&n) {
                if !seen.contains(&next) {
                    if nodes.len() >= limit {
                        truncated = true;
                        continue;
                    }
                    let Some(entry) = self.node_entry(&next) else {
                        continue;
                    };
                    nodes.insert(entry.id.clone(), entry);
                    seen.insert(next.clone());
                    queue.push_back((next, d + 1));
                }
                edges.insert(edge.id.clone(), edge);
            }
        }
        let edges = edges
            .into_values()
            .filter(|e| nodes.contains_key(&e.from) && nodes.contains_key(&e.to))
            .collect();
        Some(Neighborhood {
            center: center.to_string(),
            nodes: nodes.into_values().collect(),
            edges,
            truncated,
        })
    }
}
