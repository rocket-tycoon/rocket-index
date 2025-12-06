//! Name resolution for Ruby.

use std::path::Path;

use crate::resolve::{ResolutionPath, ResolveResult, SymbolResolver};
use crate::{CodeIndex, SymbolKind};

pub struct RubyResolver;

impl SymbolResolver for RubyResolver {
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

        // 2. Try scoping relative to modules defined in the current file
        let file_symbols = index.symbols_in_file(from_file);
        for symbol in file_symbols {
            if symbol.kind == SymbolKind::Module || symbol.kind == SymbolKind::Class {
                // Try Module::Name
                let qualified = format!("{}::{}", symbol.qualified, name);
                if let Some(resolved) = index.get(&qualified) {
                    return Some(ResolveResult {
                        symbol: resolved,
                        resolution_path: ResolutionPath::SameModule,
                    });
                }
            }
        }

        // 3. Try to find it as a top-level constant (if name doesn't start with ::)
        if !name.starts_with("::") {
            // Already checked exact match in step 1, but maybe we want to be explicit here?
            // For now, step 1 covers top-level if they are fully qualified or just simple names.
        }

        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CodeIndex, Location, Symbol, Visibility};
    use std::path::PathBuf;

    #[test]
    fn resolves_ruby_scoping() {
        let mut index = CodeIndex::new();
        // Define MyApp::Utils::Helper
        index.add_symbol(Symbol::new(
            "Helper".to_string(),
            "MyApp::Utils::Helper".to_string(),
            SymbolKind::Class,
            Location::new(PathBuf::from("utils.rb"), 1, 1),
            Visibility::Public,
            "ruby".to_string(),
        ));

        // Define MyApp::Utils (module)
        index.add_symbol(Symbol::new(
            "Utils".to_string(),
            "MyApp::Utils".to_string(),
            SymbolKind::Module,
            Location::new(PathBuf::from("utils.rb"), 1, 1),
            Visibility::Public,
            "ruby".to_string(),
        ));

        // From utils.rb, "Helper" should resolve to "MyApp::Utils::Helper"
        let resolver = RubyResolver;
        let result = resolver.resolve(&index, "Helper", Path::new("utils.rb"));
        assert!(result.is_some());
        assert_eq!(result.unwrap().symbol.qualified, "MyApp::Utils::Helper");
    }
}
