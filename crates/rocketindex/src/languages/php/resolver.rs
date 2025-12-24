//! Name resolution for PHP symbols.
//!
//! Implements namespace-based resolution following PHP's scoping rules:
//! 1. Fully qualified names (e.g., `App\Services\UserService`)
//! 2. Same-namespace symbols
//! 3. Use statements
//! 4. Same-file symbols

use std::path::Path;

use crate::parse::ParseResult;
use crate::resolve::{ResolutionPath, ResolveResult, SymbolResolver};
use crate::{CodeIndex, Reference, SymbolKind};

pub struct PhpResolver;

impl SymbolResolver for PhpResolver {
    fn resolve<'a>(
        &self,
        index: &'a CodeIndex,
        name: &str,
        from_file: &Path,
    ) -> Option<ResolveResult<'a>> {
        // 1. Try exact qualified name match (e.g., "App\Services\UserService")
        if let Some(symbol) = index.get(name) {
            return Some(ResolveResult {
                symbol,
                resolution_path: ResolutionPath::Qualified,
            });
        }

        // 2. Try same-namespace resolution
        // Get the namespace from symbols in the current file
        let file_symbols = index.symbols_in_file(from_file);
        let mut current_namespace: Option<String> = None;

        for symbol in &file_symbols {
            // Extract namespace from qualified name (e.g., "App\Services\UserService" -> "App\Services")
            if let Some(slash_pos) = symbol.qualified.rfind('\\') {
                let namespace = &symbol.qualified[..slash_pos];
                // Use this as the namespace if the symbol is a class, interface, or trait
                if symbol.kind == SymbolKind::Class || symbol.kind == SymbolKind::Interface {
                    current_namespace = Some(namespace.to_string());
                    break;
                }
            }
        }

        if let Some(namespace) = &current_namespace {
            let qualified = format!("{}\\{}", namespace, name);
            if let Some(symbol) = index.get(&qualified) {
                return Some(ResolveResult {
                    symbol,
                    resolution_path: ResolutionPath::SameModule,
                });
            }
        }

        // 3. Try via use statements (opens)
        let file_opens = index.opens_for_file(from_file);
        for open in file_opens {
            // Handle specific use like "App\Models\User"
            // The use might be the exact class we're looking for
            if open.ends_with(&format!("\\{}", name)) {
                if let Some(symbol) = index.get(open) {
                    return Some(ResolveResult {
                        symbol,
                        resolution_path: ResolutionPath::ViaOpen(open.to_string()),
                    });
                }
            }

            // Try namespace\name pattern
            let qualified = format!("{}\\{}", open, name);
            if let Some(symbol) = index.get(&qualified) {
                return Some(ResolveResult {
                    symbol,
                    resolution_path: ResolutionPath::ViaOpen(open.to_string()),
                });
            }
        }

        // 4. Try same-file resolution (methods in same class)
        for symbol in &file_symbols {
            if symbol.name == name {
                return Some(ResolveResult {
                    symbol,
                    resolution_path: ResolutionPath::SameModule,
                });
            }

            // Try Class\Method pattern for methods within the same class
            if symbol.kind == SymbolKind::Class || symbol.kind == SymbolKind::Interface {
                let qualified = format!("{}\\{}", symbol.qualified, name);
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
        // PHP uses backslashes for namespaces, not dots
        // But we still support this for consistency with other resolvers

        // First try direct qualified lookup
        if let Some(symbol) = index.get(name) {
            return Some(ResolveResult {
                symbol,
                resolution_path: ResolutionPath::Qualified,
            });
        }

        // Try resolving the first part and building up (using backslash)
        if let Some(slash_pos) = name.find('\\') {
            let first = &name[..slash_pos];
            let rest = &name[slash_pos + 1..];

            // Resolve the first part
            if let Some(result) = self.resolve(index, first, from_file) {
                // Now try to find the full path
                let full_name = format!("{}\\{}", result.symbol.qualified, rest);
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

impl PhpResolver {
    /// Resolve references in a PHP file (legacy helper for reference extraction)
    pub fn resolve_references(
        file: &Path,
        _source: &str,
        parse_result: &ParseResult,
    ) -> Vec<Reference> {
        let mut references = Vec::new();

        // For each symbol, look for references to other symbols
        for symbol in &parse_result.symbols {
            // Check for parent class reference
            if let Some(parent) = &symbol.parent {
                references.push(Reference {
                    name: parent.clone(),
                    location: symbol.location.clone(),
                });
            }

            // Check for implemented interfaces
            if let Some(impls) = &symbol.implements {
                for iface in impls {
                    references.push(Reference {
                        name: iface.clone(),
                        location: symbol.location.clone(),
                    });
                }
            }

            // Check for used traits
            if let Some(traits) = &symbol.mixins {
                for trait_name in traits {
                    references.push(Reference {
                        name: trait_name.clone(),
                        location: symbol.location.clone(),
                    });
                }
            }
        }

        // Add use statement references
        for open in &parse_result.opens {
            references.push(Reference {
                name: open.clone(),
                location: crate::Location {
                    file: file.to_path_buf(),
                    line: 1,
                    column: 1,
                    end_line: 1,
                    end_column: 1,
                },
            });
        }

        references
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::languages::php::PhpParser;
    use crate::parse::LanguageParser;
    use crate::{CodeIndex, Location, Symbol, Visibility};
    use std::path::PathBuf;

    // === Legacy reference extraction tests ===

    #[test]
    fn resolves_qualified_name() {
        let source = r#"<?php
namespace App\Services;

class UserService {
    public function save(): void {}
}
"#;
        let parser = PhpParser;
        let result = parser.extract_symbols(Path::new("UserService.php"), source, 100);

        let method = result
            .symbols
            .iter()
            .find(|s| s.name == "save")
            .expect("Should find save method");
        assert_eq!(method.qualified, "App\\Services\\UserService\\save");
    }

    #[test]
    fn resolves_via_use() {
        let source = r#"<?php
namespace App\Controllers;

use App\Models\User;
use App\Services\UserService;

class UserController {}
"#;
        let parser = PhpParser;
        let result = parser.extract_symbols(Path::new("UserController.php"), source, 100);
        let refs =
            PhpResolver::resolve_references(Path::new("UserController.php"), source, &result);

        assert!(refs.iter().any(|r| r.name == "App\\Models\\User"));
        assert!(refs.iter().any(|r| r.name == "App\\Services\\UserService"));
    }

    #[test]
    fn resolves_inheritance() {
        let source = r#"<?php
namespace App\Models;

class Admin extends User {
}
"#;
        let parser = PhpParser;
        let result = parser.extract_symbols(Path::new("Admin.php"), source, 100);
        let refs = PhpResolver::resolve_references(Path::new("Admin.php"), source, &result);

        let inherit_ref = refs
            .iter()
            .find(|r| r.name == "User")
            .expect("Should find User reference");
        assert_eq!(inherit_ref.name, "User");
    }

    #[test]
    fn resolves_trait_usage() {
        let source = r#"<?php
namespace App\Models;

class User {
    use HasRoles;
    use Notifiable;
}
"#;
        let parser = PhpParser;
        let result = parser.extract_symbols(Path::new("User.php"), source, 100);
        let refs = PhpResolver::resolve_references(Path::new("User.php"), source, &result);

        assert!(refs.iter().any(|r| r.name == "HasRoles"));
        assert!(refs.iter().any(|r| r.name == "Notifiable"));
    }

    // === SymbolResolver trait tests ===

    #[test]
    fn resolve_exact_qualified_name() {
        let mut index = CodeIndex::new();
        index.add_symbol(Symbol::new(
            "UserService".to_string(),
            "App\\Services\\UserService".to_string(),
            SymbolKind::Class,
            Location::new(PathBuf::from("src/UserService.php"), 3, 1),
            Visibility::Public,
            "php".to_string(),
        ));

        let resolver = PhpResolver;
        let result = resolver.resolve(&index, "App\\Services\\UserService", Path::new("Test.php"));
        assert!(result.is_some());
        assert_eq!(
            result.unwrap().symbol.qualified,
            "App\\Services\\UserService"
        );
    }

    #[test]
    fn resolve_same_namespace_symbol() {
        let mut index = CodeIndex::new();
        let file = PathBuf::from("src/App/Services/UserService.php");

        // Add class in same namespace
        index.add_symbol(Symbol::new(
            "UserService".to_string(),
            "App\\Services\\UserService".to_string(),
            SymbolKind::Class,
            Location::new(file.clone(), 3, 1),
            Visibility::Public,
            "php".to_string(),
        ));
        index.add_symbol(Symbol::new(
            "EmailService".to_string(),
            "App\\Services\\EmailService".to_string(),
            SymbolKind::Class,
            Location::new(PathBuf::from("src/App/Services/EmailService.php"), 3, 1),
            Visibility::Public,
            "php".to_string(),
        ));

        let resolver = PhpResolver;
        // From UserService.php, should resolve "EmailService" to "App\Services\EmailService"
        let result = resolver.resolve(&index, "EmailService", &file);
        assert!(result.is_some());
        assert_eq!(
            result.unwrap().symbol.qualified,
            "App\\Services\\EmailService"
        );
    }

    #[test]
    fn resolve_via_use_statement() {
        let mut index = CodeIndex::new();
        let file = PathBuf::from("src/UserController.php");

        // Add symbol that will be imported
        index.add_symbol(Symbol::new(
            "User".to_string(),
            "App\\Models\\User".to_string(),
            SymbolKind::Class,
            Location::new(PathBuf::from("src/Models/User.php"), 1, 1),
            Visibility::Public,
            "php".to_string(),
        ));

        // Add use statement
        index.add_open(file.clone(), "App\\Models\\User".to_string());

        let resolver = PhpResolver;
        let result = resolver.resolve(&index, "User", &file);
        assert!(result.is_some());
        let resolved = result.unwrap();
        assert_eq!(resolved.symbol.qualified, "App\\Models\\User");
        assert!(matches!(
            resolved.resolution_path,
            ResolutionPath::ViaOpen(_)
        ));
    }

    #[test]
    fn resolve_same_file_symbol() {
        let mut index = CodeIndex::new();
        let file = PathBuf::from("src/UserService.php");

        index.add_symbol(Symbol::new(
            "UserService".to_string(),
            "App\\Services\\UserService".to_string(),
            SymbolKind::Class,
            Location::new(file.clone(), 3, 1),
            Visibility::Public,
            "php".to_string(),
        ));
        index.add_symbol(Symbol::new(
            "save".to_string(),
            "App\\Services\\UserService\\save".to_string(),
            SymbolKind::Function,
            Location::new(file.clone(), 5, 5),
            Visibility::Public,
            "php".to_string(),
        ));

        let resolver = PhpResolver;
        let result = resolver.resolve(&index, "save", &file);
        assert!(result.is_some());
        assert_eq!(
            result.unwrap().symbol.qualified,
            "App\\Services\\UserService\\save"
        );
    }

    #[test]
    fn resolve_dotted_method() {
        let mut index = CodeIndex::new();

        index.add_symbol(Symbol::new(
            "UserService".to_string(),
            "App\\Services\\UserService".to_string(),
            SymbolKind::Class,
            Location::new(PathBuf::from("src/UserService.php"), 3, 1),
            Visibility::Public,
            "php".to_string(),
        ));
        index.add_symbol(Symbol::new(
            "save".to_string(),
            "App\\Services\\UserService\\save".to_string(),
            SymbolKind::Function,
            Location::new(PathBuf::from("src/UserService.php"), 5, 5),
            Visibility::Public,
            "php".to_string(),
        ));

        let resolver = PhpResolver;
        let result = resolver.resolve_dotted(
            &index,
            "App\\Services\\UserService\\save",
            Path::new("Test.php"),
        );
        assert!(result.is_some());
        assert_eq!(
            result.unwrap().symbol.qualified,
            "App\\Services\\UserService\\save"
        );
    }

    #[test]
    fn resolve_via_namespace_use() {
        let mut index = CodeIndex::new();
        let file = PathBuf::from("src/UserController.php");

        index.add_symbol(Symbol::new(
            "User".to_string(),
            "App\\Models\\User".to_string(),
            SymbolKind::Class,
            Location::new(PathBuf::from("src/User.php"), 3, 1),
            Visibility::Public,
            "php".to_string(),
        ));
        index.add_symbol(Symbol::new(
            "save".to_string(),
            "App\\Models\\User\\save".to_string(),
            SymbolKind::Function,
            Location::new(PathBuf::from("src/User.php"), 5, 5),
            Visibility::Public,
            "php".to_string(),
        ));

        // Import the namespace
        index.add_open(file.clone(), "App\\Models".to_string());

        let resolver = PhpResolver;
        // "User" should resolve via use
        let result = resolver.resolve(&index, "User", &file);
        assert!(result.is_some());
        assert_eq!(result.unwrap().symbol.qualified, "App\\Models\\User");
    }

    #[test]
    fn returns_none_for_unknown_symbol() {
        let index = CodeIndex::new();
        let resolver = PhpResolver;

        let result = resolver.resolve(&index, "NonExistent", Path::new("Test.php"));
        assert!(result.is_none());
    }
}
