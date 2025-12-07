//! Name resolution for JavaScript.

use std::path::Path;

use crate::resolve::{ResolutionPath, ResolveResult, SymbolResolver};
use crate::{CodeIndex, SymbolKind};

pub struct JavaScriptResolver;

impl SymbolResolver for JavaScriptResolver {
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

        // 2. Try scoping relative to classes in the current file
        let file_symbols = index.symbols_in_file(from_file);
        for symbol in file_symbols {
            if symbol.kind == SymbolKind::Class || symbol.kind == SymbolKind::Module {
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
            let qualified = format!("{}.{}", open, name);
            if let Some(resolved) = index.get(&qualified) {
                return Some(ResolveResult {
                    symbol: resolved,
                    resolution_path: ResolutionPath::ViaOpen(open.to_string()),
                });
            }
        }

        // 4. Try looking for the name in the same file
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

            if let Some(result) = self.resolve(index, first, from_file) {
                let full_name = format!("{}.{}", result.symbol.qualified, rest);
                if let Some(symbol) = index.get(&full_name) {
                    return Some(ResolveResult {
                        symbol,
                        resolution_path: result.resolution_path,
                    });
                }
            }
        }

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
            Location::new(PathBuf::from("src/models.js"), 1, 1),
            Visibility::Public,
            "javascript".to_string(),
        ));

        let resolver = JavaScriptResolver;
        let result = resolver.resolve(&index, "models.User", Path::new("test.js"));
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
            Location::new(PathBuf::from("src/user.js"), 1, 1),
            Visibility::Public,
            "javascript".to_string(),
        ));
        index.add_symbol(Symbol::new(
            "greet".to_string(),
            "User.greet".to_string(),
            SymbolKind::Function,
            Location::new(PathBuf::from("src/user.js"), 5, 5),
            Visibility::Public,
            "javascript".to_string(),
        ));

        let resolver = JavaScriptResolver;
        let result = resolver.resolve_dotted(&index, "User.greet", Path::new("test.js"));
        assert!(result.is_some());
        assert_eq!(result.unwrap().symbol.qualified, "User.greet");
    }

    #[test]
    fn resolves_from_same_file() {
        let mut index = CodeIndex::new();
        let file = PathBuf::from("src/utils.js");

        index.add_symbol(Symbol::new(
            "helper".to_string(),
            "helper".to_string(),
            SymbolKind::Function,
            Location::new(file.clone(), 1, 1),
            Visibility::Public,
            "javascript".to_string(),
        ));

        let resolver = JavaScriptResolver;
        let result = resolver.resolve(&index, "helper", Path::new("src/utils.js"));
        assert!(result.is_some());
        assert_eq!(result.unwrap().symbol.name, "helper");
    }

    #[test]
    fn resolves_via_import() {
        let mut index = CodeIndex::new();

        // Define a symbol in utils module
        index.add_symbol(Symbol::new(
            "formatDate".to_string(),
            "utils.formatDate".to_string(),
            SymbolKind::Function,
            Location::new(PathBuf::from("src/utils.js"), 1, 1),
            Visibility::Public,
            "javascript".to_string(),
        ));

        // Add import in main.js
        index.add_open(PathBuf::from("src/main.js"), "utils".to_string());

        let resolver = JavaScriptResolver;
        let result = resolver.resolve(&index, "formatDate", Path::new("src/main.js"));
        assert!(result.is_some());
        assert_eq!(result.unwrap().symbol.qualified, "utils.formatDate");
    }

    #[test]
    fn returns_none_for_unknown_symbol() {
        let index = CodeIndex::new();
        let resolver = JavaScriptResolver;

        let result = resolver.resolve(&index, "NonExistent", Path::new("test.js"));
        assert!(result.is_none());
    }
}
