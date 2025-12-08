//! Reference resolution for PHP symbols.

use std::path::Path;

use crate::parse::ParseResult;
use crate::Reference;

pub struct PhpResolver;

impl PhpResolver {
    /// Resolve references in a PHP file
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
}
