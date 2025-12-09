//! enrich_symbol tool - wraps `rkt enrich`

use rmcp::model::{CallToolResult, Content};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::mcp::ProjectManager;

/// Input for enrich_symbol tool
#[derive(Debug, Deserialize)]
pub struct EnrichSymbolInput {
    /// Symbol to enrich (qualified name preferred)
    pub symbol: String,
    /// Optional project root
    pub project_root: Option<String>,
}

/// Output for enrich_symbol tool
#[derive(Debug, Serialize)]
pub struct EnrichedSymbol {
    pub qualified: String,
    pub name: String,
    pub kind: String,
    pub file: String,
    pub line: u32,
    pub column: u32,
    pub language: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub doc: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_context: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub git_blame: Option<GitBlame>,
    pub callers: Vec<CallerSummary>,
    pub callees: Vec<CalleeSummary>,
    pub project_root: String,
}

/// Git blame information
#[derive(Debug, Serialize)]
pub struct GitBlame {
    pub author: String,
    pub date: String,
    pub commit: String,
    pub message: String,
}

/// Summary of a caller
#[derive(Debug, Serialize)]
pub struct CallerSummary {
    pub symbol: String,
    pub file: String,
    pub line: u32,
}

/// Summary of a callee (dependency)
#[derive(Debug, Serialize)]
pub struct CalleeSummary {
    pub symbol: String,
    pub file: String,
    pub line: u32,
}

/// Execute the enrich_symbol tool
pub async fn enrich_symbol(
    manager: Arc<ProjectManager>,
    input: EnrichSymbolInput,
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

    for root in project_roots {
        let result = manager
            .with_project(&root, |state| {
                // Find the symbol definition first
                let symbol = if let Ok(Some(sym)) = state.sqlite.find_by_qualified(&input.symbol) {
                    sym
                } else {
                    // Try search as fallback
                    let results = state
                        .sqlite
                        .search(&input.symbol, 1, None)
                        .unwrap_or_default();
                    if results.is_empty() {
                        return None;
                    }
                    results.into_iter().next()?
                };

                // Get source context (few lines around definition)
                let source_context =
                    read_source_context(&symbol.location.file, symbol.location.line as usize, 3);

                // Get git blame for the definition line
                let git_blame = get_git_blame(&symbol.location.file, symbol.location.line);

                // Get callers (depth 1 reverse spider)
                let callers = {
                    use rocketindex::spider::reverse_spider;
                    let tree = reverse_spider(&state.code_index, &symbol.qualified, 1);
                    tree.nodes
                        .into_iter()
                        .filter(|n| n.depth == 1)
                        .map(|n| CallerSummary {
                            symbol: n.symbol.qualified,
                            file: n.symbol.location.file.display().to_string(),
                            line: n.symbol.location.line,
                        })
                        .collect()
                };

                // Get callees (depth 1 forward spider)
                let callees = {
                    use rocketindex::spider::spider;
                    let tree = spider(&state.code_index, &symbol.qualified, 1);
                    tree.nodes
                        .into_iter()
                        .filter(|n| n.depth == 1)
                        .map(|n| CalleeSummary {
                            symbol: n.symbol.qualified,
                            file: n.symbol.location.file.display().to_string(),
                            line: n.symbol.location.line,
                        })
                        .collect()
                };

                Some(EnrichedSymbol {
                    qualified: symbol.qualified,
                    name: symbol.name,
                    kind: format!("{:?}", symbol.kind),
                    file: symbol.location.file.display().to_string(),
                    line: symbol.location.line,
                    column: symbol.location.column,
                    language: symbol.language,
                    signature: symbol.signature,
                    doc: symbol.doc,
                    source_context,
                    git_blame,
                    callers,
                    callees,
                    project_root: root.display().to_string(),
                })
            })
            .await
            .flatten();

        if let Some(enriched) = result {
            let json = serde_json::to_string_pretty(&enriched).unwrap_or_default();
            return CallToolResult::success(vec![Content::text(json)]);
        }
    }

    CallToolResult::error(vec![Content::text(format!(
        "Symbol '{}' not found. Try using `search_symbols` to find similar symbols.",
        input.symbol
    ))])
}

/// Read source context around a line
fn read_source_context(file: &std::path::Path, line: usize, context: usize) -> Option<String> {
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
        .map(|i| {
            let marker = if i + 1 == line { ">" } else { " " };
            format!("{}{:4} | {}", marker, i + 1, &lines[i])
        })
        .collect();

    Some(context_lines.join("\n"))
}

/// Get git blame for a specific line
fn get_git_blame(file: &std::path::Path, line: u32) -> Option<GitBlame> {
    use std::process::Command;

    let output = Command::new("git")
        .args([
            "blame",
            "-L",
            &format!("{},{}", line, line),
            "--porcelain",
            file.to_str()?,
        ])
        .current_dir(file.parent()?)
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut author = String::new();
    let mut date = String::new();
    let mut commit = String::new();
    let mut message = String::new();

    for line in stdout.lines() {
        if commit.is_empty() && line.len() >= 40 {
            commit = line[..40].to_string();
        } else if let Some(rest) = line.strip_prefix("author ") {
            author = rest.to_string();
        } else if let Some(rest) = line.strip_prefix("author-time ") {
            // Convert Unix timestamp to readable date
            if let Ok(ts) = rest.parse::<i64>() {
                date = chrono_format_timestamp(ts);
            }
        } else if let Some(rest) = line.strip_prefix("summary ") {
            message = rest.to_string();
        }
    }

    if author.is_empty() {
        return None;
    }

    Some(GitBlame {
        author,
        date,
        commit,
        message,
    })
}

/// Format a Unix timestamp as a readable date
fn chrono_format_timestamp(ts: i64) -> String {
    // Simple formatting without chrono dependency
    use std::time::{Duration, UNIX_EPOCH};
    let datetime = UNIX_EPOCH + Duration::from_secs(ts as u64);
    format!("{:?}", datetime)
}
