//! Name resolution for symbols with scope rules.
//!
//! This module implements name resolution, taking into account:
//! - Imports (open/require)
//! - Module hierarchy
//! - Qualified vs unqualified names
//! - Language-specific scoping rules

use std::path::Path;

use crate::languages::{csharp, fsharp, java, javascript, ruby, typescript};
use crate::type_cache::TypeMember;
use crate::{CodeIndex, Symbol};

/// Result of name resolution
#[derive(Debug, Clone)]
pub struct ResolveResult<'a> {
    /// The resolved symbol
    pub symbol: &'a Symbol,
    /// How the symbol was resolved
    pub resolution_path: ResolutionPath,
}

/// How a symbol was resolved
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolutionPath {
    /// Direct qualified name match
    Qualified,
    /// Resolved via an open statement (F#) or require (Ruby)
    ViaOpen(String),
    /// Resolved from the same module
    SameModule,
    /// Resolved from a parent module
    ParentModule(String),
    /// Resolved via type-aware member access (RFC-001)
    /// Contains the type name that the member was resolved on
    ViaMemberAccess { type_name: String },
}

/// Result of resolving a member access expression (e.g., `user.Name`)
#[derive(Debug, Clone)]
pub struct MemberResolveResult<'a> {
    /// The resolved type member
    pub member: &'a TypeMember,
    /// The type that contains this member
    pub type_name: String,
}

/// Trait for language-specific symbol resolution.
pub trait SymbolResolver: Send + Sync {
    fn resolve<'a>(
        &self,
        index: &'a CodeIndex,
        name: &str,
        from_file: &Path,
    ) -> Option<ResolveResult<'a>>;

    fn resolve_dotted<'a>(
        &self,
        index: &'a CodeIndex,
        name: &str,
        from_file: &Path,
    ) -> Option<ResolveResult<'a>> {
        self.resolve(index, name, from_file)
    }
}

impl CodeIndex {
    /// Resolve a symbol name from a given file context.
    ///
    /// Dispatches to the appropriate language resolver based on file extension.
    ///
    /// # Arguments
    /// * `name` - The name to resolve
    /// * `from_file` - The file context for resolution
    ///
    /// # Returns
    /// The resolved symbol if found, None otherwise
    #[must_use]
    pub fn resolve(&self, name: &str, from_file: &Path) -> Option<ResolveResult<'_>> {
        let extension = from_file
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or_default()
            .to_lowercase();

        match extension.as_str() {
            "cs" => csharp::CSharpResolver.resolve(self, name, from_file),
            "fs" | "fsi" | "fsx" => fsharp::FSharpResolver.resolve(self, name, from_file),
            "java" => java::JavaResolver.resolve(self, name, from_file),
            "rb" => ruby::RubyResolver.resolve(self, name, from_file),
            "ts" | "tsx" => typescript::TypeScriptResolver.resolve(self, name, from_file),
            "js" | "jsx" | "mjs" | "cjs" => {
                javascript::JavaScriptResolver.resolve(self, name, from_file)
            }
            // "go" => go::GoResolver.resolve(self, name, from_file), // Incomplete
            _ => None,
        }
    }

    /// Resolve a dotted name like "PaymentService.processPayment"
    #[must_use]
    pub fn resolve_dotted(&self, name: &str, from_file: &Path) -> Option<ResolveResult<'_>> {
        let extension = from_file
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or_default()
            .to_lowercase();

        match extension.as_str() {
            "cs" => csharp::CSharpResolver.resolve_dotted(self, name, from_file),
            "fs" | "fsi" | "fsx" => fsharp::FSharpResolver.resolve_dotted(self, name, from_file),
            "java" => java::JavaResolver.resolve_dotted(self, name, from_file),
            "rb" => ruby::RubyResolver.resolve_dotted(self, name, from_file),
            "ts" | "tsx" => typescript::TypeScriptResolver.resolve_dotted(self, name, from_file),
            "js" | "jsx" | "mjs" | "cjs" => {
                javascript::JavaScriptResolver.resolve_dotted(self, name, from_file)
            }
            // "go" => go::GoResolver.resolve_dotted(self, name, from_file), // Incomplete
            _ => None,
        }
    }
}
