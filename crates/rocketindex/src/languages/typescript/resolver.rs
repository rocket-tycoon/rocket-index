//! Name resolution for TypeScript.

use std::path::Path;

use crate::resolve::{ResolutionPath, ResolveResult, SymbolResolver};
use crate::{CodeIndex, SymbolKind};

pub struct TypeScriptResolver;

impl SymbolResolver for TypeScriptResolver {
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

        // 2. Try scoping relative to classes/interfaces in the current file
        let file_symbols = index.symbols_in_file(from_file);
        for symbol in file_symbols {
            if symbol.kind == SymbolKind::Class
                || symbol.kind == SymbolKind::Interface
                || symbol.kind == SymbolKind::Module
            {
                // Try Parent.Name pattern
                let qualified = format!("{}.{}", symbol.qualified, name);
                if let Some(resolved) = index.get(&qualified) {
                    return Some(ResolveResult {
                        symbol: resolved,
                        resolution_path: ResolutionPath::SameModule,
                    });
                }
            }
        }

        // 3. Try to resolve via imports (opens)
        let file_opens = index.opens_for_file(from_file);
        for open in file_opens {
            // For relative imports like "./utils", try matching against file paths
            // For package imports like "lodash", we can only do symbolic matching

            // Try open.name pattern
            let qualified = format!("{}.{}", open, name);
            if let Some(resolved) = index.get(&qualified) {
                return Some(ResolveResult {
                    symbol: resolved,
                    resolution_path: ResolutionPath::ViaOpen(open.to_string()),
                });
            }
        }

        // 4. Try looking for the name in the same file (module-level)
        let same_file_symbols = index.symbols_in_file(from_file);
        for symbol in same_file_symbols {
            if symbol.name == name {
                return Some(ResolveResult {
                    symbol,
                    resolution_path: ResolutionPath::SameModule,
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
        // First try direct lookup
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
    use crate::{CodeIndex, Location, Symbol, Visibility};
    use std::path::PathBuf;

    #[test]
    fn resolves_qualified_name() {
        let mut index = CodeIndex::new();
        index.add_symbol(Symbol::new(
            "User".to_string(),
            "models.User".to_string(),
            SymbolKind::Class,
            Location::new(PathBuf::from("src/models.ts"), 1, 1),
            Visibility::Public,
            "typescript".to_string(),
        ));

        let resolver = TypeScriptResolver;
        let result = resolver.resolve(&index, "models.User", Path::new("test.ts"));
        assert!(result.is_some());
        assert_eq!(result.unwrap().symbol.qualified, "models.User");
    }

    #[test]
    fn resolves_class_method() {
        let mut index = CodeIndex::new();
        index.add_symbol(Symbol::new(
            "User".to_string(),
            "User".to_string(),
            SymbolKind::Class,
            Location::new(PathBuf::from("src/user.ts"), 1, 1),
            Visibility::Public,
            "typescript".to_string(),
        ));
        index.add_symbol(Symbol::new(
            "greet".to_string(),
            "User.greet".to_string(),
            SymbolKind::Function,
            Location::new(PathBuf::from("src/user.ts"), 5, 5),
            Visibility::Public,
            "typescript".to_string(),
        ));

        let resolver = TypeScriptResolver;
        let result = resolver.resolve_dotted(&index, "User.greet", Path::new("test.ts"));
        assert!(result.is_some());
        assert_eq!(result.unwrap().symbol.qualified, "User.greet");
    }

    #[test]
    fn resolves_from_same_file() {
        let mut index = CodeIndex::new();
        let file = PathBuf::from("src/utils.ts");

        index.add_symbol(Symbol::new(
            "helper".to_string(),
            "helper".to_string(),
            SymbolKind::Function,
            Location::new(file.clone(), 1, 1),
            Visibility::Public,
            "typescript".to_string(),
        ));

        let resolver = TypeScriptResolver;
        let result = resolver.resolve(&index, "helper", Path::new("src/utils.ts"));
        assert!(result.is_some());
        assert_eq!(result.unwrap().symbol.name, "helper");
    }

    #[test]
    fn resolves_nested_member() {
        let mut index = CodeIndex::new();

        // Define class with nested member
        index.add_symbol(Symbol::new(
            "Config".to_string(),
            "Config".to_string(),
            SymbolKind::Class,
            Location::new(PathBuf::from("src/config.ts"), 1, 1),
            Visibility::Public,
            "typescript".to_string(),
        ));
        index.add_symbol(Symbol::new(
            "defaults".to_string(),
            "Config.defaults".to_string(),
            SymbolKind::Member,
            Location::new(PathBuf::from("src/config.ts"), 3, 5),
            Visibility::Public,
            "typescript".to_string(),
        ));

        let resolver = TypeScriptResolver;

        // From the config file, "defaults" should resolve via Config
        let result = resolver.resolve(&index, "defaults", Path::new("src/config.ts"));
        assert!(result.is_some());
        assert_eq!(result.unwrap().symbol.qualified, "Config.defaults");
    }

    #[test]
    fn resolves_via_import() {
        let mut index = CodeIndex::new();

        // Define a symbol in utils module
        index.add_symbol(Symbol::new(
            "formatDate".to_string(),
            "utils.formatDate".to_string(),
            SymbolKind::Function,
            Location::new(PathBuf::from("src/utils.ts"), 1, 1),
            Visibility::Public,
            "typescript".to_string(),
        ));

        // Add import in main.ts
        index.add_open(PathBuf::from("src/main.ts"), "utils".to_string());

        let resolver = TypeScriptResolver;
        let result = resolver.resolve(&index, "formatDate", Path::new("src/main.ts"));
        assert!(result.is_some());
        assert_eq!(result.unwrap().symbol.qualified, "utils.formatDate");
    }

    #[test]
    fn resolves_chained_dotted_name() {
        let mut index = CodeIndex::new();

        index.add_symbol(Symbol::new(
            "Api".to_string(),
            "Api".to_string(),
            SymbolKind::Class,
            Location::new(PathBuf::from("src/api.ts"), 1, 1),
            Visibility::Public,
            "typescript".to_string(),
        ));
        index.add_symbol(Symbol::new(
            "Users".to_string(),
            "Api.Users".to_string(),
            SymbolKind::Class,
            Location::new(PathBuf::from("src/api.ts"), 5, 5),
            Visibility::Public,
            "typescript".to_string(),
        ));
        index.add_symbol(Symbol::new(
            "getAll".to_string(),
            "Api.Users.getAll".to_string(),
            SymbolKind::Function,
            Location::new(PathBuf::from("src/api.ts"), 10, 10),
            Visibility::Public,
            "typescript".to_string(),
        ));

        let resolver = TypeScriptResolver;
        let result = resolver.resolve_dotted(&index, "Api.Users.getAll", Path::new("test.ts"));
        assert!(result.is_some());
        assert_eq!(result.unwrap().symbol.qualified, "Api.Users.getAll");
    }

    #[test]
    fn returns_none_for_unknown_symbol() {
        let index = CodeIndex::new();
        let resolver = TypeScriptResolver;

        let result = resolver.resolve(&index, "NonExistent", Path::new("test.ts"));
        assert!(result.is_none());
    }
}
