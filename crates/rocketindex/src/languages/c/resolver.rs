//! Reference resolution for C source files.

use std::path::Path;

use crate::parse::ParseResult;
use crate::Reference;

pub struct CResolver;

impl CResolver {
    /// Resolve references in a C file
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
}
