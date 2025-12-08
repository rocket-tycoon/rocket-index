//! rocketindex: Rocket-fast symbol extraction, indexing, and name resolution
//!
//! This crate provides the fundamental building blocks for a minimal language server:
//! - Symbol extraction from source files (F#, Ruby) using tree-sitter
//! - In-memory index for fast symbol lookup
//! - Name resolution with language-specific scoping rules
//! - Dependency graph traversal (spider)

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub mod batch;
pub mod config;
pub mod db;
pub mod external_index;
pub mod fsproj;
pub mod fuzzy;
pub mod git;
pub mod index;
pub mod languages;
pub mod parse;
pub mod pidfile;
pub mod resolve;
pub mod spider;
pub mod stacktrace;
pub mod type_cache;
pub mod watch;

// Re-export main types
pub use db::SqliteIndex;
pub use fsproj::{find_fsproj_files, parse_fsproj, FsprojInfo};
pub use index::{CodeIndex, Reference};
pub use parse::{extract_symbols, ParseWarning, SyntaxError};
pub use resolve::ResolveResult;
pub use stacktrace::{parse_stacktrace, StackFrame, StacktraceLanguage, StacktraceResult};
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

fn default_language() -> String {
    "fsharp".to_string()
}

/// A symbol extracted from source code
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
    /// Language of the symbol (e.g., "fsharp", "ruby")
    #[serde(default = "default_language")]
    pub language: String,
    /// Parent class/type (for inheritance relationships)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    /// Included/extended/prepended modules (for Ruby mixins)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mixins: Option<Vec<String>>,
    /// Decorators/attributes applied to the symbol (e.g., F# [<Obsolete>], Python @decorator)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attributes: Option<Vec<String>>,
    /// Interfaces/protocols this type implements (e.g., F# interface IComparable)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub implements: Option<Vec<String>>,
    /// Documentation comment (e.g., F# /// comment, Ruby # comment)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doc: Option<String>,
    /// Type signature (e.g., "int -> int -> int" for F# functions)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
}

impl Symbol {
    pub fn new(
        name: String,
        qualified: String,
        kind: SymbolKind,
        location: Location,
        visibility: Visibility,
        language: String,
    ) -> Self {
        Self {
            name,
            qualified,
            kind,
            location,
            visibility,
            language,
            parent: None,
            mixins: None,
            attributes: None,
            implements: None,
            doc: None,
            signature: None,
        }
    }

    /// Create a symbol with a parent class/type
    pub fn with_parent(mut self, parent: Option<String>) -> Self {
        self.parent = parent;
        self
    }

    /// Create a symbol with mixins (include/extend/prepend)
    pub fn with_mixins(mut self, mixins: Option<Vec<String>>) -> Self {
        self.mixins = mixins;
        self
    }

    /// Create a symbol with attributes/decorators
    pub fn with_attributes(mut self, attributes: Option<Vec<String>>) -> Self {
        self.attributes = attributes;
        self
    }

    /// Create a symbol with interface implementations
    pub fn with_implements(mut self, implements: Option<Vec<String>>) -> Self {
        self.implements = implements;
        self
    }

    /// Create a symbol with documentation
    pub fn with_doc(mut self, doc: Option<String>) -> Self {
        self.doc = doc;
        self
    }

    /// Create a symbol with type signature
    pub fn with_signature(mut self, signature: Option<String>) -> Self {
        self.signature = signature;
        self
    }
}

/// Errors that can occur during indexing
#[derive(Debug, thiserror::Error)]
pub enum IndexError {
    #[error("Failed to read file: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Failed to parse file: {path}")]
    ParseError { path: PathBuf },

    #[error("Index not found. Run 'rocketindex index' first.")]
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
            "fsharp".to_string(),
        );
        assert_eq!(sym.name, "add");
        assert_eq!(sym.qualified, "MyApp.Math.add");
    }
}
