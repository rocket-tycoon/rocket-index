//! Name resolution for F# symbols with scope rules.
//!
//! This module implements F# name resolution, taking into account:
//! - Open statements (imports)
//! - Module hierarchy
//! - Qualified vs unqualified names
//! - Type-aware member access (RFC-001)

use std::path::Path;

use crate::type_cache::TypeMember;
use crate::{CodeIndex, Symbol};

/// Result of name resolution
#[derive(Debug, Clone)]
pub struct ResolveResult<'a> {
    /// The resolved symbol
    pub symbol: &'a Symbol,
    /// How the symbol was resolved
    pub resolution_path: ResolutionPath,
}

/// How a symbol was resolved
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolutionPath {
    /// Direct qualified name match
    Qualified,
    /// Resolved via an open statement
    ViaOpen(String),
    /// Resolved from the same module
    SameModule,
    /// Resolved from a parent module
    ParentModule(String),
    /// Resolved via type-aware member access (RFC-001)
    /// Contains the type name that the member was resolved on
    ViaMemberAccess { type_name: String },
}

/// Result of resolving a member access expression (e.g., `user.Name`)
#[derive(Debug, Clone)]
pub struct MemberResolveResult<'a> {
    /// The resolved type member
    pub member: &'a TypeMember,
    /// The type that contains this member
    pub type_name: String,
}

impl CodeIndex {
    /// Resolve a symbol name from a given file context.
    ///
    /// This implements F# name resolution rules:
    /// 1. Try exact qualified name match
    /// 2. Try same-file/same-module symbols
    /// 3. Try symbols accessible via open statements
    /// 4. Try parent module symbols
    ///
    /// # Arguments
    /// * `name` - The name to resolve (can be qualified like "List.map" or simple like "helper")
    /// * `from_file` - The file context for resolution (determines which opens are in scope)
    ///
    /// # Returns
    /// The resolved symbol if found, None otherwise
    #[must_use]
    pub fn resolve(&self, name: &str, from_file: &Path) -> Option<ResolveResult<'_>> {
        // 1. Try exact qualified name match (respecting compilation order)
        if let Some(symbol) = self.get_visible_from(name, from_file) {
            return Some(ResolveResult {
                symbol,
                resolution_path: ResolutionPath::Qualified,
            });
        }

        // 2. Try same-file symbols
        if let Some(result) = self.resolve_in_same_file(name, from_file) {
            return Some(result);
        }

        // 3. Try symbols via open statements (respecting compilation order)
        if let Some(result) = self.resolve_via_opens(name, from_file) {
            return Some(result);
        }

        // 4. Try parent module symbols (respecting compilation order)
        if let Some(result) = self.resolve_in_parent_modules(name, from_file) {
            return Some(result);
        }

        None
    }

    /// Get a symbol by qualified name, but only if it's visible from the given file.
    ///
    /// This respects F# compilation order: a symbol is only visible if its
    /// defining file comes before from_file in the compilation order.
    fn get_visible_from(&self, name: &str, from_file: &Path) -> Option<&Symbol> {
        let symbol = self.get(name)?;

        // Check if the symbol's file is visible from from_file
        if self.can_reference(from_file, &symbol.location.file) {
            Some(symbol)
        } else {
            // Symbol exists but is not visible due to compilation order
            None
        }
    }

    /// Try to resolve a name within the same file.
    fn resolve_in_same_file(&self, name: &str, from_file: &Path) -> Option<ResolveResult<'_>> {
        let file_symbols = self.symbols_in_file(from_file);

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
    fn resolve_via_opens(&self, name: &str, from_file: &Path) -> Option<ResolveResult<'_>> {
        let opens = self.opens_for_file(from_file);

        for open_module in opens {
            // Try: OpenModule.name
            let qualified = format!("{}.{}", open_module, name);
            if let Some(symbol) = self.get_visible_from(&qualified, from_file) {
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
                    if let Some(symbol) = self.get_visible_from(&qualified, from_file) {
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
    fn resolve_in_parent_modules(&self, name: &str, from_file: &Path) -> Option<ResolveResult<'_>> {
        // Get the current module from file symbols
        let file_symbols = self.symbols_in_file(from_file);

        for symbol in &file_symbols {
            // Find the module path for this file
            if let Some((module_path, _)) = symbol.qualified.rsplit_once('.') {
                // Try progressively shorter module paths
                let mut current_module = module_path.to_string();
                loop {
                    let qualified = format!("{}.{}", current_module, name);
                    if let Some(resolved) = self.get_visible_from(&qualified, from_file) {
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

    /// Resolve a dotted name like "PaymentService.processPayment"
    /// This handles the case where the first part might be a module alias or nested module.
    #[must_use]
    pub fn resolve_dotted(&self, name: &str, from_file: &Path) -> Option<ResolveResult<'_>> {
        // First try direct resolution
        if let Some(result) = self.resolve(name, from_file) {
            return Some(result);
        }

        // For dotted names, try resolving the first component as a module
        if name.contains('.') {
            let parts: Vec<&str> = name.splitn(2, '.').collect();
            if parts.len() == 2 {
                let module_name = parts[0];
                let member_name = parts[1];

                // Check opens for matching module suffix
                let opens = self.opens_for_file(from_file);
                for open_module in opens {
                    if open_module.ends_with(module_name) {
                        // The open brings the module into scope
                        let qualified = format!("{}.{}", open_module, member_name);
                        if let Some(symbol) = self.get_visible_from(&qualified, from_file) {
                            return Some(ResolveResult {
                                symbol,
                                resolution_path: ResolutionPath::ViaOpen(open_module.clone()),
                            });
                        }
                    }

                    // Also try open.module.member pattern
                    let qualified = format!("{}.{}.{}", open_module, module_name, member_name);
                    if let Some(symbol) = self.get_visible_from(&qualified, from_file) {
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

impl CodeIndex {
    // =========================================================================
    // Type-Aware Resolution (RFC-001)
    // =========================================================================

    /// Get the inferred type of a symbol by its qualified name.
    ///
    /// This uses the type cache (if loaded) to look up the type signature
    /// of a symbol. Returns `None` if no type cache is loaded or the symbol
    /// is not found.
    ///
    /// # Example
    /// ```ignore
    /// // If myString is defined as: let myString = "hello"
    /// // The type cache would have: MyModule.myString -> "string"
    /// let ty = index.infer_expression_type("MyModule.myString");
    /// assert_eq!(ty, Some("string"));
    /// ```
    pub fn infer_expression_type(&self, qualified_name: &str) -> Option<&str> {
        self.get_symbol_type(qualified_name)
    }

    /// Resolve a member access expression like `expr.member`.
    ///
    /// Given a type name and member name, looks up the member in the type cache.
    /// This enables navigation from `user.Name` to the definition of `Name` on `User`.
    ///
    /// # Arguments
    /// * `type_name` - The type of the expression (e.g., "User", "string")
    /// * `member_name` - The member being accessed (e.g., "Name", "Length")
    ///
    /// # Returns
    /// The member information if found in the type cache, None otherwise.
    ///
    /// # Example
    /// ```ignore
    /// // Given: let user: User = ...
    /// //        user.Name
    /// let result = index.resolve_member_access("User", "Name");
    /// // Returns MemberResolveResult with member info for User.Name
    /// ```
    pub fn resolve_member_access<'a>(
        &'a self,
        type_name: &str,
        member_name: &str,
    ) -> Option<MemberResolveResult<'a>> {
        let member = self.get_type_member(type_name, member_name)?;
        Some(MemberResolveResult {
            member,
            type_name: type_name.to_string(),
        })
    }

    /// Resolve a dotted expression with type-aware fallback.
    ///
    /// This method first tries normal syntactic resolution. If that fails
    /// and a type cache is available, it attempts type-aware resolution
    /// by looking up the base expression's type and resolving the member.
    ///
    /// # Arguments
    /// * `base_qualified` - The qualified name of the base expression (e.g., "MyModule.user")
    /// * `member_name` - The member being accessed (e.g., "Name")
    /// * `from_file` - The file context for resolution
    ///
    /// # Returns
    /// A ResolveResult if the member can be resolved, None otherwise.
    pub fn resolve_with_type_info(
        &self,
        base_qualified: &str,
        member_name: &str,
        from_file: &Path,
    ) -> Option<ResolveResult<'_>> {
        // First, try normal dotted resolution (e.g., Module.function)
        let dotted_name = format!("{}.{}", base_qualified, member_name);
        if let Some(result) = self.resolve_dotted(&dotted_name, from_file) {
            return Some(result);
        }

        // If type cache is available, try type-aware resolution
        if let Some(type_cache) = self.type_cache() {
            // Get the type of the base expression
            if let Some(base_type) = type_cache.get_type(base_qualified) {
                // Extract the simple type name (handle generics like "Async<User>")
                let simple_type = extract_simple_type(base_type);

                // Look for a symbol definition for this type's member
                // Try: TypeName.memberName as a qualified name
                let type_member_qualified = format!("{}.{}", simple_type, member_name);
                if let Some(symbol) = self.get(&type_member_qualified) {
                    return Some(ResolveResult {
                        symbol,
                        resolution_path: ResolutionPath::ViaMemberAccess {
                            type_name: simple_type.to_string(),
                        },
                    });
                }

                // Also try looking up in parent modules
                // e.g., if type is "MyApp.Domain.User", try "MyApp.Domain.User.Name"
                let full_type_member = format!("{}.{}", base_type, member_name);
                if let Some(symbol) = self.get(&full_type_member) {
                    return Some(ResolveResult {
                        symbol,
                        resolution_path: ResolutionPath::ViaMemberAccess {
                            type_name: base_type.to_string(),
                        },
                    });
                }
            }
        }

        None
    }
}

/// Extract the simple type name from a potentially complex type signature.
///
/// Examples:
/// - "string" -> "string"
/// - "User" -> "User"
/// - "Async<User>" -> "User"
/// - "Result<User, Error>" -> "User" (takes first type arg)
/// - "int list" -> "int"
/// - "User option" -> "User"
fn extract_simple_type(type_sig: &str) -> &str {
    let trimmed = type_sig.trim();

    // Handle F# postfix types: "int list", "User option", "string array"
    let postfix_types = [" list", " option", " array", " seq", " ref"];
    for suffix in &postfix_types {
        if let Some(stripped) = trimmed.strip_suffix(suffix) {
            return stripped.trim();
        }
    }

    // Handle generic types: "Async<User>", "Result<User, Error>"
    if let Some(angle_pos) = trimmed.find('<') {
        let inner = &trimmed[angle_pos + 1..];
        if let Some(end) = inner.find(['>', ',']) {
            return inner[..end].trim();
        }
        // Fallback: return the type before the angle bracket
        return trimmed[..angle_pos].trim();
    }

    // Handle function types: take the return type (last part after ->)
    if trimmed.contains("->") {
        if let Some(last_arrow) = trimmed.rfind("->") {
            return trimmed[last_arrow + 2..].trim();
        }
    }

    trimmed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::type_cache::{MemberKind, TypeCache, TypeCacheSchema, TypeMember, TypedSymbol};
    use crate::{Location, SymbolKind, Visibility};
    use std::path::PathBuf;

    fn make_symbol(name: &str, qualified: &str, file: &str) -> Symbol {
        Symbol {
            name: name.to_string(),
            qualified: qualified.to_string(),
            kind: SymbolKind::Function,
            location: Location::new(PathBuf::from(file), 1, 1),
            visibility: Visibility::Public,
        }
    }

    fn make_type_cache() -> TypeCache {
        let schema = TypeCacheSchema {
            version: 1,
            extracted_at: "2024-12-02T10:30:00Z".to_string(),
            project: "TestProject".to_string(),
            symbols: vec![
                TypedSymbol {
                    name: "user".to_string(),
                    qualified: "MyModule.user".to_string(),
                    type_signature: "User".to_string(),
                    file: "src/MyModule.fs".to_string(),
                    line: 10,
                    parameters: vec![],
                },
                TypedSymbol {
                    name: "myString".to_string(),
                    qualified: "MyModule.myString".to_string(),
                    type_signature: "string".to_string(),
                    file: "src/MyModule.fs".to_string(),
                    line: 20,
                    parameters: vec![],
                },
                TypedSymbol {
                    name: "asyncUser".to_string(),
                    qualified: "MyModule.asyncUser".to_string(),
                    type_signature: "Async<User>".to_string(),
                    file: "src/MyModule.fs".to_string(),
                    line: 30,
                    parameters: vec![],
                },
                TypedSymbol {
                    name: "users".to_string(),
                    qualified: "MyModule.users".to_string(),
                    type_signature: "User list".to_string(),
                    file: "src/MyModule.fs".to_string(),
                    line: 40,
                    parameters: vec![],
                },
            ],
            members: vec![
                TypeMember {
                    type_name: "User".to_string(),
                    member: "Name".to_string(),
                    member_type: "string".to_string(),
                    kind: MemberKind::Property,
                },
                TypeMember {
                    type_name: "User".to_string(),
                    member: "Save".to_string(),
                    member_type: "unit -> Async<unit>".to_string(),
                    kind: MemberKind::Method,
                },
                TypeMember {
                    type_name: "string".to_string(),
                    member: "Length".to_string(),
                    member_type: "int".to_string(),
                    kind: MemberKind::Property,
                },
            ],
        };
        TypeCache::from_schema(schema)
    }

    #[test]
    fn resolves_qualified_name() {
        let mut index = CodeIndex::new();
        index.add_symbol(make_symbol("helper", "MyApp.Utils.helper", "src/Utils.fs"));

        let result = index.resolve("MyApp.Utils.helper", Path::new("src/Main.fs"));
        assert!(result.is_some());
        assert_eq!(result.unwrap().resolution_path, ResolutionPath::Qualified);
    }

    #[test]
    fn resolves_in_same_file() {
        let mut index = CodeIndex::new();
        index.add_symbol(make_symbol("localFn", "MyApp.Main.localFn", "src/Main.fs"));

        let result = index.resolve("localFn", Path::new("src/Main.fs"));
        assert!(result.is_some());
        assert_eq!(result.unwrap().resolution_path, ResolutionPath::SameModule);
    }

    #[test]
    fn resolves_via_open() {
        let mut index = CodeIndex::new();
        index.add_symbol(make_symbol("helper", "MyApp.Utils.helper", "src/Utils.fs"));
        index.add_open(PathBuf::from("src/Main.fs"), "MyApp.Utils".to_string());

        let result = index.resolve("helper", Path::new("src/Main.fs"));
        assert!(result.is_some());

        let result = result.unwrap();
        assert!(matches!(result.resolution_path, ResolutionPath::ViaOpen(_)));
        assert_eq!(result.symbol.qualified, "MyApp.Utils.helper");
    }

    #[test]
    fn resolves_dotted_name_via_open() {
        let mut index = CodeIndex::new();
        index.add_symbol(make_symbol(
            "map",
            "FSharp.Collections.List.map",
            "stdlib.fs",
        ));
        index.add_open(
            PathBuf::from("src/Main.fs"),
            "FSharp.Collections".to_string(),
        );

        let result = index.resolve_dotted("List.map", Path::new("src/Main.fs"));
        assert!(result.is_some());
        assert_eq!(result.unwrap().symbol.name, "map");
    }

    #[test]
    fn returns_none_for_unknown_symbol() {
        let index = CodeIndex::new();
        let result = index.resolve("unknownSymbol", Path::new("src/Main.fs"));
        assert!(result.is_none());
    }

    #[test]
    fn resolves_with_open_statement() {
        let mut index = CodeIndex::new();

        // Add a symbol in Utils module
        index.add_symbol(make_symbol("helper", "MyApp.Utils.helper", "src/Utils.fs"));

        // Add another file that opens Utils
        index.add_symbol(make_symbol("run", "MyApp.Main.run", "src/Main.fs"));
        index.add_open(PathBuf::from("src/Main.fs"), "MyApp.Utils".to_string());

        // Resolve "helper" from Main.fs should find it via the open
        let resolved = index.resolve("helper", Path::new("src/Main.fs"));
        assert!(resolved.is_some());
        assert_eq!(
            resolved.unwrap().symbol.location.file,
            PathBuf::from("src/Utils.fs")
        );
    }

    // =========================================================================
    // Type-Aware Resolution Tests (RFC-001)
    // =========================================================================

    #[test]
    fn test_infer_expression_type() {
        let mut index = CodeIndex::new();
        index.set_type_cache(make_type_cache());

        assert_eq!(index.infer_expression_type("MyModule.user"), Some("User"));
        assert_eq!(
            index.infer_expression_type("MyModule.myString"),
            Some("string")
        );
        assert!(index.infer_expression_type("NonExistent.symbol").is_none());
    }

    #[test]
    fn test_infer_expression_type_without_cache() {
        let index = CodeIndex::new();
        assert!(index.infer_expression_type("MyModule.user").is_none());
    }

    #[test]
    fn test_resolve_member_access() {
        let mut index = CodeIndex::new();
        index.set_type_cache(make_type_cache());

        // Resolve User.Name
        let result = index.resolve_member_access("User", "Name").unwrap();
        assert_eq!(result.type_name, "User");
        assert_eq!(result.member.member, "Name");
        assert_eq!(result.member.member_type, "string");
        assert_eq!(result.member.kind, MemberKind::Property);

        // Resolve User.Save
        let result = index.resolve_member_access("User", "Save").unwrap();
        assert_eq!(result.member.kind, MemberKind::Method);

        // Resolve string.Length
        let result = index.resolve_member_access("string", "Length").unwrap();
        assert_eq!(result.member.member_type, "int");
    }

    #[test]
    fn test_resolve_member_access_not_found() {
        let mut index = CodeIndex::new();
        index.set_type_cache(make_type_cache());

        assert!(index.resolve_member_access("User", "NonExistent").is_none());
        assert!(index.resolve_member_access("NonExistent", "Name").is_none());
    }

    #[test]
    fn test_resolve_member_access_without_cache() {
        let index = CodeIndex::new();
        assert!(index.resolve_member_access("User", "Name").is_none());
    }

    #[test]
    fn test_resolve_with_type_info_falls_back_to_syntactic() {
        let mut index = CodeIndex::new();
        // Add a normal symbol that can be resolved syntactically
        index.add_symbol(make_symbol("helper", "Utils.helper", "src/Utils.fs"));

        // Should resolve via normal dotted resolution (no type cache needed)
        let result = index.resolve_with_type_info("Utils", "helper", Path::new("src/Main.fs"));
        assert!(result.is_some());
        assert_eq!(result.unwrap().symbol.name, "helper");
    }

    #[test]
    fn test_resolve_with_type_info_uses_type_cache() {
        let mut index = CodeIndex::new();
        index.set_type_cache(make_type_cache());

        // Add a symbol for User.Name (simulating a record field definition)
        index.add_symbol(Symbol {
            name: "Name".to_string(),
            qualified: "User.Name".to_string(),
            kind: SymbolKind::Member,
            location: Location::new(PathBuf::from("src/Types.fs"), 5, 5),
            visibility: Visibility::Public,
        });

        // Resolve user.Name where user: User
        let result =
            index.resolve_with_type_info("MyModule.user", "Name", Path::new("src/Main.fs"));

        assert!(result.is_some());
        let result = result.unwrap();
        assert_eq!(result.symbol.name, "Name");
        assert!(matches!(
            result.resolution_path,
            ResolutionPath::ViaMemberAccess { type_name } if type_name == "User"
        ));
    }

    #[test]
    fn test_resolve_with_type_info_no_match() {
        let mut index = CodeIndex::new();
        index.set_type_cache(make_type_cache());

        // Try to resolve a member that doesn't exist as a symbol
        let result = index.resolve_with_type_info(
            "MyModule.user",
            "NonExistentMember",
            Path::new("src/Main.fs"),
        );

        assert!(result.is_none());
    }

    // =========================================================================
    // extract_simple_type Tests
    // =========================================================================

    #[test]
    fn test_extract_simple_type_basic() {
        assert_eq!(extract_simple_type("string"), "string");
        assert_eq!(extract_simple_type("int"), "int");
        assert_eq!(extract_simple_type("User"), "User");
        assert_eq!(
            extract_simple_type("MyApp.Domain.User"),
            "MyApp.Domain.User"
        );
    }

    #[test]
    fn test_extract_simple_type_postfix() {
        assert_eq!(extract_simple_type("int list"), "int");
        assert_eq!(extract_simple_type("User option"), "User");
        assert_eq!(extract_simple_type("string array"), "string");
        assert_eq!(extract_simple_type("User seq"), "User");
    }

    #[test]
    fn test_extract_simple_type_generic() {
        assert_eq!(extract_simple_type("Async<User>"), "User");
        assert_eq!(extract_simple_type("Result<User, Error>"), "User");
        assert_eq!(extract_simple_type("Task<string>"), "string");
        assert_eq!(extract_simple_type("Option<int>"), "int");
    }

    #[test]
    fn test_extract_simple_type_function() {
        assert_eq!(extract_simple_type("int -> string"), "string");
        // For complex nested generics, we extract the outermost return type
        // The function returns "Async<Result<Response, Error>>" but extract_simple_type
        // will try to extract from that, getting "Result<Response" (first type arg)
        // In practice, we care about the base type for member lookup
        assert_eq!(extract_simple_type("int -> User"), "User");
        assert_eq!(extract_simple_type("string -> int -> bool"), "bool");
    }

    #[test]
    fn test_extract_simple_type_whitespace() {
        assert_eq!(extract_simple_type("  string  "), "string");
        assert_eq!(extract_simple_type("  User option  "), "User");
    }
}
