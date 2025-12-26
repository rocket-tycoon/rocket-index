//! Name resolution for Kotlin symbols.
//!
//! Implements package-based resolution following Kotlin's scoping rules:
//! 1. Fully qualified names (e.g., `com.example.User`)
//! 2. Same-package symbols
//! 3. Import statements
//! 4. Same-file symbols

use std::path::Path;

use crate::resolve::{ResolutionPath, ResolveResult, SymbolResolver};
use crate::{CodeIndex, SymbolKind};

pub struct KotlinResolver;

impl SymbolResolver for KotlinResolver {
    fn resolve<'a>(
        &self,
        index: &'a CodeIndex,
        name: &str,
        from_file: &Path,
    ) -> Option<ResolveResult<'a>> {
        // 1. Try exact qualified name match (e.g., "com.example.User")
        if let Some(symbol) = index.get(name) {
            return Some(ResolveResult {
                symbol,
                resolution_path: ResolutionPath::Qualified,
            });
        }

        // 2. Try same-package resolution
        let file_symbols = index.symbols_in_file(from_file);
        let mut current_package: Option<String> = None;

        for symbol in &file_symbols {
            // Extract package from qualified name
            if let Some(dot_pos) = symbol.qualified.rfind('.') {
                let package = &symbol.qualified[..dot_pos];
                if package.contains('.') || symbol.kind == SymbolKind::Class {
                    current_package = Some(package.to_string());
                    break;
                }
            }
        }

        if let Some(package) = &current_package {
            let qualified = format!("{}.{}", package, name);
            if let Some(symbol) = index.get(&qualified) {
                return Some(ResolveResult {
                    symbol,
                    resolution_path: ResolutionPath::SameModule,
                });
            }
        }

        // 3. Try via imports (opens)
        let file_opens = index.opens_for_file(from_file);
        for open in file_opens {
            // Handle specific imports like "kotlin.collections.List"
            if open.ends_with(&format!(".{}", name)) {
                if let Some(symbol) = index.get(open) {
                    return Some(ResolveResult {
                        symbol,
                        resolution_path: ResolutionPath::ViaOpen(open.to_string()),
                    });
                }
            }

            // Try open.name pattern for nested classes or companion members
            let qualified = format!("{}.{}", open, name);
            if let Some(symbol) = index.get(&qualified) {
                return Some(ResolveResult {
                    symbol,
                    resolution_path: ResolutionPath::ViaOpen(open.to_string()),
                });
            }
        }

        // 4. Try same-file resolution
        for symbol in &file_symbols {
            if symbol.name == name {
                return Some(ResolveResult {
                    symbol,
                    resolution_path: ResolutionPath::SameModule,
                });
            }

            // Try Parent.Name pattern for nested classes
            if symbol.kind == SymbolKind::Class || symbol.kind == SymbolKind::Interface {
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
            "com.example.User".to_string(),
            SymbolKind::Class,
            Location::new(PathBuf::from("src/User.kt"), 3, 1),
            Visibility::Public,
            "kotlin".to_string(),
        ));

        let resolver = KotlinResolver;
        let result = resolver.resolve(&index, "com.example.User", Path::new("Test.kt"));
        assert!(result.is_some());
        assert_eq!(result.unwrap().symbol.qualified, "com.example.User");
    }

    #[test]
    fn resolve_same_package_symbol() {
        let mut index = CodeIndex::new();
        let file = PathBuf::from("src/com/example/App.kt");

        index.add_symbol(Symbol::new(
            "App".to_string(),
            "com.example.App".to_string(),
            SymbolKind::Class,
            Location::new(file.clone(), 3, 1),
            Visibility::Public,
            "kotlin".to_string(),
        ));
        index.add_symbol(Symbol::new(
            "Helper".to_string(),
            "com.example.Helper".to_string(),
            SymbolKind::Class,
            Location::new(PathBuf::from("src/com/example/Helper.kt"), 3, 1),
            Visibility::Public,
            "kotlin".to_string(),
        ));

        let resolver = KotlinResolver;
        let result = resolver.resolve(&index, "Helper", &file);
        assert!(result.is_some());
        assert_eq!(result.unwrap().symbol.qualified, "com.example.Helper");
    }

    #[test]
    fn resolve_via_import() {
        let mut index = CodeIndex::new();
        let file = PathBuf::from("src/App.kt");

        index.add_symbol(Symbol::new(
            "List".to_string(),
            "kotlin.collections.List".to_string(),
            SymbolKind::Interface,
            Location::new(PathBuf::from("kotlin/collections/List.kt"), 1, 1),
            Visibility::Public,
            "kotlin".to_string(),
        ));

        index.add_open(file.clone(), "kotlin.collections.List".to_string());

        let resolver = KotlinResolver;
        let result = resolver.resolve(&index, "List", &file);
        assert!(result.is_some());
        let resolved = result.unwrap();
        assert_eq!(resolved.symbol.qualified, "kotlin.collections.List");
        assert!(matches!(
            resolved.resolution_path,
            ResolutionPath::ViaOpen(_)
        ));
    }

    #[test]
    fn resolve_same_file_symbol() {
        let mut index = CodeIndex::new();
        let file = PathBuf::from("src/User.kt");

        index.add_symbol(Symbol::new(
            "User".to_string(),
            "com.example.User".to_string(),
            SymbolKind::Class,
            Location::new(file.clone(), 3, 1),
            Visibility::Public,
            "kotlin".to_string(),
        ));
        index.add_symbol(Symbol::new(
            "save".to_string(),
            "com.example.User.save".to_string(),
            SymbolKind::Function,
            Location::new(file.clone(), 5, 5),
            Visibility::Public,
            "kotlin".to_string(),
        ));

        let resolver = KotlinResolver;
        let result = resolver.resolve(&index, "save", &file);
        assert!(result.is_some());
        assert_eq!(result.unwrap().symbol.qualified, "com.example.User.save");
    }

    #[test]
    fn resolve_dotted_method() {
        let mut index = CodeIndex::new();
        let file = PathBuf::from("src/App.kt");

        index.add_symbol(Symbol::new(
            "User".to_string(),
            "com.example.User".to_string(),
            SymbolKind::Class,
            Location::new(PathBuf::from("src/User.kt"), 3, 1),
            Visibility::Public,
            "kotlin".to_string(),
        ));
        index.add_symbol(Symbol::new(
            "save".to_string(),
            "com.example.User.save".to_string(),
            SymbolKind::Function,
            Location::new(PathBuf::from("src/User.kt"), 5, 5),
            Visibility::Public,
            "kotlin".to_string(),
        ));

        index.add_open(file.clone(), "com.example.User".to_string());

        let resolver = KotlinResolver;
        let result = resolver.resolve_dotted(&index, "User.save", &file);
        assert!(result.is_some());
        assert_eq!(result.unwrap().symbol.qualified, "com.example.User.save");
    }

    #[test]
    fn returns_none_for_unknown_symbol() {
        let index = CodeIndex::new();
        let resolver = KotlinResolver;

        let result = resolver.resolve(&index, "NonExistent", Path::new("Test.kt"));
        assert!(result.is_none());
    }
}
