//! describe_project tool - semantic project map
//!
//! Provides a high-level overview of the project structure, listing files
//! and top-level symbols (Classes, Modules) to help the agent orient itself.
//!
//! Supports three detail levels:
//! - `summary`: Top N most important symbols across the project (default)
//! - `normal`: All files with ranked symbols (limited per file)
//! - `full`: Original behavior with all symbols

use rmcp::model::{CallToolResult, Content};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use crate::mcp::ProjectManager;
use rocketindex::{ranking::group_by_file, DetailLevel, RankedSymbol, Symbol, SymbolKind};

/// Default maximum symbols to show in summary/normal modes
const DEFAULT_MAX_SYMBOLS: usize = 50;

/// Maximum symbols per file in normal mode
const MAX_SYMBOLS_PER_FILE: usize = 5;

/// Format a single ranked symbol as a markdown list item
fn format_symbol_item(ranked: &RankedSymbol, compact: bool) -> String {
    let sym = &ranked.symbol;
    let kind_str = format!("{:?}", sym.kind);

    let ref_info = if ranked.file_diversity > 0 {
        if compact {
            format!(" - {} files", ranked.file_diversity)
        } else {
            format!(" - referenced by {} files", ranked.file_diversity)
        }
    } else {
        String::new()
    };

    let extra = if !compact {
        if let Some(sig) = &sym.signature {
            format!(" `{}`", sig)
        } else if let Some(doc) = &sym.doc {
            doc.lines()
                .next()
                .map(|s| format!(" - {}", s))
                .unwrap_or_default()
        } else {
            String::new()
        }
    } else {
        String::new()
    };

    format!("- `{}` ({}){}{}\n", sym.name, kind_str, ref_info, extra)
}

/// Format a plain symbol (non-ranked) as a markdown list item
fn format_plain_symbol(sym: &Symbol) -> String {
    let kind_str = format!("{:?}", sym.kind);

    let extra = if let Some(sig) = &sym.signature {
        format!(" `{}`", sig)
    } else if let Some(doc) = &sym.doc {
        doc.lines()
            .next()
            .map(|s| format!(" - {}", s))
            .unwrap_or_default()
    } else {
        String::new()
    };

    format!("- `{}` ({}){}\n", sym.name, kind_str, extra)
}

/// Input for describe_project tool
#[derive(Debug, Deserialize)]
pub struct DescribeProjectInput {
    /// Path to the project root directory we want to describe (or any subdirectory)
    pub path: String,

    /// Detail level: "summary" (default), "normal", or "full"
    #[serde(default)]
    pub detail: Option<String>,

    /// Maximum symbols to show (default: 50 for summary/normal, unlimited for full)
    pub max_symbols: Option<usize>,
}

/// Execute the describe_project tool
pub async fn describe_project(
    manager: Arc<ProjectManager>,
    input: DescribeProjectInput,
) -> CallToolResult {
    let path = PathBuf::from(&input.path);
    let detail = input
        .detail
        .as_deref()
        .map(DetailLevel::parse)
        .unwrap_or_default();
    let max_symbols = input.max_symbols.unwrap_or(DEFAULT_MAX_SYMBOLS);

    // Find the project root for this path
    // SECURITY: We do NOT JIT-index arbitrary paths. Only registered projects are accessible.
    let project_root = match manager.project_for_file(&path).await {
        Some(root) => root,
        None => {
            if manager.has_project(&path).await {
                path
            } else {
                return CallToolResult::error(vec![Content::text(format!(
                    "Path '{}' is not a registered project. Register it first with 'rkt serve add {}' or run 'rkt index' in that directory.",
                    input.path, input.path
                ))]);
            }
        }
    };

    let result = manager
        .with_project(&project_root, |state| match detail {
            DetailLevel::Summary => format_ranked_summary(&project_root, state, max_symbols),
            DetailLevel::Normal => format_ranked_by_file(&project_root, state, max_symbols),
            DetailLevel::Full => format_full(&project_root, state),
        })
        .await
        .unwrap_or_else(|| "Failed to access project state".to_string());

    CallToolResult::success(vec![Content::text(result)])
}

/// Format output for summary mode - top N most important symbols
fn format_ranked_summary(
    project_root: &PathBuf,
    state: &crate::mcp::project_manager::ProjectState,
    max_symbols: usize,
) -> String {
    let mut output = String::new();
    output.push_str(&format!("# Project Map: {}\n\n", project_root.display()));

    match state.sqlite.rank_symbols(max_symbols) {
        Ok(ranked) => {
            if ranked.is_empty() {
                output.push_str("No symbols found in the project.\n");
                return output;
            }

            output.push_str("## Most Important Symbols\n\n");

            // Group by file for readability
            let grouped = group_by_file(ranked);

            for (file, symbols) in grouped {
                let rel_path = file.strip_prefix(project_root).unwrap_or(&file).display();
                output.push_str(&format!("### {}\n", rel_path));

                for ranked_sym in &symbols {
                    output.push_str(&format_symbol_item(ranked_sym, false));
                }
                output.push('\n');
            }

            output.push_str(&format!("(showing top {} symbols)\n", max_symbols));
        }
        Err(e) => {
            output.push_str(&format!("Failed to rank symbols: {}\n", e));
        }
    }

    output
}

/// Format output for normal mode - top N ranked symbols per file
///
/// Uses a window function query to get the top N most important symbols
/// in each file, avoiding the logic defect of filtering a global top N.
fn format_ranked_by_file(
    project_root: &PathBuf,
    state: &crate::mcp::project_manager::ProjectState,
    max_files: usize,
) -> String {
    let mut output = String::new();
    output.push_str(&format!("# Project Map: {}\n\n", project_root.display()));

    // Get top N symbols per file using window function
    match state
        .sqlite
        .rank_symbols_per_file(MAX_SYMBOLS_PER_FILE, max_files)
    {
        Ok(ranked) => {
            if ranked.is_empty() {
                output.push_str("No symbols found in the project.\n");
                return output;
            }

            // Group by file for display
            let grouped = group_by_file(ranked);

            for (file, symbols) in grouped {
                let rel_path = file.strip_prefix(project_root).unwrap_or(&file).display();
                output.push_str(&format!("## {}\n", rel_path));

                for ranked_sym in &symbols {
                    output.push_str(&format_symbol_item(ranked_sym, true));
                }
                output.push('\n');
            }
        }
        Err(e) => {
            output.push_str(&format!("Failed to rank symbols: {}\n", e));
        }
    }

    output
}

/// Format output for full mode - all symbols without ranking
///
/// Uses a single query to fetch all symbols, avoiding N+1 query problem.
fn format_full(
    project_root: &PathBuf,
    state: &crate::mcp::project_manager::ProjectState,
) -> String {
    // Single query for all symbols, ordered by file and line
    let all_symbols = match state.sqlite.get_all_symbols_ordered() {
        Ok(syms) => syms,
        Err(e) => return format!("Failed to fetch symbols: {}", e),
    };

    let mut output = String::new();
    output.push_str(&format!("# Project Map: {}\n\n", project_root.display()));

    // Group symbols by file (preserves order since query is already sorted)
    let mut file_map: BTreeMap<PathBuf, Vec<&Symbol>> = BTreeMap::new();
    for sym in &all_symbols {
        let rel_path = sym
            .location
            .file
            .strip_prefix(project_root)
            .unwrap_or(&sym.location.file)
            .to_path_buf();
        file_map.entry(rel_path).or_default().push(sym);
    }

    // Render Markdown
    for (rel_path, symbols) in file_map {
        output.push_str(&format!("## {}\n", rel_path.display()));

        // Filter to significant top-level symbols
        let significant: Vec<_> = symbols
            .iter()
            .filter(|sym| {
                matches!(
                    sym.kind,
                    SymbolKind::Class
                        | SymbolKind::Module
                        | SymbolKind::Interface
                        | SymbolKind::Record
                        | SymbolKind::Union
                )
            })
            .collect();

        if significant.is_empty() {
            output.push_str("(No top-level symbols)\n");
        } else {
            const MAX_SYMBOLS: usize = 20;
            for sym in significant.iter().take(MAX_SYMBOLS) {
                output.push_str(&format_plain_symbol(sym));
            }
            if significant.len() > MAX_SYMBOLS {
                output.push_str(&format!(
                    "... and {} more\n",
                    significant.len() - MAX_SYMBOLS
                ));
            }
        }
        output.push('\n');
    }

    output
}
