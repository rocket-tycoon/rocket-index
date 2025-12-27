//! Symbol extraction from C source files using tree-sitter.

use std::cell::RefCell;
use std::path::Path;

use crate::parse::{find_child_by_kind, node_to_location, LanguageParser, ParseResult};
use crate::{Reference, Symbol, SymbolKind, Visibility};

// Thread-local parser reuse - avoids creating a new parser per file
thread_local! {
    static C_PARSER: RefCell<tree_sitter::Parser> = RefCell::new({
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_c::LANGUAGE.into())
            .expect("tree-sitter-c grammar incompatible with tree-sitter version");
        parser
    });
}

pub struct CParser;

impl LanguageParser for CParser {
    fn extract_symbols(&self, file: &Path, source: &str, max_depth: usize) -> ParseResult {
        C_PARSER.with(|parser| {
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

            extract_recursive(&root, source.as_bytes(), file, &mut result, None, max_depth);

            // Extract references in a separate pass
            extract_references_recursive(&root, source.as_bytes(), file, &mut result);

            result
        })
    }
}

/// Build qualified name with :: separator (for nested types)
fn qualified_name(name: &str, parent_path: Option<&str>) -> String {
    match parent_path {
        Some(p) => format!("{}::{}", p, name),
        None => name.to_string(),
    }
}

/// Extract doc comments from preceding comment nodes
fn extract_doc_comments(node: &tree_sitter::Node, source: &[u8]) -> Option<String> {
    let mut docs = Vec::new();

    // Look for preceding comment nodes (siblings before this node)
    let mut prev_sibling = node.prev_sibling();
    while let Some(sib) = prev_sibling {
        if sib.kind() == "comment" {
            if let Ok(text) = sib.utf8_text(source) {
                let doc = text
                    .trim_start_matches("//")
                    .trim_start_matches("/*")
                    .trim_end_matches("*/")
                    .trim();
                if !doc.is_empty() {
                    docs.push(doc.to_string());
                }
            }
            prev_sibling = sib.prev_sibling();
        } else {
            break;
        }
    }

    // Reverse to maintain original order (we collected them backwards)
    docs.reverse();

    if docs.is_empty() {
        None
    } else {
        Some(docs.join("\n"))
    }
}

/// Extract function signature from function definition
fn extract_function_signature(node: &tree_sitter::Node, source: &[u8]) -> Option<String> {
    // Get the text up to the compound_statement (function body)
    let start = node.start_byte();
    if let Some(body) = find_child_by_kind(node, "compound_statement") {
        let end = body.start_byte();
        if end > start {
            if let Ok(sig) = std::str::from_utf8(&source[start..end]) {
                return Some(sig.trim().to_string());
            }
        }
    }
    // For declarations without body
    if let Ok(text) = node.utf8_text(source) {
        let text = text.trim();
        if text.ends_with(';') {
            return Some(text.trim_end_matches(';').trim().to_string());
        }
    }
    None
}

/// Check if a declaration/definition has 'static' storage class (internal linkage)
fn has_static_storage_class(node: &tree_sitter::Node, source: &[u8]) -> bool {
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            if child.kind() == "storage_class_specifier" {
                if let Ok(text) = child.utf8_text(source) {
                    if text == "static" {
                        return true;
                    }
                }
            }
        }
    }
    false
}

fn extract_recursive(
    node: &tree_sitter::Node,
    source: &[u8],
    file: &Path,
    result: &mut ParseResult,
    parent_path: Option<&str>,
    max_depth: usize,
) {
    if max_depth == 0 {
        return;
    }

    match node.kind() {
        "function_definition" => {
            // Extract function name from declarator
            if let Some(declarator) = node.child_by_field_name("declarator") {
                if let Some(name) = extract_declarator_name(&declarator, source) {
                    let qualified = qualified_name(&name, parent_path);
                    let doc = extract_doc_comments(node, source);
                    let signature = extract_function_signature(node, source);
                    // static = internal linkage (file-private)
                    let visibility = if has_static_storage_class(node, source) {
                        Visibility::Private
                    } else {
                        Visibility::Public
                    };

                    result.symbols.push(Symbol {
                        name: name.clone(),
                        qualified,
                        kind: SymbolKind::Function,
                        location: node_to_location(file, &declarator),
                        visibility,
                        language: "c".to_string(),
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

        "type_definition" => {
            // typedef ... name;
            // The typedef name can be in various forms:
            // - type_declarator or any *_declarator
            // - type_identifier (custom type name)
            // - primitive_type (tree-sitter-c treats some common typedefs as primitives)
            let mut found = false;
            for i in 0..node.child_count() {
                if let Some(child) = node.child(i) {
                    if child.kind() == "type_declarator" || child.kind().ends_with("_declarator") {
                        if let Some(name) = extract_declarator_name(&child, source) {
                            let qualified = qualified_name(&name, parent_path);
                            let doc = extract_doc_comments(node, source);

                            result.symbols.push(Symbol {
                                name,
                                qualified,
                                kind: SymbolKind::Type,
                                location: node_to_location(file, &child),
                                visibility: Visibility::Public,
                                language: "c".to_string(),
                                parent: None,
                                mixins: None,
                                attributes: None,
                                implements: None,
                                doc,
                                signature: None,
                            });
                            found = true;
                            break;
                        }
                    }
                }
            }
            // Fallback: look for type_identifier or primitive_type directly
            // In tree-sitter-c, primitive types like uint32_t are treated as primitive_type
            if !found {
                for i in 0..node.child_count() {
                    if let Some(child) = node.child(i) {
                        if child.kind() == "type_identifier" || child.kind() == "primitive_type" {
                            if let Ok(name) = child.utf8_text(source) {
                                let qualified = qualified_name(name, parent_path);
                                let doc = extract_doc_comments(node, source);

                                result.symbols.push(Symbol {
                                    name: name.to_string(),
                                    qualified,
                                    kind: SymbolKind::Type,
                                    location: node_to_location(file, &child),
                                    visibility: Visibility::Public,
                                    language: "c".to_string(),
                                    parent: None,
                                    mixins: None,
                                    attributes: None,
                                    implements: None,
                                    doc,
                                    signature: None,
                                });
                                found = true;
                                break;
                            }
                        }
                    }
                }
            }
            if found {
                return; // Don't recurse for typedef
            }
        }

        "declaration" => {
            // Could be a function declaration, variable, typedef, or has struct/enum specifier
            // Check for typedef first (typedef is a storage_class_specifier in tree-sitter-c)
            let mut is_typedef = false;
            for i in 0..node.child_count() {
                if let Some(child) = node.child(i) {
                    if child.kind() == "storage_class_specifier" {
                        if let Ok(text) = child.utf8_text(source) {
                            if text == "typedef" {
                                is_typedef = true;
                                break;
                            }
                        }
                    }
                }
            }

            if is_typedef {
                // Extract typedef name from declarator - try both field access and child search
                let mut typedef_found = false;
                if let Some(declarator) = node.child_by_field_name("declarator") {
                    if let Some(name) = extract_declarator_name(&declarator, source) {
                        let qualified = qualified_name(&name, parent_path);
                        let doc = extract_doc_comments(node, source);

                        result.symbols.push(Symbol {
                            name,
                            qualified,
                            kind: SymbolKind::Type,
                            location: node_to_location(file, &declarator),
                            visibility: Visibility::Public,
                            language: "c".to_string(),
                            parent: None,
                            mixins: None,
                            attributes: None,
                            implements: None,
                            doc,
                            signature: None,
                        });
                        typedef_found = true;
                    }
                }
                // Fallback: look for type_identifier directly in children
                if !typedef_found {
                    for i in 0..node.child_count() {
                        if let Some(child) = node.child(i) {
                            if child.kind() == "type_identifier" {
                                if let Ok(name) = child.utf8_text(source) {
                                    let qualified = qualified_name(name, parent_path);
                                    let doc = extract_doc_comments(node, source);

                                    result.symbols.push(Symbol {
                                        name: name.to_string(),
                                        qualified,
                                        kind: SymbolKind::Type,
                                        location: node_to_location(file, &child),
                                        visibility: Visibility::Public,
                                        language: "c".to_string(),
                                        parent: None,
                                        mixins: None,
                                        attributes: None,
                                        implements: None,
                                        doc,
                                        signature: None,
                                    });
                                    break;
                                }
                            }
                        }
                    }
                }
                return; // Don't process further for typedef
            }

            // Regular declaration: function declaration or variable
            let mut found_declarator = false;
            // static = internal linkage (file-private)
            let visibility = if has_static_storage_class(node, source) {
                Visibility::Private
            } else {
                Visibility::Public
            };

            for i in 0..node.child_count() {
                if let Some(child) = node.child(i) {
                    if child.kind() == "init_declarator" {
                        // Variable with initializer: int x = 0;
                        if let Some(declarator) = child.child_by_field_name("declarator") {
                            if let Some(name) = extract_declarator_name(&declarator, source) {
                                found_declarator = true;
                                let qualified = qualified_name(&name, parent_path);
                                let doc = extract_doc_comments(node, source);
                                let kind = if is_function_declarator(&declarator) {
                                    SymbolKind::Function
                                } else {
                                    SymbolKind::Value
                                };

                                result.symbols.push(Symbol {
                                    name,
                                    qualified,
                                    kind,
                                    location: node_to_location(file, &declarator),
                                    visibility,
                                    language: "c".to_string(),
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
                }
            }

            // If no init_declarator, try direct declarator field
            if !found_declarator {
                if let Some(declarator) = node.child_by_field_name("declarator") {
                    if let Some(name) = extract_declarator_name(&declarator, source) {
                        let qualified = qualified_name(&name, parent_path);
                        let doc = extract_doc_comments(node, source);
                        let kind = if is_function_declarator(&declarator) {
                            SymbolKind::Function
                        } else {
                            SymbolKind::Value
                        };

                        result.symbols.push(Symbol {
                            name,
                            qualified,
                            kind,
                            location: node_to_location(file, &declarator),
                            visibility,
                            language: "c".to_string(),
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
        }

        "struct_specifier" => {
            // struct Name { ... }
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Ok(name) = name_node.utf8_text(source) {
                    let qualified = qualified_name(name, parent_path);
                    let doc = extract_doc_comments(node, source);

                    result.symbols.push(Symbol {
                        name: name.to_string(),
                        qualified: qualified.clone(),
                        kind: SymbolKind::Class,
                        location: node_to_location(file, &name_node),
                        visibility: Visibility::Public,
                        language: "c".to_string(),
                        parent: None,
                        mixins: None,
                        attributes: None,
                        implements: None,
                        doc,
                        signature: None,
                    });

                    // Extract struct fields - tree-sitter-c uses field_declaration_list
                    // Try both 'body' and find field_declaration_list child
                    if let Some(body) = node.child_by_field_name("body") {
                        extract_struct_fields(&body, source, file, result, &qualified);
                    } else {
                        // Look for field_declaration_list directly as child
                        for i in 0..node.child_count() {
                            if let Some(child) = node.child(i) {
                                if child.kind() == "field_declaration_list" {
                                    extract_struct_fields(&child, source, file, result, &qualified);
                                    break;
                                }
                            }
                        }
                    }
                }
            }
        }

        "union_specifier" => {
            // union Name { ... }
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Ok(name) = name_node.utf8_text(source) {
                    let qualified = qualified_name(name, parent_path);
                    let doc = extract_doc_comments(node, source);

                    result.symbols.push(Symbol {
                        name: name.to_string(),
                        qualified: qualified.clone(),
                        kind: SymbolKind::Union,
                        location: node_to_location(file, &name_node),
                        visibility: Visibility::Public,
                        language: "c".to_string(),
                        parent: None,
                        mixins: None,
                        attributes: None,
                        implements: None,
                        doc,
                        signature: None,
                    });

                    // Extract union fields (same structure as struct)
                    if let Some(body) = node.child_by_field_name("body") {
                        extract_struct_fields(&body, source, file, result, &qualified);
                    } else {
                        for i in 0..node.child_count() {
                            if let Some(child) = node.child(i) {
                                if child.kind() == "field_declaration_list" {
                                    extract_struct_fields(&child, source, file, result, &qualified);
                                    break;
                                }
                            }
                        }
                    }
                }
            }
        }

        "enum_specifier" => {
            // enum Name { ... }
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Ok(name) = name_node.utf8_text(source) {
                    let qualified = qualified_name(name, parent_path);
                    let doc = extract_doc_comments(node, source);

                    result.symbols.push(Symbol {
                        name: name.to_string(),
                        qualified: qualified.clone(),
                        kind: SymbolKind::Union,
                        location: node_to_location(file, &name_node),
                        visibility: Visibility::Public,
                        language: "c".to_string(),
                        parent: None,
                        mixins: None,
                        attributes: None,
                        implements: None,
                        doc,
                        signature: None,
                    });

                    // Extract enum values
                    if let Some(body) = node.child_by_field_name("body") {
                        extract_enum_values(&body, source, file, result, &qualified);
                    }
                }
            }
        }

        "preproc_def" => {
            // #define MACRO value
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Ok(name) = name_node.utf8_text(source) {
                    let qualified = qualified_name(name, parent_path);

                    result.symbols.push(Symbol {
                        name: name.to_string(),
                        qualified,
                        kind: SymbolKind::Value,
                        location: node_to_location(file, &name_node),
                        visibility: Visibility::Public,
                        language: "c".to_string(),
                        parent: None,
                        mixins: None,
                        attributes: None,
                        implements: None,
                        doc: None,
                        signature: None,
                    });
                }
            }
        }

        "preproc_function_def" => {
            // #define MACRO(x) ...
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Ok(name) = name_node.utf8_text(source) {
                    let qualified = qualified_name(name, parent_path);

                    result.symbols.push(Symbol {
                        name: name.to_string(),
                        qualified,
                        kind: SymbolKind::Function,
                        location: node_to_location(file, &name_node),
                        visibility: Visibility::Public,
                        language: "c".to_string(),
                        parent: None,
                        mixins: None,
                        attributes: None,
                        implements: None,
                        doc: None,
                        signature: None,
                    });
                }
            }
        }

        "preproc_include" => {
            // #include <header.h> or #include "header.h"
            if let Some(path_node) = node.child_by_field_name("path") {
                if let Ok(path) = path_node.utf8_text(source) {
                    // Remove quotes or angle brackets
                    let clean_path = path
                        .trim_start_matches('<')
                        .trim_end_matches('>')
                        .trim_matches('"')
                        .to_string();
                    result.opens.push(clean_path);
                }
            }
        }

        _ => {}
    }

    // Recurse into children
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            extract_recursive(&child, source, file, result, parent_path, max_depth - 1);
        }
    }
}

/// Extract the name from a declarator (handles nested declarators like function pointers)
fn extract_declarator_name(node: &tree_sitter::Node, source: &[u8]) -> Option<String> {
    match node.kind() {
        "identifier" | "type_identifier" => node.utf8_text(source).ok().map(|s| s.to_string()),
        "function_declarator" => {
            // Get the declarator field
            node.child_by_field_name("declarator")
                .and_then(|d| extract_declarator_name(&d, source))
        }
        "pointer_declarator" => {
            // Get the declarator field
            node.child_by_field_name("declarator")
                .and_then(|d| extract_declarator_name(&d, source))
        }
        "array_declarator" => {
            // Get the declarator field
            node.child_by_field_name("declarator")
                .and_then(|d| extract_declarator_name(&d, source))
        }
        "parenthesized_declarator" => {
            // Find the child declarator
            for i in 0..node.child_count() {
                if let Some(child) = node.child(i) {
                    if let Some(name) = extract_declarator_name(&child, source) {
                        return Some(name);
                    }
                }
            }
            None
        }
        _ => None,
    }
}

/// Check if a declarator is a function declarator
fn is_function_declarator(node: &tree_sitter::Node) -> bool {
    match node.kind() {
        "function_declarator" => true,
        "pointer_declarator" | "parenthesized_declarator" => {
            if let Some(child) = node.child_by_field_name("declarator") {
                is_function_declarator(&child)
            } else {
                for i in 0..node.child_count() {
                    if let Some(child) = node.child(i) {
                        if is_function_declarator(&child) {
                            return true;
                        }
                    }
                }
                false
            }
        }
        _ => false,
    }
}

/// Extract struct field declarations
fn extract_struct_fields(
    body: &tree_sitter::Node,
    source: &[u8],
    file: &Path,
    result: &mut ParseResult,
    parent_qualified: &str,
) {
    for i in 0..body.child_count() {
        if let Some(child) = body.child(i) {
            if child.kind() == "field_declaration" {
                // Try field_by_name first
                if let Some(declarator) = child.child_by_field_name("declarator") {
                    if let Some(name) = extract_declarator_name(&declarator, source) {
                        let qualified = format!("{}::{}", parent_qualified, name);

                        result.symbols.push(Symbol {
                            name,
                            qualified,
                            kind: SymbolKind::Member,
                            location: node_to_location(file, &declarator),
                            visibility: Visibility::Public,
                            language: "c".to_string(),
                            parent: Some(parent_qualified.to_string()),
                            mixins: None,
                            attributes: None,
                            implements: None,
                            doc: None,
                            signature: None,
                        });
                        continue;
                    }
                }
                // Fallback: look for field_identifier directly
                for j in 0..child.child_count() {
                    if let Some(grandchild) = child.child(j) {
                        if grandchild.kind() == "field_identifier" {
                            if let Ok(name) = grandchild.utf8_text(source) {
                                let qualified = format!("{}::{}", parent_qualified, name);

                                result.symbols.push(Symbol {
                                    name: name.to_string(),
                                    qualified,
                                    kind: SymbolKind::Member,
                                    location: node_to_location(file, &grandchild),
                                    visibility: Visibility::Public,
                                    language: "c".to_string(),
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
    }
}

/// Extract enum value declarations
fn extract_enum_values(
    body: &tree_sitter::Node,
    source: &[u8],
    file: &Path,
    result: &mut ParseResult,
    parent_qualified: &str,
) {
    for i in 0..body.child_count() {
        if let Some(child) = body.child(i) {
            if child.kind() == "enumerator" {
                if let Some(name_node) = child.child_by_field_name("name") {
                    if let Ok(name) = name_node.utf8_text(source) {
                        let qualified = format!("{}::{}", parent_qualified, name);

                        result.symbols.push(Symbol {
                            name: name.to_string(),
                            qualified,
                            kind: SymbolKind::Value,
                            location: node_to_location(file, &name_node),
                            visibility: Visibility::Public,
                            language: "c".to_string(),
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

/// Recursively extract references (types and function calls) from the AST
fn extract_references_recursive(
    node: &tree_sitter::Node,
    source: &[u8],
    file: &Path,
    result: &mut ParseResult,
) {
    // type_identifier is a custom type name (struct, typedef, etc.)
    if node.kind() == "type_identifier" && is_type_reference_context(node) {
        if let Ok(name) = node.utf8_text(source) {
            result.references.push(Reference {
                name: name.to_string(),
                location: node_to_location(file, node),
            });
        }
    }

    // call_expression represents a function call: functionName(args)
    // Extract the function name as a reference
    if node.kind() == "call_expression" {
        if let Some(func_name) = extract_call_function_name(node, source) {
            // Use the function field's location for precise positioning
            let location = if let Some(func_node) = node.child_by_field_name("function") {
                node_to_location(file, &func_node)
            } else {
                node_to_location(file, node)
            };
            result.references.push(Reference {
                name: func_name,
                location,
            });
        }
    }

    // Recurse into children
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            extract_references_recursive(&child, source, file, result);
        }
    }
}

/// Extract the function name from a call_expression node
/// Handles simple calls like `foo()` and field expressions like `obj->method()`
fn extract_call_function_name(node: &tree_sitter::Node, source: &[u8]) -> Option<String> {
    let func_node = node.child_by_field_name("function")?;

    match func_node.kind() {
        // Simple function call: functionName(args)
        "identifier" => func_node.utf8_text(source).ok().map(|s| s.to_string()),

        // Field expression: obj->method() or obj.method()
        // Extract just the method name for matching
        "field_expression" => {
            if let Some(field) = func_node.child_by_field_name("field") {
                field.utf8_text(source).ok().map(|s| s.to_string())
            } else {
                None
            }
        }

        // Pointer dereference: (*funcPtr)(args)
        "pointer_expression" => {
            // Try to get the dereferenced identifier
            for i in 0..func_node.child_count() {
                if let Some(child) = func_node.child(i) {
                    if child.kind() == "identifier" {
                        return child.utf8_text(source).ok().map(|s| s.to_string());
                    }
                }
            }
            None
        }

        // Parenthesized expression: (funcPtr)(args)
        "parenthesized_expression" => {
            for i in 0..func_node.child_count() {
                if let Some(child) = func_node.child(i) {
                    if child.kind() == "identifier" {
                        return child.utf8_text(source).ok().map(|s| s.to_string());
                    }
                }
            }
            None
        }

        _ => None,
    }
}

/// Check if a node is in a context where it represents a type reference (not a definition)
fn is_type_reference_context(node: &tree_sitter::Node) -> bool {
    // A type_identifier is a reference when it's NOT in a definition context
    // It's a definition when it's:
    // 1. The name in a struct/enum/union specifier
    // 2. The name being defined in a typedef

    let mut current = *node;
    while let Some(parent) = current.parent() {
        match parent.kind() {
            // In struct/enum/union specifiers, the type_identifier can be either:
            // - The name being defined: struct User { ... } - NOT a reference
            // - A reference: struct User user; - IS a reference (User is used, not defined)
            "struct_specifier" | "union_specifier" | "enum_specifier" => {
                // Check if this is the name being defined (has a body sibling)
                // or just a reference to an existing type (no body)
                let has_body = parent.child_by_field_name("body").is_some();
                if has_body {
                    // If there's a body, and we're the name, this is a definition
                    if let Some(name_node) = parent.child_by_field_name("name") {
                        if name_node.id() == node.id() {
                            return false; // This is the definition, not a reference
                        }
                    }
                }
                // No body means this is a forward declaration or type usage - that's a reference
                return true;
            }

            // In typedef, the type being aliased is a reference, but the new name is a definition
            "type_definition" => {
                // The first type_identifier child is typically the type being aliased (reference)
                // The last type_identifier (in a type_declarator) is the new name (definition)
                // Simple heuristic: if we're a direct child of type_definition, we're the source type
                if current.id() == node.id() {
                    // Direct child type_identifier = the type being aliased = reference
                    return true;
                }
                return false;
            }

            // In declarations (parameters, variables, return types), type_identifier is a reference
            "declaration"
            | "parameter_declaration"
            | "field_declaration"
            | "function_definition"
            | "pointer_declarator"
            | "abstract_pointer_declarator" => {
                return true;
            }

            // Cast expressions use the type
            "cast_expression" => {
                return true;
            }

            // sizeof(Type) uses the type
            "sizeof_expression" => {
                return true;
            }

            // Compound literal (Type){...}
            "compound_literal_expression" => {
                return true;
            }

            _ => {}
        }
        current = parent;
    }

    // Default: if we reach here with a type_identifier, it's likely a reference
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::LanguageParser;

    #[test]
    fn extracts_c_function() {
        let source = r#"
int add(int a, int b) {
    return a + b;
}
"#;
        let parser = CParser;
        let result = parser.extract_symbols(Path::new("test.c"), source, 100);

        assert_eq!(result.symbols.len(), 1);
        assert_eq!(result.symbols[0].name, "add");
        assert_eq!(result.symbols[0].qualified, "add");
        assert_eq!(result.symbols[0].kind, SymbolKind::Function);
    }

    #[test]
    fn extracts_c_struct() {
        let source = r#"
struct Point {
    int x;
    int y;
};
"#;
        let parser = CParser;
        let result = parser.extract_symbols(Path::new("test.c"), source, 100);

        let point = result.symbols.iter().find(|s| s.name == "Point").unwrap();
        assert_eq!(point.kind, SymbolKind::Class);

        // Check fields
        let x = result.symbols.iter().find(|s| s.name == "x").unwrap();
        assert_eq!(x.qualified, "Point::x");
        assert_eq!(x.kind, SymbolKind::Member);

        let y = result.symbols.iter().find(|s| s.name == "y").unwrap();
        assert_eq!(y.qualified, "Point::y");
    }

    #[test]
    fn extracts_c_typedef() {
        let source = r#"
typedef unsigned int uint32_t;
"#;
        let parser = CParser;
        let result = parser.extract_symbols(Path::new("test.c"), source, 100);

        let typedef = result
            .symbols
            .iter()
            .find(|s| s.name == "uint32_t")
            .unwrap();
        assert_eq!(typedef.kind, SymbolKind::Type);
    }

    #[test]
    fn extracts_c_enum() {
        let source = r#"
enum Color {
    RED,
    GREEN,
    BLUE
};
"#;
        let parser = CParser;
        let result = parser.extract_symbols(Path::new("test.c"), source, 100);

        let color = result.symbols.iter().find(|s| s.name == "Color").unwrap();
        assert_eq!(color.kind, SymbolKind::Union);

        let red = result.symbols.iter().find(|s| s.name == "RED").unwrap();
        assert_eq!(red.qualified, "Color::RED");
        assert_eq!(red.kind, SymbolKind::Value);
    }

    #[test]
    fn extracts_c_union() {
        let source = r#"
union Data {
    int i;
    float f;
    char str[20];
};
"#;
        let parser = CParser;
        let result = parser.extract_symbols(Path::new("test.c"), source, 100);

        let data = result.symbols.iter().find(|s| s.name == "Data").unwrap();
        assert_eq!(data.kind, SymbolKind::Union);

        // Check fields are extracted
        let i = result.symbols.iter().find(|s| s.name == "i").unwrap();
        assert_eq!(i.qualified, "Data::i");
        assert_eq!(i.kind, SymbolKind::Member);

        let f = result.symbols.iter().find(|s| s.name == "f").unwrap();
        assert_eq!(f.qualified, "Data::f");
    }

    #[test]
    fn extracts_c_global_variable() {
        let source = r#"
int counter;
static double pi = 3.14159;
"#;
        let parser = CParser;
        let result = parser.extract_symbols(Path::new("test.c"), source, 100);

        let counter = result.symbols.iter().find(|s| s.name == "counter").unwrap();
        assert_eq!(counter.kind, SymbolKind::Value);
        assert_eq!(counter.visibility, Visibility::Public);

        let pi = result.symbols.iter().find(|s| s.name == "pi").unwrap();
        assert_eq!(pi.kind, SymbolKind::Value);
        assert_eq!(pi.visibility, Visibility::Private); // static = internal linkage
    }

    #[test]
    fn static_function_has_private_visibility() {
        let source = r#"
static void helper() {
    // internal helper function
}

void public_func() {
    helper();
}
"#;
        let parser = CParser;
        let result = parser.extract_symbols(Path::new("test.c"), source, 100);

        let helper = result.symbols.iter().find(|s| s.name == "helper").unwrap();
        assert_eq!(helper.visibility, Visibility::Private);

        let public_func = result
            .symbols
            .iter()
            .find(|s| s.name == "public_func")
            .unwrap();
        assert_eq!(public_func.visibility, Visibility::Public);
    }

    #[test]
    fn extracts_c_macro() {
        let source = r#"
#define MAX_SIZE 100
#define MIN(a, b) ((a) < (b) ? (a) : (b))
"#;
        let parser = CParser;
        let result = parser.extract_symbols(Path::new("test.c"), source, 100);

        let max_size = result
            .symbols
            .iter()
            .find(|s| s.name == "MAX_SIZE")
            .unwrap();
        assert_eq!(max_size.kind, SymbolKind::Value);

        let min = result.symbols.iter().find(|s| s.name == "MIN").unwrap();
        assert_eq!(min.kind, SymbolKind::Function);
    }

    #[test]
    fn extracts_c_includes() {
        let source = r#"
#include <stdio.h>
#include "myheader.h"
"#;
        let parser = CParser;
        let result = parser.extract_symbols(Path::new("test.c"), source, 100);

        assert!(result.opens.contains(&"stdio.h".to_string()));
        assert!(result.opens.contains(&"myheader.h".to_string()));
    }

    #[test]
    fn extracts_function_with_doc_comment() {
        let source = r#"
// Adds two numbers together
// Returns the sum
int add(int a, int b) {
    return a + b;
}
"#;
        let parser = CParser;
        let result = parser.extract_symbols(Path::new("test.c"), source, 100);

        let add = result.symbols.iter().find(|s| s.name == "add").unwrap();
        assert!(add.doc.is_some());
        assert!(add.doc.as_ref().unwrap().contains("Adds two numbers"));
    }

    #[test]
    fn extracts_c_references() {
        use crate::parse::extract_symbols;

        let source = r#"
#include "user.h"

typedef struct User User;

void greet(User* user) {
    printf("Hello, %s\n", user->name);
}

User* create_user(const char* name) {
    User* user = (User*)malloc(sizeof(User));
    user->name = name;
    return user;
}
"#;
        let result = extract_symbols(Path::new("test.c"), source, 100);

        let ref_names: Vec<&str> = result.references.iter().map(|r| r.name.as_str()).collect();

        // Should extract references to User type (parameter, return type, cast, sizeof)
        assert!(
            ref_names.contains(&"User"),
            "Should extract references from C code: {:?}",
            ref_names
        );
    }

    #[test]
    fn extracts_function_call_references() {
        let parser = CParser;
        let source = r#"
void helper() {
    // does something
}

void processCommand(int cmd) {
    // process the command
}

int main() {
    helper();
    processCommand(42);
    return 0;
}
"#;
        let result = parser.extract_symbols(Path::new("test.c"), source, 100);

        let ref_names: Vec<&str> = result.references.iter().map(|r| r.name.as_str()).collect();

        // Should extract function call references
        assert!(
            ref_names.contains(&"helper"),
            "Should extract 'helper' function call reference: {:?}",
            ref_names
        );
        assert!(
            ref_names.contains(&"processCommand"),
            "Should extract 'processCommand' function call reference: {:?}",
            ref_names
        );
    }

    #[test]
    fn extracts_method_call_references() {
        let parser = CParser;
        let source = r#"
struct Client {
    int (*send)(struct Client*, const char*);
};

void use_client(struct Client* client) {
    client->send(client, "hello");
}
"#;
        let result = parser.extract_symbols(Path::new("test.c"), source, 100);

        let ref_names: Vec<&str> = result.references.iter().map(|r| r.name.as_str()).collect();

        // Should extract method-like call through field expression
        assert!(
            ref_names.contains(&"send"),
            "Should extract 'send' field call reference: {:?}",
            ref_names
        );
    }
}
