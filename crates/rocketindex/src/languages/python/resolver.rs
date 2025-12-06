//! Name resolution for Python.

use std::path::Path;

use crate::resolve::{ResolutionPath, ResolveResult, SymbolResolver};
use crate::{CodeIndex, SymbolKind};

pub struct PythonResolver;

impl SymbolResolver for PythonResolver {
    fn resolve<'a>(
        &self,
        index: &'a CodeIndex,
        name: &str,
        from_file: &Path,
    ) -> Option<ResolveResult<'a>> {
        // 1. Try exact qualified name match
        if let Some(symbol) = index.get(name) {
            return Some(ResolveResult {
                symbol,
                resolution_path: ResolutionPath::Qualified,
            });
        }

        // 2. Try scoping relative to classes/modules defined in the current file
        let file_symbols = index.symbols_in_file(from_file);
        for symbol in file_symbols {
            if symbol.kind == SymbolKind::Class || symbol.kind == SymbolKind::Module {
                // Try Class.Name or Module.Name
                let qualified = format!("{}.{}", symbol.qualified, name);
                if let Some(resolved) = index.get(&qualified) {
                    return Some(ResolveResult {
                        symbol: resolved,
                        resolution_path: ResolutionPath::SameModule,
                    });
                }
            }
        }

        // 3. Try to resolve via imports (opens)
        let file_opens = index.opens_for_file(from_file);
        for open in file_opens {
            // Try module.name pattern
            let qualified = format!("{}.{}", open, name);
            if let Some(resolved) = index.get(&qualified) {
                return Some(ResolveResult {
                    symbol: resolved,
                    resolution_path: ResolutionPath::ViaOpen(open.to_string()),
                });
            }
        }

        // 4. Try looking for the name as a dotted path by checking parent modules
        // e.g., if looking for "utils.helper", try to find it directly
        if name.contains('.') {
            if let Some(symbol) = index.get(name) {
                return Some(ResolveResult {
                    symbol,
                    resolution_path: ResolutionPath::Qualified,
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
        // For Python, dotted names like "MyClass.my_method" use dots
        // First try direct lookup
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CodeIndex, Location, Symbol, Visibility};
    use std::path::PathBuf;

    #[test]
    fn resolves_qualified_name() {
        let mut index = CodeIndex::new();
        index.add_symbol(Symbol::new(
            "MyClass".to_string(),
            "mymodule.MyClass".to_string(),
            SymbolKind::Class,
            Location::new(PathBuf::from("mymodule.py"), 1, 1),
            Visibility::Public,
            "python".to_string(),
        ));

        let resolver = PythonResolver;
        let result = resolver.resolve(&index, "mymodule.MyClass", Path::new("test.py"));
        assert!(result.is_some());
        assert_eq!(result.unwrap().symbol.qualified, "mymodule.MyClass");
    }

    #[test]
    fn resolves_via_same_file_class() {
        let mut index = CodeIndex::new();
        // Define MyClass in utils.py
        index.add_symbol(Symbol::new(
            "MyClass".to_string(),
            "MyClass".to_string(),
            SymbolKind::Class,
            Location::new(PathBuf::from("utils.py"), 1, 1),
            Visibility::Public,
            "python".to_string(),
        ));
        // Define MyClass.helper method
        index.add_symbol(Symbol::new(
            "helper".to_string(),
            "MyClass.helper".to_string(),
            SymbolKind::Function,
            Location::new(PathBuf::from("utils.py"), 5, 5),
            Visibility::Public,
            "python".to_string(),
        ));

        // From utils.py, "helper" should resolve to "MyClass.helper"
        let resolver = PythonResolver;
        let result = resolver.resolve(&index, "helper", Path::new("utils.py"));
        assert!(result.is_some());
        assert_eq!(result.unwrap().symbol.qualified, "MyClass.helper");
    }

    #[test]
    fn resolves_via_import() {
        let mut index = CodeIndex::new();

        // Define the target symbol
        index.add_symbol(Symbol::new(
            "helper".to_string(),
            "myutils.helper".to_string(),
            SymbolKind::Function,
            Location::new(PathBuf::from("myutils.py"), 1, 1),
            Visibility::Public,
            "python".to_string(),
        ));

        // Add an import in the calling file
        index.add_open(PathBuf::from("main.py"), "myutils".to_string());

        let resolver = PythonResolver;
        let result = resolver.resolve(&index, "helper", Path::new("main.py"));
        assert!(result.is_some());
        let resolved = result.unwrap();
        assert_eq!(resolved.symbol.qualified, "myutils.helper");
        assert!(matches!(
            resolved.resolution_path,
            ResolutionPath::ViaOpen(_)
        ));
    }

    #[test]
    fn resolves_dotted_method_access() {
        let mut index = CodeIndex::new();

        // Define Calculator class and its method
        index.add_symbol(Symbol::new(
            "Calculator".to_string(),
            "Calculator".to_string(),
            SymbolKind::Class,
            Location::new(PathBuf::from("calc.py"), 1, 1),
            Visibility::Public,
            "python".to_string(),
        ));
        index.add_symbol(Symbol::new(
            "add".to_string(),
            "Calculator.add".to_string(),
            SymbolKind::Function,
            Location::new(PathBuf::from("calc.py"), 3, 5),
            Visibility::Public,
            "python".to_string(),
        ));

        let resolver = PythonResolver;
        let result = resolver.resolve_dotted(&index, "Calculator.add", Path::new("test.py"));
        assert!(result.is_some());
        assert_eq!(result.unwrap().symbol.qualified, "Calculator.add");
    }

    #[test]
    fn resolves_nested_class_method() {
        let mut index = CodeIndex::new();

        index.add_symbol(Symbol::new(
            "Outer".to_string(),
            "Outer".to_string(),
            SymbolKind::Class,
            Location::new(PathBuf::from("nested.py"), 1, 1),
            Visibility::Public,
            "python".to_string(),
        ));
        index.add_symbol(Symbol::new(
            "Inner".to_string(),
            "Outer.Inner".to_string(),
            SymbolKind::Class,
            Location::new(PathBuf::from("nested.py"), 3, 5),
            Visibility::Public,
            "python".to_string(),
        ));
        index.add_symbol(Symbol::new(
            "method".to_string(),
            "Outer.Inner.method".to_string(),
            SymbolKind::Function,
            Location::new(PathBuf::from("nested.py"), 5, 9),
            Visibility::Public,
            "python".to_string(),
        ));

        let resolver = PythonResolver;

        // Direct qualified lookup
        let result = resolver.resolve_dotted(&index, "Outer.Inner.method", Path::new("test.py"));
        assert!(result.is_some());
        assert_eq!(result.unwrap().symbol.qualified, "Outer.Inner.method");
    }

    #[test]
    fn resolves_from_package_import() {
        let mut index = CodeIndex::new();

        // Simulate: from mypackage.submodule import MyClass
        index.add_symbol(Symbol::new(
            "MyClass".to_string(),
            "mypackage.submodule.MyClass".to_string(),
            SymbolKind::Class,
            Location::new(PathBuf::from("mypackage/submodule.py"), 1, 1),
            Visibility::Public,
            "python".to_string(),
        ));

        // Add import
        index.add_open(PathBuf::from("main.py"), "mypackage.submodule".to_string());

        let resolver = PythonResolver;
        let result = resolver.resolve(&index, "MyClass", Path::new("main.py"));
        assert!(result.is_some());
        assert_eq!(
            result.unwrap().symbol.qualified,
            "mypackage.submodule.MyClass"
        );
    }
}
