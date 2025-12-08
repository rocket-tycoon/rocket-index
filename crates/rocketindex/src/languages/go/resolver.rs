//! Name resolution for Go.

use std::path::Path;

use crate::resolve::{ResolutionPath, ResolveResult, SymbolResolver};
use crate::{CodeIndex, SymbolKind};

pub struct GoResolver;

impl SymbolResolver for GoResolver {
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

        // 2. Try scoping relative to types/packages defined in the current file
        let file_symbols = index.symbols_in_file(from_file);
        for symbol in file_symbols {
            if symbol.kind == SymbolKind::Module
                || symbol.kind == SymbolKind::Class
                || symbol.kind == SymbolKind::Interface
            {
                // Try Parent.Name pattern (Go uses dots for qualified names)
                let qualified = format!("{}.{}", symbol.qualified, name);
                if let Some(resolved) = index.get(&qualified) {
                    return Some(ResolveResult {
                        symbol: resolved,
                        resolution_path: ResolutionPath::SameModule,
                    });
                }
            }
        }

        // 3. Try to resolve via import statements
        let file_opens = index.opens_for_file(from_file);
        for open in file_opens {
            // For "import foo/bar", if we're looking for "bar.Func", check if open ends with the package
            // Extract the package name from the import path (last component)
            let package_name = open.rsplit('/').next().unwrap_or(open);

            // Check if name starts with this package name
            if name.starts_with(package_name)
                && name.get(package_name.len()..package_name.len() + 1) == Some(".")
            {
                // Try resolving the full path
                let rest = &name[package_name.len() + 1..];
                let qualified = format!("{}.{}", open, rest);
                if let Some(resolved) = index.get(&qualified) {
                    return Some(ResolveResult {
                        symbol: resolved,
                        resolution_path: ResolutionPath::ViaOpen(open.to_string()),
                    });
                }
            }

            // Also try package.name pattern directly
            let qualified = format!("{}.{}", open, name);
            if let Some(resolved) = index.get(&qualified) {
                return Some(ResolveResult {
                    symbol: resolved,
                    resolution_path: ResolutionPath::ViaOpen(open.to_string()),
                });
            }
        }

        // 4. Try looking up in the same package (unqualified name in same package)
        // Find the package path from the current file's symbols
        for symbol in index.symbols_in_file(from_file) {
            if symbol.kind == SymbolKind::Module {
                // This is the package declaration
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
        // First try direct lookup
        if let Some(symbol) = index.get(name) {
            return Some(ResolveResult {
                symbol,
                resolution_path: ResolutionPath::Qualified,
            });
        }

        // For Go, dotted names are the natural format (package.Type.Method)
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
            "Handler".to_string(),
            "mypackage.Handler".to_string(),
            SymbolKind::Class,
            Location::new(PathBuf::from("mypackage/handler.go"), 1, 1),
            Visibility::Public,
            "go".to_string(),
        ));

        let resolver = GoResolver;
        let result = resolver.resolve(&index, "mypackage.Handler", Path::new("test.go"));
        assert!(result.is_some());
        assert_eq!(result.unwrap().symbol.qualified, "mypackage.Handler");
    }

    #[test]
    fn resolves_via_same_package() {
        let mut index = CodeIndex::new();
        // Define package in handler.go
        index.add_symbol(Symbol::new(
            "mypackage".to_string(),
            "mypackage".to_string(),
            SymbolKind::Module,
            Location::new(PathBuf::from("mypackage/handler.go"), 1, 1),
            Visibility::Public,
            "go".to_string(),
        ));
        // Define function in that package
        index.add_symbol(Symbol::new(
            "Process".to_string(),
            "mypackage.Process".to_string(),
            SymbolKind::Function,
            Location::new(PathBuf::from("mypackage/handler.go"), 5, 5),
            Visibility::Public,
            "go".to_string(),
        ));

        // From handler.go, "Process" should resolve to "mypackage.Process"
        let resolver = GoResolver;
        let result = resolver.resolve(&index, "Process", Path::new("mypackage/handler.go"));
        assert!(result.is_some());
        assert_eq!(result.unwrap().symbol.qualified, "mypackage.Process");
    }

    #[test]
    fn resolves_via_import() {
        let mut index = CodeIndex::new();

        // Define the target symbol
        index.add_symbol(Symbol::new(
            "Router".to_string(),
            "github.com/gin-gonic/gin.Router".to_string(),
            SymbolKind::Class,
            Location::new(PathBuf::from("gin/router.go"), 1, 1),
            Visibility::Public,
            "go".to_string(),
        ));

        // Add an import statement in the calling file
        index.add_open(
            PathBuf::from("main.go"),
            "github.com/gin-gonic/gin".to_string(),
        );

        let resolver = GoResolver;
        // In Go, you'd reference this as gin.Router
        let result = resolver.resolve(&index, "gin.Router", Path::new("main.go"));
        assert!(result.is_some());
        let resolved = result.unwrap();
        assert_eq!(resolved.symbol.qualified, "github.com/gin-gonic/gin.Router");
        assert!(matches!(
            resolved.resolution_path,
            ResolutionPath::ViaOpen(_)
        ));
    }

    #[test]
    fn resolves_method_on_type() {
        let mut index = CodeIndex::new();

        // Define struct and its method
        index.add_symbol(Symbol::new(
            "Server".to_string(),
            "mypackage.Server".to_string(),
            SymbolKind::Class,
            Location::new(PathBuf::from("mypackage/server.go"), 1, 1),
            Visibility::Public,
            "go".to_string(),
        ));
        index.add_symbol(Symbol::new(
            "Start".to_string(),
            "mypackage.Server.Start".to_string(),
            SymbolKind::Function,
            Location::new(PathBuf::from("mypackage/server.go"), 10, 1),
            Visibility::Public,
            "go".to_string(),
        ));

        let resolver = GoResolver;
        let result =
            resolver.resolve_dotted(&index, "mypackage.Server.Start", Path::new("test.go"));
        assert!(result.is_some());
        assert_eq!(result.unwrap().symbol.qualified, "mypackage.Server.Start");
    }

    #[test]
    fn resolves_interface_method() {
        let mut index = CodeIndex::new();

        // Define interface and its method
        index.add_symbol(Symbol::new(
            "Reader".to_string(),
            "io.Reader".to_string(),
            SymbolKind::Interface,
            Location::new(PathBuf::from("io/io.go"), 1, 1),
            Visibility::Public,
            "go".to_string(),
        ));
        index.add_symbol(Symbol::new(
            "Read".to_string(),
            "io.Reader.Read".to_string(),
            SymbolKind::Function,
            Location::new(PathBuf::from("io/io.go"), 3, 5),
            Visibility::Public,
            "go".to_string(),
        ));

        let resolver = GoResolver;

        let result = resolver.resolve_dotted(&index, "io.Reader.Read", Path::new("test.go"));
        assert!(result.is_some());
        assert_eq!(result.unwrap().symbol.qualified, "io.Reader.Read");
    }

    #[test]
    fn handles_unexported_symbols() {
        let mut index = CodeIndex::new();

        // Unexported (private) function
        index.add_symbol(Symbol::new(
            "helper".to_string(),
            "mypackage.helper".to_string(),
            SymbolKind::Function,
            Location::new(PathBuf::from("mypackage/utils.go"), 5, 1),
            Visibility::Private,
            "go".to_string(),
        ));

        let resolver = GoResolver;
        let result = resolver.resolve(&index, "mypackage.helper", Path::new("test.go"));
        // Should still resolve (visibility checking is a separate concern)
        assert!(result.is_some());
        assert_eq!(result.unwrap().symbol.visibility, Visibility::Private);
    }

    #[test]
    fn resolves_embedded_struct_field() {
        let mut index = CodeIndex::new();

        // Define Container struct
        index.add_symbol(Symbol::new(
            "Container".to_string(),
            "container.Container".to_string(),
            SymbolKind::Class,
            Location::new(PathBuf::from("container/container.go"), 1, 1),
            Visibility::Public,
            "go".to_string(),
        ));

        // Define embedded State field (embedded struct)
        index.add_symbol(Symbol::new(
            "State".to_string(),
            "container.Container.State".to_string(),
            SymbolKind::Member,
            Location::new(PathBuf::from("container/container.go"), 5, 5),
            Visibility::Public,
            "go".to_string(),
        ));

        // Define Config field (named field)
        index.add_symbol(Symbol::new(
            "Config".to_string(),
            "container.Container.Config".to_string(),
            SymbolKind::Member,
            Location::new(PathBuf::from("container/container.go"), 6, 5),
            Visibility::Public,
            "go".to_string(),
        ));

        let resolver = GoResolver;

        // Should resolve Container.Config directly
        let result =
            resolver.resolve_dotted(&index, "container.Container.Config", Path::new("test.go"));
        assert!(result.is_some());
        assert_eq!(
            result.unwrap().symbol.qualified,
            "container.Container.Config"
        );

        // Should resolve Container.State (embedded field)
        let result =
            resolver.resolve_dotted(&index, "container.Container.State", Path::new("test.go"));
        assert!(result.is_some());
        assert_eq!(
            result.unwrap().symbol.qualified,
            "container.Container.State"
        );
    }

    #[test]
    fn resolves_cross_package_symbol() {
        let mut index = CodeIndex::new();

        // Define a symbol in package A
        index.add_symbol(Symbol::new(
            "Handler".to_string(),
            "handlers.Handler".to_string(),
            SymbolKind::Class,
            Location::new(PathBuf::from("handlers/handler.go"), 1, 1),
            Visibility::Public,
            "go".to_string(),
        ));

        // Add import in the calling file
        index.add_open(PathBuf::from("main.go"), "handlers".to_string());

        let resolver = GoResolver;

        // From main.go, "handlers.Handler" should resolve
        let result = resolver.resolve(&index, "handlers.Handler", Path::new("main.go"));
        assert!(result.is_some());
        assert_eq!(result.unwrap().symbol.qualified, "handlers.Handler");
    }

    #[test]
    fn resolves_method_via_type() {
        let mut index = CodeIndex::new();

        // Define struct
        index.add_symbol(Symbol::new(
            "Client".to_string(),
            "http.Client".to_string(),
            SymbolKind::Class,
            Location::new(PathBuf::from("http/client.go"), 1, 1),
            Visibility::Public,
            "go".to_string(),
        ));

        // Define method on struct
        index.add_symbol(Symbol::new(
            "Get".to_string(),
            "http.Client.Get".to_string(),
            SymbolKind::Function,
            Location::new(PathBuf::from("http/client.go"), 10, 1),
            Visibility::Public,
            "go".to_string(),
        ));

        let resolver = GoResolver;

        // Should resolve method via dotted name
        let result = resolver.resolve_dotted(&index, "http.Client.Get", Path::new("test.go"));
        assert!(result.is_some());
        assert_eq!(result.unwrap().symbol.qualified, "http.Client.Get");
    }

    #[test]
    fn resolves_interface_embedded_method() {
        let mut index = CodeIndex::new();

        // Define ReadWriter interface (which embeds Reader and Writer)
        index.add_symbol(Symbol::new(
            "ReadWriter".to_string(),
            "io.ReadWriter".to_string(),
            SymbolKind::Interface,
            Location::new(PathBuf::from("io/io.go"), 1, 1),
            Visibility::Public,
            "go".to_string(),
        ));

        // Define Reader interface
        index.add_symbol(Symbol::new(
            "Reader".to_string(),
            "io.Reader".to_string(),
            SymbolKind::Interface,
            Location::new(PathBuf::from("io/io.go"), 10, 1),
            Visibility::Public,
            "go".to_string(),
        ));

        // Define Read method on Reader
        index.add_symbol(Symbol::new(
            "Read".to_string(),
            "io.Reader.Read".to_string(),
            SymbolKind::Function,
            Location::new(PathBuf::from("io/io.go"), 12, 5),
            Visibility::Public,
            "go".to_string(),
        ));

        let resolver = GoResolver;

        // Should resolve Reader.Read
        let result = resolver.resolve_dotted(&index, "io.Reader.Read", Path::new("test.go"));
        assert!(result.is_some());
        assert_eq!(result.unwrap().symbol.qualified, "io.Reader.Read");
    }

    #[test]
    fn resolves_symbols_in_same_package_different_files() {
        let mut index = CodeIndex::new();

        // Define package in file1.go
        index.add_symbol(Symbol::new(
            "mypackage".to_string(),
            "mypackage".to_string(),
            SymbolKind::Module,
            Location::new(PathBuf::from("mypackage/file1.go"), 1, 1),
            Visibility::Public,
            "go".to_string(),
        ));

        // Define function in file1.go
        index.add_symbol(Symbol::new(
            "HelperFunc".to_string(),
            "mypackage.HelperFunc".to_string(),
            SymbolKind::Function,
            Location::new(PathBuf::from("mypackage/file1.go"), 5, 1),
            Visibility::Public,
            "go".to_string(),
        ));

        // Define another function in file2.go (same package)
        index.add_symbol(Symbol::new(
            "MainFunc".to_string(),
            "mypackage.MainFunc".to_string(),
            SymbolKind::Function,
            Location::new(PathBuf::from("mypackage/file2.go"), 5, 1),
            Visibility::Public,
            "go".to_string(),
        ));

        // Add package declaration for file2.go too
        index.add_symbol(Symbol::new(
            "mypackage".to_string(),
            "mypackage".to_string(),
            SymbolKind::Module,
            Location::new(PathBuf::from("mypackage/file2.go"), 1, 1),
            Visibility::Public,
            "go".to_string(),
        ));

        let resolver = GoResolver;

        // From file2.go, should be able to resolve HelperFunc (from file1.go) via same package
        let result = resolver.resolve(&index, "HelperFunc", Path::new("mypackage/file2.go"));
        assert!(result.is_some());
        assert_eq!(result.unwrap().symbol.qualified, "mypackage.HelperFunc");

        // From file1.go, should be able to resolve MainFunc (from file2.go) via same package
        let result = resolver.resolve(&index, "MainFunc", Path::new("mypackage/file1.go"));
        assert!(result.is_some());
        assert_eq!(result.unwrap().symbol.qualified, "mypackage.MainFunc");
    }

    #[test]
    fn resolves_nested_type_method() {
        let mut index = CodeIndex::new();

        // Define package.Type.Method pattern
        index.add_symbol(Symbol::new(
            "Server".to_string(),
            "grpc.Server".to_string(),
            SymbolKind::Class,
            Location::new(PathBuf::from("grpc/server.go"), 1, 1),
            Visibility::Public,
            "go".to_string(),
        ));

        index.add_symbol(Symbol::new(
            "Serve".to_string(),
            "grpc.Server.Serve".to_string(),
            SymbolKind::Function,
            Location::new(PathBuf::from("grpc/server.go"), 50, 1),
            Visibility::Public,
            "go".to_string(),
        ));

        index.add_symbol(Symbol::new(
            "Stop".to_string(),
            "grpc.Server.Stop".to_string(),
            SymbolKind::Function,
            Location::new(PathBuf::from("grpc/server.go"), 100, 1),
            Visibility::Public,
            "go".to_string(),
        ));

        let resolver = GoResolver;

        // Should resolve Server.Serve
        let result = resolver.resolve_dotted(&index, "grpc.Server.Serve", Path::new("test.go"));
        assert!(result.is_some());
        assert_eq!(result.unwrap().symbol.qualified, "grpc.Server.Serve");

        // Should resolve Server.Stop
        let result = resolver.resolve_dotted(&index, "grpc.Server.Stop", Path::new("test.go"));
        assert!(result.is_some());
        assert_eq!(result.unwrap().symbol.qualified, "grpc.Server.Stop");
    }

    #[test]
    fn resolves_type_alias() {
        let mut index = CodeIndex::new();

        // Define a type alias
        index.add_symbol(Symbol::new(
            "Duration".to_string(),
            "time.Duration".to_string(),
            SymbolKind::Type,
            Location::new(PathBuf::from("time/time.go"), 1, 1),
            Visibility::Public,
            "go".to_string(),
        ));

        let resolver = GoResolver;

        let result = resolver.resolve(&index, "time.Duration", Path::new("test.go"));
        assert!(result.is_some());
        assert_eq!(result.unwrap().symbol.kind, SymbolKind::Type);
    }

    #[test]
    fn resolves_constant() {
        let mut index = CodeIndex::new();

        // Define constants
        index.add_symbol(Symbol::new(
            "StatusOK".to_string(),
            "http.StatusOK".to_string(),
            SymbolKind::Value,
            Location::new(PathBuf::from("http/status.go"), 1, 1),
            Visibility::Public,
            "go".to_string(),
        ));

        let resolver = GoResolver;

        let result = resolver.resolve(&index, "http.StatusOK", Path::new("test.go"));
        assert!(result.is_some());
        assert_eq!(result.unwrap().symbol.kind, SymbolKind::Value);
    }
}
