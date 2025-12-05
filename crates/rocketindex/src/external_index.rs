//! External assembly indexer for .NET libraries.
//!
//! **Current Status: Stub Implementation**
//!
//! This module provides basic symbol information for common .NET BCL types.
//! It does NOT read actual .NET assemblies - instead it provides hardcoded
//! type definitions for the most commonly used System.* types.
//!
//! This enables basic completion and hover for types like:
//! - System.String, System.Int32, System.Boolean
//! - System.Console (WriteLine, ReadLine)
//! - System.Collections.Generic (List, Dictionary)
//! - System.IO (File, Path, Directory)
//! - System.Linq (common extension methods)
//!
//! For full external assembly support, consider using fsautocomplete alongside
//! fsharp-tools.
//!
//! ## Future Work
//!
//! A proper implementation would:
//! 1. Parse .dll files using PE metadata readers
//! 2. Extract public type and member information
//! 3. Handle NuGet package resolution

use crate::{Location, Symbol, SymbolKind, Visibility};
use std::collections::HashMap;
use std::path::Path;

/// External symbol from a .NET assembly
#[derive(Debug, Clone)]
pub struct ExternalSymbol {
    /// The symbol information
    pub symbol: Symbol,
    /// The assembly name (e.g., "System.Runtime")
    pub assembly: String,
}

/// Index of external symbols from .NET assemblies
#[derive(Debug, Default)]
pub struct ExternalIndex {
    /// Symbols indexed by qualified name
    symbols: HashMap<String, ExternalSymbol>,
}

impl ExternalIndex {
    /// Create a new empty external index
    pub fn new() -> Self {
        Self::default()
    }

    /// Index common .NET BCL types that are frequently used in F# code.
    ///
    /// This provides basic symbol information for common types without
    /// reading actual .NET assemblies. It's a pragmatic compromise that
    /// covers ~80% of typical usage.
    pub fn index_common_types(&mut self) {
        // =========================================================
        // System namespace - core types
        // =========================================================

        self.add_type("System.Object", "System.Runtime");
        self.add_type("System.String", "System.Runtime");
        self.add_type("System.Int32", "System.Runtime");
        self.add_type("System.Int64", "System.Runtime");
        self.add_type("System.Double", "System.Runtime");
        self.add_type("System.Single", "System.Runtime");
        self.add_type("System.Boolean", "System.Runtime");
        self.add_type("System.Char", "System.Runtime");
        self.add_type("System.Byte", "System.Runtime");
        self.add_type("System.DateTime", "System.Runtime");
        self.add_type("System.TimeSpan", "System.Runtime");
        self.add_type("System.Guid", "System.Runtime");
        self.add_type("System.Uri", "System.Runtime");
        self.add_type("System.Exception", "System.Runtime");
        self.add_type("System.ArgumentException", "System.Runtime");
        self.add_type("System.InvalidOperationException", "System.Runtime");
        self.add_type("System.NotImplementedException", "System.Runtime");

        // =========================================================
        // System.Console
        // =========================================================

        self.add_type("System.Console", "System.Console");
        self.add_method("System.Console.WriteLine", "System.Console");
        self.add_method("System.Console.Write", "System.Console");
        self.add_method("System.Console.ReadLine", "System.Console");
        self.add_method("System.Console.ReadKey", "System.Console");
        self.add_method("System.Console.Clear", "System.Console");

        // =========================================================
        // System.IO
        // =========================================================

        self.add_type("System.IO.File", "System.IO");
        self.add_method("System.IO.File.ReadAllText", "System.IO");
        self.add_method("System.IO.File.WriteAllText", "System.IO");
        self.add_method("System.IO.File.ReadAllLines", "System.IO");
        self.add_method("System.IO.File.WriteAllLines", "System.IO");
        self.add_method("System.IO.File.Exists", "System.IO");
        self.add_method("System.IO.File.Delete", "System.IO");
        self.add_method("System.IO.File.Copy", "System.IO");
        self.add_method("System.IO.File.Move", "System.IO");

        self.add_type("System.IO.Directory", "System.IO");
        self.add_method("System.IO.Directory.Exists", "System.IO");
        self.add_method("System.IO.Directory.CreateDirectory", "System.IO");
        self.add_method("System.IO.Directory.Delete", "System.IO");
        self.add_method("System.IO.Directory.GetFiles", "System.IO");
        self.add_method("System.IO.Directory.GetDirectories", "System.IO");

        self.add_type("System.IO.Path", "System.IO");
        self.add_method("System.IO.Path.Combine", "System.IO");
        self.add_method("System.IO.Path.GetFileName", "System.IO");
        self.add_method("System.IO.Path.GetDirectoryName", "System.IO");
        self.add_method("System.IO.Path.GetExtension", "System.IO");
        self.add_method("System.IO.Path.GetFullPath", "System.IO");

        self.add_type("System.IO.StreamReader", "System.IO");
        self.add_type("System.IO.StreamWriter", "System.IO");

        // =========================================================
        // System.Collections.Generic
        // =========================================================

        self.add_type("System.Collections.Generic.List`1", "System.Collections");
        self.add_method(
            "System.Collections.Generic.List`1.Add",
            "System.Collections",
        );
        self.add_method(
            "System.Collections.Generic.List`1.Remove",
            "System.Collections",
        );
        self.add_method(
            "System.Collections.Generic.List`1.Clear",
            "System.Collections",
        );
        self.add_method(
            "System.Collections.Generic.List`1.Contains",
            "System.Collections",
        );
        self.add_property(
            "System.Collections.Generic.List`1.Count",
            "System.Collections",
        );

        self.add_type(
            "System.Collections.Generic.Dictionary`2",
            "System.Collections",
        );
        self.add_method(
            "System.Collections.Generic.Dictionary`2.Add",
            "System.Collections",
        );
        self.add_method(
            "System.Collections.Generic.Dictionary`2.Remove",
            "System.Collections",
        );
        self.add_method(
            "System.Collections.Generic.Dictionary`2.ContainsKey",
            "System.Collections",
        );
        self.add_method(
            "System.Collections.Generic.Dictionary`2.TryGetValue",
            "System.Collections",
        );
        self.add_property(
            "System.Collections.Generic.Dictionary`2.Count",
            "System.Collections",
        );
        self.add_property(
            "System.Collections.Generic.Dictionary`2.Keys",
            "System.Collections",
        );
        self.add_property(
            "System.Collections.Generic.Dictionary`2.Values",
            "System.Collections",
        );

        self.add_type("System.Collections.Generic.HashSet`1", "System.Collections");
        self.add_type("System.Collections.Generic.Queue`1", "System.Collections");
        self.add_type("System.Collections.Generic.Stack`1", "System.Collections");

        // =========================================================
        // System.Linq
        // =========================================================

        self.add_type("System.Linq.Enumerable", "System.Linq");
        self.add_method("System.Linq.Enumerable.Select", "System.Linq");
        self.add_method("System.Linq.Enumerable.Where", "System.Linq");
        self.add_method("System.Linq.Enumerable.First", "System.Linq");
        self.add_method("System.Linq.Enumerable.FirstOrDefault", "System.Linq");
        self.add_method("System.Linq.Enumerable.Last", "System.Linq");
        self.add_method("System.Linq.Enumerable.LastOrDefault", "System.Linq");
        self.add_method("System.Linq.Enumerable.Single", "System.Linq");
        self.add_method("System.Linq.Enumerable.SingleOrDefault", "System.Linq");
        self.add_method("System.Linq.Enumerable.ToList", "System.Linq");
        self.add_method("System.Linq.Enumerable.ToArray", "System.Linq");
        self.add_method("System.Linq.Enumerable.ToDictionary", "System.Linq");
        self.add_method("System.Linq.Enumerable.Count", "System.Linq");
        self.add_method("System.Linq.Enumerable.Any", "System.Linq");
        self.add_method("System.Linq.Enumerable.All", "System.Linq");
        self.add_method("System.Linq.Enumerable.Skip", "System.Linq");
        self.add_method("System.Linq.Enumerable.Take", "System.Linq");
        self.add_method("System.Linq.Enumerable.OrderBy", "System.Linq");
        self.add_method("System.Linq.Enumerable.OrderByDescending", "System.Linq");
        self.add_method("System.Linq.Enumerable.GroupBy", "System.Linq");
        self.add_method("System.Linq.Enumerable.Distinct", "System.Linq");
        self.add_method("System.Linq.Enumerable.Concat", "System.Linq");
        self.add_method("System.Linq.Enumerable.Zip", "System.Linq");

        // =========================================================
        // System.Text
        // =========================================================

        self.add_type("System.Text.StringBuilder", "System.Runtime");
        self.add_method("System.Text.StringBuilder.Append", "System.Runtime");
        self.add_method("System.Text.StringBuilder.AppendLine", "System.Runtime");
        self.add_method("System.Text.StringBuilder.Clear", "System.Runtime");
        self.add_method("System.Text.StringBuilder.ToString", "System.Runtime");

        self.add_type(
            "System.Text.RegularExpressions.Regex",
            "System.Text.RegularExpressions",
        );
        self.add_method(
            "System.Text.RegularExpressions.Regex.Match",
            "System.Text.RegularExpressions",
        );
        self.add_method(
            "System.Text.RegularExpressions.Regex.Matches",
            "System.Text.RegularExpressions",
        );
        self.add_method(
            "System.Text.RegularExpressions.Regex.Replace",
            "System.Text.RegularExpressions",
        );
        self.add_method(
            "System.Text.RegularExpressions.Regex.IsMatch",
            "System.Text.RegularExpressions",
        );

        // =========================================================
        // System.Threading.Tasks
        // =========================================================

        self.add_type("System.Threading.Tasks.Task", "System.Runtime");
        self.add_type("System.Threading.Tasks.Task`1", "System.Runtime");
        self.add_method("System.Threading.Tasks.Task.Run", "System.Runtime");
        self.add_method("System.Threading.Tasks.Task.WhenAll", "System.Runtime");
        self.add_method("System.Threading.Tasks.Task.WhenAny", "System.Runtime");
        self.add_method("System.Threading.Tasks.Task.Delay", "System.Runtime");

        // =========================================================
        // System.Net.Http
        // =========================================================

        self.add_type("System.Net.Http.HttpClient", "System.Net.Http");
        self.add_method("System.Net.Http.HttpClient.GetAsync", "System.Net.Http");
        self.add_method("System.Net.Http.HttpClient.PostAsync", "System.Net.Http");
        self.add_method("System.Net.Http.HttpClient.PutAsync", "System.Net.Http");
        self.add_method("System.Net.Http.HttpClient.DeleteAsync", "System.Net.Http");
        self.add_method("System.Net.Http.HttpClient.SendAsync", "System.Net.Http");
    }

    /// Add a type symbol
    fn add_type(&mut self, qualified_name: &str, assembly: &str) {
        self.add_symbol(
            qualified_name.to_string(),
            SymbolKind::Class,
            assembly.to_string(),
        );
    }

    /// Add a method symbol
    fn add_method(&mut self, qualified_name: &str, assembly: &str) {
        self.add_symbol(
            qualified_name.to_string(),
            SymbolKind::Function,
            assembly.to_string(),
        );
    }

    /// Add a property symbol
    fn add_property(&mut self, qualified_name: &str, assembly: &str) {
        self.add_symbol(
            qualified_name.to_string(),
            SymbolKind::Value,
            assembly.to_string(),
        );
    }

    /// Add a symbol to the index
    fn add_symbol(&mut self, qualified_name: String, kind: SymbolKind, assembly: String) {
        let symbol = Symbol::new(
            qualified_name
                .rsplit('.')
                .next()
                .unwrap_or(&qualified_name)
                .to_string(),
            qualified_name.clone(),
            kind,
            Location::new(Path::new(&assembly).to_path_buf(), 1, 1),
            Visibility::Public,
            "fsharp".to_string(),
        );

        let ext_symbol = ExternalSymbol { symbol, assembly };

        self.symbols.insert(qualified_name, ext_symbol);
    }

    /// Find a symbol by qualified name
    pub fn find_symbol(&self, qualified_name: &str) -> Option<&ExternalSymbol> {
        self.symbols.get(qualified_name)
    }

    /// Search for symbols matching a pattern
    pub fn search(&self, pattern: &str) -> Vec<&ExternalSymbol> {
        self.symbols
            .values()
            .filter(|ext_sym| {
                ext_sym.symbol.name.contains(pattern) || ext_sym.symbol.qualified.contains(pattern)
            })
            .collect()
    }

    /// Get all symbols from a specific assembly
    pub fn symbols_in_assembly(&self, assembly: &str) -> Vec<&ExternalSymbol> {
        self.symbols
            .values()
            .filter(|ext_sym| ext_sym.assembly == assembly)
            .collect()
    }

    /// Check if the index contains any symbols
    pub fn is_empty(&self) -> bool {
        self.symbols.is_empty()
    }

    /// Get the number of indexed symbols
    pub fn len(&self) -> usize {
        self.symbols.len()
    }
}

/// Index external assemblies based on package references
pub fn index_external_assemblies(
    package_refs: &[crate::fsproj::PackageReference],
) -> ExternalIndex {
    let mut index = ExternalIndex::new();

    // For now, just index common types regardless of packages
    // In a full implementation, this would:
    // 1. Find the actual .dll files for each package
    // 2. Read their metadata using a .NET reflection library
    // 3. Extract public types and members
    index.index_common_types();

    // TODO: Actually read the referenced packages
    for _package in package_refs {
        // Find package in NuGet cache or local packages folder
        // Read .dll files and extract metadata
        // This requires additional dependencies for PE file parsing
    }

    index
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_index_common_types() {
        let mut index = ExternalIndex::new();
        index.index_common_types();

        assert!(!index.is_empty());
        assert!(index.len() > 5);

        // Check some basic types
        assert!(index.find_symbol("System.String").is_some());
        assert!(index.find_symbol("System.Console").is_some());
        assert!(index.find_symbol("System.Int32").is_some());
    }

    #[test]
    fn test_search_symbols() {
        let mut index = ExternalIndex::new();
        index.index_common_types();

        let results = index.search("Console");
        assert!(!results.is_empty());
        assert!(results
            .iter()
            .any(|ext| ext.symbol.qualified == "System.Console"));
    }

    #[test]
    fn test_symbols_in_assembly() {
        let mut index = ExternalIndex::new();
        index.index_common_types();

        let console_symbols = index.symbols_in_assembly("System.Console");
        assert!(!console_symbols.is_empty());
        assert!(console_symbols
            .iter()
            .any(|ext| ext.symbol.qualified == "System.Console"));
    }

    #[test]
    fn test_empty_index() {
        let index = ExternalIndex::new();
        assert!(index.is_empty());
        assert_eq!(index.len(), 0);
        assert!(index.find_symbol("System.String").is_none());
    }
}
