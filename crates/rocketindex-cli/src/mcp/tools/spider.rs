//! analyze_dependencies tool - wraps `rkt spider`

use rmcp::model::{CallToolResult, Content};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::mcp::ProjectManager;

/// Input for analyze_dependencies tool
#[derive(Debug, Deserialize)]
pub struct AnalyzeDepsInput {
    /// Entry point symbol to start from
    pub symbol: String,
    /// Max depth to traverse (default: 3)
    #[serde(default = "default_depth")]
    pub depth: usize,
    /// Reverse direction (find what leads TO this symbol instead of FROM)
    #[serde(default)]
    pub reverse: bool,
    /// Optional project root
    pub project_root: Option<String>,
}

fn default_depth() -> usize {
    3
}

/// A node in the dependency graph
#[derive(Debug, Serialize)]
pub struct DependencyNode {
    pub symbol: String,
    pub kind: String,
    pub file: String,
    pub line: u32,
    pub depth: usize,
}

/// Output for analyze_dependencies tool
#[derive(Debug, Serialize)]
pub struct DependencyGraph {
    pub entry_point: String,
    pub direction: String,
    pub max_depth: usize,
    pub nodes: Vec<DependencyNode>,
    pub unresolved: Vec<String>,
    pub project_root: String,
}

/// Execute the analyze_dependencies tool
pub async fn analyze_dependencies(
    manager: Arc<ProjectManager>,
    input: AnalyzeDepsInput,
) -> CallToolResult {
    // Determine which project to search (CWD-aware)
    let project_roots = manager
        .resolve_projects(input.project_root.as_deref(), None)
        .await;

    if project_roots.is_empty() {
        return CallToolResult::error(vec![Content::text(
            "No projects registered. Use `register_project` to add a project first.",
        )]);
    }

    let mut all_results = Vec::new();

    for root in project_roots {
        let result = manager
            .with_project(&root, |state| {
                use rocketindex::spider::{reverse_spider, spider};

                let tree = if input.reverse {
                    reverse_spider(&state.code_index, &input.symbol, input.depth)
                } else {
                    spider(&state.code_index, &input.symbol, input.depth)
                };

                let nodes: Vec<DependencyNode> = tree
                    .nodes
                    .into_iter()
                    .map(|n| DependencyNode {
                        symbol: n.symbol.qualified,
                        kind: format!("{:?}", n.symbol.kind),
                        file: n.symbol.location.file.display().to_string(),
                        line: n.symbol.location.line,
                        depth: n.depth,
                    })
                    .collect();

                DependencyGraph {
                    entry_point: input.symbol.clone(),
                    direction: if input.reverse {
                        "reverse (callers)".to_string()
                    } else {
                        "forward (dependencies)".to_string()
                    },
                    max_depth: input.depth,
                    nodes,
                    unresolved: tree.unresolved,
                    project_root: root.display().to_string(),
                }
            })
            .await;

        if let Some(graph) = result {
            if !graph.nodes.is_empty() || !graph.unresolved.is_empty() {
                all_results.push(graph);
            }
        }
    }

    if all_results.is_empty() {
        return CallToolResult::success(vec![Content::text(format!(
            "No dependencies found for '{}'. The symbol may not exist or have no {}.",
            input.symbol,
            if input.reverse {
                "callers"
            } else {
                "dependencies"
            }
        ))]);
    }

    let json = serde_json::to_string(&all_results).unwrap_or_default();
    CallToolResult::success(vec![Content::text(json)])
}
