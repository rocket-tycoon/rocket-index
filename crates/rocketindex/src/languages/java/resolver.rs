//! Reference resolution for Java symbols.

use std::path::Path;

use crate::parse::ParseResult;
use crate::Reference;

pub struct JavaResolver;

impl JavaResolver {
    /// Resolve references in a Java file
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

    #[test]
    fn resolves_qualified_name() {
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
    fn resolves_via_import() {
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
    fn resolves_inheritance() {
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
}
