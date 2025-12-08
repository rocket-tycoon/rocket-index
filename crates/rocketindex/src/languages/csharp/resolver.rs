//! Reference resolution for C# symbols.

use std::path::Path;

use crate::parse::ParseResult;
use crate::Reference;

pub struct CSharpResolver;

impl CSharpResolver {
    /// Resolve references in a C# file
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

        // Add using directive references
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
