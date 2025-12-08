//! Reference resolution for C++ source files.

use std::path::Path;

use crate::parse::ParseResult;
use crate::Reference;

pub struct CppResolver;

impl CppResolver {
    /// Resolve references in a C++ file
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
}
