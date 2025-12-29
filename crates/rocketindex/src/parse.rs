//! Symbol extraction from source files using tree-sitter.
//!
//! This module defines the common interface for parsing and extracting symbols
//! from different languages.
//!
//! # Examples
//!
//! Extract symbols from Python source code:
//!
//! ```
//! use rocketindex::extract_symbols;
//! use std::path::Path;
//!
//! let source = r#"
//! class User:
//!     def __init__(self, name):
//!         self.name = name
//!
//!     def greet(self):
//!         return f"Hello, {self.name}"
//! "#;
//!
//! let result = extract_symbols(Path::new("user.py"), source, 100);
//!
//! // Should find the User class and its methods
//! assert!(result.symbols.iter().any(|s| s.name == "User"));
//! assert!(result.symbols.iter().any(|s| s.name == "__init__"));
//! assert!(result.symbols.iter().any(|s| s.name == "greet"));
//! ```
//!
//! The file extension determines which parser is used:
//!
//! ```
//! use rocketindex::extract_symbols;
//! use std::path::Path;
//!
//! // Rust code
//! let rust_result = extract_symbols(
//!     Path::new("lib.rs"),
//!     "pub fn hello() {}",
//!     100
//! );
//! assert_eq!(rust_result.symbols[0].name, "hello");
//!
//! // Go code
//! let go_result = extract_symbols(
//!     Path::new("main.go"),
//!     "package main\nfunc Hello() {}",
//!     100
//! );
//! assert_eq!(go_result.symbols[0].name, "Hello");
//! ```

use std::path::Path;

use crate::languages::{
    c, cpp, csharp, fsharp, go, haxe, java, javascript, kotlin, objc, php, python, ruby, rust,
    swift, typescript,
};
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
        "c" | "h" => c::CParser.extract_symbols(file, source, max_depth),
        "cpp" | "cc" | "cxx" | "hpp" | "hxx" | "hh" => {
            cpp::CppParser.extract_symbols(file, source, max_depth)
        }
        "fs" | "fsi" | "fsx" => fsharp::FSharpParser.extract_symbols(file, source, max_depth),
        "rb" => ruby::RubyParser.extract_symbols(file, source, max_depth),
        "py" | "pyi" => python::parser::PythonParser.extract_symbols(file, source, max_depth),
        "rs" => rust::RustParser.extract_symbols(file, source, max_depth),
        "go" => go::GoParser.extract_symbols(file, source, max_depth),
        "java" => java::JavaParser.extract_symbols(file, source, max_depth),
        "kt" | "kts" => kotlin::KotlinParser.extract_symbols(file, source, max_depth),
        "m" | "mm" => objc::ObjCParser.extract_symbols(file, source, max_depth),
        "swift" => swift::SwiftParser.extract_symbols(file, source, max_depth),
        "cs" => csharp::CSharpParser.extract_symbols(file, source, max_depth),
        "ts" | "tsx" => typescript::TypeScriptParser.extract_symbols(file, source, max_depth),
        "js" | "jsx" | "mjs" | "cjs" => {
            javascript::JavaScriptParser.extract_symbols(file, source, max_depth)
        }
        "php" => php::PhpParser.extract_symbols(file, source, max_depth),
        "hx" => haxe::HaxeParser.extract_symbols(file, source, max_depth),
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
/// Uses cursor-based iteration for O(n) instead of O(n²) performance.
pub fn find_child_by_kind<'a>(
    node: &tree_sitter::Node<'a>,
    kind: &str,
) -> Option<tree_sitter::Node<'a>> {
    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            if cursor.node().kind() == kind {
                return Some(cursor.node());
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
    None
}
