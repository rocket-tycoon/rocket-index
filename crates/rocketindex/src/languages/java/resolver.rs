//! Name resolution for Java symbols.
//!
//! Implements package-based resolution following Java's scoping rules:
//! 1. Fully qualified names (e.g., `com.example.User`)
//! 2. Same-package symbols
//! 3. Import statements
//! 4. Same-file symbols

use std::path::Path;

use crate::parse::ParseResult;
use crate::resolve::{ResolutionPath, ResolveResult, SymbolResolver};
use crate::{CodeIndex, Reference, SymbolKind};

pub struct JavaResolver;

impl SymbolResolver for JavaResolver {
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
        // Get the package from symbols in the current file
        let file_symbols = index.symbols_in_file(from_file);
        let mut current_package: Option<String> = None;

        for symbol in &file_symbols {
            // Extract package from qualified name (e.g., "com.example.User" -> "com.example")
            if let Some(dot_pos) = symbol.qualified.rfind('.') {
                let package = &symbol.qualified[..dot_pos];
                // Only use if it looks like a package (has dots or is the class itself)
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
            // Handle specific imports like "java.util.List"
            // The import might be the exact class we're looking for
            if open.ends_with(&format!(".{}", name)) {
                if let Some(symbol) = index.get(open) {
                    return Some(ResolveResult {
                        symbol,
                        resolution_path: ResolutionPath::ViaOpen(open.to_string()),
                    });
                }
            }

            // Try open.name pattern for nested classes or static members
            let qualified = format!("{}.{}", open, name);
            if let Some(symbol) = index.get(&qualified) {
                return Some(ResolveResult {
                    symbol,
                    resolution_path: ResolutionPath::ViaOpen(open.to_string()),
                });
            }
        }

        // 4. Try same-file resolution (inner classes, methods in same class)
        for symbol in &file_symbols {
            if symbol.name == name {
                return Some(ResolveResult {
                    symbol,
                    resolution_path: ResolutionPath::SameModule,
                });
            }

            // Try Parent.Name pattern for inner classes
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

impl JavaResolver {
    /// Resolve references in a Java file (legacy helper for reference extraction)
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
        }

        // Add import references
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
    use crate::languages::java::JavaParser;
    use crate::parse::LanguageParser;
    use crate::{CodeIndex, Location, Symbol, Visibility};
    use std::path::PathBuf;

    // === Legacy reference extraction tests ===

    #[test]
    fn resolves_qualified_name_parser() {
        let source = r#"
package com.example;

public class User {
    public void save() {}
}
"#;
        let parser = JavaParser;
        let result = parser.extract_symbols(Path::new("User.java"), source, 100);

        let method = result
            .symbols
            .iter()
            .find(|s| s.name == "save")
            .expect("Should find save method");
        assert_eq!(method.qualified, "com.example.User.save");
    }

    #[test]
    fn resolves_via_import_references() {
        let source = r#"
package com.example;

import java.util.List;
import java.util.Map;

public class App {}
"#;
        let parser = JavaParser;
        let result = parser.extract_symbols(Path::new("App.java"), source, 100);
        let refs = JavaResolver::resolve_references(Path::new("App.java"), source, &result);

        assert!(refs.iter().any(|r| r.name == "java.util.List"));
        assert!(refs.iter().any(|r| r.name == "java.util.Map"));
    }

    #[test]
    fn resolves_inheritance_reference() {
        let source = r#"
package com.example;

public class Dog extends Animal {
}
"#;
        let parser = JavaParser;
        let result = parser.extract_symbols(Path::new("Dog.java"), source, 100);
        let refs = JavaResolver::resolve_references(Path::new("Dog.java"), source, &result);

        let inherit_ref = refs
            .iter()
            .find(|r| r.name == "Animal")
            .expect("Should find Animal reference");
        assert_eq!(inherit_ref.name, "Animal");
    }

    // === SymbolResolver trait tests ===

    #[test]
    fn resolve_exact_qualified_name() {
        let mut index = CodeIndex::new();
        index.add_symbol(Symbol::new(
            "User".to_string(),
            "com.example.User".to_string(),
            SymbolKind::Class,
            Location::new(PathBuf::from("src/User.java"), 3, 1),
            Visibility::Public,
            "java".to_string(),
        ));

        let resolver = JavaResolver;
        let result = resolver.resolve(&index, "com.example.User", Path::new("Test.java"));
        assert!(result.is_some());
        assert_eq!(result.unwrap().symbol.qualified, "com.example.User");
    }

    #[test]
    fn resolve_same_package_symbol() {
        let mut index = CodeIndex::new();
        let file = PathBuf::from("src/com/example/App.java");

        // Add class in same package
        index.add_symbol(Symbol::new(
            "App".to_string(),
            "com.example.App".to_string(),
            SymbolKind::Class,
            Location::new(file.clone(), 3, 1),
            Visibility::Public,
            "java".to_string(),
        ));
        index.add_symbol(Symbol::new(
            "Helper".to_string(),
            "com.example.Helper".to_string(),
            SymbolKind::Class,
            Location::new(PathBuf::from("src/com/example/Helper.java"), 3, 1),
            Visibility::Public,
            "java".to_string(),
        ));

        let resolver = JavaResolver;
        // From App.java, should resolve "Helper" to "com.example.Helper"
        let result = resolver.resolve(&index, "Helper", &file);
        assert!(result.is_some());
        assert_eq!(result.unwrap().symbol.qualified, "com.example.Helper");
    }

    #[test]
    fn resolve_via_import() {
        let mut index = CodeIndex::new();
        let file = PathBuf::from("src/App.java");

        // Add symbol that will be imported
        index.add_symbol(Symbol::new(
            "List".to_string(),
            "java.util.List".to_string(),
            SymbolKind::Interface,
            Location::new(PathBuf::from("java/util/List.java"), 1, 1),
            Visibility::Public,
            "java".to_string(),
        ));

        // Add import statement
        index.add_open(file.clone(), "java.util.List".to_string());

        let resolver = JavaResolver;
        let result = resolver.resolve(&index, "List", &file);
        assert!(result.is_some());
        let resolved = result.unwrap();
        assert_eq!(resolved.symbol.qualified, "java.util.List");
        assert!(matches!(
            resolved.resolution_path,
            ResolutionPath::ViaOpen(_)
        ));
    }

    #[test]
    fn resolve_same_file_symbol() {
        let mut index = CodeIndex::new();
        let file = PathBuf::from("src/User.java");

        index.add_symbol(Symbol::new(
            "User".to_string(),
            "com.example.User".to_string(),
            SymbolKind::Class,
            Location::new(file.clone(), 3, 1),
            Visibility::Public,
            "java".to_string(),
        ));
        index.add_symbol(Symbol::new(
            "save".to_string(),
            "com.example.User.save".to_string(),
            SymbolKind::Function,
            Location::new(file.clone(), 5, 5),
            Visibility::Public,
            "java".to_string(),
        ));

        let resolver = JavaResolver;
        let result = resolver.resolve(&index, "save", &file);
        assert!(result.is_some());
        assert_eq!(result.unwrap().symbol.qualified, "com.example.User.save");
    }

    #[test]
    fn resolve_inner_class() {
        let mut index = CodeIndex::new();
        let file = PathBuf::from("src/Outer.java");

        index.add_symbol(Symbol::new(
            "Outer".to_string(),
            "com.example.Outer".to_string(),
            SymbolKind::Class,
            Location::new(file.clone(), 3, 1),
            Visibility::Public,
            "java".to_string(),
        ));
        index.add_symbol(Symbol::new(
            "Inner".to_string(),
            "com.example.Outer.Inner".to_string(),
            SymbolKind::Class,
            Location::new(file.clone(), 5, 5),
            Visibility::Public,
            "java".to_string(),
        ));

        let resolver = JavaResolver;
        // From same file, "Inner" should resolve to "Outer.Inner"
        let result = resolver.resolve(&index, "Inner", &file);
        assert!(result.is_some());
        assert_eq!(result.unwrap().symbol.qualified, "com.example.Outer.Inner");
    }

    #[test]
    fn resolve_dotted_method() {
        let mut index = CodeIndex::new();

        index.add_symbol(Symbol::new(
            "User".to_string(),
            "com.example.User".to_string(),
            SymbolKind::Class,
            Location::new(PathBuf::from("src/User.java"), 3, 1),
            Visibility::Public,
            "java".to_string(),
        ));
        index.add_symbol(Symbol::new(
            "save".to_string(),
            "com.example.User.save".to_string(),
            SymbolKind::Function,
            Location::new(PathBuf::from("src/User.java"), 5, 5),
            Visibility::Public,
            "java".to_string(),
        ));

        let resolver = JavaResolver;
        let result =
            resolver.resolve_dotted(&index, "com.example.User.save", Path::new("Test.java"));
        assert!(result.is_some());
        assert_eq!(result.unwrap().symbol.qualified, "com.example.User.save");
    }

    #[test]
    fn resolve_dotted_via_import() {
        let mut index = CodeIndex::new();
        let file = PathBuf::from("src/App.java");

        index.add_symbol(Symbol::new(
            "User".to_string(),
            "com.example.User".to_string(),
            SymbolKind::Class,
            Location::new(PathBuf::from("src/User.java"), 3, 1),
            Visibility::Public,
            "java".to_string(),
        ));
        index.add_symbol(Symbol::new(
            "save".to_string(),
            "com.example.User.save".to_string(),
            SymbolKind::Function,
            Location::new(PathBuf::from("src/User.java"), 5, 5),
            Visibility::Public,
            "java".to_string(),
        ));

        // Import the class
        index.add_open(file.clone(), "com.example.User".to_string());

        let resolver = JavaResolver;
        // "User.save" should resolve via import
        let result = resolver.resolve_dotted(&index, "User.save", &file);
        assert!(result.is_some());
        assert_eq!(result.unwrap().symbol.qualified, "com.example.User.save");
    }

    #[test]
    fn returns_none_for_unknown_symbol() {
        let index = CodeIndex::new();
        let resolver = JavaResolver;

        let result = resolver.resolve(&index, "NonExistent", Path::new("Test.java"));
        assert!(result.is_none());
    }
}
