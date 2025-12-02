//! Name resolution for F# symbols with scope rules.
//!
//! This module implements F# name resolution, taking into account:
//! - Open statements (imports)
//! - Module hierarchy
//! - Qualified vs unqualified names

use std::path::Path;

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
    /// Resolved via an open statement
    ViaOpen(String),
    /// Resolved from the same module
    SameModule,
    /// Resolved from a parent module
    ParentModule(String),
}

impl CodeIndex {
    /// Resolve a symbol name from a given file context.
    ///
    /// This implements F# name resolution rules:
    /// 1. Try exact qualified name match
    /// 2. Try same-file/same-module symbols
    /// 3. Try symbols accessible via open statements
    /// 4. Try parent module symbols
    ///
    /// # Arguments
    /// * `name` - The name to resolve (can be qualified like "List.map" or simple like "helper")
    /// * `from_file` - The file context for resolution (determines which opens are in scope)
    ///
    /// # Returns
    /// The resolved symbol if found, None otherwise
    pub fn resolve(&self, name: &str, from_file: &Path) -> Option<ResolveResult<'_>> {
        // 1. Try exact qualified name match (respecting compilation order)
        if let Some(symbol) = self.get_visible_from(name, from_file) {
            return Some(ResolveResult {
                symbol,
                resolution_path: ResolutionPath::Qualified,
            });
        }

        // 2. Try same-file symbols
        if let Some(result) = self.resolve_in_same_file(name, from_file) {
            return Some(result);
        }

        // 3. Try symbols via open statements (respecting compilation order)
        if let Some(result) = self.resolve_via_opens(name, from_file) {
            return Some(result);
        }

        // 4. Try parent module symbols (respecting compilation order)
        if let Some(result) = self.resolve_in_parent_modules(name, from_file) {
            return Some(result);
        }

        None
    }

    /// Get a symbol by qualified name, but only if it's visible from the given file.
    ///
    /// This respects F# compilation order: a symbol is only visible if its
    /// defining file comes before from_file in the compilation order.
    fn get_visible_from(&self, name: &str, from_file: &Path) -> Option<&Symbol> {
        let symbol = self.get(name)?;

        // Check if the symbol's file is visible from from_file
        if self.can_reference(from_file, &symbol.location.file) {
            Some(symbol)
        } else {
            // Symbol exists but is not visible due to compilation order
            None
        }
    }

    /// Try to resolve a name within the same file.
    fn resolve_in_same_file(&self, name: &str, from_file: &Path) -> Option<ResolveResult<'_>> {
        let file_symbols = self.symbols_in_file(from_file);

        // Try unqualified match within file symbols
        for symbol in file_symbols {
            if symbol.name == name {
                return Some(ResolveResult {
                    symbol,
                    resolution_path: ResolutionPath::SameModule,
                });
            }
            // Also check if the name matches the end of the qualified name
            if symbol.qualified.ends_with(&format!(".{}", name)) {
                return Some(ResolveResult {
                    symbol,
                    resolution_path: ResolutionPath::SameModule,
                });
            }
        }

        None
    }

    /// Try to resolve a name using open statements.
    fn resolve_via_opens(&self, name: &str, from_file: &Path) -> Option<ResolveResult<'_>> {
        let opens = self.opens_for_file(from_file);

        for open_module in opens {
            // Try: OpenModule.name
            let qualified = format!("{}.{}", open_module, name);
            if let Some(symbol) = self.get_visible_from(&qualified, from_file) {
                return Some(ResolveResult {
                    symbol,
                    resolution_path: ResolutionPath::ViaOpen(open_module.clone()),
                });
            }

            // For dotted names like "List.map", try "OpenModule.List.map"
            if name.contains('.') {
                let parts: Vec<&str> = name.splitn(2, '.').collect();
                if parts.len() == 2 {
                    let qualified = format!("{}.{}", open_module, name);
                    if let Some(symbol) = self.get_visible_from(&qualified, from_file) {
                        return Some(ResolveResult {
                            symbol,
                            resolution_path: ResolutionPath::ViaOpen(open_module.clone()),
                        });
                    }
                }
            }
        }

        None
    }

    /// Try to resolve a name in parent modules.
    fn resolve_in_parent_modules(&self, name: &str, from_file: &Path) -> Option<ResolveResult<'_>> {
        // Get the current module from file symbols
        let file_symbols = self.symbols_in_file(from_file);

        for symbol in &file_symbols {
            // Find the module path for this file
            if let Some((module_path, _)) = symbol.qualified.rsplit_once('.') {
                // Try progressively shorter module paths
                let mut current_module = module_path.to_string();
                loop {
                    let qualified = format!("{}.{}", current_module, name);
                    if let Some(resolved) = self.get_visible_from(&qualified, from_file) {
                        return Some(ResolveResult {
                            symbol: resolved,
                            resolution_path: ResolutionPath::ParentModule(current_module),
                        });
                    }

                    // Move up to parent module
                    match current_module.rsplit_once('.') {
                        Some((parent, _)) => current_module = parent.to_string(),
                        None => break,
                    }
                }
            }
        }

        None
    }

    /// Resolve a dotted name like "PaymentService.processPayment"
    /// This handles the case where the first part might be a module alias or nested module.
    pub fn resolve_dotted(&self, name: &str, from_file: &Path) -> Option<ResolveResult<'_>> {
        // First try direct resolution
        if let Some(result) = self.resolve(name, from_file) {
            return Some(result);
        }

        // For dotted names, try resolving the first component as a module
        if name.contains('.') {
            let parts: Vec<&str> = name.splitn(2, '.').collect();
            if parts.len() == 2 {
                let module_name = parts[0];
                let member_name = parts[1];

                // Check opens for matching module suffix
                let opens = self.opens_for_file(from_file);
                for open_module in opens {
                    if open_module.ends_with(module_name) {
                        // The open brings the module into scope
                        let qualified = format!("{}.{}", open_module, member_name);
                        if let Some(symbol) = self.get_visible_from(&qualified, from_file) {
                            return Some(ResolveResult {
                                symbol,
                                resolution_path: ResolutionPath::ViaOpen(open_module.clone()),
                            });
                        }
                    }

                    // Also try open.module.member pattern
                    let qualified = format!("{}.{}.{}", open_module, module_name, member_name);
                    if let Some(symbol) = self.get_visible_from(&qualified, from_file) {
                        return Some(ResolveResult {
                            symbol,
                            resolution_path: ResolutionPath::ViaOpen(open_module.clone()),
                        });
                    }
                }
            }
        }

        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Location, SymbolKind, Visibility};
    use std::path::PathBuf;

    fn make_symbol(name: &str, qualified: &str, file: &str) -> Symbol {
        Symbol {
            name: name.to_string(),
            qualified: qualified.to_string(),
            kind: SymbolKind::Function,
            location: Location::new(PathBuf::from(file), 1, 1),
            visibility: Visibility::Public,
        }
    }

    #[test]
    fn resolves_qualified_name() {
        let mut index = CodeIndex::new();
        index.add_symbol(make_symbol("helper", "MyApp.Utils.helper", "src/Utils.fs"));

        let result = index.resolve("MyApp.Utils.helper", Path::new("src/Main.fs"));
        assert!(result.is_some());
        assert_eq!(result.unwrap().resolution_path, ResolutionPath::Qualified);
    }

    #[test]
    fn resolves_in_same_file() {
        let mut index = CodeIndex::new();
        index.add_symbol(make_symbol("localFn", "MyApp.Main.localFn", "src/Main.fs"));

        let result = index.resolve("localFn", Path::new("src/Main.fs"));
        assert!(result.is_some());
        assert_eq!(result.unwrap().resolution_path, ResolutionPath::SameModule);
    }

    #[test]
    fn resolves_via_open() {
        let mut index = CodeIndex::new();
        index.add_symbol(make_symbol("helper", "MyApp.Utils.helper", "src/Utils.fs"));
        index.add_open(PathBuf::from("src/Main.fs"), "MyApp.Utils".to_string());

        let result = index.resolve("helper", Path::new("src/Main.fs"));
        assert!(result.is_some());

        let result = result.unwrap();
        assert!(matches!(result.resolution_path, ResolutionPath::ViaOpen(_)));
        assert_eq!(result.symbol.qualified, "MyApp.Utils.helper");
    }

    #[test]
    fn resolves_dotted_name_via_open() {
        let mut index = CodeIndex::new();
        index.add_symbol(make_symbol(
            "map",
            "FSharp.Collections.List.map",
            "stdlib.fs",
        ));
        index.add_open(
            PathBuf::from("src/Main.fs"),
            "FSharp.Collections".to_string(),
        );

        let result = index.resolve_dotted("List.map", Path::new("src/Main.fs"));
        assert!(result.is_some());
        assert_eq!(result.unwrap().symbol.name, "map");
    }

    #[test]
    fn returns_none_for_unknown_symbol() {
        let index = CodeIndex::new();
        let result = index.resolve("unknownSymbol", Path::new("src/Main.fs"));
        assert!(result.is_none());
    }

    #[test]
    fn resolves_with_open_statement() {
        let mut index = CodeIndex::new();

        // Add a symbol in Utils module
        index.add_symbol(make_symbol("helper", "MyApp.Utils.helper", "src/Utils.fs"));

        // Add another file that opens Utils
        index.add_symbol(make_symbol("run", "MyApp.Main.run", "src/Main.fs"));
        index.add_open(PathBuf::from("src/Main.fs"), "MyApp.Utils".to_string());

        // Resolve "helper" from Main.fs should find it via the open
        let resolved = index.resolve("helper", Path::new("src/Main.fs"));
        assert!(resolved.is_some());
        assert_eq!(
            resolved.unwrap().symbol.location.file,
            PathBuf::from("src/Utils.fs")
        );
    }
}
