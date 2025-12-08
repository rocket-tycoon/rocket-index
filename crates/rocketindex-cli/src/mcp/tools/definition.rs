//! find_definition tool - wraps `rkt def`

use rmcp::model::{CallToolResult, Content};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::mcp::ProjectManager;

/// Input for find_definition tool
#[derive(Debug, Deserialize)]
pub struct FindDefinitionInput {
    /// Symbol name (qualified like "MyModule.function" or short like "function")
    pub symbol: String,
    /// Optional file context for resolution hints
    pub file: Option<String>,
    /// Optional explicit project root
    pub project_root: Option<String>,
    /// Include source context line
    #[serde(default)]
    pub include_context: bool,
}

/// Output for find_definition tool
#[derive(Debug, Serialize)]
pub struct DefinitionResult {
    pub qualified: String,
    pub name: String,
    pub kind: String,
    pub file: String,
    pub line: u32,
    pub column: u32,
    pub language: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_line: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub doc: Option<String>,
    pub project_root: String,
}

/// Execute the find_definition tool
pub async fn find_definition(
    manager: Arc<ProjectManager>,
    input: FindDefinitionInput,
) -> CallToolResult {
    // Determine which project to search
    let project_root = if let Some(ref root) = input.project_root {
        Some(std::path::PathBuf::from(root))
    } else if let Some(ref file) = input.file {
        manager.project_for_file(std::path::Path::new(file)).await
    } else {
        None
    };

    // Search for the symbol
    let results = if let Some(root) = project_root {
        // Search specific project
        let result = manager
            .with_project(&root, |state| {
                // Try exact match first
                if let Ok(Some(sym)) = state.sqlite.find_by_qualified(&input.symbol) {
                    return vec![sym];
                }
                // Fall back to search
                state
                    .sqlite
                    .search(&input.symbol, 10, None)
                    .unwrap_or_default()
            })
            .await
            .unwrap_or_default();

        if result.is_empty() {
            vec![]
        } else {
            result.into_iter().map(|s| (root.clone(), s)).collect()
        }
    } else {
        // Search all projects
        manager.find_definition_all(&input.symbol).await
    };

    if results.is_empty() {
        return CallToolResult::error(vec![Content::text(format!(
            "Symbol '{}' not found. Try using `search_symbols` to find similar symbols.",
            input.symbol
        ))]);
    }

    // Convert to output format
    let mut output_results = Vec::new();
    for (root, sym) in results {
        let context_line = if input.include_context {
            read_context_line(&sym.location.file, sym.location.line as usize)
        } else {
            None
        };

        output_results.push(DefinitionResult {
            qualified: sym.qualified,
            name: sym.name,
            kind: format!("{:?}", sym.kind),
            file: sym.location.file.display().to_string(),
            line: sym.location.line,
            column: sym.location.column,
            language: sym.language,
            context_line,
            signature: sym.signature,
            doc: sym.doc,
            project_root: root.display().to_string(),
        });
    }

    let json = serde_json::to_string_pretty(&output_results).unwrap_or_default();
    CallToolResult::success(vec![Content::text(json)])
}

/// Read a single line from a file for context
fn read_context_line(file: &std::path::Path, line: usize) -> Option<String> {
    use std::io::BufRead;

    let f = std::fs::File::open(file).ok()?;
    let reader = std::io::BufReader::new(f);
    reader
        .lines()
        .nth(line.saturating_sub(1))
        .and_then(|l| l.ok())
}
