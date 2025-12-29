//! Name resolution for Haxe symbols.
//!
//! Implements Haxe's scoping rules:
//! 1. Fully qualified names (e.g., `my.package.User`)
//! 2. Same-package symbols
//! 3. Import statements
//! 4. Same-file symbols

use std::path::Path;

use crate::resolve::{ResolutionPath, ResolveResult, SymbolResolver};
use crate::{CodeIndex, SymbolKind};

pub struct HaxeResolver;

impl SymbolResolver for HaxeResolver {
    fn resolve<'a>(
        &self,
        index: &'a CodeIndex,
        name: &str,
        from_file: &Path,
    ) -> Option<ResolveResult<'a>> {
        // 1. Try exact qualified name match (e.g., "my.package.User")
        if let Some(symbol) = index.get(name) {
            return Some(ResolveResult {
                symbol,
                resolution_path: ResolutionPath::Qualified,
            });
        }

        // 2. Try same-file resolution first (most common case)
        let file_symbols = index.symbols_in_file(from_file);
        for symbol in &file_symbols {
            if symbol.name == name {
                return Some(ResolveResult {
                    symbol,
                    resolution_path: ResolutionPath::SameModule,
                });
            }
        }

        // 3. Try via imports
        let file_opens = index.opens_for_file(from_file);
        for open in file_opens {
            // Try package.name pattern
            let qualified = format!("{}.{}", open, name);
            if let Some(symbol) = index.get(&qualified) {
                return Some(ResolveResult {
                    symbol,
                    resolution_path: ResolutionPath::ViaOpen(open.to_string()),
                });
            }
        }

        // 4. Try parent.name pattern for nested types and methods
        for symbol in &file_symbols {
            if symbol.kind == SymbolKind::Class
                || symbol.kind == SymbolKind::Interface
                || symbol.kind == SymbolKind::Type
            {
                let qualified = format!("{}.{}", symbol.qualified, name);
                if let Some(resolved) = index.get(&qualified) {
                    return Some(ResolveResult {
                        symbol: resolved,
                        resolution_path: ResolutionPath::SameModule,
                    });
                }
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
        // First try direct qualified lookup
        if let Some(symbol) = index.get(name) {
            return Some(ResolveResult {
                symbol,
                resolution_path: ResolutionPath::Qualified,
            });
        }

        // Try resolving the first part and building up
        if let Some(dot_pos) = name.find('.') {
            let first = &name[..dot_pos];
            let rest = &name[dot_pos + 1..];

            // Resolve the first part
            if let Some(result) = self.resolve(index, first, from_file) {
                // Now try to find the full path
                let full_name = format!("{}.{}", result.symbol.qualified, rest);
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
    use crate::{Location, Symbol, Visibility};
    use std::path::PathBuf;

    #[test]
    fn resolve_exact_qualified_name() {
        let mut index = CodeIndex::new();
        index.add_symbol(Symbol::new(
            "User".to_string(),
            "my.app.User".to_string(),
            SymbolKind::Class,
            Location::new(PathBuf::from("src/User.hx"), 3, 1),
            Visibility::Public,
            "haxe".to_string(),
        ));

        let resolver = HaxeResolver;
        let result = resolver.resolve(&index, "my.app.User", Path::new("Test.hx"));
        assert!(result.is_some());
        assert_eq!(result.unwrap().symbol.qualified, "my.app.User");
    }

    #[test]
    fn resolve_same_file_symbol() {
        let mut index = CodeIndex::new();
        let file = PathBuf::from("src/User.hx");

        index.add_symbol(Symbol::new(
            "User".to_string(),
            "User".to_string(),
            SymbolKind::Class,
            Location::new(file.clone(), 3, 1),
            Visibility::Public,
            "haxe".to_string(),
        ));
        index.add_symbol(Symbol::new(
            "save".to_string(),
            "User.save".to_string(),
            SymbolKind::Function,
            Location::new(file.clone(), 5, 5),
            Visibility::Public,
            "haxe".to_string(),
        ));

        let resolver = HaxeResolver;
        let result = resolver.resolve(&index, "User", &file);
        assert!(result.is_some());
        assert_eq!(result.unwrap().symbol.qualified, "User");
    }

    #[test]
    fn resolve_via_import() {
        let mut index = CodeIndex::new();
        let file = PathBuf::from("src/App.hx");

        index.add_symbol(Symbol::new(
            "User".to_string(),
            "my.models.User".to_string(),
            SymbolKind::Class,
            Location::new(PathBuf::from("models/User.hx"), 1, 1),
            Visibility::Public,
            "haxe".to_string(),
        ));

        index.add_open(file.clone(), "my.models".to_string());

        let resolver = HaxeResolver;
        let result = resolver.resolve(&index, "User", &file);
        assert!(result.is_some());
        let resolved = result.unwrap();
        assert_eq!(resolved.symbol.qualified, "my.models.User");
        assert!(matches!(
            resolved.resolution_path,
            ResolutionPath::ViaOpen(_)
        ));
    }

    #[test]
    fn resolve_dotted_method() {
        let mut index = CodeIndex::new();
        let file = PathBuf::from("src/App.hx");

        index.add_symbol(Symbol::new(
            "User".to_string(),
            "User".to_string(),
            SymbolKind::Class,
            Location::new(PathBuf::from("src/User.hx"), 3, 1),
            Visibility::Public,
            "haxe".to_string(),
        ));
        index.add_symbol(Symbol::new(
            "save".to_string(),
            "User.save".to_string(),
            SymbolKind::Function,
            Location::new(PathBuf::from("src/User.hx"), 5, 5),
            Visibility::Public,
            "haxe".to_string(),
        ));

        let resolver = HaxeResolver;
        let result = resolver.resolve_dotted(&index, "User.save", &file);
        assert!(result.is_some());
        assert_eq!(result.unwrap().symbol.qualified, "User.save");
    }

    #[test]
    fn returns_none_for_unknown_symbol() {
        let index = CodeIndex::new();
        let resolver = HaxeResolver;

        let result = resolver.resolve(&index, "NonExistent", Path::new("Test.hx"));
        assert!(result.is_none());
    }
}
