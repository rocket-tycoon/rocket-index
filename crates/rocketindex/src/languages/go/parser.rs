//! Symbol extraction from Go source files using tree-sitter.

use std::cell::RefCell;
use std::path::Path;

use crate::parse::{find_child_by_kind, node_to_location, LanguageParser, ParseResult};
use crate::{Reference, Symbol, SymbolKind, Visibility};

// Thread-local parser reuse - avoids creating a new parser per file
thread_local! {
    static GO_PARSER: RefCell<tree_sitter::Parser> = RefCell::new({
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_go::LANGUAGE.into())
            .expect("tree-sitter-go grammar incompatible with tree-sitter version");
        parser
    });
}

pub struct GoParser;

impl LanguageParser for GoParser {
    fn extract_symbols(&self, file: &Path, source: &str, max_depth: usize) -> ParseResult {
        GO_PARSER.with(|parser| {
            let mut parser = parser.borrow_mut();

            let tree = match parser.parse(source, None) {
                Some(tree) => tree,
                None => {
                    tracing::warn!("Failed to parse file: {:?}", file);
                    return ParseResult::default();
                }
            };

            let mut result = ParseResult::default();
            let root = tree.root_node();

            // Extract package name first for qualified names
            let package_name = extract_package_name(&root, source);

            extract_recursive(
                &root,
                source.as_bytes(),
                file,
                &mut result,
                package_name.as_deref(),
                max_depth,
            );

            // Set module path from package
            result.module_path = package_name;

            result
        })
    }
}

/// Extract the package name from a source file
fn extract_package_name(root: &tree_sitter::Node, source: &str) -> Option<String> {
    let source_bytes = source.as_bytes();
    for i in 0..root.child_count() {
        if let Some(child) = root.child(i) {
            if child.kind() == "package_clause" {
                // Look for package_identifier child (not a named field)
                for j in 0..child.child_count() {
                    if let Some(pkg_id) = child.child(j) {
                        if pkg_id.kind() == "package_identifier" {
                            if let Ok(name) = pkg_id.utf8_text(source_bytes) {
                                return Some(name.to_string());
                            }
                        }
                    }
                }
            }
        }
    }
    None
}

/// Determine visibility based on Go's capitalization convention
/// Exported (public) identifiers start with an uppercase letter
fn extract_visibility(name: &str) -> Visibility {
    if name
        .chars()
        .next()
        .map(|c| c.is_uppercase())
        .unwrap_or(false)
    {
        Visibility::Public
    } else {
        Visibility::Private
    }
}

/// Build a qualified name with optional package prefix
fn qualified_name(name: &str, package: Option<&str>) -> String {
    match package {
        Some(pkg) => format!("{}.{}", pkg, name),
        None => name.to_string(),
    }
}

/// Extract doc comments from preceding comment nodes
fn extract_doc_comments(node: &tree_sitter::Node, source: &[u8]) -> Option<String> {
    let mut docs = Vec::new();

    // Look for preceding comment nodes (siblings before this node)
    if let Some(parent) = node.parent() {
        let mut prev_sibling = None;
        for i in 0..parent.child_count() {
            if let Some(child) = parent.child(i) {
                if child.id() == node.id() {
                    break;
                }
                prev_sibling = Some(child);
            }
        }

        // Check if the previous sibling is a comment
        if let Some(prev) = prev_sibling {
            if prev.kind() == "comment" {
                if let Ok(text) = prev.utf8_text(source) {
                    let doc = text
                        .trim_start_matches("//")
                        .trim_start_matches("/*")
                        .trim_end_matches("*/")
                        .trim();
                    if !doc.is_empty() {
                        docs.push(doc.to_string());
                    }
                }
            }
        }
    }

    if docs.is_empty() {
        None
    } else {
        Some(docs.join("\n"))
    }
}

/// Extract function/method signature
fn extract_function_signature(
    node: &tree_sitter::Node,
    source: &[u8],
    name: &str,
) -> Option<String> {
    let mut sig = format!("func {}", name);

    // Get parameters
    if let Some(params) = node.child_by_field_name("parameters") {
        if let Ok(params_text) = params.utf8_text(source) {
            sig.push_str(params_text);
        }
    }

    // Get return type if present
    if let Some(result) = node.child_by_field_name("result") {
        if let Ok(result_text) = result.utf8_text(source) {
            sig.push(' ');
            sig.push_str(result_text);
        }
    }

    Some(sig)
}

/// Extract the receiver type from a method declaration
fn extract_receiver_type(node: &tree_sitter::Node, source: &[u8]) -> Option<String> {
    let receiver = node.child_by_field_name("receiver")?;

    // The receiver is a parameter_list, find the type inside
    for i in 0..receiver.child_count() {
        if let Some(child) = receiver.child(i) {
            if child.kind() == "parameter_declaration" {
                // Look for the type (could be pointer or value receiver)
                if let Some(type_node) = child.child_by_field_name("type") {
                    let type_text = type_node.utf8_text(source).ok()?;
                    // Strip pointer prefix if present
                    let type_name = type_text.trim_start_matches('*');
                    return Some(type_name.to_string());
                }
            }
        }
    }
    None
}

/// Extract import paths from import declarations
fn extract_imports(node: &tree_sitter::Node, source: &[u8], result: &mut ParseResult) {
    match node.kind() {
        "import_spec" => {
            // Single import: import "fmt" or import alias "fmt"
            if let Some(path_node) = node.child_by_field_name("path") {
                if let Ok(path) = path_node.utf8_text(source) {
                    // Remove quotes from import path
                    let clean_path = path.trim_matches('"').to_string();
                    result.opens.push(clean_path);
                }
            }
        }
        "import_spec_list" => {
            // Grouped imports: import ( "fmt" \n "os" )
            for i in 0..node.child_count() {
                if let Some(child) = node.child(i) {
                    if child.kind() == "import_spec" {
                        extract_imports(&child, source, result);
                    }
                }
            }
        }
        _ => {}
    }
}

fn extract_recursive(
    node: &tree_sitter::Node,
    source: &[u8],
    file: &Path,
    result: &mut ParseResult,
    package: Option<&str>,
    max_depth: usize,
) {
    if max_depth == 0 {
        return;
    }

    match node.kind() {
        "function_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Ok(name) = name_node.utf8_text(source) {
                    let qualified = qualified_name(name, package);
                    let visibility = extract_visibility(name);
                    let doc = extract_doc_comments(node, source);
                    let signature = extract_function_signature(node, source, name);

                    result.symbols.push(Symbol {
                        name: name.to_string(),
                        qualified,
                        kind: SymbolKind::Function,
                        location: node_to_location(file, &name_node),
                        visibility,
                        language: "go".to_string(),
                        parent: None,
                        mixins: None,
                        attributes: None,
                        implements: None,
                        doc,
                        signature,
                    });
                }
            }
        }

        "method_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Ok(name) = name_node.utf8_text(source) {
                    let receiver_type = extract_receiver_type(node, source);
                    let visibility = extract_visibility(name);
                    let doc = extract_doc_comments(node, source);

                    // Build qualified name as Package.Type.Method
                    let qualified = match (&receiver_type, package) {
                        (Some(recv), Some(pkg)) => format!("{}.{}.{}", pkg, recv, name),
                        (Some(recv), None) => format!("{}.{}", recv, name),
                        (None, Some(pkg)) => format!("{}.{}", pkg, name),
                        (None, None) => name.to_string(),
                    };

                    // Build signature with receiver
                    let mut sig = String::from("func ");
                    if let Some(recv) = &receiver_type {
                        sig.push_str(&format!("({}) ", recv));
                    }
                    sig.push_str(name);
                    if let Some(params) = node.child_by_field_name("parameters") {
                        if let Ok(params_text) = params.utf8_text(source) {
                            sig.push_str(params_text);
                        }
                    }
                    if let Some(ret) = node.child_by_field_name("result") {
                        if let Ok(ret_text) = ret.utf8_text(source) {
                            sig.push(' ');
                            sig.push_str(ret_text);
                        }
                    }

                    result.symbols.push(Symbol {
                        name: name.to_string(),
                        qualified,
                        kind: SymbolKind::Function,
                        location: node_to_location(file, &name_node),
                        visibility,
                        language: "go".to_string(),
                        parent: receiver_type,
                        mixins: None,
                        attributes: None,
                        implements: None,
                        doc,
                        signature: Some(sig),
                    });
                }
            }
        }

        "type_declaration" => {
            // type_declaration contains one or more type_spec
            for i in 0..node.child_count() {
                if let Some(child) = node.child(i) {
                    if child.kind() == "type_spec" {
                        extract_type_spec(&child, source, file, result, package, max_depth);
                    }
                }
            }
        }

        "const_declaration" => {
            // const_declaration contains one or more const_spec
            for i in 0..node.child_count() {
                if let Some(child) = node.child(i) {
                    if child.kind() == "const_spec" {
                        extract_const_or_var_spec(
                            &child,
                            source,
                            file,
                            result,
                            package,
                            SymbolKind::Value,
                        );
                    }
                }
            }
        }

        "var_declaration" => {
            // var_declaration contains one or more var_spec
            for i in 0..node.child_count() {
                if let Some(child) = node.child(i) {
                    if child.kind() == "var_spec" {
                        extract_const_or_var_spec(
                            &child,
                            source,
                            file,
                            result,
                            package,
                            SymbolKind::Value,
                        );
                    }
                }
            }
        }

        "import_declaration" => {
            // Extract imports for resolution
            for i in 0..node.child_count() {
                if let Some(child) = node.child(i) {
                    extract_imports(&child, source, result);
                }
            }
        }

        // Extract references from identifiers
        "identifier" | "type_identifier" => {
            if is_reference_context(node) {
                if let Ok(name) = node.utf8_text(source) {
                    result.references.push(Reference {
                        name: name.to_string(),
                        location: node_to_location(file, node),
                    });
                }
            }
        }

        // Extract references from selector expressions (like fmt.Println)
        "selector_expression" => {
            if is_reference_context(node) {
                if let Ok(name) = node.utf8_text(source) {
                    result.references.push(Reference {
                        name: name.to_string(),
                        location: node_to_location(file, node),
                    });
                }
            }
        }

        _ => {}
    }

    // Recurse into children
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            extract_recursive(&child, source, file, result, package, max_depth - 1);
        }
    }
}

/// Extract a type specification (struct, interface, or type alias)
fn extract_type_spec(
    node: &tree_sitter::Node,
    source: &[u8],
    file: &Path,
    result: &mut ParseResult,
    package: Option<&str>,
    max_depth: usize,
) {
    let name_node = match node.child_by_field_name("name") {
        Some(n) => n,
        None => return,
    };

    let name = match name_node.utf8_text(source) {
        Ok(n) => n,
        Err(_) => return,
    };

    let type_node = match node.child_by_field_name("type") {
        Some(n) => n,
        None => return,
    };

    let qualified = qualified_name(name, package);
    let visibility = extract_visibility(name);
    let doc = extract_doc_comments(node, source);

    match type_node.kind() {
        "struct_type" => {
            result.symbols.push(Symbol {
                name: name.to_string(),
                qualified: qualified.clone(),
                kind: SymbolKind::Class,
                location: node_to_location(file, &name_node),
                visibility,
                language: "go".to_string(),
                parent: None,
                mixins: None,
                attributes: None,
                implements: None,
                doc,
                signature: None,
            });

            // Extract struct fields
            if let Some(field_list) = find_child_by_kind(&type_node, "field_declaration_list") {
                extract_struct_fields(&field_list, source, file, result, &qualified, max_depth);
            }
        }

        "interface_type" => {
            // Extract embedded interfaces first
            let mut embedded_interfaces = Vec::new();
            for i in 0..type_node.child_count() {
                if let Some(child) = type_node.child(i) {
                    // Embedded interfaces appear as type_elem nodes in tree-sitter-go
                    match child.kind() {
                        "type_identifier" => {
                            // Simple embedded interface: Reader
                            if let Ok(embedded_name) = child.utf8_text(source) {
                                embedded_interfaces.push(embedded_name.to_string());
                            }
                        }
                        "qualified_type" => {
                            // Qualified embedded interface: io.Reader
                            if let Ok(qualified_name) = child.utf8_text(source) {
                                embedded_interfaces.push(qualified_name.to_string());
                            }
                        }
                        "type_elem" => {
                            // type_elem contains embedded interfaces
                            // Look for type_identifier or qualified_type inside
                            for j in 0..child.child_count() {
                                if let Some(inner) = child.child(j) {
                                    match inner.kind() {
                                        "type_identifier" => {
                                            if let Ok(embedded_name) = inner.utf8_text(source) {
                                                embedded_interfaces.push(embedded_name.to_string());
                                            }
                                        }
                                        "qualified_type" => {
                                            if let Ok(qualified_name) = inner.utf8_text(source) {
                                                embedded_interfaces
                                                    .push(qualified_name.to_string());
                                            }
                                        }
                                        _ => {}
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }

            let mixins = if embedded_interfaces.is_empty() {
                None
            } else {
                Some(embedded_interfaces)
            };

            result.symbols.push(Symbol {
                name: name.to_string(),
                qualified: qualified.clone(),
                kind: SymbolKind::Interface,
                location: node_to_location(file, &name_node),
                visibility,
                language: "go".to_string(),
                parent: None,
                mixins,
                attributes: None,
                implements: None,
                doc,
                signature: None,
            });

            // Extract interface methods
            for i in 0..type_node.child_count() {
                if let Some(child) = type_node.child(i) {
                    if child.kind() == "method_elem" {
                        extract_interface_method(&child, source, file, result, &qualified);
                    }
                }
            }
        }

        _ => {
            // Type alias or other type definition
            result.symbols.push(Symbol {
                name: name.to_string(),
                qualified,
                kind: SymbolKind::Type,
                location: node_to_location(file, &name_node),
                visibility,
                language: "go".to_string(),
                parent: None,
                mixins: None,
                attributes: None,
                implements: None,
                doc,
                signature: None,
            });
        }
    }
}

/// Extract struct fields
fn extract_struct_fields(
    field_list: &tree_sitter::Node,
    source: &[u8],
    file: &Path,
    result: &mut ParseResult,
    parent_qualified: &str,
    _max_depth: usize,
) {
    for i in 0..field_list.child_count() {
        if let Some(field) = field_list.child(i) {
            if field.kind() == "field_declaration" {
                // A field can have multiple names: x, y int
                if let Some(name_node) = field.child_by_field_name("name") {
                    // Named field: has explicit field_identifier
                    if let Ok(name) = name_node.utf8_text(source) {
                        let qualified = format!("{}.{}", parent_qualified, name);
                        let visibility = extract_visibility(name);

                        result.symbols.push(Symbol {
                            name: name.to_string(),
                            qualified,
                            kind: SymbolKind::Member,
                            location: node_to_location(file, &name_node),
                            visibility,
                            language: "go".to_string(),
                            parent: Some(parent_qualified.to_string()),
                            mixins: None,
                            attributes: None,
                            implements: None,
                            doc: None,
                            signature: None,
                        });
                    }
                } else {
                    // Embedded field: no field_identifier, type name becomes field name
                    // Handle both `Type` and `*Type` patterns
                    if let Some((name, name_node)) = extract_embedded_field_name(&field, source) {
                        let qualified = format!("{}.{}", parent_qualified, name);
                        let visibility = extract_visibility(&name);

                        result.symbols.push(Symbol {
                            name: name.to_string(),
                            qualified,
                            kind: SymbolKind::Member,
                            location: node_to_location(file, &name_node),
                            visibility,
                            language: "go".to_string(),
                            parent: Some(parent_qualified.to_string()),
                            mixins: None,
                            attributes: None,
                            implements: None,
                            doc: None,
                            signature: None,
                        });
                    }
                }
            }
        }
    }
}

/// Extract embedded field name from a field_declaration without a name
///
/// Handles patterns like:
/// - `State` (type_identifier)
/// - `*State` (* followed by type_identifier)
/// - `pkg.Type` (qualified_type - for now we just use the type part)
fn extract_embedded_field_name<'a>(
    field: &'a tree_sitter::Node<'a>,
    source: &[u8],
) -> Option<(String, tree_sitter::Node<'a>)> {
    // Look for type_identifier directly in the field_declaration
    for i in 0..field.child_count() {
        if let Some(child) = field.child(i) {
            match child.kind() {
                "type_identifier" => {
                    // Simple embedded field: `SecurityOptions`
                    if let Ok(name) = child.utf8_text(source) {
                        return Some((name.to_string(), child));
                    }
                }
                "pointer_type" => {
                    // Pointer embedded field: `*State` - but this is for named fields like `*stream.Config`
                    // For embedded pointers like `*State`, the structure is:
                    // field_declaration -> * -> type_identifier
                    // NOT field_declaration -> pointer_type -> ...
                    // So this case handles `*pkg.Type` which is a named field
                }
                "qualified_type" => {
                    // Qualified embedded field: `pkg.Type` - use the type part
                    if let Some(type_node) = find_child_by_kind(&child, "type_identifier") {
                        if let Ok(name) = type_node.utf8_text(source) {
                            return Some((name.to_string(), type_node));
                        }
                    }
                }
                _ => {}
            }
        }
    }

    // Check for `*Type` pattern where `*` is a direct child followed by type_identifier
    // This is the embedded pointer pattern
    let mut found_star = false;
    for i in 0..field.child_count() {
        if let Some(child) = field.child(i) {
            if child.kind() == "*" {
                found_star = true;
            } else if found_star && child.kind() == "type_identifier" {
                if let Ok(name) = child.utf8_text(source) {
                    return Some((name.to_string(), child));
                }
            }
        }
    }

    None
}

/// Extract interface method signatures
fn extract_interface_method(
    method_elem: &tree_sitter::Node,
    source: &[u8],
    file: &Path,
    result: &mut ParseResult,
    parent_qualified: &str,
) {
    if let Some(name_node) = method_elem.child_by_field_name("name") {
        if let Ok(name) = name_node.utf8_text(source) {
            let qualified = format!("{}.{}", parent_qualified, name);
            let visibility = extract_visibility(name);

            // Build signature
            let mut sig = format!("func {}", name);
            if let Some(params) = method_elem.child_by_field_name("parameters") {
                if let Ok(params_text) = params.utf8_text(source) {
                    sig.push_str(params_text);
                }
            }
            if let Some(ret) = method_elem.child_by_field_name("result") {
                if let Ok(ret_text) = ret.utf8_text(source) {
                    sig.push(' ');
                    sig.push_str(ret_text);
                }
            }

            result.symbols.push(Symbol {
                name: name.to_string(),
                qualified,
                kind: SymbolKind::Function,
                location: node_to_location(file, &name_node),
                visibility,
                language: "go".to_string(),
                parent: Some(parent_qualified.to_string()),
                mixins: None,
                attributes: None,
                implements: None,
                doc: None,
                signature: Some(sig),
            });
        }
    }
}

/// Extract const or var specifications
fn extract_const_or_var_spec(
    node: &tree_sitter::Node,
    source: &[u8],
    file: &Path,
    result: &mut ParseResult,
    package: Option<&str>,
    kind: SymbolKind,
) {
    // const/var spec can have multiple names: x, y = 1, 2
    if let Some(name_node) = node.child_by_field_name("name") {
        if let Ok(name) = name_node.utf8_text(source) {
            let qualified = qualified_name(name, package);
            let visibility = extract_visibility(name);
            let doc = extract_doc_comments(node, source);

            result.symbols.push(Symbol {
                name: name.to_string(),
                qualified,
                kind,
                location: node_to_location(file, &name_node),
                visibility,
                language: "go".to_string(),
                parent: None,
                mixins: None,
                attributes: None,
                implements: None,
                doc,
                signature: None,
            });
        }
    }
}

/// Check if a node is a descendant of a node with the given kind
fn is_descendant_of(node: &tree_sitter::Node, kind: &str) -> bool {
    let mut current = node.parent();
    while let Some(parent) = current {
        if parent.kind() == kind {
            return true;
        }
        current = parent.parent();
    }
    false
}

/// Determine if an identifier/type_identifier node is in a reference context (not a definition)
fn is_reference_context(node: &tree_sitter::Node) -> bool {
    let parent = match node.parent() {
        Some(p) => p,
        None => return false,
    };

    let parent_kind = parent.kind();

    // Definition contexts (NOT references)
    // Function/method declarations - the name field
    if parent_kind == "function_declaration" || parent_kind == "method_declaration" {
        if let Some(name_node) = parent.child_by_field_name("name") {
            if name_node.id() == node.id() {
                return false;
            }
        }
    }

    // Type spec definitions
    if parent_kind == "type_spec" {
        if let Some(name_node) = parent.child_by_field_name("name") {
            if name_node.id() == node.id() {
                return false;
            }
        }
    }

    // Const/var spec definitions - the name field
    if parent_kind == "const_spec" || parent_kind == "var_spec" {
        if let Some(name_node) = parent.child_by_field_name("name") {
            if name_node.id() == node.id() {
                return false;
            }
        }
    }

    // Field declarations (struct field names)
    if parent_kind == "field_declaration" {
        if let Some(name_node) = parent.child_by_field_name("name") {
            if name_node.id() == node.id() {
                return false;
            }
        }
    }

    // Method spec definitions (interface methods)
    if parent_kind == "method_spec" || parent_kind == "method_elem" {
        if let Some(name_node) = parent.child_by_field_name("name") {
            if name_node.id() == node.id() {
                return false;
            }
        }
    }

    // Parameter names
    if parent_kind == "parameter_declaration" {
        if let Some(name_node) = parent.child_by_field_name("name") {
            if name_node.id() == node.id() {
                return false;
            }
        }
    }

    // Short variable declarations (left side) - x := expr
    if (parent_kind == "short_var_declaration" || parent_kind == "expression_list")
        && is_descendant_of(node, "short_var_declaration")
    {
        let mut p = node.parent();
        while let Some(parent) = p {
            if parent.kind() == "short_var_declaration" {
                // Check if we're on the left side (before :=)
                if let Some(left) = parent.child_by_field_name("left") {
                    if node.start_byte() >= left.start_byte() && node.end_byte() <= left.end_byte()
                    {
                        return false;
                    }
                }
                break;
            }
            p = parent.parent();
        }
    }

    // Import specs (package identifier)
    if parent_kind == "import_spec" || parent_kind == "package_clause" {
        return false;
    }

    // Package identifiers in package clause
    if parent_kind == "package_identifier" || node.kind() == "package_identifier" {
        return false;
    }

    // Skip identifiers that are keywords or labels
    if parent_kind == "label_name" || parent_kind == "labeled_statement" {
        return false;
    }

    // Range clause variable declarations
    if parent_kind == "range_clause" {
        if let Some(left) = parent.child_by_field_name("left") {
            if node.start_byte() >= left.start_byte() && node.end_byte() <= left.end_byte() {
                return false;
            }
        }
    }

    // For clause (for i := 0; ...)
    if parent_kind == "for_clause" {
        return true; // Let short_var_declaration handle definitions
    }

    // Receiver parameter declarations
    if is_descendant_of(node, "parameter_list") {
        // Could be a receiver, which is a definition
        if is_descendant_of(node, "method_declaration") {
            // Check if we're in the receiver field
            let mut p = node.parent();
            while let Some(parent) = p {
                if parent.kind() == "method_declaration" {
                    if let Some(recv) = parent.child_by_field_name("receiver") {
                        if node.start_byte() >= recv.start_byte()
                            && node.end_byte() <= recv.end_byte()
                        {
                            // Inside receiver - parameter names are definitions
                            if let Some(pp) = node.parent() {
                                if pp.kind() == "parameter_declaration" {
                                    if let Some(name_node) = pp.child_by_field_name("name") {
                                        if name_node.id() == node.id() {
                                            return false;
                                        }
                                    }
                                }
                            }
                        }
                    }
                    break;
                }
                p = parent.parent();
            }
        }
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::LanguageParser;

    #[test]
    fn extracts_go_function() {
        let source = r#"
package main

func HelloWorld() string {
    return "Hello, World!"
}
"#;
        let parser = GoParser;
        let result = parser.extract_symbols(Path::new("test.go"), source, 100);

        assert_eq!(result.symbols.len(), 1);
        assert_eq!(result.symbols[0].name, "HelloWorld");
        assert_eq!(result.symbols[0].qualified, "main.HelloWorld");
        assert_eq!(result.symbols[0].kind, SymbolKind::Function);
        assert_eq!(result.symbols[0].visibility, Visibility::Public);
    }

    #[test]
    fn extracts_unexported_function() {
        let source = r#"
package utils

func helperFunc() {
}
"#;
        let parser = GoParser;
        let result = parser.extract_symbols(Path::new("test.go"), source, 100);

        assert_eq!(result.symbols.len(), 1);
        assert_eq!(result.symbols[0].name, "helperFunc");
        assert_eq!(result.symbols[0].visibility, Visibility::Private);
    }

    #[test]
    fn extracts_go_struct() {
        let source = r#"
package models

type User struct {
    ID   int
    Name string
}
"#;
        let parser = GoParser;
        let result = parser.extract_symbols(Path::new("test.go"), source, 100);

        // Should have User struct + 2 fields
        assert!(!result.symbols.is_empty());
        let user = result.symbols.iter().find(|s| s.name == "User").unwrap();
        assert_eq!(user.qualified, "models.User");
        assert_eq!(user.kind, SymbolKind::Class);
        assert_eq!(user.visibility, Visibility::Public);
    }

    #[test]
    fn extracts_go_interface() {
        let source = r#"
package io

type Reader interface {
    Read(p []byte) (n int, err error)
}
"#;
        let parser = GoParser;
        let result = parser.extract_symbols(Path::new("test.go"), source, 100);

        let reader = result.symbols.iter().find(|s| s.name == "Reader").unwrap();
        assert_eq!(reader.qualified, "io.Reader");
        assert_eq!(reader.kind, SymbolKind::Interface);

        // Should also have the Read method
        let read = result.symbols.iter().find(|s| s.name == "Read");
        assert!(read.is_some());
    }

    #[test]
    fn extracts_go_method() {
        let source = r#"
package models

type User struct {
    Name string
}

func (u *User) GetName() string {
    return u.Name
}

func (u User) String() string {
    return u.Name
}
"#;
        let parser = GoParser;
        let result = parser.extract_symbols(Path::new("test.go"), source, 100);

        let get_name = result.symbols.iter().find(|s| s.name == "GetName").unwrap();
        assert_eq!(get_name.qualified, "models.User.GetName");
        assert_eq!(get_name.kind, SymbolKind::Function);
        assert_eq!(get_name.parent, Some("User".to_string()));

        let string_method = result.symbols.iter().find(|s| s.name == "String").unwrap();
        assert_eq!(string_method.qualified, "models.User.String");
    }

    #[test]
    fn extracts_go_const() {
        let source = r#"
package constants

const MaxSize = 100
const minSize = 10
"#;
        let parser = GoParser;
        let result = parser.extract_symbols(Path::new("test.go"), source, 100);

        let max_size = result.symbols.iter().find(|s| s.name == "MaxSize").unwrap();
        assert_eq!(max_size.kind, SymbolKind::Value);
        assert_eq!(max_size.visibility, Visibility::Public);

        let min_size = result.symbols.iter().find(|s| s.name == "minSize").unwrap();
        assert_eq!(min_size.visibility, Visibility::Private);
    }

    #[test]
    fn extracts_go_var() {
        let source = r#"
package globals

var DefaultTimeout = 30
"#;
        let parser = GoParser;
        let result = parser.extract_symbols(Path::new("test.go"), source, 100);

        let timeout = result
            .symbols
            .iter()
            .find(|s| s.name == "DefaultTimeout")
            .unwrap();
        assert_eq!(timeout.kind, SymbolKind::Value);
        assert_eq!(timeout.visibility, Visibility::Public);
    }

    #[test]
    fn extracts_go_imports() {
        let source = r#"
package main

import (
    "fmt"
    "os"
    "encoding/json"
)

func main() {}
"#;
        let parser = GoParser;
        let result = parser.extract_symbols(Path::new("test.go"), source, 100);

        assert!(result.opens.contains(&"fmt".to_string()));
        assert!(result.opens.contains(&"os".to_string()));
        assert!(result.opens.contains(&"encoding/json".to_string()));
    }

    #[test]
    fn extracts_type_alias() {
        let source = r#"
package types

type ID int64
type StringList []string
"#;
        let parser = GoParser;
        let result = parser.extract_symbols(Path::new("test.go"), source, 100);

        let id = result.symbols.iter().find(|s| s.name == "ID").unwrap();
        assert_eq!(id.kind, SymbolKind::Type);

        let string_list = result
            .symbols
            .iter()
            .find(|s| s.name == "StringList")
            .unwrap();
        assert_eq!(string_list.kind, SymbolKind::Type);
    }

    #[test]
    fn extracts_package_name() {
        let source = r#"
package mypackage

func Foo() {}
"#;
        let parser = GoParser;
        let result = parser.extract_symbols(Path::new("test.go"), source, 100);

        assert_eq!(result.module_path, Some("mypackage".to_string()));
    }

    #[test]
    fn handles_grouped_const_declaration() {
        let source = r#"
package status

const (
    StatusOK = 200
    StatusNotFound = 404
)
"#;
        let parser = GoParser;
        let result = parser.extract_symbols(Path::new("test.go"), source, 100);

        assert!(result.symbols.iter().any(|s| s.name == "StatusOK"));
        assert!(result.symbols.iter().any(|s| s.name == "StatusNotFound"));
    }

    #[test]
    fn extracts_interface_embedding() {
        let source = r#"
package test

type Reader interface {
    Read(p []byte) (n int, err error)
}

type Writer interface {
    Write(p []byte) (n int, err error)
}

type ReadWriter interface {
    Reader
    Writer
}
"#;
        let parser = GoParser;
        let result = parser.extract_symbols(Path::new("test.go"), source, 100);

        // Should find Reader interface with Read method
        let reader = result
            .symbols
            .iter()
            .find(|s| s.name == "Reader" && s.kind == SymbolKind::Interface)
            .expect("Should find Reader interface");
        assert_eq!(reader.qualified, "test.Reader");

        let read_method = result
            .symbols
            .iter()
            .find(|s| s.name == "Read" && s.qualified == "test.Reader.Read")
            .expect("Should find Read method");
        assert_eq!(read_method.kind, SymbolKind::Function);

        // Should find Writer interface with Write method
        let writer = result
            .symbols
            .iter()
            .find(|s| s.name == "Writer" && s.kind == SymbolKind::Interface)
            .expect("Should find Writer interface");
        assert_eq!(writer.qualified, "test.Writer");

        // Should find ReadWriter interface with embedded interfaces tracked in mixins
        let read_writer = result
            .symbols
            .iter()
            .find(|s| s.name == "ReadWriter" && s.kind == SymbolKind::Interface)
            .expect("Should find ReadWriter interface");
        assert_eq!(read_writer.qualified, "test.ReadWriter");

        // Interface embedding: Reader and Writer should be recorded in mixins
        let mixins = read_writer
            .mixins
            .as_ref()
            .expect("ReadWriter should have mixins");
        assert!(
            mixins.contains(&"Reader".to_string()),
            "Should embed Reader"
        );
        assert!(
            mixins.contains(&"Writer".to_string()),
            "Should embed Writer"
        );
    }

    #[test]
    fn extracts_qualified_interface_embedding() {
        let source = r#"
package gin

type ResponseWriter interface {
    http.ResponseWriter
    http.Hijacker
    http.Flusher
    Status() int
    Size() int
}
"#;
        let parser = GoParser;
        let result = parser.extract_symbols(Path::new("test.go"), source, 100);

        // Should find ResponseWriter interface
        let rw = result
            .symbols
            .iter()
            .find(|s| s.name == "ResponseWriter" && s.kind == SymbolKind::Interface)
            .expect("Should find ResponseWriter interface");
        assert_eq!(rw.qualified, "gin.ResponseWriter");

        // Should have embedded interfaces (qualified types)
        let mixins = rw.mixins.as_ref().expect("Should have mixins");
        assert!(
            mixins.contains(&"http.ResponseWriter".to_string()),
            "Should embed http.ResponseWriter, got: {:?}",
            mixins
        );
        assert!(
            mixins.contains(&"http.Hijacker".to_string()),
            "Should embed http.Hijacker"
        );
        assert!(
            mixins.contains(&"http.Flusher".to_string()),
            "Should embed http.Flusher"
        );

        // Should also have the declared methods
        assert!(result
            .symbols
            .iter()
            .any(|s| s.name == "Status" && s.qualified == "gin.ResponseWriter.Status"));
        assert!(result
            .symbols
            .iter()
            .any(|s| s.name == "Size" && s.qualified == "gin.ResponseWriter.Size"));
    }

    #[test]
    fn extracts_interface_with_multiple_methods() {
        let source = r#"
package http

type Handler interface {
    ServeHTTP(w ResponseWriter, r *Request)
}

type ResponseWriter interface {
    Header() Header
    Write([]byte) (int, error)
    WriteHeader(statusCode int)
}
"#;
        let parser = GoParser;
        let result = parser.extract_symbols(Path::new("test.go"), source, 100);

        // Handler interface
        let handler = result
            .symbols
            .iter()
            .find(|s| s.name == "Handler")
            .expect("Should find Handler");
        assert_eq!(handler.kind, SymbolKind::Interface);

        // ResponseWriter interface with 3 methods
        let rw = result
            .symbols
            .iter()
            .find(|s| s.name == "ResponseWriter")
            .expect("Should find ResponseWriter");
        assert_eq!(rw.kind, SymbolKind::Interface);

        // Check methods are extracted
        assert!(result
            .symbols
            .iter()
            .any(|s| s.name == "Header" && s.qualified == "http.ResponseWriter.Header"));
        assert!(result
            .symbols
            .iter()
            .any(|s| s.name == "Write" && s.qualified == "http.ResponseWriter.Write"));
        assert!(result
            .symbols
            .iter()
            .any(|s| s.name == "WriteHeader" && s.qualified == "http.ResponseWriter.WriteHeader"));
    }

    #[test]
    fn extracts_variadic_function() {
        let source = r#"
package fmt

func Printf(format string, a ...interface{}) (n int, err error) {
    return
}
"#;
        let parser = GoParser;
        let result = parser.extract_symbols(Path::new("test.go"), source, 100);

        let printf = result
            .symbols
            .iter()
            .find(|s| s.name == "Printf")
            .expect("Should find Printf");
        assert_eq!(printf.kind, SymbolKind::Function);
        assert!(printf.signature.as_ref().unwrap().contains("..."));
    }

    #[test]
    fn extracts_generic_type() {
        let source = r#"
package collections

type List[T any] struct {
    items []T
}

func (l *List[T]) Add(item T) {
    l.items = append(l.items, item)
}
"#;
        let parser = GoParser;
        let result = parser.extract_symbols(Path::new("test.go"), source, 100);

        // Generic struct
        let list = result
            .symbols
            .iter()
            .find(|s| s.name == "List")
            .expect("Should find List");
        assert_eq!(list.kind, SymbolKind::Class);

        // Method on generic type
        let add = result
            .symbols
            .iter()
            .find(|s| s.name == "Add")
            .expect("Should find Add method");
        assert_eq!(add.kind, SymbolKind::Function);
    }

    #[test]
    fn extracts_iota_constants() {
        let source = r#"
package status

const (
    Pending = iota
    Running
    Completed
    Failed
)
"#;
        let parser = GoParser;
        let result = parser.extract_symbols(Path::new("test.go"), source, 100);

        // Should find all constants
        assert!(result.symbols.iter().any(|s| s.name == "Pending"));
        assert!(result.symbols.iter().any(|s| s.name == "Running"));
        assert!(result.symbols.iter().any(|s| s.name == "Completed"));
        assert!(result.symbols.iter().any(|s| s.name == "Failed"));
    }

    #[test]
    fn extracts_import_alias() {
        let source = r#"
package main

import (
    "fmt"
    json "encoding/json"
    . "strings"
    _ "net/http/pprof"
)

func main() {}
"#;
        let parser = GoParser;
        let result = parser.extract_symbols(Path::new("test.go"), source, 100);

        // All imports should be captured
        assert!(result.opens.contains(&"fmt".to_string()));
        assert!(result.opens.contains(&"encoding/json".to_string()));
        assert!(result.opens.contains(&"strings".to_string()));
        assert!(result.opens.contains(&"net/http/pprof".to_string()));
    }

    #[test]
    fn extracts_anonymous_struct_field() {
        let source = r#"
package config

type Config struct {
    Server struct {
        Host string
        Port int
    }
    Database struct {
        URL string
    }
}
"#;
        let parser = GoParser;
        let result = parser.extract_symbols(Path::new("test.go"), source, 100);

        // Should find Config struct
        let config = result
            .symbols
            .iter()
            .find(|s| s.name == "Config")
            .expect("Should find Config");
        assert_eq!(config.kind, SymbolKind::Class);

        // Named fields with anonymous struct types should be captured
        assert!(result
            .symbols
            .iter()
            .any(|s| s.name == "Server" && s.qualified == "config.Config.Server"));
        assert!(result
            .symbols
            .iter()
            .any(|s| s.name == "Database" && s.qualified == "config.Config.Database"));
    }

    #[test]
    fn extracts_pointer_receiver_method() {
        let source = r#"
package user

type User struct {
    name string
}

func (u *User) SetName(name string) {
    u.name = name
}

func (u User) GetName() string {
    return u.name
}
"#;
        let parser = GoParser;
        let result = parser.extract_symbols(Path::new("test.go"), source, 100);

        // Both pointer and value receiver methods should be found
        let set_name = result
            .symbols
            .iter()
            .find(|s| s.name == "SetName")
            .expect("Should find SetName");
        assert_eq!(set_name.qualified, "user.User.SetName");
        assert_eq!(set_name.parent, Some("User".to_string()));

        let get_name = result
            .symbols
            .iter()
            .find(|s| s.name == "GetName")
            .expect("Should find GetName");
        assert_eq!(get_name.qualified, "user.User.GetName");
        assert_eq!(get_name.parent, Some("User".to_string()));
    }

    #[test]
    fn extracts_embedded_struct_fields() {
        let source = r#"
package container

type Container struct {
    StreamConfig *stream.Config
    *State
    Root         string
    SecurityOptions
}
"#;
        let parser = GoParser;
        let result = parser.extract_symbols(Path::new("test.go"), source, 100);

        // Container struct should be found
        let container = result
            .symbols
            .iter()
            .find(|s| s.name == "Container")
            .expect("Should find Container");
        assert_eq!(container.kind, SymbolKind::Class);

        // Named fields should be indexed
        let stream_config = result
            .symbols
            .iter()
            .find(|s| s.name == "StreamConfig" && s.qualified == "container.Container.StreamConfig")
            .expect("Should find StreamConfig");
        assert_eq!(stream_config.kind, SymbolKind::Member);

        let root = result
            .symbols
            .iter()
            .find(|s| s.name == "Root" && s.qualified == "container.Container.Root")
            .expect("Should find Root");
        assert_eq!(root.kind, SymbolKind::Member);

        // Embedded pointer field: *State should be indexed as Container.State
        let state = result
            .symbols
            .iter()
            .find(|s| s.name == "State" && s.qualified == "container.Container.State")
            .expect("Should find embedded State");
        assert_eq!(state.kind, SymbolKind::Member);
        assert_eq!(state.parent, Some("container.Container".to_string()));

        // Embedded value field: SecurityOptions should be indexed
        let security = result
            .symbols
            .iter()
            .find(|s| {
                s.name == "SecurityOptions" && s.qualified == "container.Container.SecurityOptions"
            })
            .expect("Should find embedded SecurityOptions");
        assert_eq!(security.kind, SymbolKind::Member);
    }

    #[test]
    fn extracts_go_references() {
        let source = r#"
package main

import "fmt"

type User struct {
    Name string
}

func (u *User) Greet() string {
    return fmt.Sprintf("Hello, %s", u.Name)
}

func main() {
    user := &User{Name: "Alice"}
    message := user.Greet()
    fmt.Println(message)
}
"#;
        let parser = GoParser;
        let result = parser.extract_symbols(Path::new("test.go"), source, 100);

        assert!(
            !result.references.is_empty(),
            "Should extract references from Go code"
        );

        let ref_names: Vec<_> = result.references.iter().map(|r| r.name.as_str()).collect();

        // Should have references to User (in main)
        assert!(
            ref_names.contains(&"User"),
            "Should have reference to User: {:?}",
            ref_names
        );

        // Should have references to fmt.Println/Sprintf
        assert!(
            ref_names
                .iter()
                .any(|n| *n == "fmt" || n.contains("Println") || n.contains("Sprintf")),
            "Should have reference to fmt functions: {:?}",
            ref_names
        );
    }
}
