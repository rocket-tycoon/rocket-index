//! Symbol extraction from Kotlin source files using tree-sitter.

use std::cell::RefCell;
use std::path::Path;

use crate::parse::{find_child_by_kind, node_to_location, LanguageParser, ParseResult};
use crate::{Reference, Symbol, SymbolKind, Visibility};

// Thread-local parser reuse - avoids creating a new parser per file
thread_local! {
    static KOTLIN_PARSER: RefCell<tree_sitter::Parser> = RefCell::new({
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_kotlin_ng::LANGUAGE.into())
            .expect("tree-sitter-kotlin grammar incompatible with tree-sitter version");
        parser
    });
}

pub struct KotlinParser;

impl LanguageParser for KotlinParser {
    fn extract_symbols(&self, file: &Path, source: &str, max_depth: usize) -> ParseResult {
        KOTLIN_PARSER.with(|parser| {
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

            // Extract package name from package_header -> qualified_identifier
            let package = extract_package_name(&root, source.as_bytes());

            extract_recursive(
                &root,
                source.as_bytes(),
                file,
                &mut result,
                package.as_deref(),
                max_depth,
            );

            result
        })
    }
}

/// Extract package name from package_header -> qualified_identifier
fn extract_package_name(root: &tree_sitter::Node, source: &[u8]) -> Option<String> {
    let mut cursor = root.walk();
    if cursor.goto_first_child() {
        loop {
            if cursor.node().kind() == "package_header" {
                let pkg_node = cursor.node();
                // Find qualified_identifier inside package_header
                if let Some(qid) = find_child_by_kind(&pkg_node, "qualified_identifier") {
                    if let Ok(name) = qid.utf8_text(source) {
                        return Some(name.to_string());
                    }
                }
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
    None
}

/// Determine visibility from Kotlin modifiers
fn extract_visibility(node: &tree_sitter::Node, source: &[u8]) -> Visibility {
    if let Some(modifiers) = find_child_by_kind(node, "modifiers") {
        let mut cursor = modifiers.walk();
        if cursor.goto_first_child() {
            loop {
                let child = cursor.node();
                if child.kind() == "visibility_modifier" {
                    if let Ok(text) = child.utf8_text(source) {
                        return match text {
                            "public" => Visibility::Public,
                            "private" => Visibility::Private,
                            "protected" => Visibility::Internal,
                            "internal" => Visibility::Internal,
                            _ => Visibility::Public,
                        };
                    }
                }
                if !cursor.goto_next_sibling() {
                    break;
                }
            }
        }
    }
    // Kotlin defaults to public visibility
    Visibility::Public
}

/// Build a qualified name with package prefix
fn qualified_name(name: &str, package: Option<&str>) -> String {
    match package {
        Some(p) => format!("{}.{}", p, name),
        None => name.to_string(),
    }
}

/// Extract KDoc comments (/** ... */)
fn extract_doc_comments(node: &tree_sitter::Node, source: &[u8]) -> Option<String> {
    let mut prev = node.prev_sibling();
    while let Some(sib) = prev {
        // tree-sitter-kotlin-ng uses "block_comment" for /** ... */
        if sib.kind() == "block_comment" || sib.kind() == "multiline_comment" {
            if let Ok(text) = sib.utf8_text(source) {
                if text.starts_with("/**") {
                    let cleaned = text
                        .trim_start_matches("/**")
                        .trim_end_matches("*/")
                        .lines()
                        .map(|line| line.trim().trim_start_matches('*').trim())
                        .filter(|line| !line.is_empty())
                        .collect::<Vec<_>>()
                        .join("\n");
                    return Some(cleaned);
                }
            }
        } else if sib.kind() != "line_comment" {
            break;
        }
        prev = sib.prev_sibling();
    }
    None
}

/// Extract annotations (@Override, @Service, etc.)
fn extract_annotations(node: &tree_sitter::Node, source: &[u8]) -> Option<Vec<String>> {
    let mut annotations = Vec::new();

    if let Some(modifiers) = find_child_by_kind(node, "modifiers") {
        let mut cursor = modifiers.walk();
        if cursor.goto_first_child() {
            loop {
                let child = cursor.node();
                if child.kind() == "annotation" {
                    if let Ok(text) = child.utf8_text(source) {
                        annotations.push(text.to_string());
                    }
                }
                if !cursor.goto_next_sibling() {
                    break;
                }
            }
        }
    }

    if annotations.is_empty() {
        None
    } else {
        Some(annotations)
    }
}

/// Extract function signature
fn extract_function_signature(
    node: &tree_sitter::Node,
    source: &[u8],
    name: &str,
) -> Option<String> {
    let mut sig = String::new();
    sig.push_str("fun ");
    sig.push_str(name);

    // Get parameters
    if let Some(params) = find_child_by_kind(node, "function_value_parameters") {
        if let Ok(params_text) = params.utf8_text(source) {
            sig.push_str(params_text);
        }
    }

    // Get return type (the user_type after :)
    // Find the : token and then the user_type
    let mut found_colon = false;
    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            let child = cursor.node();
            if child.kind() == ":" {
                found_colon = true;
            } else if found_colon
                && (child.kind() == "user_type" || child.kind() == "nullable_type")
            {
                if let Ok(rt) = child.utf8_text(source) {
                    sig.push_str(": ");
                    sig.push_str(rt);
                }
                break;
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }

    Some(sig)
}

/// Check if a class_declaration is actually an interface (has "interface" keyword)
fn is_interface(node: &tree_sitter::Node, _source: &[u8]) -> bool {
    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            let child = cursor.node();
            if child.kind() == "interface" {
                return true;
            }
            if child.kind() == "class" {
                return false;
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
    false
}

/// Check if a class has the "enum" modifier
fn is_enum_class(node: &tree_sitter::Node, source: &[u8]) -> bool {
    if let Some(modifiers) = find_child_by_kind(node, "modifiers") {
        let mut cursor = modifiers.walk();
        if cursor.goto_first_child() {
            loop {
                let child = cursor.node();
                if child.kind() == "class_modifier" {
                    if let Ok(text) = child.utf8_text(source) {
                        if text == "enum" {
                            return true;
                        }
                    }
                }
                if !cursor.goto_next_sibling() {
                    break;
                }
            }
        }
    }
    false
}

/// Check if a class has the "data" modifier
fn is_data_class(node: &tree_sitter::Node, source: &[u8]) -> bool {
    if let Some(modifiers) = find_child_by_kind(node, "modifiers") {
        let mut cursor = modifiers.walk();
        if cursor.goto_first_child() {
            loop {
                let child = cursor.node();
                if child.kind() == "class_modifier" {
                    if let Ok(text) = child.utf8_text(source) {
                        if text == "data" {
                            return true;
                        }
                    }
                }
                if !cursor.goto_next_sibling() {
                    break;
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
    package: Option<&str>,
    max_depth: usize,
) {
    if max_depth == 0 {
        return;
    }

    match node.kind() {
        "class_declaration" => {
            // class_declaration can be: class, interface, or enum class
            if let Some(name_node) = find_child_by_kind(node, "identifier") {
                if let Ok(name) = name_node.utf8_text(source) {
                    let qualified = qualified_name(name, package);
                    let visibility = extract_visibility(node, source);
                    let doc = extract_doc_comments(node, source);
                    let annotations = extract_annotations(node, source);

                    // Determine the kind
                    let kind = if is_interface(node, source) {
                        SymbolKind::Interface
                    } else if is_enum_class(node, source) {
                        SymbolKind::Union
                    } else {
                        SymbolKind::Class
                    };

                    result.symbols.push(Symbol {
                        name: name.to_string(),
                        qualified: qualified.clone(),
                        kind,
                        location: node_to_location(file, &name_node),
                        visibility,
                        language: "kotlin".to_string(),
                        parent: None,
                        mixins: None,
                        attributes: annotations,
                        implements: None,
                        doc,
                        signature: None,
                    });

                    // Extract data class constructor params
                    if is_data_class(node, source) {
                        extract_primary_constructor_params(node, source, file, result, &qualified);
                    }

                    // Extract enum entries for enum classes
                    if is_enum_class(node, source) {
                        if let Some(body) = find_child_by_kind(node, "enum_class_body") {
                            extract_enum_entries(&body, source, file, result, &qualified);
                        }
                    }

                    // Recurse into class body
                    if let Some(body) = find_child_by_kind(node, "class_body") {
                        let mut cursor = body.walk();
                        if cursor.goto_first_child() {
                            loop {
                                extract_recursive(
                                    &cursor.node(),
                                    source,
                                    file,
                                    result,
                                    Some(&qualified),
                                    max_depth - 1,
                                );
                                if !cursor.goto_next_sibling() {
                                    break;
                                }
                            }
                        }
                    }
                    return;
                }
            }
        }

        "object_declaration" => {
            if let Some(name_node) = find_child_by_kind(node, "identifier") {
                if let Ok(name) = name_node.utf8_text(source) {
                    let qualified = qualified_name(name, package);
                    let visibility = extract_visibility(node, source);
                    let doc = extract_doc_comments(node, source);
                    let annotations = extract_annotations(node, source);

                    result.symbols.push(Symbol {
                        name: name.to_string(),
                        qualified: qualified.clone(),
                        kind: SymbolKind::Class,
                        location: node_to_location(file, &name_node),
                        visibility,
                        language: "kotlin".to_string(),
                        parent: None,
                        mixins: None,
                        attributes: annotations,
                        implements: None,
                        doc,
                        signature: None,
                    });

                    // Recurse into object body
                    if let Some(body) = find_child_by_kind(node, "class_body") {
                        let mut cursor = body.walk();
                        if cursor.goto_first_child() {
                            loop {
                                extract_recursive(
                                    &cursor.node(),
                                    source,
                                    file,
                                    result,
                                    Some(&qualified),
                                    max_depth - 1,
                                );
                                if !cursor.goto_next_sibling() {
                                    break;
                                }
                            }
                        }
                    }
                    return;
                }
            }
        }

        "companion_object" => {
            let companion_name = "Companion";
            let qualified = match package {
                Some(p) => format!("{}.{}", p, companion_name),
                None => companion_name.to_string(),
            };

            result.symbols.push(Symbol {
                name: companion_name.to_string(),
                qualified: qualified.clone(),
                kind: SymbolKind::Class,
                location: node_to_location(file, node),
                visibility: Visibility::Public,
                language: "kotlin".to_string(),
                parent: package.map(|s| s.to_string()),
                mixins: None,
                attributes: None,
                implements: None,
                doc: None,
                signature: None,
            });

            // Recurse into companion body
            if let Some(body) = find_child_by_kind(node, "class_body") {
                let mut cursor = body.walk();
                if cursor.goto_first_child() {
                    loop {
                        extract_recursive(
                            &cursor.node(),
                            source,
                            file,
                            result,
                            Some(&qualified),
                            max_depth - 1,
                        );
                        if !cursor.goto_next_sibling() {
                            break;
                        }
                    }
                }
            }
            return;
        }

        "function_declaration" => {
            if let Some(name_node) = find_child_by_kind(node, "identifier") {
                if let Ok(name) = name_node.utf8_text(source) {
                    let qualified = qualified_name(name, package);
                    let visibility = extract_visibility(node, source);
                    let doc = extract_doc_comments(node, source);
                    let annotations = extract_annotations(node, source);
                    let signature = extract_function_signature(node, source, name);

                    result.symbols.push(Symbol {
                        name: name.to_string(),
                        qualified,
                        kind: SymbolKind::Function,
                        location: node_to_location(file, &name_node),
                        visibility,
                        language: "kotlin".to_string(),
                        parent: None,
                        mixins: None,
                        attributes: annotations,
                        implements: None,
                        doc,
                        signature,
                    });
                }
            }
        }

        "property_declaration" => {
            // property_declaration contains variable_declaration which has identifier
            if let Some(var_decl) = find_child_by_kind(node, "variable_declaration") {
                if let Some(id_node) = find_child_by_kind(&var_decl, "identifier") {
                    if let Ok(name) = id_node.utf8_text(source) {
                        let qualified = qualified_name(name, package);
                        let visibility = extract_visibility(node, source);
                        let doc = extract_doc_comments(node, source);
                        let annotations = extract_annotations(node, source);

                        result.symbols.push(Symbol {
                            name: name.to_string(),
                            qualified,
                            kind: SymbolKind::Value,
                            location: node_to_location(file, &id_node),
                            visibility,
                            language: "kotlin".to_string(),
                            parent: None,
                            mixins: None,
                            attributes: annotations,
                            implements: None,
                            doc,
                            signature: None,
                        });
                    }
                }
            }
        }

        "type_alias" => {
            if let Some(name_node) = find_child_by_kind(node, "identifier") {
                if let Ok(name) = name_node.utf8_text(source) {
                    let qualified = qualified_name(name, package);
                    let visibility = extract_visibility(node, source);
                    let doc = extract_doc_comments(node, source);

                    result.symbols.push(Symbol {
                        name: name.to_string(),
                        qualified,
                        kind: SymbolKind::Type,
                        location: node_to_location(file, &name_node),
                        visibility,
                        language: "kotlin".to_string(),
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

        "import" => {
            // Extract import path from qualified_identifier
            if let Some(qid) = find_child_by_kind(node, "qualified_identifier") {
                if let Ok(text) = qid.utf8_text(source) {
                    result.opens.push(text.to_string());
                }
            }
        }

        // Extract references from user_type identifiers
        "user_type" => {
            if is_reference_context(node) {
                if let Some(id) = find_child_by_kind(node, "identifier") {
                    if let Ok(name) = id.utf8_text(source) {
                        result.references.push(Reference {
                            name: name.to_string(),
                            location: node_to_location(file, &id),
                        });
                    }
                }
            }
        }

        // Extract function/method call references
        // e.g., greet() -> reference to "greet"
        // e.g., fuel.get() -> reference to "fuel.get" and "get"
        "call_expression" => {
            // A call_expression in Kotlin contains the callee and arguments
            // Look for the callee which could be:
            // - simple_identifier for direct calls: getFuel()
            // - navigation_expression for method calls: fuel.get()
            // - identifier for some function calls
            // The structure can be: call_expression -> callee + call_suffix
            for i in 0..node.child_count() {
                if let Some(child) = node.child(i) {
                    match child.kind() {
                        "simple_identifier" | "identifier" => {
                            // Direct function call: greet(...)
                            if let Ok(name) = child.utf8_text(source) {
                                result.references.push(Reference {
                                    name: name.to_string(),
                                    location: node_to_location(file, &child),
                                });
                            }
                            break; // Found the callee
                        }
                        "navigation_expression" => {
                            // Method call: obj.method(...)
                            // Extract the full dotted name and also just the method name
                            if let Ok(full_name) = child.utf8_text(source) {
                                result.references.push(Reference {
                                    name: full_name.to_string(),
                                    location: node_to_location(file, &child),
                                });
                            }
                            // Also extract just the method name (last part after the dot)
                            if let Some(method_name) = extract_navigation_suffix(&child, source) {
                                result.references.push(Reference {
                                    name: method_name,
                                    location: node_to_location(file, &child),
                                });
                            }
                            break; // Found the callee
                        }
                        "call_suffix" | "value_arguments" => {
                            // These are arguments, not the callee - stop looking
                            break;
                        }
                        _ => {}
                    }
                }
            }
        }

        // Extract navigation expressions (member access) that aren't call targets
        // e.g., user.name (property access)
        "navigation_expression" => {
            // Only add as reference if not already handled by call_expression parent
            if let Some(parent_node) = node.parent() {
                if parent_node.kind() != "call_expression" {
                    if let Ok(name) = node.utf8_text(source) {
                        result.references.push(Reference {
                            name: name.to_string(),
                            location: node_to_location(file, node),
                        });
                    }
                    // Also extract just the suffix (property name)
                    if let Some(prop_name) = extract_navigation_suffix(node, source) {
                        result.references.push(Reference {
                            name: prop_name,
                            location: node_to_location(file, node),
                        });
                    }
                }
            }
        }

        // Extract simple identifiers in reference contexts
        "simple_identifier" => {
            if is_simple_identifier_reference(node) {
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
    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            extract_recursive(&cursor.node(), source, file, result, package, max_depth - 1);
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
}

/// Extract primary constructor parameters for data classes
fn extract_primary_constructor_params(
    node: &tree_sitter::Node,
    source: &[u8],
    file: &Path,
    result: &mut ParseResult,
    class_qualified: &str,
) {
    if let Some(ctor) = find_child_by_kind(node, "primary_constructor") {
        if let Some(params) = find_child_by_kind(&ctor, "class_parameters") {
            let mut cursor = params.walk();
            if cursor.goto_first_child() {
                loop {
                    let child = cursor.node();
                    if child.kind() == "class_parameter" {
                        if let Some(id) = find_child_by_kind(&child, "identifier") {
                            if let Ok(name) = id.utf8_text(source) {
                                let qualified = format!("{}.{}", class_qualified, name);
                                result.symbols.push(Symbol {
                                    name: name.to_string(),
                                    qualified,
                                    kind: SymbolKind::Member,
                                    location: node_to_location(file, &id),
                                    visibility: Visibility::Public,
                                    language: "kotlin".to_string(),
                                    parent: Some(class_qualified.to_string()),
                                    mixins: None,
                                    attributes: None,
                                    implements: None,
                                    doc: None,
                                    signature: None,
                                });
                            }
                        }
                    }
                    if !cursor.goto_next_sibling() {
                        break;
                    }
                }
            }
        }
    }
}

/// Extract enum entries
fn extract_enum_entries(
    body: &tree_sitter::Node,
    source: &[u8],
    file: &Path,
    result: &mut ParseResult,
    enum_qualified: &str,
) {
    let mut cursor = body.walk();
    if cursor.goto_first_child() {
        loop {
            let child = cursor.node();
            if child.kind() == "enum_entry" {
                if let Some(id) = find_child_by_kind(&child, "identifier") {
                    if let Ok(name) = id.utf8_text(source) {
                        let qualified = format!("{}.{}", enum_qualified, name);
                        result.symbols.push(Symbol {
                            name: name.to_string(),
                            qualified,
                            kind: SymbolKind::Member,
                            location: node_to_location(file, &id),
                            visibility: Visibility::Public,
                            language: "kotlin".to_string(),
                            parent: Some(enum_qualified.to_string()),
                            mixins: None,
                            attributes: None,
                            implements: None,
                            doc: extract_doc_comments(&child, source),
                            signature: None,
                        });
                    }
                }
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
}

/// Check if a node is in a reference context (not a definition)
fn is_reference_context(node: &tree_sitter::Node) -> bool {
    let parent = match node.parent() {
        Some(p) => p,
        None => return false,
    };

    matches!(
        parent.kind(),
        "nullable_type" | "parameter" | "class_parameter" | "variable_declaration"
    )
}

/// Extract the suffix (method/property name) from a navigation expression
/// For "obj.method", returns "method"
fn extract_navigation_suffix(node: &tree_sitter::Node, source: &[u8]) -> Option<String> {
    // navigation_expression has structure: operand, navigation_suffix
    // navigation_suffix contains the method/property name
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            if child.kind() == "navigation_suffix" {
                // Get the simple_identifier inside the navigation_suffix
                for j in 0..child.child_count() {
                    if let Some(inner) = child.child(j) {
                        if inner.kind() == "simple_identifier" {
                            return inner.utf8_text(source).ok().map(|s| s.to_string());
                        }
                    }
                }
            }
        }
    }
    None
}

/// Check if a simple_identifier is in a reference context (not a definition)
fn is_simple_identifier_reference(node: &tree_sitter::Node) -> bool {
    let parent = match node.parent() {
        Some(p) => p,
        None => return false,
    };

    // Definition contexts (NOT references)
    let parent_kind = parent.kind();

    // Function/property/class declarations
    if matches!(
        parent_kind,
        "function_declaration"
            | "property_declaration"
            | "class_declaration"
            | "object_declaration"
            | "type_alias"
    ) {
        // Check if this is the name being defined
        if let Some(name_node) = parent.child_by_field_name("name") {
            if name_node.id() == node.id() {
                return false;
            }
        }
    }

    // Parameter names
    if parent_kind == "parameter" || parent_kind == "class_parameter" {
        if let Some(name_node) = parent.child_by_field_name("name") {
            if name_node.id() == node.id() {
                return false;
            }
        }
    }

    // Package name
    if parent_kind == "package_header" || parent_kind == "identifier" {
        return false;
    }

    // Import references (not really useful as references)
    if parent_kind == "import_header" {
        return false;
    }

    // Navigation suffix (handled separately)
    if parent_kind == "navigation_suffix" {
        return false;
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::LanguageParser;

    #[test]
    fn extracts_kotlin_class() {
        let source = r#"
package com.example

/**
 * A simple user class.
 */
class User {
    val name: String = ""
}
"#;
        let parser = KotlinParser;
        let result = parser.extract_symbols(std::path::Path::new("User.kt"), source, 100);

        let class_sym = result
            .symbols
            .iter()
            .find(|s| s.name == "User")
            .expect("Should find User class");
        assert_eq!(class_sym.kind, SymbolKind::Class);
        assert_eq!(class_sym.qualified, "com.example.User");
        assert!(class_sym.doc.is_some());
    }

    #[test]
    fn extracts_kotlin_data_class() {
        let source = r#"
package com.example

data class Point(val x: Int, val y: Int)
"#;
        let parser = KotlinParser;
        let result = parser.extract_symbols(std::path::Path::new("Point.kt"), source, 100);

        let class_sym = result
            .symbols
            .iter()
            .find(|s| s.name == "Point")
            .expect("Should find Point data class");
        assert_eq!(class_sym.kind, SymbolKind::Class);

        let x_member = result.symbols.iter().find(|s| s.name == "x");
        assert!(x_member.is_some(), "Should find x property");

        let y_member = result.symbols.iter().find(|s| s.name == "y");
        assert!(y_member.is_some(), "Should find y property");
    }

    #[test]
    fn extracts_kotlin_interface() {
        let source = r#"
package com.example

interface Repository<T> {
    fun findById(id: Int): T?
}
"#;
        let parser = KotlinParser;
        let result = parser.extract_symbols(std::path::Path::new("Repository.kt"), source, 100);

        let iface = result
            .symbols
            .iter()
            .find(|s| s.name == "Repository")
            .expect("Should find Repository interface");
        assert_eq!(iface.kind, SymbolKind::Interface);
    }

    #[test]
    fn extracts_kotlin_function() {
        let source = r#"
package com.example

/**
 * Adds two numbers.
 */
fun add(a: Int, b: Int): Int {
    return a + b
}
"#;
        let parser = KotlinParser;
        let result = parser.extract_symbols(std::path::Path::new("Math.kt"), source, 100);

        let func = result
            .symbols
            .iter()
            .find(|s| s.name == "add")
            .expect("Should find add function");
        assert_eq!(func.kind, SymbolKind::Function);
        assert!(func.signature.is_some());
    }

    #[test]
    fn extracts_kotlin_object() {
        let source = r#"
package com.example

object Singleton {
    fun getInstance(): Singleton = this
}
"#;
        let parser = KotlinParser;
        let result = parser.extract_symbols(std::path::Path::new("Singleton.kt"), source, 100);

        let obj = result
            .symbols
            .iter()
            .find(|s| s.name == "Singleton")
            .expect("Should find Singleton object");
        assert_eq!(obj.kind, SymbolKind::Class);
    }

    #[test]
    fn extracts_kotlin_enum() {
        let source = r#"
package com.example

enum class Status {
    PENDING,
    ACTIVE,
    COMPLETED
}
"#;
        let parser = KotlinParser;
        let result = parser.extract_symbols(std::path::Path::new("Status.kt"), source, 100);

        let enum_sym = result
            .symbols
            .iter()
            .find(|s| s.name == "Status")
            .expect("Should find Status enum");
        assert_eq!(enum_sym.kind, SymbolKind::Union);

        let pending = result
            .symbols
            .iter()
            .find(|s| s.name == "PENDING")
            .expect("Should find PENDING entry");
        assert_eq!(pending.kind, SymbolKind::Member);
    }

    #[test]
    fn extracts_kotlin_imports() {
        let source = r#"
package com.example

import java.util.List
import kotlin.collections.Map

class App
"#;
        let parser = KotlinParser;
        let result = parser.extract_symbols(std::path::Path::new("App.kt"), source, 100);

        assert!(result.opens.contains(&"java.util.List".to_string()));
        assert!(result.opens.contains(&"kotlin.collections.Map".to_string()));
    }

    #[test]
    fn handles_kotlin_visibility_modifiers() {
        let source = r#"
package com.example

class Example {
    public fun publicMethod() {}
    private fun privateMethod() {}
    protected fun protectedMethod() {}
    internal fun internalMethod() {}
    fun defaultMethod() {}
}
"#;
        let parser = KotlinParser;
        let result = parser.extract_symbols(std::path::Path::new("Example.kt"), source, 100);

        let public = result
            .symbols
            .iter()
            .find(|s| s.name == "publicMethod")
            .unwrap();
        assert_eq!(public.visibility, Visibility::Public);

        let private = result
            .symbols
            .iter()
            .find(|s| s.name == "privateMethod")
            .unwrap();
        assert_eq!(private.visibility, Visibility::Private);

        let protected = result
            .symbols
            .iter()
            .find(|s| s.name == "protectedMethod")
            .unwrap();
        assert_eq!(protected.visibility, Visibility::Internal);

        let internal = result
            .symbols
            .iter()
            .find(|s| s.name == "internalMethod")
            .unwrap();
        assert_eq!(internal.visibility, Visibility::Internal);

        // Default is public in Kotlin
        let default = result
            .symbols
            .iter()
            .find(|s| s.name == "defaultMethod")
            .unwrap();
        assert_eq!(default.visibility, Visibility::Public);
    }

    #[test]
    fn extracts_kotlin_property() {
        let source = r#"
package com.example

val globalCounter: Int = 0
"#;
        let parser = KotlinParser;
        let result = parser.extract_symbols(std::path::Path::new("Config.kt"), source, 100);

        let prop = result
            .symbols
            .iter()
            .find(|s| s.name == "globalCounter")
            .expect("Should find globalCounter property");
        assert_eq!(prop.kind, SymbolKind::Value);
    }

    #[test]
    fn extracts_kotlin_annotations() {
        let source = r#"
package com.example

@Service
@Transactional
class UserService {
    @Override
    fun doSomething() {}
}
"#;
        let parser = KotlinParser;
        let result = parser.extract_symbols(std::path::Path::new("UserService.kt"), source, 100);

        let class_sym = result
            .symbols
            .iter()
            .find(|s| s.name == "UserService")
            .expect("Should find UserService");

        assert!(class_sym.attributes.is_some());
        let attrs = class_sym.attributes.as_ref().unwrap();
        assert!(attrs.iter().any(|a| a.contains("Service")));
    }

    #[test]
    fn extracts_typealias() {
        let source = r#"
package com.example

typealias StringList = List<String>
"#;
        let parser = KotlinParser;
        let result = parser.extract_symbols(std::path::Path::new("Types.kt"), source, 100);

        let alias = result
            .symbols
            .iter()
            .find(|s| s.name == "StringList")
            .expect("Should find StringList typealias");
        assert_eq!(alias.kind, SymbolKind::Type);
    }

    #[test]
    fn extracts_method_call_references() {
        let source = r#"
package com.example

class Service {
    fun perform() {
        val fuel = getFuel()
        fuel.get()
        fuel.post("data")
        helper.process(data)
    }
}
"#;
        let parser = KotlinParser;
        let result = parser.extract_symbols(std::path::Path::new("Service.kt"), source, 100);

        let ref_names: Vec<_> = result.references.iter().map(|r| r.name.as_str()).collect();

        // Should have reference to direct function call
        assert!(
            ref_names.contains(&"getFuel"),
            "Should contain reference to 'getFuel', found: {:?}",
            ref_names
        );

        // Should have reference to method calls
        assert!(
            ref_names.contains(&"get") || ref_names.iter().any(|n| n.contains("get")),
            "Should contain reference to 'get', found: {:?}",
            ref_names
        );

        assert!(
            ref_names.contains(&"post") || ref_names.iter().any(|n| n.contains("post")),
            "Should contain reference to 'post', found: {:?}",
            ref_names
        );

        assert!(
            ref_names.contains(&"process") || ref_names.iter().any(|n| n.contains("process")),
            "Should contain reference to 'process', found: {:?}",
            ref_names
        );
    }

    #[test]
    fn extracts_qualified_method_references() {
        let source = r#"
package com.example

class Client {
    fun call() {
        UserService.findById(123)
        api.client.get("/endpoint")
    }
}
"#;
        let parser = KotlinParser;
        let result = parser.extract_symbols(std::path::Path::new("Client.kt"), source, 100);

        let ref_names: Vec<_> = result.references.iter().map(|r| r.name.as_str()).collect();

        // Should have reference to qualified method call
        assert!(
            ref_names.iter().any(|n| n.contains("findById")),
            "Should contain reference to 'findById', found: {:?}",
            ref_names
        );

        // Should have reference to chained method call
        assert!(
            ref_names.iter().any(|n| n.contains("get")),
            "Should contain reference to 'get', found: {:?}",
            ref_names
        );
    }
}
