//! find_callers tool - wraps `rkt callers`

use rmcp::model::{CallToolResult, Content};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::mcp::ProjectManager;

/// Input for find_callers tool
#[derive(Debug, Deserialize)]
pub struct FindCallersInput {
    /// Symbol to find callers for (qualified name preferred)
    pub symbol: String,
    /// Optional project root
    pub project_root: Option<String>,
}

/// A single caller
#[derive(Debug, Serialize)]
pub struct CallerInfo {
    pub caller_symbol: String,
    pub file: String,
    pub line: u32,
    pub column: u32,
}

/// Output for find_callers tool
#[derive(Debug, Serialize)]
pub struct CallersResult {
    pub target_symbol: String,
    pub callers: Vec<CallerInfo>,
    pub caller_count: usize,
    pub project_root: String,
}

/// Execute the find_callers tool
pub async fn find_callers(manager: Arc<ProjectManager>, input: FindCallersInput) -> CallToolResult {
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
                // Use spider with reverse=true and depth=1 to find callers
                use rocketindex::spider::reverse_spider;

                let tree = reverse_spider(&state.code_index, &input.symbol, 1);
                let mut callers = Vec::new();
                for node in tree.nodes {
                    if node.depth == 1 {
                        callers.push(CallerInfo {
                            caller_symbol: node.symbol.qualified.clone(),
                            file: node.symbol.location.file.display().to_string(),
                            line: node.symbol.location.line,
                            column: node.symbol.location.column,
                        });
                    }
                }
                callers
            })
            .await
            .unwrap_or_default();

        if !result.is_empty() {
            all_results.push(CallersResult {
                target_symbol: input.symbol.clone(),
                caller_count: result.len(),
                callers: result,
                project_root: root.display().to_string(),
            });
        }
    }

    if all_results.is_empty() {
        return CallToolResult::success(vec![Content::text(format!(
            "No callers found for '{}'. This symbol may not be called anywhere, or the index may need updating.",
            input.symbol
        ))]);
    }

    let json = serde_json::to_string(&all_results).unwrap_or_default();
    CallToolResult::success(vec![Content::text(json)])
}
