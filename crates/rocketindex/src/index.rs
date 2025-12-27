//! In-memory symbol storage and query index.
//!
//! The [`CodeIndex`] is the central data structure that stores all extracted symbols
//! and provides efficient lookup operations. It's used for name resolution and
//! dependency analysis.
//!
//! For persistent storage, see [`crate::SqliteIndex`].
//!
//! # Examples
//!
//! Build an in-memory index and query symbols:
//!
//! ```
//! use rocketindex::CodeIndex;
//! use rocketindex::{Symbol, SymbolKind, Location, Visibility};
//! use std::path::PathBuf;
//!
//! let mut index = CodeIndex::new();
//!
//! // Add a symbol
//! let symbol = Symbol::new(
//!     "PaymentService".to_string(),
//!     "services.PaymentService".to_string(),
//!     SymbolKind::Class,
//!     Location::new(PathBuf::from("src/payment.rs"), 42, 5),
//!     Visibility::Public,
//!     "rust".to_string(),
//! );
//! index.add_symbol(symbol);
//!
//! // Look up by qualified name
//! let found = index.get("services.PaymentService");
//! assert!(found.is_some());
//! assert_eq!(found.unwrap().name, "PaymentService");
//!
//! // Search with glob patterns (matches symbol name)
//! let results = index.search("Payment*");
//! assert_eq!(results.len(), 1);
//! ```
//!
//! ## Path Handling
//!
//! Internally, all paths are stored relative to the workspace root for portability.
//! When an index is serialized and moved to another machine, paths remain valid
//! as long as the workspace structure is preserved.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::type_cache::{TypeCache, TypeMember};
use crate::{Location, Symbol};

/// A reference to a symbol (an identifier usage, not a definition).
///
/// References track where symbols are used throughout the codebase,
/// enabling "find all usages" functionality.
///
/// # Examples
///
/// ```
/// use rocketindex::Reference;
/// use rocketindex::Location;
/// use std::path::PathBuf;
///
/// let reference = Reference {
///     name: "process_payment".to_string(),
///     location: Location::new(PathBuf::from("src/main.rs"), 25, 10),
/// };
/// assert_eq!(reference.name, "process_payment");
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Reference {
    /// The identifier as written: "List.map" or "processPayment"
    pub name: String,
    /// Where the reference appears (path is relative to workspace root)
    pub location: Location,
}

/// The main index storing all symbols and their relationships.
///
/// All file paths within the index are stored relative to `workspace_root`.
/// This ensures the index is portable across machines.
///
/// # Examples
///
/// ```
/// use rocketindex::CodeIndex;
/// use rocketindex::{Symbol, SymbolKind, Location, Visibility};
/// use std::path::PathBuf;
///
/// // Create an index with a workspace root
/// let mut index = CodeIndex::with_root(PathBuf::from("/project"));
///
/// // Add symbols
/// let symbol = Symbol::new(
///     "User".to_string(),
///     "models.User".to_string(),
///     SymbolKind::Class,
///     Location::new(PathBuf::from("/project/src/models.py"), 10, 1),
///     Visibility::Public,
///     "python".to_string(),
/// );
/// index.add_symbol(symbol);
///
/// // Query the index
/// assert_eq!(index.symbol_count(), 1);
/// assert!(index.get("models.User").is_some());
///
/// // Search with patterns
/// let results = index.search("User");
/// assert_eq!(results.len(), 1);
/// ```
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct CodeIndex {
    /// Workspace root directory (not serialized - set on load)
    #[serde(skip)]
    workspace_root: Option<PathBuf>,

    /// Symbol qualified name -> definitions (supports overloading/shadowing)
    /// Key is qualified name: "MyApp.Services.PaymentService.processPayment"
    /// Value is a Vec to handle method overloading and shadowing
    definitions: HashMap<String, Vec<Symbol>>,

    /// File (relative path) -> symbols defined in that file
    file_symbols: HashMap<PathBuf, Vec<String>>,

    /// File (relative path) -> symbol references (identifiers used, not defined)
    file_references: HashMap<PathBuf, Vec<Reference>>,

    /// Module/namespace -> files that define symbols in it
    module_files: HashMap<String, Vec<PathBuf>>,

    /// File (relative path) -> parsed opens/imports
    file_opens: HashMap<PathBuf, Vec<String>>,

    /// File compilation order from .fsproj (relative paths)
    /// Index 0 = first file compiled, higher = later
    /// Empty if no .fsproj was found
    file_order: Vec<PathBuf>,

    /// Optional type cache for type-aware resolution (not serialized - loaded separately)
    #[serde(skip)]
    type_cache: Option<TypeCache>,

    /// Optional external index for .NET assembly symbols (not serialized - loaded separately)
    #[serde(skip)]
    external_index: Option<crate::external_index::ExternalIndex>,
}

impl CodeIndex {
    /// Create a new empty index.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a new index with a workspace root.
    pub fn with_root(root: PathBuf) -> Self {
        Self {
            workspace_root: Some(root),
            ..Default::default()
        }
    }

    /// Set the workspace root directory.
    ///
    /// This should be called after deserializing an index to enable
    /// absolute path resolution.
    pub fn set_workspace_root(&mut self, root: PathBuf) {
        self.workspace_root = Some(root);
    }

    /// Get the workspace root directory.
    pub fn workspace_root(&self) -> Option<&Path> {
        self.workspace_root.as_deref()
    }

    /// Convert an absolute path to a path relative to workspace root.
    fn to_relative(&self, path: &Path) -> PathBuf {
        if let Some(root) = &self.workspace_root {
            path.strip_prefix(root).unwrap_or(path).to_path_buf()
        } else {
            path.to_path_buf()
        }
    }

    /// Convert a relative path to an absolute path using workspace root.
    fn to_absolute(&self, path: &Path) -> PathBuf {
        if path.is_absolute() {
            return path.to_path_buf();
        }
        if let Some(root) = &self.workspace_root {
            root.join(path)
        } else {
            path.to_path_buf()
        }
    }

    /// Convert a location's file path to absolute.
    ///
    /// Use this when you need to convert a symbol's location to an absolute path
    /// for use with the file system or LSP.
    pub fn make_location_absolute(&self, location: &crate::Location) -> crate::Location {
        crate::Location {
            file: self.to_absolute(&location.file),
            line: location.line,
            column: location.column,
            end_line: location.end_line,
            end_column: location.end_column,
        }
    }

    /// Add a symbol to the index.
    ///
    /// The symbol's file path will be converted to a relative path.
    /// Multiple symbols with the same qualified name are allowed (for overloading/shadowing).
    pub fn add_symbol(&mut self, mut symbol: Symbol) {
        let qualified = symbol.qualified.clone();

        // Convert to relative path for storage
        let relative_file = self.to_relative(&symbol.location.file);
        symbol.location.file = relative_file.clone();

        // Add to definitions map (append to vec for overloading support)
        self.definitions
            .entry(qualified.clone())
            .or_default()
            .push(symbol);

        // Add to file_symbols map
        self.file_symbols
            .entry(relative_file.clone())
            .or_default()
            .push(qualified.clone());

        // Add to module_files map
        if let Some((module, _)) = qualified.rsplit_once('.') {
            self.module_files
                .entry(module.to_string())
                .or_default()
                .push(relative_file);
        }
    }

    /// Add a reference to the index.
    ///
    /// The file path will be converted to a relative path.
    pub fn add_reference(&mut self, file: PathBuf, mut reference: Reference) {
        let relative_file = self.to_relative(&file);
        reference.location.file = self.to_relative(&reference.location.file);

        self.file_references
            .entry(relative_file)
            .or_default()
            .push(reference);
    }

    /// Add an open/import statement for a file.
    ///
    /// The file path will be converted to a relative path.
    pub fn add_open(&mut self, file: PathBuf, module: String) {
        let relative_file = self.to_relative(&file);
        self.file_opens
            .entry(relative_file)
            .or_default()
            .push(module);
    }

    /// Get a symbol by its qualified name.
    ///
    /// Note: The returned symbol's file path is relative to the workspace root.
    /// Returns the first (or most recently added) symbol for overloaded names.
    pub fn get(&self, qualified_name: &str) -> Option<&Symbol> {
        self.definitions
            .get(qualified_name)
            .and_then(|syms| syms.last())
    }

    /// Get all symbols with a given qualified name (for handling overloads).
    pub fn get_all(&self, qualified_name: &str) -> &[Symbol] {
        self.definitions
            .get(qualified_name)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    /// Get a symbol by its qualified name, with file path resolved to absolute.
    /// Returns the first (or most recently added) symbol for overloaded names.
    pub fn get_absolute(&self, qualified_name: &str) -> Option<Symbol> {
        self.definitions
            .get(qualified_name)
            .and_then(|syms| syms.last())
            .map(|sym| {
                let mut sym = sym.clone();
                sym.location.file = self.to_absolute(&sym.location.file);
                sym
            })
    }

    /// Get all symbols defined in a file.
    ///
    /// The file path can be either absolute or relative.
    pub fn symbols_in_file(&self, file: &Path) -> Vec<&Symbol> {
        let relative_file = self.to_relative(file);
        self.file_symbols
            .get(&relative_file)
            .map(|names| {
                names
                    .iter()
                    .filter_map(|name| self.definitions.get(name))
                    .flat_map(|syms| syms.iter())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Get all references in a file.
    ///
    /// The file path can be either absolute or relative.
    pub fn references_in_file(&self, file: &Path) -> &[Reference] {
        let relative_file = self.to_relative(file);
        self.file_references
            .get(&relative_file)
            .map(|refs| refs.as_slice())
            .unwrap_or(&[])
    }

    /// Get all open statements for a file.
    ///
    /// The file path can be either absolute or relative.
    pub fn opens_for_file(&self, file: &Path) -> &[String] {
        let relative_file = self.to_relative(file);
        self.file_opens
            .get(&relative_file)
            .map(|opens| opens.as_slice())
            .unwrap_or(&[])
    }

    /// Find all references to a symbol across the codebase.
    ///
    /// Returns a list of locations where the symbol (or a name that could refer to it)
    /// is referenced. This searches for:
    /// - The short name (e.g., "helper")
    /// - The qualified name (e.g., "Utils.helper")
    /// - Any suffix of the qualified name
    pub fn find_references(&self, qualified_name: &str) -> Vec<&Reference> {
        let symbols = match self.definitions.get(qualified_name) {
            Some(s) if !s.is_empty() => s,
            _ => return Vec::new(),
        };

        // Use the first symbol's name for matching references
        let short_name = &symbols[0].name;

        // Collect all references that might refer to this symbol
        let mut results = Vec::new();

        for refs in self.file_references.values() {
            for reference in refs {
                // Check if this reference could be referring to our symbol
                // It could be:
                // 1. The short name: "helper"
                // 2. The full qualified name: "MyApp.Utils.helper"
                // 3. A partial qualified name: "Utils.helper"
                // 4. A receiver.method call: "obj.helper" where obj is a variable
                //    (for languages like Go, Java, C++ where method calls use receiver syntax)
                if reference.name == *short_name
                    || reference.name == qualified_name
                    || qualified_name.ends_with(&format!(".{}", reference.name))
                    || reference.name.ends_with(&format!(".{}", short_name))
                {
                    results.push(reference);
                }
            }
        }

        results
    }

    /// Search for symbols matching a pattern (simple prefix/contains match).
    #[must_use]
    pub fn search(&self, query: &str) -> Vec<&Symbol> {
        let query_lower = query.to_lowercase();
        let is_glob = query.contains('*');

        self.definitions
            .values()
            .flat_map(|syms| syms.iter())
            .filter(|sym| {
                if is_glob {
                    // Simple glob matching: "Payment*" matches "PaymentService"
                    let pattern = query_lower.replace('*', "");
                    if query.starts_with('*') && query.ends_with('*') {
                        sym.name.to_lowercase().contains(&pattern)
                    } else if query.starts_with('*') {
                        sym.name.to_lowercase().ends_with(&pattern)
                    } else if query.ends_with('*') {
                        sym.name.to_lowercase().starts_with(&pattern)
                    } else {
                        sym.name.to_lowercase().contains(&pattern)
                    }
                } else {
                    // Exact prefix match or contains
                    sym.name.to_lowercase().starts_with(&query_lower)
                        || sym.qualified.to_lowercase().contains(&query_lower)
                }
            })
            .collect()
    }

    /// Get all qualified names in the index (for fuzzy matching).
    #[must_use]
    pub fn all_qualified_names(&self) -> Vec<String> {
        self.definitions.keys().cloned().collect()
    }

    /// Get all symbol names (short and qualified) for fuzzy matching.
    #[must_use]
    pub fn all_names_for_fuzzy(&self) -> Vec<String> {
        let mut names = std::collections::HashSet::new();
        for symbols in self.definitions.values() {
            for sym in symbols {
                names.insert(sym.name.clone());
                names.insert(sym.qualified.clone());
            }
        }
        names.into_iter().collect()
    }

    /// Clear all data for a specific file (used before re-indexing).
    ///
    /// The file path can be either absolute or relative.
    pub fn clear_file(&mut self, file: &Path) {
        let relative_file = self.to_relative(file);

        // Remove from file_symbols and definitions
        if let Some(symbols) = self.file_symbols.remove(&relative_file) {
            for qualified in symbols {
                // Remove symbols from this file, but keep symbols from other files
                if let Some(syms) = self.definitions.get_mut(&qualified) {
                    // Clone relative_file to avoid borrowing self in the closure
                    let target_file = relative_file.clone();
                    syms.retain(|s| s.location.file != target_file);
                }
            }
            // Clean up empty entries in a second pass
            self.definitions.retain(|_, syms| !syms.is_empty());
        }

        // Remove from file_references
        self.file_references.remove(&relative_file);

        // Remove from file_opens
        self.file_opens.remove(&relative_file);

        // Clean up module_files (remove file from all module entries)
        for files in self.module_files.values_mut() {
            files.retain(|f| f != &relative_file);
        }
    }

    /// Get all symbols defined in a specific module.
    ///
    /// Returns symbols whose qualified name starts with the given module prefix.
    #[must_use]
    pub fn symbols_in_module(&self, module: &str) -> Vec<&Symbol> {
        let prefix = format!("{}.", module);
        self.definitions
            .iter()
            .filter(|(qualified, _)| qualified.starts_with(&prefix) || *qualified == module)
            .flat_map(|(_, symbols)| symbols.iter())
            .collect()
    }

    /// Get the total number of indexed symbols.
    pub fn symbol_count(&self) -> usize {
        self.definitions.values().map(|v| v.len()).sum()
    }

    /// Get the number of indexed files.
    pub fn file_count(&self) -> usize {
        self.file_symbols.len()
    }

    /// Get all indexed files.
    pub fn files(&self) -> impl Iterator<Item = &PathBuf> {
        self.file_symbols.keys()
    }

    /// Check if a file is indexed.
    ///
    /// The file path can be either absolute or relative.
    pub fn contains_file(&self, file: &Path) -> bool {
        let relative_file = self.to_relative(file);
        self.file_symbols.contains_key(&relative_file)
    }

    /// Set the file compilation order from a .fsproj file.
    ///
    /// Files are stored in compilation order (index 0 = first compiled).
    /// Paths are converted to relative paths.
    pub fn set_file_order(&mut self, files: Vec<PathBuf>) {
        self.file_order = files.into_iter().map(|f| self.to_relative(&f)).collect();
    }

    /// Get the compilation order index for a file.
    ///
    /// Returns None if the file is not in the compilation order
    /// (either no .fsproj was loaded or file is not in the project).
    pub fn compilation_order(&self, file: &Path) -> Option<usize> {
        let relative_file = self.to_relative(file);
        self.file_order.iter().position(|f| f == &relative_file)
    }

    /// Check if file A can reference file B based on compilation order.
    ///
    /// In F#, a file can only reference symbols from files that come
    /// before it in the compilation order.
    ///
    /// Returns true if:
    /// - No compilation order is set (permissive mode)
    /// - Either file is not in the project
    /// - to_file comes before from_file in compilation order
    pub fn can_reference(&self, from_file: &Path, to_file: &Path) -> bool {
        if self.file_order.is_empty() {
            return true; // No compilation order set, allow all references
        }

        match (
            self.compilation_order(from_file),
            self.compilation_order(to_file),
        ) {
            (Some(from_order), Some(to_order)) => to_order < from_order,
            _ => true, // If either file is not in order, allow reference
        }
    }

    /// Get all files that can be referenced from the given file.
    ///
    /// Returns files that come before the given file in compilation order.
    /// If no compilation order is set, returns all indexed files.
    pub fn files_visible_from(&self, file: &Path) -> Vec<PathBuf> {
        if self.file_order.is_empty() {
            return self.file_symbols.keys().cloned().collect();
        }

        match self.compilation_order(file) {
            Some(order) => self.file_order[..order].to_vec(),
            None => self.file_symbols.keys().cloned().collect(),
        }
    }

    /// Check if compilation order information is available.
    pub fn has_file_order(&self) -> bool {
        !self.file_order.is_empty()
    }

    /// Get the number of files in the compilation order.
    pub fn file_order_count(&self) -> usize {
        self.file_order.len()
    }

    // =========================================================================
    // Type Cache Integration (RFC-001)
    // =========================================================================

    /// Load a type cache from a JSON file and attach it to this index.
    ///
    /// The type cache provides type information extracted at build time,
    /// enabling type-aware symbol resolution.
    pub fn load_type_cache(&mut self, path: &Path) -> crate::Result<()> {
        let cache = TypeCache::load(path)?;
        self.type_cache = Some(cache);
        Ok(())
    }

    /// Attach an already-loaded type cache to this index.
    pub fn set_type_cache(&mut self, cache: TypeCache) {
        self.type_cache = Some(cache);
    }

    /// Check if a type cache is loaded.
    pub fn has_type_cache(&self) -> bool {
        self.type_cache.is_some()
    }

    /// Get a reference to the type cache, if loaded.
    pub fn type_cache(&self) -> Option<&TypeCache> {
        self.type_cache.as_ref()
    }

    /// Get the type signature of a symbol by its qualified name.
    ///
    /// Returns `None` if no type cache is loaded or the symbol is not found.
    pub fn get_symbol_type(&self, qualified_name: &str) -> Option<&str> {
        self.type_cache.as_ref()?.get_type(qualified_name)
    }

    /// Get all members of a type.
    ///
    /// Returns `None` if no type cache is loaded or the type is not found.
    pub fn get_type_members(&self, type_name: &str) -> Option<&[TypeMember]> {
        self.type_cache.as_ref()?.get_members(type_name)
    }

    /// Get a specific member of a type.
    ///
    /// Returns `None` if no type cache is loaded, type not found, or member not found.
    pub fn get_type_member(&self, type_name: &str, member_name: &str) -> Option<&TypeMember> {
        self.type_cache.as_ref()?.get_member(type_name, member_name)
    }

    // =========================================================================
    // External Index Integration
    // =========================================================================

    /// Set the external index for .NET assembly symbols.
    pub fn set_external_index(&mut self, index: crate::external_index::ExternalIndex) {
        self.external_index = Some(index);
    }

    /// Check if an external index is loaded.
    pub fn has_external_index(&self) -> bool {
        self.external_index.is_some()
    }

    /// Get a reference to the external index, if loaded.
    pub fn external_index(&self) -> Option<&crate::external_index::ExternalIndex> {
        self.external_index.as_ref()
    }

    /// Find an external symbol by qualified name.
    pub fn find_external_symbol(
        &self,
        qualified_name: &str,
    ) -> Option<&crate::external_index::ExternalSymbol> {
        self.external_index.as_ref()?.find_symbol(qualified_name)
    }

    /// Search for external symbols matching a pattern.
    pub fn search_external(&self, pattern: &str) -> Vec<&crate::external_index::ExternalSymbol> {
        self.external_index
            .as_ref()
            .map_or(Vec::new(), |idx| idx.search(pattern))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::type_cache::{MemberKind, TypeCacheSchema, TypeMember, TypedSymbol};
    use crate::{Location, SymbolKind, Visibility};

    fn make_symbol(name: &str, qualified: &str, file: &str) -> Symbol {
        Symbol {
            name: name.to_string(),
            qualified: qualified.to_string(),
            kind: SymbolKind::Function,
            location: Location::new(PathBuf::from(file), 1, 1),
            visibility: Visibility::Public,
            language: "fsharp".to_string(),
            parent: None,
            mixins: None,
            attributes: None,
            implements: None,
            doc: None,
            signature: None,
        }
    }

    #[test]
    fn test_add_and_get_symbol() {
        let mut index = CodeIndex::new();
        let sym = make_symbol("foo", "MyModule.foo", "src/test.fs");
        index.add_symbol(sym.clone());

        assert_eq!(index.get("MyModule.foo").unwrap().name, "foo");
        assert_eq!(index.symbol_count(), 1);
    }

    #[test]
    fn test_symbols_in_file() {
        let mut index = CodeIndex::new();
        index.add_symbol(make_symbol("foo", "M.foo", "src/a.fs"));
        index.add_symbol(make_symbol("bar", "M.bar", "src/a.fs"));
        index.add_symbol(make_symbol("baz", "M.baz", "src/b.fs"));

        let symbols = index.symbols_in_file(Path::new("src/a.fs"));
        assert_eq!(symbols.len(), 2);
    }

    #[test]
    fn test_search() {
        let mut index = CodeIndex::new();
        index.add_symbol(make_symbol(
            "PaymentService",
            "App.PaymentService",
            "src/a.fs",
        ));
        index.add_symbol(make_symbol(
            "PaymentRequest",
            "App.PaymentRequest",
            "src/a.fs",
        ));
        index.add_symbol(make_symbol("OrderService", "App.OrderService", "src/b.fs"));

        let results = index.search("Payment*");
        assert_eq!(results.len(), 2);

        let results = index.search("Order");
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_clear_file() {
        let mut index = CodeIndex::new();
        index.add_symbol(make_symbol("foo", "M.foo", "src/a.fs"));
        index.add_symbol(make_symbol("bar", "M.bar", "src/b.fs"));

        assert_eq!(index.symbol_count(), 2);

        index.clear_file(Path::new("src/a.fs"));

        assert_eq!(index.symbol_count(), 1);
        assert!(index.get("M.foo").is_none());
        assert!(index.get("M.bar").is_some());
    }

    #[test]
    fn test_find_references() {
        let mut index = CodeIndex::new();

        // Add a symbol
        index.add_symbol(make_symbol("helper", "Utils.helper", "src/Utils.fs"));

        // Add references from different files
        index.add_reference(
            PathBuf::from("src/Main.fs"),
            Reference {
                name: "helper".to_string(),
                location: Location::new(PathBuf::from("src/Main.fs"), 10, 5),
            },
        );
        index.add_reference(
            PathBuf::from("src/Main.fs"),
            Reference {
                name: "Utils.helper".to_string(),
                location: Location::new(PathBuf::from("src/Main.fs"), 15, 5),
            },
        );
        index.add_reference(
            PathBuf::from("src/Other.fs"),
            Reference {
                name: "helper".to_string(),
                location: Location::new(PathBuf::from("src/Other.fs"), 20, 5),
            },
        );

        // Find references to Utils.helper
        let refs = index.find_references("Utils.helper");
        assert_eq!(refs.len(), 3);
    }

    #[test]
    fn test_find_references_no_symbol() {
        let index = CodeIndex::new();
        let refs = index.find_references("NonExistent.symbol");
        assert!(refs.is_empty());
    }

    #[test]
    fn test_symbol_overloading() {
        let mut index = CodeIndex::new();

        // Add two symbols with the same qualified name (simulating method overloading)
        let sym1 = Symbol {
            name: "parse".to_string(),
            qualified: "Parser.parse".to_string(),
            kind: SymbolKind::Function,
            location: Location::new(PathBuf::from("src/Parser.fs"), 10, 1),
            visibility: Visibility::Public,
            language: "fsharp".to_string(),
            parent: None,
            mixins: None,
            attributes: None,
            implements: None,
            doc: None,
            signature: None,
        };
        let sym2 = Symbol {
            name: "parse".to_string(),
            qualified: "Parser.parse".to_string(),
            kind: SymbolKind::Function,
            location: Location::new(PathBuf::from("src/Parser.fs"), 20, 1),
            visibility: Visibility::Public,
            language: "fsharp".to_string(),
            parent: None,
            mixins: None,
            attributes: None,
            implements: None,
            doc: None,
            signature: None,
        };

        index.add_symbol(sym1);
        index.add_symbol(sym2);

        // Both symbols should be stored
        assert_eq!(index.symbol_count(), 2);

        // get() returns the most recent (last added)
        let sym = index.get("Parser.parse").unwrap();
        assert_eq!(sym.location.line, 20);

        // get_all() returns all overloads
        let all_syms = index.get_all("Parser.parse");
        assert_eq!(all_syms.len(), 2);
        assert_eq!(all_syms[0].location.line, 10);
        assert_eq!(all_syms[1].location.line, 20);
    }

    #[test]
    fn test_symbol_shadowing_across_files() {
        let mut index = CodeIndex::new();

        // Add symbols with the same qualified name from different files
        let sym1 = Symbol {
            name: "config".to_string(),
            qualified: "App.config".to_string(),
            kind: SymbolKind::Value,
            location: Location::new(PathBuf::from("src/Config.fs"), 5, 1),
            visibility: Visibility::Public,
            language: "fsharp".to_string(),
            parent: None,
            mixins: None,
            attributes: None,
            implements: None,
            doc: None,
            signature: None,
        };
        let sym2 = Symbol {
            name: "config".to_string(),
            qualified: "App.config".to_string(),
            kind: SymbolKind::Value,
            location: Location::new(PathBuf::from("src/Override.fs"), 10, 1),
            visibility: Visibility::Public,
            language: "fsharp".to_string(),
            parent: None,
            mixins: None,
            attributes: None,
            implements: None,
            doc: None,
            signature: None,
        };

        index.add_symbol(sym1);
        index.add_symbol(sym2);

        assert_eq!(index.symbol_count(), 2);

        // Clear one file - should only remove symbols from that file
        index.clear_file(Path::new("src/Config.fs"));

        assert_eq!(index.symbol_count(), 1);
        let remaining = index.get("App.config").unwrap();
        assert_eq!(remaining.location.file, PathBuf::from("src/Override.fs"));
    }

    #[test]
    fn test_file_order_basic() {
        let mut index = CodeIndex::new();

        // Set compilation order: A.fs, B.fs, C.fs
        index.set_file_order(vec![
            PathBuf::from("src/A.fs"),
            PathBuf::from("src/B.fs"),
            PathBuf::from("src/C.fs"),
        ]);

        assert!(index.has_file_order());
        assert_eq!(index.file_order_count(), 3);

        // Check compilation order indices
        assert_eq!(index.compilation_order(Path::new("src/A.fs")), Some(0));
        assert_eq!(index.compilation_order(Path::new("src/B.fs")), Some(1));
        assert_eq!(index.compilation_order(Path::new("src/C.fs")), Some(2));
        assert_eq!(index.compilation_order(Path::new("src/D.fs")), None);
    }

    #[test]
    fn test_can_reference_with_order() {
        let mut index = CodeIndex::new();

        // Set compilation order: A.fs, B.fs, C.fs
        index.set_file_order(vec![
            PathBuf::from("src/A.fs"),
            PathBuf::from("src/B.fs"),
            PathBuf::from("src/C.fs"),
        ]);

        // C can reference A and B (they come before C)
        assert!(index.can_reference(Path::new("src/C.fs"), Path::new("src/A.fs")));
        assert!(index.can_reference(Path::new("src/C.fs"), Path::new("src/B.fs")));

        // B can reference A
        assert!(index.can_reference(Path::new("src/B.fs"), Path::new("src/A.fs")));

        // A cannot reference B or C (they come after A)
        assert!(!index.can_reference(Path::new("src/A.fs"), Path::new("src/B.fs")));
        assert!(!index.can_reference(Path::new("src/A.fs"), Path::new("src/C.fs")));

        // B cannot reference C
        assert!(!index.can_reference(Path::new("src/B.fs"), Path::new("src/C.fs")));
    }

    #[test]
    fn test_can_reference_without_order() {
        let index = CodeIndex::new();

        // Without file order, all references should be allowed
        assert!(!index.has_file_order());
        assert!(index.can_reference(Path::new("src/A.fs"), Path::new("src/B.fs")));
        assert!(index.can_reference(Path::new("src/B.fs"), Path::new("src/A.fs")));
    }

    #[test]
    fn test_files_visible_from() {
        let mut index = CodeIndex::new();

        // Set compilation order: A.fs, B.fs, C.fs
        index.set_file_order(vec![
            PathBuf::from("src/A.fs"),
            PathBuf::from("src/B.fs"),
            PathBuf::from("src/C.fs"),
        ]);

        // From A, nothing is visible
        let visible = index.files_visible_from(Path::new("src/A.fs"));
        assert!(visible.is_empty());

        // From B, only A is visible
        let visible = index.files_visible_from(Path::new("src/B.fs"));
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0], PathBuf::from("src/A.fs"));

        // From C, A and B are visible
        let visible = index.files_visible_from(Path::new("src/C.fs"));
        assert_eq!(visible.len(), 2);
    }

    #[test]
    fn test_file_order_with_workspace_root() {
        let mut index = CodeIndex::with_root(PathBuf::from("/workspace"));

        // Set file order with absolute paths (should be converted to relative)
        index.set_file_order(vec![
            PathBuf::from("/workspace/src/A.fs"),
            PathBuf::from("/workspace/src/B.fs"),
        ]);

        // Should be able to look up by relative path
        assert_eq!(index.compilation_order(Path::new("src/A.fs")), Some(0));
        assert_eq!(index.compilation_order(Path::new("src/B.fs")), Some(1));

        // Should also work with absolute paths
        assert_eq!(
            index.compilation_order(Path::new("/workspace/src/A.fs")),
            Some(0)
        );
    }

    // =========================================================================
    // Type Cache Integration Tests (RFC-001)
    // =========================================================================

    fn make_type_cache() -> TypeCache {
        let schema = TypeCacheSchema {
            version: 1,
            extracted_at: "2024-12-02T10:30:00Z".to_string(),
            project: "TestProject".to_string(),
            symbols: vec![
                TypedSymbol {
                    name: "myString".to_string(),
                    qualified: "MyModule.myString".to_string(),
                    type_signature: "string".to_string(),
                    file: "src/MyModule.fs".to_string(),
                    line: 42,
                    parameters: vec![],
                },
                TypedSymbol {
                    name: "processUser".to_string(),
                    qualified: "UserService.processUser".to_string(),
                    type_signature: "User -> Async<Result<Response, Error>>".to_string(),
                    file: "src/UserService.fs".to_string(),
                    line: 15,
                    parameters: vec![],
                },
            ],
            members: vec![
                TypeMember {
                    type_name: "User".to_string(),
                    member: "Name".to_string(),
                    member_type: "string".to_string(),
                    kind: MemberKind::Property,
                },
                TypeMember {
                    type_name: "User".to_string(),
                    member: "Save".to_string(),
                    member_type: "unit -> Async<unit>".to_string(),
                    kind: MemberKind::Method,
                },
            ],
        };
        TypeCache::from_schema(schema)
    }

    #[test]
    fn test_index_without_type_cache() {
        let index = CodeIndex::new();
        assert!(!index.has_type_cache());
        assert!(index.type_cache().is_none());
        assert!(index.get_symbol_type("MyModule.myString").is_none());
        assert!(index.get_type_members("User").is_none());
    }

    #[test]
    fn test_set_type_cache() {
        let mut index = CodeIndex::new();
        let cache = make_type_cache();

        index.set_type_cache(cache);

        assert!(index.has_type_cache());
        assert!(index.type_cache().is_some());
    }

    #[test]
    fn test_get_symbol_type_with_cache() {
        let mut index = CodeIndex::new();
        index.set_type_cache(make_type_cache());

        assert_eq!(index.get_symbol_type("MyModule.myString"), Some("string"));
        assert_eq!(
            index.get_symbol_type("UserService.processUser"),
            Some("User -> Async<Result<Response, Error>>")
        );
        assert!(index.get_symbol_type("NonExistent.symbol").is_none());
    }

    #[test]
    fn test_get_type_members_with_cache() {
        let mut index = CodeIndex::new();
        index.set_type_cache(make_type_cache());

        let members = index.get_type_members("User").unwrap();
        assert_eq!(members.len(), 2);

        let member_names: Vec<&str> = members.iter().map(|m| m.member.as_str()).collect();
        assert!(member_names.contains(&"Name"));
        assert!(member_names.contains(&"Save"));

        assert!(index.get_type_members("NonExistentType").is_none());
    }

    #[test]
    fn test_get_type_member_with_cache() {
        let mut index = CodeIndex::new();
        index.set_type_cache(make_type_cache());

        let name_prop = index.get_type_member("User", "Name").unwrap();
        assert_eq!(name_prop.member_type, "string");
        assert_eq!(name_prop.kind, MemberKind::Property);

        let save_method = index.get_type_member("User", "Save").unwrap();
        assert_eq!(save_method.kind, MemberKind::Method);

        assert!(index.get_type_member("User", "NonExistent").is_none());
        assert!(index.get_type_member("NonExistent", "Name").is_none());
    }

    #[test]
    fn test_load_type_cache_missing_file() {
        let mut index = CodeIndex::new();
        let result = index.load_type_cache(Path::new("/nonexistent/cache.json"));
        assert!(result.is_err());
        assert!(!index.has_type_cache());
    }

    #[test]
    fn test_load_type_cache_from_file() {
        use std::io::Write;
        let temp_dir = tempfile::tempdir().unwrap();
        let cache_path = temp_dir.path().join("cache.json");

        let json = r#"{
            "version": 1,
            "extracted_at": "2024-12-02T10:30:00Z",
            "project": "TestProject",
            "symbols": [
                {
                    "name": "foo",
                    "qualified": "Test.foo",
                    "type": "int",
                    "file": "src/Test.fs",
                    "line": 1
                }
            ],
            "members": []
        }"#;

        let mut file = std::fs::File::create(&cache_path).unwrap();
        file.write_all(json.as_bytes()).unwrap();

        let mut index = CodeIndex::new();
        index.load_type_cache(&cache_path).unwrap();

        assert!(index.has_type_cache());
        assert_eq!(index.get_symbol_type("Test.foo"), Some("int"));
    }
}
