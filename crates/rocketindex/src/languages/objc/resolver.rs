//! Name resolution for Objective-C symbols.
//!
//! Implements resolution for Objective-C's namespace:
//! 1. Exact qualified name match
//! 2. Same-file symbols
//!
//! Note: Objective-C uses class prefixes for namespacing (e.g., NS*, UI*).

use std::path::Path;

use crate::resolve::{ResolutionPath, ResolveResult, SymbolResolver};
use crate::CodeIndex;

pub struct ObjCResolver;

impl SymbolResolver for ObjCResolver {
    fn resolve<'a>(
        &self,
        index: &'a CodeIndex,
        name: &str,
        from_file: &Path,
    ) -> Option<ResolveResult<'a>> {
        // 1. Try exact name match
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
        // Objective-C doesn't have dotted names in the same way
        // Just fall back to normal resolution
        self.resolve(index, name, from_file)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CodeIndex, Location, Symbol, SymbolKind, Visibility};
    use std::path::PathBuf;

    #[test]
    fn resolve_exact_name() {
        let mut index = CodeIndex::new();
        index.add_symbol(Symbol::new(
            "NSObject".to_string(),
            "NSObject".to_string(),
            SymbolKind::Class,
            Location::new(PathBuf::from("Foundation.h"), 3, 1),
            Visibility::Public,
            "objc".to_string(),
        ));

        let resolver = ObjCResolver;
        let result = resolver.resolve(&index, "NSObject", Path::new("test.m"));
        assert!(result.is_some());
        assert_eq!(result.unwrap().symbol.qualified, "NSObject");
    }

    #[test]
    fn resolve_method() {
        let mut index = CodeIndex::new();
        index.add_symbol(Symbol::new(
            "init".to_string(),
            "MyClass.init".to_string(),
            SymbolKind::Function,
            Location::new(PathBuf::from("MyClass.m"), 10, 1),
            Visibility::Public,
            "objc".to_string(),
        ));

        let resolver = ObjCResolver;
        let result = resolver.resolve(&index, "MyClass.init", Path::new("test.m"));
        assert!(result.is_some());
        assert_eq!(result.unwrap().symbol.qualified, "MyClass.init");
    }

    #[test]
    fn returns_none_for_unknown_symbol() {
        let index = CodeIndex::new();
        let resolver = ObjCResolver;

        let result = resolver.resolve(&index, "NonExistent", Path::new("test.m"));
        assert!(result.is_none());
    }

    #[test]
    fn resolve_same_file_symbol() {
        let mut index = CodeIndex::new();
        let file = PathBuf::from("MyViewController.m");

        index.add_symbol(Symbol::new(
            "MyViewController".to_string(),
            "MyViewController".to_string(),
            SymbolKind::Class,
            Location::new(file.clone(), 5, 1),
            Visibility::Public,
            "objc".to_string(),
        ));
        index.add_symbol(Symbol::new(
            "viewDidLoad".to_string(),
            "MyViewController.viewDidLoad".to_string(),
            SymbolKind::Function,
            Location::new(file.clone(), 10, 1),
            Visibility::Public,
            "objc".to_string(),
        ));

        let resolver = ObjCResolver;
        let result = resolver.resolve(&index, "viewDidLoad", &file);
        assert!(result.is_some());
        assert_eq!(
            result.unwrap().symbol.qualified,
            "MyViewController.viewDidLoad"
        );
    }

    #[test]
    fn resolve_protocol() {
        let mut index = CodeIndex::new();
        index.add_symbol(Symbol::new(
            "UITableViewDelegate".to_string(),
            "UITableViewDelegate".to_string(),
            SymbolKind::Interface,
            Location::new(PathBuf::from("UIKit.h"), 100, 1),
            Visibility::Public,
            "objc".to_string(),
        ));

        let resolver = ObjCResolver;
        let result = resolver.resolve(&index, "UITableViewDelegate", Path::new("test.m"));
        assert!(result.is_some());
        assert_eq!(result.unwrap().symbol.kind, SymbolKind::Interface);
    }

    #[test]
    fn resolve_category_method() {
        let mut index = CodeIndex::new();
        index.add_symbol(Symbol::new(
            "stringByTrimmingWhitespace".to_string(),
            "NSString(Utilities).stringByTrimmingWhitespace".to_string(),
            SymbolKind::Function,
            Location::new(PathBuf::from("NSString+Utilities.m"), 15, 1),
            Visibility::Public,
            "objc".to_string(),
        ));

        let resolver = ObjCResolver;
        let result = resolver.resolve(
            &index,
            "NSString(Utilities).stringByTrimmingWhitespace",
            Path::new("test.m"),
        );
        assert!(result.is_some());
    }

    #[test]
    fn resolve_property() {
        let mut index = CodeIndex::new();
        index.add_symbol(Symbol::new(
            "name".to_string(),
            "User.name".to_string(),
            SymbolKind::Member,
            Location::new(PathBuf::from("User.h"), 8, 1),
            Visibility::Public,
            "objc".to_string(),
        ));

        let resolver = ObjCResolver;
        let result = resolver.resolve(&index, "User.name", Path::new("test.m"));
        assert!(result.is_some());
        assert_eq!(result.unwrap().symbol.kind, SymbolKind::Member);
    }

    #[test]
    fn resolve_dotted_falls_back() {
        let mut index = CodeIndex::new();
        index.add_symbol(Symbol::new(
            "sharedInstance".to_string(),
            "AppDelegate.sharedInstance".to_string(),
            SymbolKind::Function,
            Location::new(PathBuf::from("AppDelegate.m"), 20, 1),
            Visibility::Public,
            "objc".to_string(),
        ));

        let resolver = ObjCResolver;
        let result =
            resolver.resolve_dotted(&index, "AppDelegate.sharedInstance", Path::new("test.m"));
        assert!(result.is_some());
        assert_eq!(
            result.unwrap().symbol.qualified,
            "AppDelegate.sharedInstance"
        );
    }
}
