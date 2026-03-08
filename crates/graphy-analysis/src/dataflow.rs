//! Phase 8: Basic data flow analysis.
//!
//! Builds def-use chains, connects actual parameters to formal parameters at
//! call sites, and creates DATA_FLOWS_TO edges.

use std::collections::HashMap;

use graphy_core::{
    CodeGraph, DataFlowTransform, EdgeKind, EdgeMetadata, GirEdge, NodeKind, SymbolId,
};
use petgraph::visit::EdgeRef;
use petgraph::Direction;
use tracing::debug;

/// Phase 8: Build data flow edges.
///
/// This simplified version focuses on:
/// 1. Parameter passing: actual args at call sites flow to formal params
/// 2. Return values: function return types flow back to callers
pub fn analyze_dataflow(graph: &mut CodeGraph) {
    let mut edges_to_add: Vec<(SymbolId, SymbolId, GirEdge)> = Vec::new();

    // Build a map of function_id -> ordered list of parameter SymbolIds
    let mut func_params: HashMap<SymbolId, Vec<SymbolId>> = HashMap::new();

    for node in graph.all_nodes() {
        if !node.kind.is_callable() {
            continue;
        }

        let func_id = node.id;
        let params: Vec<SymbolId> = graph
            .children(func_id)
            .iter()
            .filter(|n| n.kind == NodeKind::Parameter)
            .map(|n| n.id)
            .collect();

        if !params.is_empty() {
            func_params.insert(func_id, params);
        }
    }

    // For each CALLS edge, try to connect the caller's context to the callee's parameters.
    // Collect call relationships: (caller_id, callee_id)
    let call_edges: Vec<(SymbolId, SymbolId)> = graph
        .all_nodes()
        .filter(|n| n.kind.is_callable())
        .flat_map(|caller| {
            let caller_id = caller.id;
            graph
                .callees(caller_id)
                .iter()
                .map(move |callee| (caller_id, callee.id))
                .collect::<Vec<_>>()
        })
        .collect();

    for (caller_id, callee_id) in &call_edges {
        // If the callee has known parameters, create data flow edges
        // from the caller to each parameter of the callee.
        if let Some(callee_params) = func_params.get(callee_id) {
            // Similarly, get the caller's parameters/variables as potential sources
            let caller_params: Vec<SymbolId> = graph
                .children(*caller_id)
                .iter()
                .filter(|n| {
                    matches!(
                        n.kind,
                        NodeKind::Parameter | NodeKind::Variable | NodeKind::Constant
                    )
                })
                .map(|n| n.id)
                .collect();

            // Heuristic: match parameters by position.
            // Skip 'self' parameter for methods.
            let callee_params_filtered: Vec<SymbolId> = callee_params
                .iter()
                .filter(|&&pid| {
                    graph
                        .get_node(pid)
                        .map_or(true, |n| n.name != "self" && n.name != "cls")
                })
                .copied()
                .collect();

            // Positional matching: map caller params to callee params by position
            for (i, &callee_param) in callee_params_filtered.iter().enumerate() {
                if i < caller_params.len() {
                    let edge = GirEdge::new(EdgeKind::DataFlowsTo)
                        .with_confidence(0.5)
                        .with_metadata(EdgeMetadata::DataFlow {
                            transform: DataFlowTransform::Identity,
                        });
                    edges_to_add.push((caller_params[i], callee_param, edge));
                }
            }
        }

        // Return value flow: if callee has a ReturnsType edge,
        // create a flow from the callee back to the caller.
        let return_types: Vec<SymbolId> = {
            let Some(idx) = graph.get_node_index(*callee_id) else {
                continue;
            };
            graph
                .graph
                .edges_directed(idx, Direction::Outgoing)
                .filter(|e| e.weight().kind == EdgeKind::ReturnsType)
                .filter_map(|e| graph.graph.node_weight(e.target()).map(|n| n.id))
                .collect()
        };

        for ret_type_id in return_types {
            let edge = GirEdge::new(EdgeKind::DataFlowsTo)
                .with_confidence(0.5)
                .with_metadata(EdgeMetadata::DataFlow {
                    transform: DataFlowTransform::Transform,
                });
            edges_to_add.push((ret_type_id, *caller_id, edge));
        }
    }

    // Intra-procedural: connect parameters to variables within the same function
    // that share the same name (simple alias tracking).
    for node in graph.all_nodes() {
        if !node.kind.is_callable() {
            continue;
        }

        let func_id = node.id;
        let children: Vec<(SymbolId, String, NodeKind)> = graph
            .children(func_id)
            .iter()
            .map(|n| (n.id, n.name.clone(), n.kind))
            .collect();

        let params: Vec<(SymbolId, String)> = children
            .iter()
            .filter(|(_, _, k)| *k == NodeKind::Parameter)
            .map(|(id, name, _)| (*id, name.clone()))
            .collect();

        let vars: Vec<(SymbolId, String)> = children
            .iter()
            .filter(|(_, _, k)| *k == NodeKind::Variable)
            .map(|(id, name, _)| (*id, name.clone()))
            .collect();

        // If a variable shadows a parameter name, create a data flow
        for (param_id, param_name) in &params {
            for (var_id, var_name) in &vars {
                if param_name == var_name && param_id != var_id {
                    let edge = GirEdge::new(EdgeKind::DataFlowsTo)
                        .with_confidence(0.7)
                        .with_metadata(EdgeMetadata::DataFlow {
                            transform: DataFlowTransform::Identity,
                        });
                    edges_to_add.push((*param_id, *var_id, edge));
                }
            }
        }
    }

    let count = edges_to_add.len();
    for (src, tgt, edge) in edges_to_add {
        graph.add_edge(src, tgt, edge);
    }

    debug!("Phase 8 (Data Flow): added {} DATA_FLOWS_TO edges", count);
}

#[cfg(test)]
mod tests {
    use super::*;
    use graphy_core::{GirNode, Language, Span, Visibility};
    use std::path::PathBuf;

    fn make_file(name: &str) -> GirNode {
        let path = PathBuf::from(name);
        GirNode {
            id: SymbolId::new(&path, name, NodeKind::File, 0),
            name: name.to_string(),
            kind: NodeKind::File,
            file_path: path,
            span: Span::new(0, 0, 100, 0),
            visibility: Visibility::Public,
            language: Language::Python,
            signature: None,
            complexity: None,
            confidence: 1.0,
            doc: None,
            coverage: None,
        }
    }

    fn make_fn(name: &str, file: &str, line: u32) -> GirNode {
        GirNode::new(
            name.to_string(),
            NodeKind::Function,
            PathBuf::from(file),
            Span::new(line, 0, line + 5, 0),
            Language::Python,
        )
    }

    fn make_param(name: &str, file: &str, line: u32) -> GirNode {
        GirNode::new(
            name.to_string(),
            NodeKind::Parameter,
            PathBuf::from(file),
            Span::new(line, 0, line, 10),
            Language::Python,
        )
    }

    #[test]
    fn dataflow_empty_graph() {
        let mut g = CodeGraph::new();
        analyze_dataflow(&mut g);
        assert_eq!(g.edge_count(), 0);
    }

    #[test]
    fn dataflow_parameter_passing() {
        let mut g = CodeGraph::new();
        let file = make_file("test.py");
        let file_id = file.id;
        g.add_node(file);

        // Caller with parameter x
        let caller = make_fn("caller", "test.py", 1);
        let caller_id = caller.id;
        g.add_node(caller);
        g.add_edge(file_id, caller_id, GirEdge::new(EdgeKind::Contains));

        let caller_param = make_param("x", "test.py", 1);
        let caller_param_id = caller_param.id;
        g.add_node(caller_param);
        g.add_edge(caller_id, caller_param_id, GirEdge::new(EdgeKind::Contains));

        // Callee with parameter y
        let callee = make_fn("callee", "test.py", 10);
        let callee_id = callee.id;
        g.add_node(callee);
        g.add_edge(file_id, callee_id, GirEdge::new(EdgeKind::Contains));

        let callee_param = make_param("y", "test.py", 10);
        let callee_param_id = callee_param.id;
        g.add_node(callee_param);
        g.add_edge(callee_id, callee_param_id, GirEdge::new(EdgeKind::Contains));

        // Caller calls callee
        g.add_edge(caller_id, callee_id, GirEdge::new(EdgeKind::Calls));

        analyze_dataflow(&mut g);

        // Should have a DataFlowsTo edge from caller's x to callee's y
        let flows = g.outgoing(caller_param_id, EdgeKind::DataFlowsTo);
        assert!(
            flows.iter().any(|n| n.id == callee_param_id),
            "Expected DataFlowsTo from caller param to callee param"
        );
    }

    #[test]
    fn dataflow_skips_self_parameter() {
        let mut g = CodeGraph::new();
        let file = make_file("test.py");
        let file_id = file.id;
        g.add_node(file);

        let caller = make_fn("caller", "test.py", 1);
        let caller_id = caller.id;
        g.add_node(caller);
        g.add_edge(file_id, caller_id, GirEdge::new(EdgeKind::Contains));

        let caller_param = make_param("arg", "test.py", 1);
        let caller_param_id = caller_param.id;
        g.add_node(caller_param);
        g.add_edge(caller_id, caller_param_id, GirEdge::new(EdgeKind::Contains));

        // Method with 'self' as first param
        let method = GirNode::new(
            "method".to_string(),
            NodeKind::Method,
            PathBuf::from("test.py"),
            Span::new(10, 0, 15, 0),
            Language::Python,
        );
        let method_id = method.id;
        g.add_node(method);
        g.add_edge(file_id, method_id, GirEdge::new(EdgeKind::Contains));

        let self_param = make_param("self", "test.py", 10);
        let self_param_id = self_param.id;
        g.add_node(self_param);
        g.add_edge(method_id, self_param_id, GirEdge::new(EdgeKind::Contains));

        let real_param = make_param("data", "test.py", 11);
        let real_param_id = real_param.id;
        g.add_node(real_param);
        g.add_edge(method_id, real_param_id, GirEdge::new(EdgeKind::Contains));

        g.add_edge(caller_id, method_id, GirEdge::new(EdgeKind::Calls));

        analyze_dataflow(&mut g);

        // 'self' should be skipped, so caller_param (arg) maps to real_param (data)
        let flows = g.outgoing(caller_param_id, EdgeKind::DataFlowsTo);
        assert!(
            flows.iter().any(|n| n.id == real_param_id),
            "Expected DataFlowsTo from arg to data (skipping self)"
        );
    }
}
