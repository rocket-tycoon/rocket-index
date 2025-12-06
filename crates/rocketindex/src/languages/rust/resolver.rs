//! Name resolution for Rust.

use std::path::Path;

use crate::resolve::{ResolutionPath, ResolveResult, SymbolResolver};
use crate::{CodeIndex, SymbolKind};

pub struct RustResolver;

impl SymbolResolver for RustResolver {
    fn resolve<'a>(
        &self,
        index: &'a CodeIndex,
        name: &str,
        from_file: &Path,
    ) -> Option<ResolveResult<'a>> {
        // 1. Try exact qualified name match
        if let Some(symbol) = index.get(name) {
            return Some(ResolveResult {
                symbol,
                resolution_path: ResolutionPath::Qualified,
            });
        }

        // 2. Try scoping relative to modules/structs/traits defined in the current file
        let file_symbols = index.symbols_in_file(from_file);
        for symbol in file_symbols {
            if symbol.kind == SymbolKind::Module
                || symbol.kind == SymbolKind::Class
                || symbol.kind == SymbolKind::Interface
            {
                // Try Parent::Name pattern
                let qualified = format!("{}::{}", symbol.qualified, name);
                if let Some(resolved) = index.get(&qualified) {
                    return Some(ResolveResult {
                        symbol: resolved,
                        resolution_path: ResolutionPath::SameModule,
                    });
                }
            }
        }

        // 3. Try to resolve via use statements (opens)
        let file_opens = index.opens_for_file(from_file);
        for open in file_opens {
            // For "use foo::bar", if we're looking for "bar", check if open ends with "::bar"
            if open.ends_with(&format!("::{}", name)) {
                if let Some(resolved) = index.get(open) {
                    return Some(ResolveResult {
                        symbol: resolved,
                        resolution_path: ResolutionPath::ViaOpen(open.to_string()),
                    });
                }
            }

            // Also try parent::name pattern
            let qualified = format!("{}::{}", open, name);
            if let Some(resolved) = index.get(&qualified) {
                return Some(ResolveResult {
                    symbol: resolved,
                    resolution_path: ResolutionPath::ViaOpen(open.to_string()),
                });
            }
        }

        // 4. Try looking for the name as a path by checking parent modules
        if name.contains("::") {
            if let Some(symbol) = index.get(name) {
                return Some(ResolveResult {
                    symbol,
                    resolution_path: ResolutionPath::Qualified,
                });
            }
        }

        None
    }

    fn resolve_dotted<'a>(
        &self,
        index: &'a CodeIndex,
        name: &str,
        from_file: &Path,
    ) -> Option<ResolveResult<'a>> {
        // For Rust, dotted names should be converted to :: paths
        // But we also handle cases where :: is already used

        // First try direct lookup (the name might already be in :: format)
        if let Some(symbol) = index.get(name) {
            return Some(ResolveResult {
                symbol,
                resolution_path: ResolutionPath::Qualified,
            });
        }

        // If the name contains dots, convert to :: and try again
        if name.contains('.') {
            let rust_path = name.replace('.', "::");
            if let Some(symbol) = index.get(&rust_path) {
                return Some(ResolveResult {
                    symbol,
                    resolution_path: ResolutionPath::Qualified,
                });
            }
        }

        // Try resolving the first part and building up
        let separator = if name.contains("::") { "::" } else { "." };
        if let Some(sep_pos) = name.find(separator) {
            let first = &name[..sep_pos];
            let rest = &name[sep_pos + separator.len()..];

            // Resolve the first part
            if let Some(result) = self.resolve(index, first, from_file) {
                // Now try to find the full path
                let full_name = format!("{}::{}", result.symbol.qualified, rest.replace('.', "::"));
                if let Some(symbol) = index.get(&full_name) {
                    return Some(ResolveResult {
                        symbol,
                        resolution_path: result.resolution_path,
                    });
                }
            }
        }

        // Fall back to normal resolution
        self.resolve(index, name, from_file)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CodeIndex, Location, Symbol, Visibility};
    use std::path::PathBuf;

    #[test]
    fn resolves_qualified_name() {
        let mut index = CodeIndex::new();
        index.add_symbol(Symbol::new(
            "MyStruct".to_string(),
            "mymod::MyStruct".to_string(),
            SymbolKind::Class,
            Location::new(PathBuf::from("src/mymod.rs"), 1, 1),
            Visibility::Public,
            "rust".to_string(),
        ));

        let resolver = RustResolver;
        let result = resolver.resolve(&index, "mymod::MyStruct", Path::new("test.rs"));
        assert!(result.is_some());
        assert_eq!(result.unwrap().symbol.qualified, "mymod::MyStruct");
    }

    #[test]
    fn resolves_via_same_file_module() {
        let mut index = CodeIndex::new();
        // Define module in utils.rs
        index.add_symbol(Symbol::new(
            "helpers".to_string(),
            "helpers".to_string(),
            SymbolKind::Module,
            Location::new(PathBuf::from("src/utils.rs"), 1, 1),
            Visibility::Public,
            "rust".to_string(),
        ));
        // Define function in that module
        index.add_symbol(Symbol::new(
            "process".to_string(),
            "helpers::process".to_string(),
            SymbolKind::Function,
            Location::new(PathBuf::from("src/utils.rs"), 5, 5),
            Visibility::Public,
            "rust".to_string(),
        ));

        // From utils.rs, "process" should resolve to "helpers::process"
        let resolver = RustResolver;
        let result = resolver.resolve(&index, "process", Path::new("src/utils.rs"));
        assert!(result.is_some());
        assert_eq!(result.unwrap().symbol.qualified, "helpers::process");
    }

    #[test]
    fn resolves_via_use_import() {
        let mut index = CodeIndex::new();

        // Define the target symbol
        index.add_symbol(Symbol::new(
            "HashMap".to_string(),
            "std::collections::HashMap".to_string(),
            SymbolKind::Class,
            Location::new(PathBuf::from("std/collections.rs"), 1, 1),
            Visibility::Public,
            "rust".to_string(),
        ));

        // Add a use statement in the calling file
        index.add_open(
            PathBuf::from("src/main.rs"),
            "std::collections::HashMap".to_string(),
        );

        let resolver = RustResolver;
        let result = resolver.resolve(&index, "HashMap", Path::new("src/main.rs"));
        assert!(result.is_some());
        let resolved = result.unwrap();
        assert_eq!(resolved.symbol.qualified, "std::collections::HashMap");
        assert!(matches!(
            resolved.resolution_path,
            ResolutionPath::ViaOpen(_)
        ));
    }

    #[test]
    fn resolves_impl_method() {
        let mut index = CodeIndex::new();

        // Define struct and its method
        index.add_symbol(Symbol::new(
            "Calculator".to_string(),
            "Calculator".to_string(),
            SymbolKind::Class,
            Location::new(PathBuf::from("src/calc.rs"), 1, 1),
            Visibility::Public,
            "rust".to_string(),
        ));
        index.add_symbol(Symbol::new(
            "add".to_string(),
            "Calculator::add".to_string(),
            SymbolKind::Function,
            Location::new(PathBuf::from("src/calc.rs"), 5, 5),
            Visibility::Public,
            "rust".to_string(),
        ));

        let resolver = RustResolver;
        let result = resolver.resolve_dotted(&index, "Calculator::add", Path::new("test.rs"));
        assert!(result.is_some());
        assert_eq!(result.unwrap().symbol.qualified, "Calculator::add");
    }

    #[test]
    fn resolves_nested_module_path() {
        let mut index = CodeIndex::new();

        index.add_symbol(Symbol::new(
            "helpers".to_string(),
            "utils::helpers".to_string(),
            SymbolKind::Module,
            Location::new(PathBuf::from("src/utils/helpers.rs"), 1, 1),
            Visibility::Public,
            "rust".to_string(),
        ));
        index.add_symbol(Symbol::new(
            "format".to_string(),
            "utils::helpers::format".to_string(),
            SymbolKind::Function,
            Location::new(PathBuf::from("src/utils/helpers.rs"), 5, 5),
            Visibility::Public,
            "rust".to_string(),
        ));

        let resolver = RustResolver;

        // Direct qualified lookup
        let result =
            resolver.resolve_dotted(&index, "utils::helpers::format", Path::new("test.rs"));
        assert!(result.is_some());
        assert_eq!(result.unwrap().symbol.qualified, "utils::helpers::format");
    }

    #[test]
    fn resolves_trait_method() {
        let mut index = CodeIndex::new();

        // Define trait and its method
        index.add_symbol(Symbol::new(
            "Display".to_string(),
            "Display".to_string(),
            SymbolKind::Interface,
            Location::new(PathBuf::from("src/traits.rs"), 1, 1),
            Visibility::Public,
            "rust".to_string(),
        ));
        index.add_symbol(Symbol::new(
            "fmt".to_string(),
            "Display::fmt".to_string(),
            SymbolKind::Function,
            Location::new(PathBuf::from("src/traits.rs"), 3, 5),
            Visibility::Public,
            "rust".to_string(),
        ));

        let resolver = RustResolver;

        // From the traits file, "fmt" should resolve via Display
        let result = resolver.resolve(&index, "fmt", Path::new("src/traits.rs"));
        assert!(result.is_some());
        assert_eq!(result.unwrap().symbol.qualified, "Display::fmt");
    }

    #[test]
    fn resolves_via_module_use() {
        let mut index = CodeIndex::new();

        // Define a module with a function
        index.add_symbol(Symbol::new(
            "utils".to_string(),
            "crate::utils".to_string(),
            SymbolKind::Module,
            Location::new(PathBuf::from("src/utils.rs"), 1, 1),
            Visibility::Public,
            "rust".to_string(),
        ));
        index.add_symbol(Symbol::new(
            "helper".to_string(),
            "crate::utils::helper".to_string(),
            SymbolKind::Function,
            Location::new(PathBuf::from("src/utils.rs"), 5, 5),
            Visibility::Public,
            "rust".to_string(),
        ));

        // Add use crate::utils in main
        index.add_open(PathBuf::from("src/main.rs"), "crate::utils".to_string());

        let resolver = RustResolver;
        let result = resolver.resolve(&index, "helper", Path::new("src/main.rs"));
        assert!(result.is_some());
        assert_eq!(result.unwrap().symbol.qualified, "crate::utils::helper");
    }
}
