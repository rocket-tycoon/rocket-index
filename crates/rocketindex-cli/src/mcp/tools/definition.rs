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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub match_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
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
        let path = std::path::Path::new(file);
        match manager.project_for_file(path).await {
            Some(root) => Some(root),
            None => {
                // If file is provided but not in any project, suggest JIT
                return CallToolResult::error(vec![Content::text(format!(
                    "File '{}' is not part of any registered project. Use `describe_project` on the project root to index it.",
                    file
                ))]);
            }
        }
    } else {
        None
    };

    // Search for the symbol
    let (results, is_fuzzy) = if let Some(root) = project_root {
        // Search specific project
        let (res, fuzzy) = manager
            .with_project(&root, |state| {
                // Try exact match first
                if let Ok(Some(sym)) = state.sqlite.find_by_qualified(&input.symbol) {
                    return (vec![sym], false);
                }
                // Fall back to fuzzy search
                let fuzzy_results = state
                    .sqlite
                    .fuzzy_search(&input.symbol, 3, 5, None)
                    .map(|res| res.into_iter().map(|(s, _)| s).collect())
                    .unwrap_or_default();
                (fuzzy_results, true)
            })
            .await
            .unwrap_or_default();

        if res.is_empty() {
            (vec![], false)
        } else {
            (res.into_iter().map(|s| (root.clone(), s)).collect(), fuzzy)
        }
    } else {
        // Search all projects
        // Try exact match first across all projects
        let exact_results = manager.find_definition_all(&input.symbol).await;
        if !exact_results.is_empty() {
            (exact_results, false)
        } else {
            // Fall back to broad fuzzy search
            // We'll search top 5 fuzzy matches across all
            let fuzzy = manager.fuzzy_search_all_projects(&input.symbol, 5).await;
            // Flatten results
            let mut flat = Vec::new();
            for (root, syms) in fuzzy {
                for sym in syms {
                    flat.push((root.clone(), sym));
                }
            }
            (flat, true)
        }
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

        // Check for staleness
        let warning = check_staleness(&sym.location.file, &root);

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
            match_type: Some(if is_fuzzy {
                "fuzzy".to_string()
            } else {
                "exact".to_string()
            }),
            confidence: Some(if is_fuzzy { 0.5 } else { 1.0 }),
            warning,
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

/// Check if a file is newer than the index database
fn check_staleness(file: &std::path::Path, project_root: &std::path::Path) -> Option<String> {
    let index_path = project_root.join(".rocketindex").join("index.db");

    if let (Ok(file_meta), Ok(index_meta)) =
        (std::fs::metadata(file), std::fs::metadata(index_path))
    {
        if let (Ok(file_time), Ok(index_time)) = (file_meta.modified(), index_meta.modified()) {
            if file_time > index_time {
                // Approximate time diff? For now just warned.
                return Some(
                    "Warning: Index may be stale for this file (modified since last index)"
                        .to_string(),
                );
            }
        }
    }
    None
}
