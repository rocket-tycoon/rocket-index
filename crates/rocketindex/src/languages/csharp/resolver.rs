//! Name resolution for C# symbols.
//!
//! Implements namespace-based resolution following C#'s scoping rules:
//! 1. Fully qualified names (e.g., `MyNamespace.MyClass`)
//! 2. Same-namespace symbols
//! 3. Using directives
//! 4. Same-file symbols

use std::path::Path;

use crate::parse::ParseResult;
use crate::resolve::{ResolutionPath, ResolveResult, SymbolResolver};
use crate::{CodeIndex, Reference, SymbolKind};

pub struct CSharpResolver;

impl SymbolResolver for CSharpResolver {
    fn resolve<'a>(
        &self,
        index: &'a CodeIndex,
        name: &str,
        from_file: &Path,
    ) -> Option<ResolveResult<'a>> {
        // 1. Try exact qualified name match (e.g., "MyNamespace.MyClass")
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
            // Extract namespace from qualified name (e.g., "MyNamespace.MyClass" -> "MyNamespace")
            if let Some(dot_pos) = symbol.qualified.rfind('.') {
                let namespace = &symbol.qualified[..dot_pos];
                // Use this as the namespace if it looks valid
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
            let qualified = format!("{}.{}", namespace, name);
            if let Some(symbol) = index.get(&qualified) {
                return Some(ResolveResult {
                    symbol,
                    resolution_path: ResolutionPath::SameModule,
                });
            }
        }

        // 3. Try via using directives (opens)
        let file_opens = index.opens_for_file(from_file);
        for open in file_opens {
            // Handle specific usings like "System.Collections.Generic.List"
            // The using might be the exact type we're looking for
            if open.ends_with(&format!(".{}", name)) {
                if let Some(symbol) = index.get(open) {
                    return Some(ResolveResult {
                        symbol,
                        resolution_path: ResolutionPath::ViaOpen(open.to_string()),
                    });
                }
            }

            // Try namespace.name pattern
            let qualified = format!("{}.{}", open, name);
            if let Some(symbol) = index.get(&qualified) {
                return Some(ResolveResult {
                    symbol,
                    resolution_path: ResolutionPath::ViaOpen(open.to_string()),
                });
            }
        }

        // 4. Try same-file resolution (nested types, methods in same class)
        for symbol in &file_symbols {
            if symbol.name == name {
                return Some(ResolveResult {
                    symbol,
                    resolution_path: ResolutionPath::SameModule,
                });
            }

            // Try Parent.Name pattern for nested types
            if symbol.kind == SymbolKind::Class
                || symbol.kind == SymbolKind::Interface
                || symbol.kind == SymbolKind::Record
            {
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

impl CSharpResolver {
    /// Resolve references in a C# file (legacy helper for reference extraction)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::languages::csharp::CSharpParser;
    use crate::parse::LanguageParser;
    use crate::{CodeIndex, Location, Symbol, Visibility};
    use std::path::PathBuf;

    // === Legacy reference extraction tests ===

    #[test]
    fn resolves_qualified_name_parser() {
        let source = r#"
namespace MyNamespace
{
    public class User
    {
        public void Save() {}
    }
}
"#;
        let parser = CSharpParser;
        let result = parser.extract_symbols(Path::new("User.cs"), source, 100);

        let method = result
            .symbols
            .iter()
            .find(|s| s.name == "Save")
            .expect("Should find Save method");
        assert_eq!(method.qualified, "MyNamespace.User.Save");
    }

    #[test]
    fn resolves_via_using_references() {
        let source = r#"
using System.Collections.Generic;
using System.Linq;

namespace MyApp
{
    public class App {}
}
"#;
        let parser = CSharpParser;
        let result = parser.extract_symbols(Path::new("App.cs"), source, 100);
        let refs = CSharpResolver::resolve_references(Path::new("App.cs"), source, &result);

        assert!(refs.iter().any(|r| r.name == "System.Collections.Generic"));
        assert!(refs.iter().any(|r| r.name == "System.Linq"));
    }

    #[test]
    fn resolves_using_references() {
        // Note: C# parser doesn't currently extract parent class information,
        // so we test using directive resolution instead
        let source = r#"
using System.Text;

namespace MyApp
{
    public class App {}
}
"#;
        let parser = CSharpParser;
        let result = parser.extract_symbols(Path::new("App.cs"), source, 100);
        let refs = CSharpResolver::resolve_references(Path::new("App.cs"), source, &result);

        let using_ref = refs
            .iter()
            .find(|r| r.name == "System.Text")
            .expect("Should find System.Text reference");
        assert_eq!(using_ref.name, "System.Text");
    }

    // === SymbolResolver trait tests ===

    #[test]
    fn resolve_exact_qualified_name() {
        let mut index = CodeIndex::new();
        index.add_symbol(Symbol::new(
            "User".to_string(),
            "MyNamespace.User".to_string(),
            SymbolKind::Class,
            Location::new(PathBuf::from("src/User.cs"), 3, 1),
            Visibility::Public,
            "csharp".to_string(),
        ));

        let resolver = CSharpResolver;
        let result = resolver.resolve(&index, "MyNamespace.User", Path::new("Test.cs"));
        assert!(result.is_some());
        assert_eq!(result.unwrap().symbol.qualified, "MyNamespace.User");
    }

    #[test]
    fn resolve_same_namespace_symbol() {
        let mut index = CodeIndex::new();
        let file = PathBuf::from("src/MyApp/App.cs");

        // Add class in same namespace
        index.add_symbol(Symbol::new(
            "App".to_string(),
            "MyApp.App".to_string(),
            SymbolKind::Class,
            Location::new(file.clone(), 3, 1),
            Visibility::Public,
            "csharp".to_string(),
        ));
        index.add_symbol(Symbol::new(
            "Helper".to_string(),
            "MyApp.Helper".to_string(),
            SymbolKind::Class,
            Location::new(PathBuf::from("src/MyApp/Helper.cs"), 3, 1),
            Visibility::Public,
            "csharp".to_string(),
        ));

        let resolver = CSharpResolver;
        // From App.cs, should resolve "Helper" to "MyApp.Helper"
        let result = resolver.resolve(&index, "Helper", &file);
        assert!(result.is_some());
        assert_eq!(result.unwrap().symbol.qualified, "MyApp.Helper");
    }

    #[test]
    fn resolve_via_using() {
        let mut index = CodeIndex::new();
        let file = PathBuf::from("src/App.cs");

        // Add symbol that will be imported
        index.add_symbol(Symbol::new(
            "List".to_string(),
            "System.Collections.Generic.List".to_string(),
            SymbolKind::Class,
            Location::new(PathBuf::from("System/Collections/Generic/List.cs"), 1, 1),
            Visibility::Public,
            "csharp".to_string(),
        ));

        // Add using directive
        index.add_open(file.clone(), "System.Collections.Generic".to_string());

        let resolver = CSharpResolver;
        let result = resolver.resolve(&index, "List", &file);
        assert!(result.is_some());
        let resolved = result.unwrap();
        assert_eq!(resolved.symbol.qualified, "System.Collections.Generic.List");
        assert!(matches!(
            resolved.resolution_path,
            ResolutionPath::ViaOpen(_)
        ));
    }

    #[test]
    fn resolve_same_file_symbol() {
        let mut index = CodeIndex::new();
        let file = PathBuf::from("src/User.cs");

        index.add_symbol(Symbol::new(
            "User".to_string(),
            "MyApp.User".to_string(),
            SymbolKind::Class,
            Location::new(file.clone(), 3, 1),
            Visibility::Public,
            "csharp".to_string(),
        ));
        index.add_symbol(Symbol::new(
            "Save".to_string(),
            "MyApp.User.Save".to_string(),
            SymbolKind::Function,
            Location::new(file.clone(), 5, 5),
            Visibility::Public,
            "csharp".to_string(),
        ));

        let resolver = CSharpResolver;
        let result = resolver.resolve(&index, "Save", &file);
        assert!(result.is_some());
        assert_eq!(result.unwrap().symbol.qualified, "MyApp.User.Save");
    }

    #[test]
    fn resolve_nested_type() {
        let mut index = CodeIndex::new();
        let file = PathBuf::from("src/Outer.cs");

        index.add_symbol(Symbol::new(
            "Outer".to_string(),
            "MyApp.Outer".to_string(),
            SymbolKind::Class,
            Location::new(file.clone(), 3, 1),
            Visibility::Public,
            "csharp".to_string(),
        ));
        index.add_symbol(Symbol::new(
            "Inner".to_string(),
            "MyApp.Outer.Inner".to_string(),
            SymbolKind::Class,
            Location::new(file.clone(), 5, 5),
            Visibility::Public,
            "csharp".to_string(),
        ));

        let resolver = CSharpResolver;
        // From same file, "Inner" should resolve to "Outer.Inner"
        let result = resolver.resolve(&index, "Inner", &file);
        assert!(result.is_some());
        assert_eq!(result.unwrap().symbol.qualified, "MyApp.Outer.Inner");
    }

    #[test]
    fn resolve_dotted_method() {
        let mut index = CodeIndex::new();

        index.add_symbol(Symbol::new(
            "User".to_string(),
            "MyApp.User".to_string(),
            SymbolKind::Class,
            Location::new(PathBuf::from("src/User.cs"), 3, 1),
            Visibility::Public,
            "csharp".to_string(),
        ));
        index.add_symbol(Symbol::new(
            "Save".to_string(),
            "MyApp.User.Save".to_string(),
            SymbolKind::Function,
            Location::new(PathBuf::from("src/User.cs"), 5, 5),
            Visibility::Public,
            "csharp".to_string(),
        ));

        let resolver = CSharpResolver;
        let result = resolver.resolve_dotted(&index, "MyApp.User.Save", Path::new("Test.cs"));
        assert!(result.is_some());
        assert_eq!(result.unwrap().symbol.qualified, "MyApp.User.Save");
    }

    #[test]
    fn resolve_dotted_via_using() {
        let mut index = CodeIndex::new();
        let file = PathBuf::from("src/App.cs");

        index.add_symbol(Symbol::new(
            "User".to_string(),
            "MyApp.User".to_string(),
            SymbolKind::Class,
            Location::new(PathBuf::from("src/User.cs"), 3, 1),
            Visibility::Public,
            "csharp".to_string(),
        ));
        index.add_symbol(Symbol::new(
            "Save".to_string(),
            "MyApp.User.Save".to_string(),
            SymbolKind::Function,
            Location::new(PathBuf::from("src/User.cs"), 5, 5),
            Visibility::Public,
            "csharp".to_string(),
        ));

        // Import the namespace
        index.add_open(file.clone(), "MyApp".to_string());

        let resolver = CSharpResolver;
        // "User.Save" should resolve via using
        let result = resolver.resolve_dotted(&index, "User.Save", &file);
        assert!(result.is_some());
        assert_eq!(result.unwrap().symbol.qualified, "MyApp.User.Save");
    }

    #[test]
    fn resolve_struct_in_same_namespace() {
        let mut index = CodeIndex::new();
        let file = PathBuf::from("src/MyApp/Point.cs");

        index.add_symbol(Symbol::new(
            "Point".to_string(),
            "MyApp.Point".to_string(),
            SymbolKind::Record, // C# structs map to Record
            Location::new(file.clone(), 3, 1),
            Visibility::Public,
            "csharp".to_string(),
        ));
        index.add_symbol(Symbol::new(
            "Vector".to_string(),
            "MyApp.Vector".to_string(),
            SymbolKind::Record, // C# structs map to Record
            Location::new(PathBuf::from("src/MyApp/Vector.cs"), 3, 1),
            Visibility::Public,
            "csharp".to_string(),
        ));

        let resolver = CSharpResolver;
        let result = resolver.resolve(&index, "Vector", &file);
        assert!(result.is_some());
        assert_eq!(result.unwrap().symbol.qualified, "MyApp.Vector");
    }

    #[test]
    fn returns_none_for_unknown_symbol() {
        let index = CodeIndex::new();
        let resolver = CSharpResolver;

        let result = resolver.resolve(&index, "NonExistent", Path::new("Test.cs"));
        assert!(result.is_none());
    }
}
