//! describe_project tool - semantic project map
//!
//! Provides a high-level overview of the project structure, listing files
//! and top-level symbols (Classes, Modules) to help the agent orient itself.

use rmcp::model::{CallToolResult, Content};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use crate::mcp::ProjectManager;
use rocketindex::SymbolKind;

/// Input for describe_project tool
#[derive(Debug, Deserialize)]
pub struct DescribeProjectInput {
    /// Path to the project root directory we want to describe (or any subdirectory)
    pub path: String,
}

/// Execute the describe_project tool
pub async fn describe_project(
    manager: Arc<ProjectManager>,
    input: DescribeProjectInput,
) -> CallToolResult {
    let path = PathBuf::from(&input.path);

    // Find the project root for this path
    // We allow passing a subdirectory, but we generally want the root
    // SECURITY: We do NOT JIT-index arbitrary paths. Only registered projects are accessible.
    let project_root = match manager.project_for_file(&path).await {
        Some(root) => root,
        None => {
            // Check if it is a registered root itself
            if manager.has_project(&path).await {
                path
            } else {
                // SECURITY: Reject unregistered paths to prevent arbitrary directory access
                return CallToolResult::error(vec![Content::text(format!(
                    "Path '{}' is not a registered project. Register it first with 'rkt serve add {}' or run 'rkt index' in that directory.",
                    input.path, input.path
                ))]);
            }
        }
    };

    let result = manager
        .with_project(&project_root, |state| {
            // Get all files
            let files = match state.sqlite.list_files() {
                Ok(f) => f,
                Err(e) => return format!("Failed to list files: {}", e),
            };

            // Build a tree structure
            let mut output = String::new();
            output.push_str(&format!("# Project Map: {}\n\n", project_root.display()));

            // Group by directory for cleaner output?
            // For now, let's just list files and key symbols
            let mut file_map: BTreeMap<PathBuf, Vec<String>> = BTreeMap::new();

            for file_path in files {
                // Get relative path
                let rel_path = file_path
                    .strip_prefix(&project_root)
                    .unwrap_or(&file_path)
                    .to_path_buf();

                let mut symbols_desc = Vec::new();

                // Get top-level symbols
                if let Ok(symbols) = state.sqlite.symbols_in_file(&file_path) {
                    for sym in symbols {
                        // Only show significant top-level symbols
                        let is_significant = match sym.kind {
                            SymbolKind::Class
                            | SymbolKind::Module
                            | SymbolKind::Interface
                            | SymbolKind::Record
                            | SymbolKind::Union => true,
                            SymbolKind::Function => {
                                // Only show top-level functions (not nested methods)
                                // Skip functions to keep output focused on structure
                                false
                            }
                            _ => false,
                        };

                        if is_significant {
                            let signature = sym.signature.as_deref().unwrap_or("");
                            let doc_summary = sym
                                .doc
                                .as_deref()
                                .and_then(|d| d.lines().next()) // First line of doc
                                .unwrap_or("");

                            let kind_str = format!("{:?}", sym.kind);
                            let desc = if !signature.is_empty() {
                                format!("- `{}` ({}) `{}`", sym.name, kind_str, signature)
                            } else if !doc_summary.is_empty() {
                                format!("- `{}` ({}) - {}", sym.name, kind_str, doc_summary)
                            } else {
                                format!("- `{}` ({})", sym.name, kind_str)
                            };
                            symbols_desc.push(desc);
                        }
                    }
                }

                file_map.insert(rel_path, symbols_desc);
            }

            // Render Markdown
            for (rel_path, symbols) in file_map {
                output.push_str(&format!("## {}\n", rel_path.display()));
                if symbols.is_empty() {
                    output.push_str("(No top-level symbols)\n");
                } else {
                    // Limit symbols to prevent massive output
                    const MAX_SYMBOLS: usize = 20;
                    for sym in symbols.iter().take(MAX_SYMBOLS) {
                        output.push_str(sym);
                        output.push('\n');
                    }
                    if symbols.len() > MAX_SYMBOLS {
                        output.push_str(&format!("... and {} more\n", symbols.len() - MAX_SYMBOLS));
                    }
                }
                output.push('\n');
            }

            output
        })
        .await
        .unwrap_or_else(|| "Failed to access project state".to_string());

    CallToolResult::success(vec![Content::text(result)])
}
