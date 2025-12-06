//! Symbol extraction from Rust source files using tree-sitter.

use std::cell::RefCell;
use std::path::Path;

use crate::parse::{find_child_by_kind, node_to_location, LanguageParser, ParseResult};
use crate::{Symbol, SymbolKind, Visibility};

// Thread-local parser reuse - avoids creating a new parser per file
thread_local! {
    static RUST_PARSER: RefCell<tree_sitter::Parser> = RefCell::new({
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_rust::LANGUAGE.into())
            .expect("tree-sitter-rust grammar incompatible with tree-sitter version");
        parser
    });
}

pub struct RustParser;

impl LanguageParser for RustParser {
    fn extract_symbols(&self, file: &Path, source: &str, max_depth: usize) -> ParseResult {
        RUST_PARSER.with(|parser| {
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

            result
        })
    }
}

/// Extract visibility from a visibility_modifier node
fn extract_visibility(node: &tree_sitter::Node, source: &[u8]) -> Visibility {
    if let Some(vis) = find_child_by_kind(node, "visibility_modifier") {
        if let Ok(text) = vis.utf8_text(source) {
            return match text {
                "pub" => Visibility::Public,
                s if s.starts_with("pub(crate)") => Visibility::Internal,
                s if s.starts_with("pub(super)") => Visibility::Internal,
                s if s.starts_with("pub(in") => Visibility::Internal,
                _ => Visibility::Private,
            };
        }
    }
    Visibility::Private
}

/// Extract doc comments from attributes
fn extract_doc_comments(node: &tree_sitter::Node, source: &[u8]) -> Option<String> {
    let mut docs = Vec::new();

    // Look for preceding line_comment or block_comment nodes
    let mut cursor = node.walk();
    if let Some(parent) = node.parent() {
        let mut sibling = parent.child(0);
        while let Some(sib) = sibling {
            if sib.id() == node.id() {
                break;
            }
            if sib.kind() == "line_comment" {
                if let Ok(text) = sib.utf8_text(source) {
                    // /// doc comments
                    if text.starts_with("///") {
                        let doc = text.trim_start_matches("///").trim();
                        if !doc.is_empty() {
                            docs.push(doc.to_string());
                        }
                    }
                }
            }
            sibling = sib.next_sibling();
        }
    }

    // Also check for attribute doc comments like #[doc = "..."]
    cursor.reset(*node);
    if cursor.goto_first_child() {
        loop {
            let child = cursor.node();
            if child.kind() == "attribute_item" {
                if let Ok(text) = child.utf8_text(source) {
                    if text.contains("doc") {
                        // Extract the doc string
                        if let Some(start) = text.find('"') {
                            if let Some(end) = text.rfind('"') {
                                if end > start {
                                    docs.push(text[start + 1..end].to_string());
                                }
                            }
                        }
                    }
                }
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }

    if docs.is_empty() {
        None
    } else {
        Some(docs.join("\n"))
    }
}

/// Extract attributes (derive, cfg, etc.)
/// In tree-sitter-rust, attributes are preceding siblings of the item they decorate.
fn extract_attributes(node: &tree_sitter::Node, source: &[u8]) -> Option<Vec<String>> {
    let mut attrs = Vec::new();

    // Look for preceding sibling attribute_item nodes
    let mut prev_sibling = node.prev_sibling();
    while let Some(sib) = prev_sibling {
        if sib.kind() == "attribute_item" {
            if let Ok(text) = sib.utf8_text(source) {
                // Skip doc attributes
                if !text.contains("doc =") && !text.contains("doc=") {
                    // Clean up the attribute: remove #[ and ]
                    let cleaned = text
                        .trim_start_matches("#[")
                        .trim_end_matches(']')
                        .to_string();
                    attrs.push(cleaned);
                }
            }
            prev_sibling = sib.prev_sibling();
        } else {
            // Stop at first non-attribute sibling
            break;
        }
    }

    // Reverse to maintain original order (we collected them backwards)
    attrs.reverse();

    if attrs.is_empty() {
        None
    } else {
        Some(attrs)
    }
}

/// Extract function signature
fn extract_function_signature(node: &tree_sitter::Node, source: &[u8]) -> Option<String> {
    // Get the text up to the block (function body)
    let start = node.start_byte();
    if let Some(body) = find_child_by_kind(node, "block") {
        let end = body.start_byte();
        if end > start {
            if let Ok(sig) = std::str::from_utf8(&source[start..end]) {
                return Some(sig.trim().to_string());
            }
        }
    }
    None
}

/// Build qualified name with :: separator
fn qualified_name(name: &str, parent_path: Option<&str>) -> String {
    match parent_path {
        Some(p) => format!("{}::{}", p, name),
        None => name.to_string(),
    }
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
        "mod_item" => {
            // Module definition: mod foo { ... } or mod foo;
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Ok(name) = name_node.utf8_text(source) {
                    let qualified = qualified_name(name, parent_path);
                    let visibility = extract_visibility(node, source);
                    let doc = extract_doc_comments(node, source);

                    result.symbols.push(Symbol {
                        name: name.to_string(),
                        qualified: qualified.clone(),
                        kind: SymbolKind::Module,
                        location: node_to_location(file, &name_node),
                        visibility,
                        language: "rust".to_string(),
                        parent: None,
                        mixins: None,
                        attributes: extract_attributes(node, source),
                        implements: None,
                        doc,
                        signature: None,
                    });

                    // Recurse into module body
                    if let Some(body) = find_child_by_kind(node, "declaration_list") {
                        for i in 0..body.child_count() {
                            if let Some(child) = body.child(i) {
                                extract_recursive(
                                    &child,
                                    source,
                                    file,
                                    result,
                                    Some(&qualified),
                                    max_depth - 1,
                                );
                            }
                        }
                    }
                    return;
                }
            }
        }

        "function_item" | "function_signature_item" => {
            // function_item: fn with body
            // function_signature_item: fn declaration in traits (no body)
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Ok(name) = name_node.utf8_text(source) {
                    let qualified = qualified_name(name, parent_path);
                    let visibility = extract_visibility(node, source);
                    let doc = extract_doc_comments(node, source);
                    let signature = extract_function_signature(node, source);

                    result.symbols.push(Symbol {
                        name: name.to_string(),
                        qualified,
                        kind: SymbolKind::Function,
                        location: node_to_location(file, &name_node),
                        visibility,
                        language: "rust".to_string(),
                        parent: None,
                        mixins: None,
                        attributes: extract_attributes(node, source),
                        implements: None,
                        doc,
                        signature,
                    });
                }
            }
        }

        "struct_item" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Ok(name) = name_node.utf8_text(source) {
                    let qualified = qualified_name(name, parent_path);
                    let visibility = extract_visibility(node, source);
                    let doc = extract_doc_comments(node, source);

                    result.symbols.push(Symbol {
                        name: name.to_string(),
                        qualified,
                        kind: SymbolKind::Class, // Using Class for struct
                        location: node_to_location(file, &name_node),
                        visibility,
                        language: "rust".to_string(),
                        parent: None,
                        mixins: None,
                        attributes: extract_attributes(node, source),
                        implements: None,
                        doc,
                        signature: None,
                    });
                }
            }
        }

        "enum_item" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Ok(name) = name_node.utf8_text(source) {
                    let qualified = qualified_name(name, parent_path);
                    let visibility = extract_visibility(node, source);
                    let doc = extract_doc_comments(node, source);

                    result.symbols.push(Symbol {
                        name: name.to_string(),
                        qualified: qualified.clone(),
                        kind: SymbolKind::Union, // Using Union for Rust enums (sum types)
                        location: node_to_location(file, &name_node),
                        visibility,
                        language: "rust".to_string(),
                        parent: None,
                        mixins: None,
                        attributes: extract_attributes(node, source),
                        implements: None,
                        doc,
                        signature: None,
                    });

                    // Extract enum variants
                    if let Some(body) = find_child_by_kind(node, "enum_variant_list") {
                        extract_enum_variants(&body, source, file, result, &qualified);
                    }
                }
            }
        }

        "trait_item" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Ok(name) = name_node.utf8_text(source) {
                    let qualified = qualified_name(name, parent_path);
                    let visibility = extract_visibility(node, source);
                    let doc = extract_doc_comments(node, source);

                    result.symbols.push(Symbol {
                        name: name.to_string(),
                        qualified: qualified.clone(),
                        kind: SymbolKind::Interface, // Using Interface for trait
                        location: node_to_location(file, &name_node),
                        visibility,
                        language: "rust".to_string(),
                        parent: None,
                        mixins: None,
                        attributes: extract_attributes(node, source),
                        implements: None,
                        doc,
                        signature: None,
                    });

                    // Extract trait methods
                    if let Some(body) = find_child_by_kind(node, "declaration_list") {
                        for i in 0..body.child_count() {
                            if let Some(child) = body.child(i) {
                                extract_recursive(
                                    &child,
                                    source,
                                    file,
                                    result,
                                    Some(&qualified),
                                    max_depth - 1,
                                );
                            }
                        }
                    }
                }
            }
        }

        "impl_item" => {
            // impl Foo { ... } or impl Trait for Foo { ... }
            let type_name = extract_impl_type_name(node, source);
            let trait_name = extract_impl_trait_name(node, source);

            if let Some(type_name) = type_name {
                // Build the impl context path
                let impl_path = match parent_path {
                    Some(p) => format!("{}::{}", p, type_name),
                    None => type_name.clone(),
                };

                // Extract methods from impl block
                if let Some(body) = find_child_by_kind(node, "declaration_list") {
                    for i in 0..body.child_count() {
                        if let Some(child) = body.child(i) {
                            if child.kind() == "function_item" {
                                extract_impl_method(
                                    &child,
                                    source,
                                    file,
                                    result,
                                    &impl_path,
                                    trait_name.as_deref(),
                                );
                            } else if child.kind() == "const_item" || child.kind() == "type_item" {
                                extract_recursive(
                                    &child,
                                    source,
                                    file,
                                    result,
                                    Some(&impl_path),
                                    max_depth - 1,
                                );
                            }
                        }
                    }
                }
            }
            return; // Don't recurse normally for impl blocks
        }

        "const_item" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Ok(name) = name_node.utf8_text(source) {
                    let qualified = qualified_name(name, parent_path);
                    let visibility = extract_visibility(node, source);
                    let doc = extract_doc_comments(node, source);

                    result.symbols.push(Symbol {
                        name: name.to_string(),
                        qualified,
                        kind: SymbolKind::Value, // Using Value for const
                        location: node_to_location(file, &name_node),
                        visibility,
                        language: "rust".to_string(),
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

        "static_item" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Ok(name) = name_node.utf8_text(source) {
                    let qualified = qualified_name(name, parent_path);
                    let visibility = extract_visibility(node, source);
                    let doc = extract_doc_comments(node, source);

                    result.symbols.push(Symbol {
                        name: name.to_string(),
                        qualified,
                        kind: SymbolKind::Value, // Using Value for static
                        location: node_to_location(file, &name_node),
                        visibility,
                        language: "rust".to_string(),
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

        "type_item" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Ok(name) = name_node.utf8_text(source) {
                    let qualified = qualified_name(name, parent_path);
                    let visibility = extract_visibility(node, source);
                    let doc = extract_doc_comments(node, source);

                    result.symbols.push(Symbol {
                        name: name.to_string(),
                        qualified,
                        kind: SymbolKind::Type, // Using Type for type alias
                        location: node_to_location(file, &name_node),
                        visibility,
                        language: "rust".to_string(),
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

        "macro_definition" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Ok(name) = name_node.utf8_text(source) {
                    let qualified = qualified_name(name, parent_path);
                    let doc = extract_doc_comments(node, source);

                    result.symbols.push(Symbol {
                        name: name.to_string(),
                        qualified,
                        kind: SymbolKind::Function, // Using Function for macro_rules!
                        location: node_to_location(file, &name_node),
                        visibility: Visibility::Public, // macro_rules! are pub by default in module
                        language: "rust".to_string(),
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

        "use_declaration" => {
            // Extract use statements for imports
            extract_use_statement(node, source, result);
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

/// Extract enum variants
fn extract_enum_variants(
    body: &tree_sitter::Node,
    source: &[u8],
    file: &Path,
    result: &mut ParseResult,
    enum_path: &str,
) {
    for i in 0..body.child_count() {
        if let Some(child) = body.child(i) {
            if child.kind() == "enum_variant" {
                if let Some(name_node) = child.child_by_field_name("name") {
                    if let Ok(name) = name_node.utf8_text(source) {
                        let qualified = format!("{}::{}", enum_path, name);

                        result.symbols.push(Symbol {
                            name: name.to_string(),
                            qualified,
                            kind: SymbolKind::Member, // Using Member for enum variants
                            location: node_to_location(file, &name_node),
                            visibility: Visibility::Public, // Variants inherit enum visibility
                            language: "rust".to_string(),
                            parent: Some(enum_path.to_string()),
                            mixins: None,
                            attributes: None,
                            implements: None,
                            doc: extract_doc_comments(&child, source),
                            signature: None,
                        });
                    }
                }
            }
        }
    }
}

/// Extract the type name from an impl block (e.g., "Foo" from "impl Foo")
fn extract_impl_type_name(node: &tree_sitter::Node, source: &[u8]) -> Option<String> {
    // Look for type_identifier in the impl
    // For "impl Foo { ... }" - the type is direct
    // For "impl Trait for Foo { ... }" - the type is after "for"

    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        let mut found_for = false;
        loop {
            let child = cursor.node();
            let kind = child.kind();

            if kind == "for" {
                found_for = true;
            } else if (kind == "type_identifier"
                || kind == "generic_type"
                || kind == "scoped_type_identifier")
                && (found_for || !has_for_keyword(node, source))
            {
                // This is the type being implemented
                return extract_type_name_from_node(&child, source);
            }

            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
    None
}

/// Check if impl has "for" keyword (impl Trait for Type)
fn has_for_keyword(node: &tree_sitter::Node, _source: &[u8]) -> bool {
    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            if cursor.node().kind() == "for" {
                return true;
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
    false
}

/// Extract type name from a type node (handles generics and paths)
fn extract_type_name_from_node(node: &tree_sitter::Node, source: &[u8]) -> Option<String> {
    match node.kind() {
        "type_identifier" => node.utf8_text(source).ok().map(|s| s.to_string()),
        "generic_type" => {
            // Get the base type name without generic params
            if let Some(type_node) = find_child_by_kind(node, "type_identifier") {
                return type_node.utf8_text(source).ok().map(|s| s.to_string());
            }
            if let Some(scoped) = find_child_by_kind(node, "scoped_type_identifier") {
                return extract_type_name_from_node(&scoped, source);
            }
            None
        }
        "scoped_type_identifier" => {
            // Get the last part of the path (e.g., "Bar" from "foo::Bar")
            let mut cursor = node.walk();
            let mut last_ident = None;
            if cursor.goto_first_child() {
                loop {
                    let child = cursor.node();
                    if child.kind() == "type_identifier" {
                        last_ident = child.utf8_text(source).ok().map(|s| s.to_string());
                    }
                    if !cursor.goto_next_sibling() {
                        break;
                    }
                }
            }
            last_ident
        }
        _ => None,
    }
}

/// Extract trait name from "impl Trait for Type"
fn extract_impl_trait_name(node: &tree_sitter::Node, source: &[u8]) -> Option<String> {
    if !has_for_keyword(node, source) {
        return None;
    }

    // The trait name comes before "for"
    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            let child = cursor.node();
            let kind = child.kind();

            if kind == "for" {
                break; // Stop before "for"
            }

            if kind == "type_identifier"
                || kind == "generic_type"
                || kind == "scoped_type_identifier"
            {
                return extract_type_name_from_node(&child, source);
            }

            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
    None
}

/// Extract a method from an impl block
fn extract_impl_method(
    node: &tree_sitter::Node,
    source: &[u8],
    file: &Path,
    result: &mut ParseResult,
    impl_path: &str,
    _trait_name: Option<&str>,
) {
    if let Some(name_node) = node.child_by_field_name("name") {
        if let Ok(name) = name_node.utf8_text(source) {
            let qualified = format!("{}::{}", impl_path, name);
            let visibility = extract_visibility(node, source);
            let doc = extract_doc_comments(node, source);
            let signature = extract_function_signature(node, source);

            result.symbols.push(Symbol {
                name: name.to_string(),
                qualified,
                kind: SymbolKind::Function,
                location: node_to_location(file, &name_node),
                visibility,
                language: "rust".to_string(),
                parent: Some(impl_path.to_string()),
                mixins: None,
                attributes: extract_attributes(node, source),
                implements: None,
                doc,
                signature,
            });
        }
    }
}

/// Extract use statements
fn extract_use_statement(node: &tree_sitter::Node, source: &[u8], result: &mut ParseResult) {
    // Simple extraction: get the full use path
    if let Some(arg) = node.child_by_field_name("argument") {
        if let Ok(text) = arg.utf8_text(source) {
            // Handle use foo::bar; -> opens "foo::bar"
            // Handle use foo::bar as baz; -> opens "foo::bar"
            // Handle use foo::{a, b}; -> opens "foo::a", "foo::b"

            let text = text.trim();

            // Check for use list: foo::{a, b}
            if text.contains('{') {
                if let Some(brace_pos) = text.find('{') {
                    let prefix = text[..brace_pos].trim_end_matches("::");
                    // Extract items from braces
                    if let Some(end_brace) = text.find('}') {
                        let items = &text[brace_pos + 1..end_brace];
                        for item in items.split(',') {
                            let item = item.trim();
                            if !item.is_empty() {
                                // Handle "self" specially
                                if item == "self" {
                                    result.opens.push(prefix.to_string());
                                } else {
                                    result.opens.push(format!("{}::{}", prefix, item));
                                }
                            }
                        }
                    }
                }
            } else if text.contains(" as ") {
                // use foo::bar as baz -> import foo::bar
                if let Some(as_pos) = text.find(" as ") {
                    result.opens.push(text[..as_pos].trim().to_string());
                }
            } else {
                result.opens.push(text.to_string());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::extract_symbols;

    #[test]
    fn extracts_rust_struct() {
        let source = r#"
/// A point in 2D space.
pub struct Point {
    pub x: i32,
    pub y: i32,
}
"#;
        let result = extract_symbols(std::path::Path::new("test.rs"), source, 100);

        let sym = result
            .symbols
            .iter()
            .find(|s| s.name == "Point")
            .expect("Should find Point");
        assert_eq!(sym.kind, SymbolKind::Class);
        assert_eq!(sym.qualified, "Point");
        assert_eq!(sym.visibility, Visibility::Public);
    }

    #[test]
    fn extracts_rust_function() {
        let source = r#"
/// Adds two numbers.
pub fn add(a: i32, b: i32) -> i32 {
    a + b
}
"#;
        let result = extract_symbols(std::path::Path::new("test.rs"), source, 100);

        let func = result
            .symbols
            .iter()
            .find(|s| s.name == "add")
            .expect("Should find add");
        assert_eq!(func.kind, SymbolKind::Function);
        assert_eq!(func.visibility, Visibility::Public);
        assert!(func.signature.is_some());
    }

    #[test]
    fn extracts_rust_enum() {
        let source = r#"
pub enum Color {
    Red,
    Green,
    Blue,
}
"#;
        let result = extract_symbols(std::path::Path::new("test.rs"), source, 100);

        let enum_sym = result
            .symbols
            .iter()
            .find(|s| s.name == "Color")
            .expect("Should find Color");
        assert_eq!(enum_sym.kind, SymbolKind::Union);

        let red = result
            .symbols
            .iter()
            .find(|s| s.name == "Red")
            .expect("Should find Red variant");
        assert_eq!(red.kind, SymbolKind::Member);
        assert_eq!(red.qualified, "Color::Red");
    }

    #[test]
    fn extracts_rust_trait() {
        let source = r#"
pub trait Display {
    fn fmt(&self) -> String;
}
"#;
        let result = extract_symbols(std::path::Path::new("test.rs"), source, 100);

        let trait_sym = result
            .symbols
            .iter()
            .find(|s| s.name == "Display")
            .expect("Should find Display");
        assert_eq!(trait_sym.kind, SymbolKind::Interface);

        let method = result
            .symbols
            .iter()
            .find(|s| s.name == "fmt")
            .expect("Should find fmt method");
        assert_eq!(method.qualified, "Display::fmt");
    }

    #[test]
    fn extracts_rust_impl_methods() {
        let source = r#"
pub struct Calculator;

impl Calculator {
    pub fn new() -> Self {
        Calculator
    }

    pub fn add(&self, a: i32, b: i32) -> i32 {
        a + b
    }
}
"#;
        let result = extract_symbols(std::path::Path::new("test.rs"), source, 100);

        let new_method = result
            .symbols
            .iter()
            .find(|s| s.name == "new" && s.qualified.contains("Calculator"))
            .expect("Should find Calculator::new");
        assert_eq!(new_method.qualified, "Calculator::new");

        let add_method = result
            .symbols
            .iter()
            .find(|s| s.name == "add" && s.qualified.contains("Calculator"))
            .expect("Should find Calculator::add");
        assert_eq!(add_method.qualified, "Calculator::add");
    }

    #[test]
    fn extracts_rust_module() {
        let source = r#"
pub mod utils {
    pub fn helper() {}
}
"#;
        let result = extract_symbols(std::path::Path::new("test.rs"), source, 100);

        let module = result
            .symbols
            .iter()
            .find(|s| s.name == "utils")
            .expect("Should find utils module");
        assert_eq!(module.kind, SymbolKind::Module);

        let helper = result
            .symbols
            .iter()
            .find(|s| s.name == "helper")
            .expect("Should find helper function");
        assert_eq!(helper.qualified, "utils::helper");
    }

    #[test]
    fn extracts_rust_use_statements() {
        let source = r#"
use std::collections::HashMap;
use std::io::{Read, Write};
use crate::utils;
"#;
        let result = extract_symbols(std::path::Path::new("test.rs"), source, 100);

        assert!(result
            .opens
            .contains(&"std::collections::HashMap".to_string()));
        assert!(result.opens.contains(&"std::io::Read".to_string()));
        assert!(result.opens.contains(&"std::io::Write".to_string()));
        assert!(result.opens.contains(&"crate::utils".to_string()));
    }

    #[test]
    fn extracts_rust_const_and_static() {
        let source = r#"
pub const MAX_SIZE: usize = 100;
pub static COUNTER: AtomicUsize = AtomicUsize::new(0);
"#;
        let result = extract_symbols(std::path::Path::new("test.rs"), source, 100);

        let constant = result
            .symbols
            .iter()
            .find(|s| s.name == "MAX_SIZE")
            .expect("Should find MAX_SIZE");
        assert_eq!(constant.kind, SymbolKind::Value);

        let static_var = result
            .symbols
            .iter()
            .find(|s| s.name == "COUNTER")
            .expect("Should find COUNTER");
        assert_eq!(static_var.kind, SymbolKind::Value);
    }

    #[test]
    fn extracts_rust_type_alias() {
        let source = r#"
pub type Result<T> = std::result::Result<T, MyError>;
"#;
        let result = extract_symbols(std::path::Path::new("test.rs"), source, 100);

        let alias = result
            .symbols
            .iter()
            .find(|s| s.name == "Result")
            .expect("Should find Result type alias");
        assert_eq!(alias.kind, SymbolKind::Type);
    }

    #[test]
    fn extracts_rust_macro() {
        let source = r#"
macro_rules! my_macro {
    () => {};
}
"#;
        let result = extract_symbols(std::path::Path::new("test.rs"), source, 100);

        let mac = result
            .symbols
            .iter()
            .find(|s| s.name == "my_macro")
            .expect("Should find my_macro");
        assert_eq!(mac.kind, SymbolKind::Function);
    }

    #[test]
    fn handles_visibility_modifiers() {
        let source = r#"
pub fn public_fn() {}
fn private_fn() {}
pub(crate) fn crate_fn() {}
pub(super) fn super_fn() {}
"#;
        let result = extract_symbols(std::path::Path::new("test.rs"), source, 100);

        let public = result
            .symbols
            .iter()
            .find(|s| s.name == "public_fn")
            .unwrap();
        assert_eq!(public.visibility, Visibility::Public);

        let private = result
            .symbols
            .iter()
            .find(|s| s.name == "private_fn")
            .unwrap();
        assert_eq!(private.visibility, Visibility::Private);

        let crate_vis = result
            .symbols
            .iter()
            .find(|s| s.name == "crate_fn")
            .unwrap();
        assert_eq!(crate_vis.visibility, Visibility::Internal);
    }

    #[test]
    fn extracts_trait_impl() {
        let source = r#"
pub struct MyType;

impl std::fmt::Display for MyType {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "MyType")
    }
}
"#;
        let result = extract_symbols(std::path::Path::new("test.rs"), source, 100);

        let fmt_method = result
            .symbols
            .iter()
            .find(|s| s.name == "fmt" && s.qualified.contains("MyType"))
            .expect("Should find MyType::fmt");
        assert_eq!(fmt_method.qualified, "MyType::fmt");
    }

    #[test]
    fn extracts_derive_attributes() {
        let source = r#"
#[derive(Debug, Clone, PartialEq)]
pub struct Data {
    value: i32,
}
"#;
        let result = extract_symbols(std::path::Path::new("test.rs"), source, 100);

        let data = result
            .symbols
            .iter()
            .find(|s| s.name == "Data")
            .expect("Should find Data");

        assert!(data.attributes.is_some());
        let attrs = data.attributes.as_ref().unwrap();
        assert!(attrs.iter().any(|a| a.contains("derive")));
    }
}
