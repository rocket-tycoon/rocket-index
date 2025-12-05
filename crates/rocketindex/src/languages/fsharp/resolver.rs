//! F# specific symbol resolution logic.

use std::path::Path;

use crate::resolve::{ResolutionPath, ResolveResult, SymbolResolver};
use crate::{CodeIndex, Symbol};

pub struct FSharpResolver;

impl SymbolResolver for FSharpResolver {
    fn resolve<'a>(
        &self,
        index: &'a CodeIndex,
        name: &str,
        from_file: &Path,
    ) -> Option<ResolveResult<'a>> {
        // 1. Try exact qualified name match (respecting compilation order)
        if let Some(symbol) = get_visible_from(index, name, from_file) {
            return Some(ResolveResult {
                symbol,
                resolution_path: ResolutionPath::Qualified,
            });
        }

        // 2. Try same-file symbols
        if let Some(result) = resolve_in_same_file(index, name, from_file) {
            return Some(result);
        }

        // 3. Try symbols via open statements (respecting compilation order)
        if let Some(result) = resolve_via_opens(index, name, from_file) {
            return Some(result);
        }

        // 4. Try parent module symbols (respecting compilation order)
        if let Some(result) = resolve_in_parent_modules(index, name, from_file) {
            return Some(result);
        }

        None
    }

    fn resolve_dotted<'a>(
        &self,
        index: &'a CodeIndex,
        name: &str,
        from_file: &Path,
    ) -> Option<ResolveResult<'a>> {
        // First try direct resolution
        if let Some(result) = self.resolve(index, name, from_file) {
            return Some(result);
        }

        // For dotted names, try resolving the first component as a module
        if name.contains('.') {
            let parts: Vec<&str> = name.splitn(2, '.').collect();
            if parts.len() == 2 {
                let module_name = parts[0];
                let member_name = parts[1];

                // Check opens for matching module suffix
                let opens = index.opens_for_file(from_file);
                for open_module in opens {
                    if open_module.ends_with(module_name) {
                        // The open brings the module into scope
                        let qualified = format!("{}.{}", open_module, member_name);
                        if let Some(symbol) = get_visible_from(index, &qualified, from_file) {
                            return Some(ResolveResult {
                                symbol,
                                resolution_path: ResolutionPath::ViaOpen(open_module.clone()),
                            });
                        }
                    }

                    // Also try open.module.member pattern
                    let qualified = format!("{}.{}.{}", open_module, module_name, member_name);
                    if let Some(symbol) = get_visible_from(index, &qualified, from_file) {
                        return Some(ResolveResult {
                            symbol,
                            resolution_path: ResolutionPath::ViaOpen(open_module.clone()),
                        });
                    }
                }
            }
        }

        None
    }
}

/// Get a symbol by qualified name, but only if it's visible from the given file.
fn get_visible_from<'a>(index: &'a CodeIndex, name: &str, from_file: &Path) -> Option<&'a Symbol> {
    let symbol = index.get(name)?;

    // Check if the symbol's file is visible from from_file
    if index.can_reference(from_file, &symbol.location.file) {
        Some(symbol)
    } else {
        // Symbol exists but is not visible due to compilation order
        None
    }
}

/// Try to resolve a name within the same file.
fn resolve_in_same_file<'a>(
    index: &'a CodeIndex,
    name: &str,
    from_file: &Path,
) -> Option<ResolveResult<'a>> {
    let file_symbols = index.symbols_in_file(from_file);

    // Try unqualified match within file symbols
    for symbol in file_symbols {
        if symbol.name == name {
            return Some(ResolveResult {
                symbol,
                resolution_path: ResolutionPath::SameModule,
            });
        }
        // Also check if the name matches the end of the qualified name
        if symbol.qualified.ends_with(&format!(".{}", name)) {
            return Some(ResolveResult {
                symbol,
                resolution_path: ResolutionPath::SameModule,
            });
        }
    }

    None
}

/// Try to resolve a name using open statements.
fn resolve_via_opens<'a>(
    index: &'a CodeIndex,
    name: &str,
    from_file: &Path,
) -> Option<ResolveResult<'a>> {
    let opens = index.opens_for_file(from_file);

    for open_module in opens {
        // Try: OpenModule.name
        let qualified = format!("{}.{}", open_module, name);
        if let Some(symbol) = get_visible_from(index, &qualified, from_file) {
            return Some(ResolveResult {
                symbol,
                resolution_path: ResolutionPath::ViaOpen(open_module.clone()),
            });
        }

        // For dotted names like "List.map", try "OpenModule.List.map"
        if name.contains('.') {
            let parts: Vec<&str> = name.splitn(2, '.').collect();
            if parts.len() == 2 {
                let qualified = format!("{}.{}", open_module, name);
                if let Some(symbol) = get_visible_from(index, &qualified, from_file) {
                    return Some(ResolveResult {
                        symbol,
                        resolution_path: ResolutionPath::ViaOpen(open_module.clone()),
                    });
                }
            }
        }
    }

    None
}

/// Try to resolve a name in parent modules.
fn resolve_in_parent_modules<'a>(
    index: &'a CodeIndex,
    name: &str,
    from_file: &Path,
) -> Option<ResolveResult<'a>> {
    // Get the current module from file symbols
    let file_symbols = index.symbols_in_file(from_file);

    for symbol in &file_symbols {
        // Find the module path for this file
        if let Some((module_path, _)) = symbol.qualified.rsplit_once('.') {
            // Try progressively shorter module paths
            let mut current_module = module_path.to_string();
            loop {
                let qualified = format!("{}.{}", current_module, name);
                if let Some(resolved) = get_visible_from(index, &qualified, from_file) {
                    return Some(ResolveResult {
                        symbol: resolved,
                        resolution_path: ResolutionPath::ParentModule(current_module),
                    });
                }

                // Move up to parent module
                match current_module.rsplit_once('.') {
                    Some((parent, _)) => current_module = parent.to_string(),
                    None => break,
                }
            }
        }
    }

    None
}

// Note: Type-aware resolution logic (resolve_with_type_info) is currently not included here
// as it was part of CodeIndex impl. If needed, it should be moved to a separate trait or
// helper module.
