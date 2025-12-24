//! Name resolution for C symbols.
//!
//! Implements simple resolution for C's flat namespace:
//! 1. Exact name match
//! 2. Same-file symbols
//!
//! Note: C has no namespace system, so resolution is primarily by direct name lookup.

use std::path::Path;

use crate::parse::ParseResult;
use crate::resolve::{ResolutionPath, ResolveResult, SymbolResolver};
use crate::{CodeIndex, Reference};

pub struct CResolver;

impl SymbolResolver for CResolver {
    fn resolve<'a>(
        &self,
        index: &'a CodeIndex,
        name: &str,
        from_file: &Path,
    ) -> Option<ResolveResult<'a>> {
        // 1. Try exact name match (C has flat namespace)
        if let Some(symbol) = index.get(name) {
            return Some(ResolveResult {
                symbol,
                resolution_path: ResolutionPath::Qualified,
            });
        }

        // 2. Try same-file resolution
        let file_symbols = index.symbols_in_file(from_file);
        for symbol in &file_symbols {
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
        // C doesn't have dotted names in the same way as OO languages
        // Just fall back to normal resolution
        self.resolve(index, name, from_file)
    }
}

impl CResolver {
    /// Resolve references in a C file (legacy helper for reference extraction)
    pub fn resolve_references(
        file: &Path,
        _source: &str,
        parse_result: &ParseResult,
    ) -> Vec<Reference> {
        let mut references = Vec::new();

        // For each symbol, look for references to other symbols
        for symbol in &parse_result.symbols {
            // Check for parent struct reference
            if let Some(parent) = &symbol.parent {
                references.push(Reference {
                    name: parent.clone(),
                    location: symbol.location.clone(),
                });
            }
        }

        // Add include references
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
    use crate::languages::c::CParser;
    use crate::parse::LanguageParser;
    use crate::{CodeIndex, Location, Symbol, SymbolKind, Visibility};
    use std::path::PathBuf;

    // === Legacy reference extraction tests ===

    #[test]
    fn resolves_include_references() {
        let source = r#"
#include <stdio.h>
#include "myheader.h"

int main() {
    return 0;
}
"#;
        let parser = CParser;
        let parse_result = parser.extract_symbols(Path::new("test.c"), source, 100);
        let references = CResolver::resolve_references(Path::new("test.c"), source, &parse_result);

        assert!(references.iter().any(|r| r.name == "stdio.h"));
        assert!(references.iter().any(|r| r.name == "myheader.h"));
    }

    // === SymbolResolver trait tests ===

    #[test]
    fn resolve_exact_name() {
        let mut index = CodeIndex::new();
        index.add_symbol(Symbol::new(
            "main".to_string(),
            "main".to_string(),
            SymbolKind::Function,
            Location::new(PathBuf::from("src/main.c"), 3, 1),
            Visibility::Public,
            "c".to_string(),
        ));

        let resolver = CResolver;
        let result = resolver.resolve(&index, "main", Path::new("test.c"));
        assert!(result.is_some());
        assert_eq!(result.unwrap().symbol.qualified, "main");
    }

    #[test]
    fn resolve_same_file_symbol() {
        let mut index = CodeIndex::new();
        let file = PathBuf::from("src/utils.c");

        index.add_symbol(Symbol::new(
            "helper".to_string(),
            "helper".to_string(),
            SymbolKind::Function,
            Location::new(file.clone(), 3, 1),
            Visibility::Public,
            "c".to_string(),
        ));
        index.add_symbol(Symbol::new(
            "main".to_string(),
            "main".to_string(),
            SymbolKind::Function,
            Location::new(file.clone(), 10, 1),
            Visibility::Public,
            "c".to_string(),
        ));

        let resolver = CResolver;
        let result = resolver.resolve(&index, "helper", &file);
        assert!(result.is_some());
        assert_eq!(result.unwrap().symbol.qualified, "helper");
    }

    #[test]
    fn resolve_struct() {
        let mut index = CodeIndex::new();
        index.add_symbol(Symbol::new(
            "Point".to_string(),
            "Point".to_string(),
            SymbolKind::Record,
            Location::new(PathBuf::from("include/types.h"), 3, 1),
            Visibility::Public,
            "c".to_string(),
        ));

        let resolver = CResolver;
        let result = resolver.resolve(&index, "Point", Path::new("test.c"));
        assert!(result.is_some());
        assert_eq!(result.unwrap().symbol.qualified, "Point");
    }

    #[test]
    fn returns_none_for_unknown_symbol() {
        let index = CodeIndex::new();
        let resolver = CResolver;

        let result = resolver.resolve(&index, "NonExistent", Path::new("test.c"));
        assert!(result.is_none());
    }
}
