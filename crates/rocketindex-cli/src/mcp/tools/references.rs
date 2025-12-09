//! find_references tool - wraps `rkt refs`

use rmcp::model::{CallToolResult, Content};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::mcp::ProjectManager;

/// Input for find_references tool
#[derive(Debug, Deserialize)]
pub struct FindReferencesInput {
    /// Symbol to find usages of
    pub symbol: String,
    /// Optional project root
    pub project_root: Option<String>,
    /// Context lines around each reference (default: 0)
    #[serde(default)]
    pub context_lines: usize,
}

/// A single reference
#[derive(Debug, Serialize)]
pub struct ReferenceInfo {
    pub file: String,
    pub line: u32,
    pub column: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
}

/// Output for find_references tool
#[derive(Debug, Serialize)]
pub struct ReferencesResult {
    pub symbol: String,
    pub references: Vec<ReferenceInfo>,
    pub reference_count: usize,
    pub project_root: String,
}

/// Execute the find_references tool
pub async fn find_references(
    manager: Arc<ProjectManager>,
    input: FindReferencesInput,
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
                match state.sqlite.find_references(&input.symbol) {
                    Ok(refs) => refs
                        .into_iter()
                        .map(|r| {
                            let context = if input.context_lines > 0 {
                                read_context(
                                    &r.location.file,
                                    r.location.line as usize,
                                    input.context_lines,
                                )
                            } else {
                                None
                            };
                            ReferenceInfo {
                                file: r.location.file.display().to_string(),
                                line: r.location.line,
                                column: r.location.column,
                                context,
                            }
                        })
                        .collect(),
                    Err(_) => Vec::new(),
                }
            })
            .await
            .unwrap_or_default();

        if !result.is_empty() {
            all_results.push(ReferencesResult {
                symbol: input.symbol.clone(),
                reference_count: result.len(),
                references: result,
                project_root: root.display().to_string(),
            });
        }
    }

    if all_results.is_empty() {
        return CallToolResult::success(vec![Content::text(format!(
            "No references found for '{}'. The symbol may not be used anywhere, or try a different name.",
            input.symbol
        ))]);
    }

    let json = serde_json::to_string(&all_results).unwrap_or_default();
    CallToolResult::success(vec![Content::text(json)])
}

/// Read context lines around a specific line
fn read_context(file: &std::path::Path, line: usize, context: usize) -> Option<String> {
    use std::io::BufRead;

    let f = std::fs::File::open(file).ok()?;
    let reader = std::io::BufReader::new(f);
    let lines: Vec<String> = reader.lines().map_while(Result::ok).collect();

    let start = line.saturating_sub(context + 1);
    let end = (line + context).min(lines.len());

    if start >= lines.len() {
        return None;
    }

    let context_lines: Vec<String> = (start..end)
        .map(|i| format!("{:4} | {}", i + 1, &lines[i]))
        .collect();

    Some(context_lines.join("\n"))
}
