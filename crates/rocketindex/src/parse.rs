//! Symbol extraction from source files using tree-sitter.
//!
//! This module defines the common interface for parsing and extracting symbols
//! from different languages.

use std::path::Path;

use crate::languages::{fsharp, python, ruby};
use crate::{Location, Reference, Symbol};

/// A syntax error detected during parsing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyntaxError {
    /// Error message describing the issue
    pub message: String,
    /// Location in the source file
    pub location: Location,
}

/// A warning generated during parsing (non-fatal issues).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseWarning {
    /// Warning message describing the issue
    pub message: String,
    /// Optional location in the source file
    pub location: Option<Location>,
}

/// Result of extracting symbols from a single file.
#[derive(Debug, Clone, Default)]
pub struct ParseResult {
    /// Symbols defined in this file
    pub symbols: Vec<Symbol>,
    /// References to symbols (identifiers used)
    pub references: Vec<Reference>,
    /// Module opens/imports in this file
    pub opens: Vec<String>,
    /// The module/namespace path for this file
    pub module_path: Option<String>,
    /// Syntax errors detected during parsing
    pub errors: Vec<SyntaxError>,
    /// Warnings generated during parsing (non-fatal issues like depth limits)
    pub warnings: Vec<ParseWarning>,
}

/// Trait for language-specific parsers.
pub trait LanguageParser: Send + Sync {
    fn extract_symbols(&self, file: &Path, source: &str, max_depth: usize) -> ParseResult;
}

/// Extract symbols and references from source code.
///
/// Dispatches to the appropriate language parser based on file extension.
///
/// # Arguments
/// * `file` - Path to the source file (for location tracking and language detection)
/// * `source` - The source code content
/// * `max_depth` - Maximum recursion depth for parsing
///
/// # Returns
/// A `ParseResult` containing all extracted symbols, references, and syntax errors.
pub fn extract_symbols(file: &Path, source: &str, max_depth: usize) -> ParseResult {
    let extension = file
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_lowercase();

    match extension.as_str() {
        "fs" | "fsi" | "fsx" => fsharp::FSharpParser.extract_symbols(file, source, max_depth),
        "rb" => ruby::RubyParser.extract_symbols(file, source, max_depth),
        "py" => python::parser::PythonParser.extract_symbols(file, source, max_depth),
        _ => {
            tracing::warn!("Unsupported file extension: {}", extension);
            ParseResult::default()
        }
    }
}

/// Convert a tree-sitter node position to our Location type.
pub fn node_to_location(file: &Path, node: &tree_sitter::Node) -> Location {
    let start = node.start_position();
    let end = node.end_position();
    Location::with_end(
        file.to_path_buf(),
        (start.row + 1) as u32,    // Convert to 1-indexed
        (start.column + 1) as u32, // Convert to 1-indexed
        (end.row + 1) as u32,      // Convert to 1-indexed
        (end.column + 1) as u32,   // Convert to 1-indexed
    )
}

/// Find a child node by its kind.
pub fn find_child_by_kind<'a>(
    node: &'a tree_sitter::Node<'a>,
    kind: &str,
) -> Option<tree_sitter::Node<'a>> {
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            if child.kind() == kind {
                return Some(child);
            }
        }
    }
    None
}
