//! Transitive caller lookup.
//!
//! Builds a name-indexed call graph on demand by re-parsing the source of
//! every Rust function/method leaf with tree-sitter, extracting call and
//! method-call expressions, and keying on the trailing identifier of the
//! callee.
//!
//! Resolution is by simple name only — the graph has no type information, so
//! `self.foo()` and `other::foo()` both count as a call to any leaf named
//! `foo`. Callers relying on high precision should follow up with a targeted
//! `orbit.graph.show` on each hit. The tool schema surfaces this limitation
//! explicitly.

use std::collections::{HashMap, HashSet, VecDeque};

use crate::error::KnowledgeError;
use crate::graph::call_extraction::rust_callee_names;
use crate::graph::navigator::GraphNodeRef;
use crate::graph::nodes::{CodebaseGraphV1, LeafKind};
use orbit_graph_extract::Selector;

use super::GraphContextService;

/// Maximum `depth` any caller is willing to accept. The tool clamps to this.
pub const MAX_CALLER_DEPTH: usize = 5;

pub struct CallerHit {
    pub selector: String,
    pub name: String,
    pub file: String,
    pub kind: String,
    pub distance: usize,
    /// Simple name the caller's source matched on to reach this hit. For
    /// direct callers (`distance == 1`) this is the target's simple name; for
    /// indirect callers it is the simple name of an intermediate hop.
    pub via: String,
}

/// BFS upward from the symbol named by `selector`, returning every function or
/// method leaf whose source contains a call-like mention of the target name
/// within `depth` hops.
pub fn transitive_callers<'a>(
    svc: &'a GraphContextService<'a>,
    graph: &'a CodebaseGraphV1,
    selector: &Selector,
    depth: usize,
) -> Result<Vec<CallerHit>, KnowledgeError> {
    if depth == 0 {
        return Ok(Vec::new());
    }
    let depth = depth.min(MAX_CALLER_DEPTH);

    let target_node = svc.resolve_selector(selector)?;
    let target_leaf = match target_node {
        GraphNodeRef::Leaf(leaf) => leaf,
        _ => {
            return Err(KnowledgeError::invalid_data(format!(
                "`{selector}` does not resolve to a symbol"
            )));
        }
    };
    let target_selector_string = svc.selector_for_node(GraphNodeRef::Leaf(target_leaf));

    let call_index = build_call_index(graph)?;

    let mut visited: HashSet<String> = HashSet::new();
    visited.insert(target_selector_string.clone());

    let mut queue: VecDeque<(String, String, usize)> = VecDeque::new();
    queue.push_back((target_selector_string, target_leaf.base.name.clone(), 0));

    let mut hits: Vec<CallerHit> = Vec::new();

    while let Some((_current_selector, current_name, current_distance)) = queue.pop_front() {
        if current_distance >= depth {
            continue;
        }
        let Some(callers) = call_index.get(&current_name) else {
            continue;
        };
        for caller_id in callers {
            let Ok(GraphNodeRef::Leaf(caller_leaf)) = svc.navigator().get_node(caller_id) else {
                continue;
            };
            let caller_selector = svc.selector_for_node(GraphNodeRef::Leaf(caller_leaf));
            if !visited.insert(caller_selector.clone()) {
                continue;
            }
            let file = caller_leaf
                .base
                .location
                .split_once('#')
                .map(|(path, _)| path.to_string())
                .unwrap_or_else(|| caller_leaf.base.location.clone());
            hits.push(CallerHit {
                selector: caller_selector.clone(),
                name: caller_leaf.base.name.clone(),
                file,
                kind: caller_leaf.kind.to_string(),
                distance: current_distance + 1,
                via: current_name.clone(),
            });
            queue.push_back((
                caller_selector,
                caller_leaf.base.name.clone(),
                current_distance + 1,
            ));
        }
    }

    Ok(hits)
}

/// `callee_simple_name -> [leaf ids that call it]`.
///
/// A leaf may appear multiple times under different callee names; the
/// per-caller entry is deduped.
fn build_call_index(
    graph: &CodebaseGraphV1,
) -> Result<HashMap<String, Vec<String>>, KnowledgeError> {
    let mut index: HashMap<String, Vec<String>> = HashMap::new();

    for leaf in &graph.leaves {
        if leaf.base.language != "rust" {
            continue;
        }
        if !matches!(leaf.kind, LeafKind::Function | LeafKind::Method) {
            continue;
        }
        let callees = rust_callee_names(&leaf.source)?;
        for callee in callees {
            index.entry(callee).or_default().push(leaf.base.id.clone());
        }
    }

    Ok(index)
}
