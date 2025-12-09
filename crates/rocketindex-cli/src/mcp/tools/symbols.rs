//! search_symbols tool - wraps `rkt symbols`

use rmcp::model::{CallToolResult, Content};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::mcp::ProjectManager;

/// Input for search_symbols tool
#[derive(Debug, Deserialize)]
pub struct SearchSymbolsInput {
    /// Pattern to match (supports * wildcards)
    pub pattern: String,
    /// Filter by language (e.g., "rust", "typescript", "fsharp")
    pub language: Option<String>,
    /// Use fuzzy matching
    #[serde(default)]
    pub fuzzy: bool,
    /// Maximum results per project (default: 20)
    #[serde(default = "default_limit")]
    pub limit: usize,
}

fn default_limit() -> usize {
    20
}

/// A single symbol result
#[derive(Debug, Serialize)]
pub struct SymbolInfo {
    pub qualified: String,
    pub name: String,
    pub kind: String,
    pub file: String,
    pub line: u32,
    pub language: String,
    pub project_root: String,
}

/// Execute the search_symbols tool
pub async fn search_symbols(
    manager: Arc<ProjectManager>,
    input: SearchSymbolsInput,
) -> CallToolResult {
    // Use CWD-aware project resolution (no explicit project_root in this tool)
    let project_roots = manager.resolve_projects(None, None).await;

    if project_roots.is_empty() {
        return CallToolResult::error(vec![Content::text(
            "No projects registered. Use `register_project` to add a project first.",
        )]);
    }

    let mut all_results = Vec::new();

    for root in project_roots {
        let result = manager
            .with_project(&root, |state| {
                if input.fuzzy {
                    // Fuzzy search returns (Symbol, score)
                    state
                        .sqlite
                        .fuzzy_search(&input.pattern, 2, input.limit, input.language.as_deref())
                        .unwrap_or_default()
                        .into_iter()
                        .map(|(s, _score)| SymbolInfo {
                            qualified: s.qualified,
                            name: s.name,
                            kind: format!("{:?}", s.kind),
                            file: s.location.file.display().to_string(),
                            line: s.location.line,
                            language: s.language,
                            project_root: root.display().to_string(),
                        })
                        .collect::<Vec<_>>()
                } else {
                    // Pattern search (supports * wildcards)
                    state
                        .sqlite
                        .search(&input.pattern, input.limit, input.language.as_deref())
                        .unwrap_or_default()
                        .into_iter()
                        .map(|s| SymbolInfo {
                            qualified: s.qualified,
                            name: s.name,
                            kind: format!("{:?}", s.kind),
                            file: s.location.file.display().to_string(),
                            line: s.location.line,
                            language: s.language,
                            project_root: root.display().to_string(),
                        })
                        .collect::<Vec<_>>()
                }
            })
            .await
            .unwrap_or_default();

        all_results.extend(result);
    }

    if all_results.is_empty() {
        return CallToolResult::success(vec![Content::text(format!(
            "No symbols found matching '{}'. Try a different pattern or use fuzzy=true for approximate matching.",
            input.pattern
        ))]);
    }

    // Sort by relevance (exact matches first, then by name length)
    all_results.sort_by(|a, b| {
        let a_exact = a.name == input.pattern || a.qualified == input.pattern;
        let b_exact = b.name == input.pattern || b.qualified == input.pattern;
        match (a_exact, b_exact) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.name.len().cmp(&b.name.len()),
        }
    });

    let json = serde_json::to_string(&all_results).unwrap_or_default();
    CallToolResult::success(vec![Content::text(json)])
}
