//! Name resolution for C++ symbols.
//!
//! Implements namespace-based resolution following C++'s scoping rules:
//! 1. Fully qualified names (e.g., `std::vector`)
//! 2. Same-namespace symbols
//! 3. Using declarations/directives
//! 4. Same-file symbols

use std::path::Path;

use crate::parse::ParseResult;
use crate::resolve::{ResolutionPath, ResolveResult, SymbolResolver};
use crate::{CodeIndex, Reference, Symbol, SymbolKind};

pub struct CppResolver;

/// Select the best matching symbol from a list of candidates.
/// Priorities:
/// 1. Source files (.c, .cpp, .cc, .cxx)
/// 2. Header files (.h, .hpp, .hh, .hxx)
/// 3. Others/Last added
fn select_best_match(symbols: &[Symbol]) -> Option<&Symbol> {
    if symbols.is_empty() {
        return None;
    }
    symbols.iter().max_by_key(|s| {
        let ext = s
            .location
            .file
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        match ext.as_str() {
            "cpp" | "cc" | "c" | "cxx" => 2,
            "hpp" | "h" | "hh" | "hxx" => 1,
            _ => 0,
        }
    })
}

impl SymbolResolver for CppResolver {
    fn resolve<'a>(
        &self,
        index: &'a CodeIndex,
        name: &str,
        from_file: &Path,
    ) -> Option<ResolveResult<'a>> {
        // 1. Try exact qualified name match (e.g., "std::vector")
        if let Some(symbol) = select_best_match(index.get_all(name)) {
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
            // Extract namespace from qualified name (e.g., "mylib::MyClass" -> "mylib")
            if let Some(colon_pos) = symbol.qualified.rfind("::") {
                let namespace = &symbol.qualified[..colon_pos];
                if symbol.kind == SymbolKind::Class
                    || symbol.kind == SymbolKind::Interface
                    || symbol.kind == SymbolKind::Record
                {
                    current_namespace = Some(namespace.to_string());
                    break;
                }
            }
        }

        if let Some(namespace) = &current_namespace {
            let qualified = format!("{}::{}", namespace, name);
            if let Some(symbol) = select_best_match(index.get_all(&qualified)) {
                return Some(ResolveResult {
                    symbol,
                    resolution_path: ResolutionPath::SameModule,
                });
            }
        }

        // 3. Try via using declarations (opens)
        let file_opens = index.opens_for_file(from_file);
        for open in file_opens {
            // Handle specific using like "std::vector"
            if open.ends_with(&format!("::{}", name)) {
                if let Some(symbol) = select_best_match(index.get_all(open)) {
                    return Some(ResolveResult {
                        symbol,
                        resolution_path: ResolutionPath::ViaOpen(open.to_string()),
                    });
                }
            }

            // Try namespace::name pattern
            let qualified = format!("{}::{}", open, name);
            if let Some(symbol) = select_best_match(index.get_all(&qualified)) {
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

            // Try Class::Method pattern
            if symbol.kind == SymbolKind::Class || symbol.kind == SymbolKind::Record {
                let qualified = format!("{}::{}", symbol.qualified, name);
                if let Some(resolved) = select_best_match(index.get_all(&qualified)) {
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
        // C++ uses :: for namespaces, not dots
        // First try direct qualified lookup
        if let Some(symbol) = select_best_match(index.get_all(name)) {
            return Some(ResolveResult {
                symbol,
                resolution_path: ResolutionPath::Qualified,
            });
        }

        // Try resolving the first part and building up (using ::)
        if let Some(colon_pos) = name.find("::") {
            let first = &name[..colon_pos];
            let rest = &name[colon_pos + 2..];

            // Resolve the first part
            if let Some(result) = self.resolve(index, first, from_file) {
                // Now try to find the full path
                let full_name = format!("{}::{}", result.symbol.qualified, rest);
                if let Some(symbol) = select_best_match(index.get_all(&full_name)) {
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

impl CppResolver {
    /// Resolve references in a C++ file (legacy helper for reference extraction)
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

            // Check for base classes
            if let Some(bases) = &symbol.implements {
                for base in bases {
                    references.push(Reference {
                        name: base.clone(),
                        location: symbol.location.clone(),
                    });
                }
            }
        }

        // Add include/using references
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
    use crate::languages::cpp::CppParser;
    use crate::parse::LanguageParser;
    use crate::{CodeIndex, Location, Symbol, Visibility};
    use std::path::PathBuf;

    // === Legacy reference extraction tests ===

    #[test]
    fn resolves_include_references() {
        let source = r#"
#include <iostream>
#include "myheader.hpp"

int main() {
    return 0;
}
"#;
        let parser = CppParser;
        let parse_result = parser.extract_symbols(Path::new("test.cpp"), source, 100);
        let references =
            CppResolver::resolve_references(Path::new("test.cpp"), source, &parse_result);

        assert!(references.iter().any(|r| r.name == "iostream"));
        assert!(references.iter().any(|r| r.name == "myheader.hpp"));
    }

    // === SymbolResolver trait tests ===

    #[test]
    fn resolve_exact_qualified_name() {
        let mut index = CodeIndex::new();
        index.add_symbol(Symbol::new(
            "vector".to_string(),
            "std::vector".to_string(),
            SymbolKind::Class,
            Location::new(PathBuf::from("include/vector.hpp"), 3, 1),
            Visibility::Public,
            "cpp".to_string(),
        ));

        let resolver = CppResolver;
        let result = resolver.resolve(&index, "std::vector", Path::new("test.cpp"));
        assert!(result.is_some());
        assert_eq!(result.unwrap().symbol.qualified, "std::vector");
    }

    #[test]
    fn resolve_same_namespace_symbol() {
        let mut index = CodeIndex::new();
        let file = PathBuf::from("src/mylib/container.cpp");

        // Add class in same namespace
        index.add_symbol(Symbol::new(
            "Container".to_string(),
            "mylib::Container".to_string(),
            SymbolKind::Class,
            Location::new(file.clone(), 3, 1),
            Visibility::Public,
            "cpp".to_string(),
        ));
        index.add_symbol(Symbol::new(
            "Iterator".to_string(),
            "mylib::Iterator".to_string(),
            SymbolKind::Class,
            Location::new(PathBuf::from("src/mylib/iterator.cpp"), 3, 1),
            Visibility::Public,
            "cpp".to_string(),
        ));

        let resolver = CppResolver;
        // From container.cpp, should resolve "Iterator" to "mylib::Iterator"
        let result = resolver.resolve(&index, "Iterator", &file);
        assert!(result.is_some());
        assert_eq!(result.unwrap().symbol.qualified, "mylib::Iterator");
    }

    #[test]
    fn resolve_via_using() {
        let mut index = CodeIndex::new();
        let file = PathBuf::from("src/main.cpp");

        // Add symbol that will be imported
        index.add_symbol(Symbol::new(
            "string".to_string(),
            "std::string".to_string(),
            SymbolKind::Class,
            Location::new(PathBuf::from("include/string.hpp"), 1, 1),
            Visibility::Public,
            "cpp".to_string(),
        ));

        // Add using declaration
        index.add_open(file.clone(), "std".to_string());

        let resolver = CppResolver;
        let result = resolver.resolve(&index, "string", &file);
        assert!(result.is_some());
        let resolved = result.unwrap();
        assert_eq!(resolved.symbol.qualified, "std::string");
        assert!(matches!(
            resolved.resolution_path,
            ResolutionPath::ViaOpen(_)
        ));
    }

    #[test]
    fn resolve_same_file_symbol() {
        let mut index = CodeIndex::new();
        let file = PathBuf::from("src/myclass.cpp");

        index.add_symbol(Symbol::new(
            "MyClass".to_string(),
            "mylib::MyClass".to_string(),
            SymbolKind::Class,
            Location::new(file.clone(), 3, 1),
            Visibility::Public,
            "cpp".to_string(),
        ));
        index.add_symbol(Symbol::new(
            "doWork".to_string(),
            "mylib::MyClass::doWork".to_string(),
            SymbolKind::Function,
            Location::new(file.clone(), 5, 5),
            Visibility::Public,
            "cpp".to_string(),
        ));

        let resolver = CppResolver;
        let result = resolver.resolve(&index, "doWork", &file);
        assert!(result.is_some());
        assert_eq!(result.unwrap().symbol.qualified, "mylib::MyClass::doWork");
    }

    #[test]
    fn resolve_dotted_method() {
        let mut index = CodeIndex::new();

        index.add_symbol(Symbol::new(
            "MyClass".to_string(),
            "mylib::MyClass".to_string(),
            SymbolKind::Class,
            Location::new(PathBuf::from("src/myclass.cpp"), 3, 1),
            Visibility::Public,
            "cpp".to_string(),
        ));
        index.add_symbol(Symbol::new(
            "doWork".to_string(),
            "mylib::MyClass::doWork".to_string(),
            SymbolKind::Function,
            Location::new(PathBuf::from("src/myclass.cpp"), 5, 5),
            Visibility::Public,
            "cpp".to_string(),
        ));

        let resolver = CppResolver;
        let result =
            resolver.resolve_dotted(&index, "mylib::MyClass::doWork", Path::new("test.cpp"));
        assert!(result.is_some());
        assert_eq!(result.unwrap().symbol.qualified, "mylib::MyClass::doWork");
    }

    #[test]
    fn returns_none_for_unknown_symbol() {
        let index = CodeIndex::new();
        let resolver = CppResolver;

        let result = resolver.resolve(&index, "NonExistent", Path::new("test.cpp"));
        assert!(result.is_none());
    }

    #[test]
    fn prioritizes_definition_over_declaration() {
        let mut index = CodeIndex::new();
        // Add declaration in header (priority 1)
        index.add_symbol(Symbol::new(
            "MyFunc".to_string(),
            "MyFunc".to_string(),
            SymbolKind::Function,
            Location::new(PathBuf::from("src/test.h"), 1, 1),
            Visibility::Public,
            "cpp".to_string(),
        ));
        // Add definition in source (priority 2)
        index.add_symbol(Symbol::new(
            "MyFunc".to_string(),
            "MyFunc".to_string(),
            SymbolKind::Function,
            Location::new(PathBuf::from("src/test.cpp"), 1, 1),
            Visibility::Public,
            "cpp".to_string(),
        ));

        let resolver = CppResolver;
        let result = resolver.resolve(&index, "MyFunc", Path::new("main.cpp"));
        assert!(result.is_some());
        let symbol = result.unwrap().symbol;
        assert_eq!(symbol.location.file.to_string_lossy(), "src/test.cpp");

        // Try adding in reverse order to ensure it's not just "last added"
        let mut index2 = CodeIndex::new();
        index2.add_symbol(Symbol::new(
            "MyFunc".to_string(),
            "MyFunc".to_string(),
            SymbolKind::Function,
            Location::new(PathBuf::from("src/test.cpp"), 1, 1),
            Visibility::Public,
            "cpp".to_string(),
        ));
        index2.add_symbol(Symbol::new(
            "MyFunc".to_string(),
            "MyFunc".to_string(),
            SymbolKind::Function,
            Location::new(PathBuf::from("src/test.h"), 1, 1),
            Visibility::Public,
            "cpp".to_string(),
        ));

        let result2 = resolver.resolve(&index2, "MyFunc", Path::new("main.cpp"));
        assert!(result2.is_some());
        assert_eq!(
            result2.unwrap().symbol.location.file.to_string_lossy(),
            "src/test.cpp"
        );
    }
}
