//! fsharp-index: Core library for F# symbol extraction, indexing, and name resolution
//!
//! This crate provides the fundamental building blocks for a minimal F# language server:
//! - Symbol extraction from F# source files using tree-sitter
//! - In-memory index for fast symbol lookup
//! - Name resolution with F# scoping rules
//! - Dependency graph traversal (spider)

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub mod db;
pub mod fsproj;
pub mod index;
pub mod parse;
pub mod resolve;
pub mod spider;
pub mod type_cache;
pub mod watch;

// Re-export main types
pub use db::SqliteIndex;
pub use fsproj::{find_fsproj_files, parse_fsproj, FsprojInfo};
pub use index::{CodeIndex, Reference};
pub use parse::extract_symbols;
pub use resolve::ResolveResult;
pub use type_cache::{MemberKind, TypeCache, TypeCacheSchema, TypeMember, TypedSymbol};

/// A location in source code (file, line, column) with start and end positions
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Location {
    pub file: PathBuf,
    pub line: u32,       // 1-indexed start line
    pub column: u32,     // 1-indexed start column
    pub end_line: u32,   // 1-indexed end line
    pub end_column: u32, // 1-indexed end column
}

impl Location {
    pub fn new(file: PathBuf, line: u32, column: u32) -> Self {
        Self {
            file,
            line,
            column,
            end_line: line,
            end_column: column,
        }
    }

    /// Create a location with explicit start and end positions
    pub fn with_end(file: PathBuf, line: u32, column: u32, end_line: u32, end_column: u32) -> Self {
        Self {
            file,
            line,
            column,
            end_line,
            end_column,
        }
    }
}

/// The kind of symbol (function, type, module, etc.)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SymbolKind {
    Module,
    Function,
    Value,
    Type,
    Record,
    Union,
    Interface,
    Class,
    Member,
}

impl std::fmt::Display for SymbolKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SymbolKind::Module => write!(f, "Module"),
            SymbolKind::Function => write!(f, "Function"),
            SymbolKind::Value => write!(f, "Value"),
            SymbolKind::Type => write!(f, "Type"),
            SymbolKind::Record => write!(f, "Record"),
            SymbolKind::Union => write!(f, "Union"),
            SymbolKind::Interface => write!(f, "Interface"),
            SymbolKind::Class => write!(f, "Class"),
            SymbolKind::Member => write!(f, "Member"),
        }
    }
}

/// Visibility of a symbol
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Visibility {
    #[default]
    Public,
    Internal,
    Private,
}

/// A symbol extracted from F# source code
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Symbol {
    /// Short name: "processPayment"
    pub name: String,
    /// Full path: "MyApp.Services.PaymentService.processPayment"
    pub qualified: String,
    /// Kind of symbol
    pub kind: SymbolKind,
    /// Source location
    pub location: Location,
    /// Visibility modifier
    pub visibility: Visibility,
}

impl Symbol {
    pub fn new(
        name: String,
        qualified: String,
        kind: SymbolKind,
        location: Location,
        visibility: Visibility,
    ) -> Self {
        Self {
            name,
            qualified,
            kind,
            location,
            visibility,
        }
    }
}

/// Errors that can occur during indexing
#[derive(Debug, thiserror::Error)]
pub enum IndexError {
    #[error("Failed to read file: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Failed to parse file: {path}")]
    ParseError { path: PathBuf },

    #[error("Index not found. Run 'fsharp-index build' first.")]
    IndexNotFound,

    #[error("Failed to serialize index: {0}")]
    SerializationError(#[from] serde_json::Error),

    #[error("Symbol not found: {0}")]
    SymbolNotFound(String),

    #[error("Database error: {0}")]
    DatabaseError(#[from] rusqlite::Error),
}

pub type Result<T> = std::result::Result<T, IndexError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_location_creation() {
        let loc = Location::new(PathBuf::from("test.fs"), 10, 5);
        assert_eq!(loc.line, 10);
        assert_eq!(loc.column, 5);
        // Default end position equals start position
        assert_eq!(loc.end_line, 10);
        assert_eq!(loc.end_column, 5);
    }

    #[test]
    fn test_location_with_end() {
        let loc = Location::with_end(PathBuf::from("test.fs"), 10, 5, 10, 15);
        assert_eq!(loc.line, 10);
        assert_eq!(loc.column, 5);
        assert_eq!(loc.end_line, 10);
        assert_eq!(loc.end_column, 15);
    }

    #[test]
    fn test_symbol_kind_display() {
        assert_eq!(format!("{}", SymbolKind::Function), "Function");
        assert_eq!(format!("{}", SymbolKind::Module), "Module");
    }

    #[test]
    fn test_symbol_creation() {
        let sym = Symbol::new(
            "add".to_string(),
            "MyApp.Math.add".to_string(),
            SymbolKind::Function,
            Location::new(PathBuf::from("math.fs"), 5, 1),
            Visibility::Public,
        );
        assert_eq!(sym.name, "add");
        assert_eq!(sym.qualified, "MyApp.Math.add");
    }
}
