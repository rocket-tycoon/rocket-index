//! Symbol extraction from C++ source files using tree-sitter.

use std::cell::RefCell;
use std::path::Path;

use crate::parse::{find_child_by_kind, node_to_location, LanguageParser, ParseResult};
use crate::{Symbol, SymbolKind, Visibility};

// Thread-local parser reuse - avoids creating a new parser per file
thread_local! {
    static CPP_PARSER: RefCell<tree_sitter::Parser> = RefCell::new({
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_cpp::LANGUAGE.into())
            .expect("tree-sitter-cpp grammar incompatible with tree-sitter version");
        parser
    });
}

pub struct CppParser;

impl LanguageParser for CppParser {
    fn extract_symbols(&self, file: &Path, source: &str, max_depth: usize) -> ParseResult {
        CPP_PARSER.with(|parser| {
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

/// Build qualified name with :: separator
fn qualified_name(name: &str, parent_path: Option<&str>) -> String {
    match parent_path {
        Some(p) => format!("{}::{}", p, name),
        None => name.to_string(),
    }
}

/// Extract visibility from access specifier in class/struct
fn extract_visibility_from_specifier(node: &tree_sitter::Node, source: &[u8]) -> Visibility {
    // The access_specifier node contains a child with the actual keyword
    // Check children for public/protected/private
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            match child.kind() {
                "public" => return Visibility::Public,
                "protected" => return Visibility::Internal,
                "private" => return Visibility::Private,
                _ => {}
            }
        }
    }
    // Fallback: check node text
    if let Ok(text) = node.utf8_text(source) {
        let text = text.trim().trim_end_matches(':');
        match text {
            "public" => return Visibility::Public,
            "protected" => return Visibility::Internal,
            "private" => return Visibility::Private,
            _ => {}
        }
    }
    Visibility::Private // Default for C++ classes
}

/// Extract doc comments from preceding comment nodes
fn extract_doc_comments(node: &tree_sitter::Node, source: &[u8]) -> Option<String> {
    let mut docs = Vec::new();

    // Look for preceding comment nodes
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

    docs.reverse();

    if docs.is_empty() {
        None
    } else {
        Some(docs.join("\n"))
    }
}

/// Extract function signature
fn extract_function_signature(node: &tree_sitter::Node, source: &[u8]) -> Option<String> {
    let start = node.start_byte();
    if let Some(body) = find_child_by_kind(node, "compound_statement") {
        let end = body.start_byte();
        if end > start {
            if let Ok(sig) = std::str::from_utf8(&source[start..end]) {
                return Some(sig.trim().to_string());
            }
        }
    }
    None
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
        "namespace_definition" => {
            // namespace Name { ... }
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Ok(name) = name_node.utf8_text(source) {
                    let qualified = qualified_name(name, parent_path);
                    let doc = extract_doc_comments(node, source);

                    result.symbols.push(Symbol {
                        name: name.to_string(),
                        qualified: qualified.clone(),
                        kind: SymbolKind::Module,
                        location: node_to_location(file, &name_node),
                        visibility: Visibility::Public,
                        language: "cpp".to_string(),
                        parent: None,
                        mixins: None,
                        attributes: None,
                        implements: None,
                        doc,
                        signature: None,
                    });

                    // Recurse into namespace body
                    if let Some(body) = node.child_by_field_name("body") {
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

        "function_definition" => {
            if let Some(declarator) = node.child_by_field_name("declarator") {
                if let Some(name) = extract_declarator_name(&declarator, source) {
                    let qualified = qualified_name(&name, parent_path);
                    let doc = extract_doc_comments(node, source);
                    let signature = extract_function_signature(node, source);

                    result.symbols.push(Symbol {
                        name: name.clone(),
                        qualified,
                        kind: SymbolKind::Function,
                        location: node_to_location(file, &declarator),
                        visibility: Visibility::Public,
                        language: "cpp".to_string(),
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

        "template_declaration" => {
            // template<...> class/struct/function
            // Extract the underlying declaration
            for i in 0..node.child_count() {
                if let Some(child) = node.child(i) {
                    match child.kind() {
                        "class_specifier" => {
                            extract_class_or_struct(
                                &child,
                                source,
                                file,
                                result,
                                parent_path,
                                max_depth,
                                true,
                                false,
                            );
                            return;
                        }
                        "struct_specifier" => {
                            extract_class_or_struct(
                                &child,
                                source,
                                file,
                                result,
                                parent_path,
                                max_depth,
                                true,
                                true,
                            );
                            return;
                        }
                        "function_definition" => {
                            if let Some(declarator) = child.child_by_field_name("declarator") {
                                if let Some(name) = extract_declarator_name(&declarator, source) {
                                    let qualified = qualified_name(&name, parent_path);
                                    let doc = extract_doc_comments(node, source);

                                    result.symbols.push(Symbol {
                                        name: name.clone(),
                                        qualified,
                                        kind: SymbolKind::Function,
                                        location: node_to_location(file, &declarator),
                                        visibility: Visibility::Public,
                                        language: "cpp".to_string(),
                                        parent: None,
                                        mixins: None,
                                        attributes: Some(vec!["template".to_string()]),
                                        implements: None,
                                        doc,
                                        signature: None,
                                    });
                                }
                            }
                            return;
                        }
                        _ => {}
                    }
                }
            }
        }

        "class_specifier" => {
            extract_class_or_struct(
                node,
                source,
                file,
                result,
                parent_path,
                max_depth,
                false,
                false,
            );
            return;
        }

        "struct_specifier" => {
            extract_class_or_struct(
                node,
                source,
                file,
                result,
                parent_path,
                max_depth,
                false,
                true,
            );
            return;
        }

        "enum_specifier" => {
            // enum Name { ... } or enum class Name { ... }
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Ok(name) = name_node.utf8_text(source) {
                    let qualified = qualified_name(name, parent_path);
                    let doc = extract_doc_comments(node, source);

                    // Check if it's an enum class
                    let mut is_scoped = false;
                    for i in 0..node.child_count() {
                        if let Some(child) = node.child(i) {
                            if child.kind() == "class" || child.kind() == "struct" {
                                is_scoped = true;
                                break;
                            }
                        }
                    }

                    result.symbols.push(Symbol {
                        name: name.to_string(),
                        qualified: qualified.clone(),
                        kind: SymbolKind::Union,
                        location: node_to_location(file, &name_node),
                        visibility: Visibility::Public,
                        language: "cpp".to_string(),
                        parent: None,
                        mixins: None,
                        attributes: if is_scoped {
                            Some(vec!["scoped".to_string()])
                        } else {
                            None
                        },
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

        "declaration" => {
            // Variable or function declaration
            if let Some(declarator) = node.child_by_field_name("declarator") {
                let is_function = is_function_declarator(&declarator);
                if let Some(name) = extract_declarator_name(&declarator, source) {
                    let qualified = qualified_name(&name, parent_path);
                    let doc = extract_doc_comments(node, source);

                    result.symbols.push(Symbol {
                        name,
                        qualified,
                        kind: if is_function {
                            SymbolKind::Function
                        } else {
                            SymbolKind::Value
                        },
                        location: node_to_location(file, &declarator),
                        visibility: Visibility::Public,
                        language: "cpp".to_string(),
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

        "using_declaration" => {
            // using namespace std; or using std::cout;
            if let Ok(text) = node.utf8_text(source) {
                let clean = text
                    .trim_start_matches("using")
                    .trim()
                    .trim_end_matches(';')
                    .trim();
                if !clean.is_empty() {
                    result.opens.push(clean.to_string());
                }
            }
        }

        "preproc_include" => {
            // #include <header> or #include "header"
            if let Some(path_node) = node.child_by_field_name("path") {
                if let Ok(path) = path_node.utf8_text(source) {
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

/// Extract class or struct definition
#[allow(clippy::too_many_arguments)]
fn extract_class_or_struct(
    node: &tree_sitter::Node,
    source: &[u8],
    file: &Path,
    result: &mut ParseResult,
    parent_path: Option<&str>,
    max_depth: usize,
    is_template: bool,
    is_struct: bool,
) {
    if let Some(name_node) = node.child_by_field_name("name") {
        if let Ok(name) = name_node.utf8_text(source) {
            let qualified = qualified_name(name, parent_path);
            let doc = extract_doc_comments(node, source);

            // Extract base classes
            let bases = extract_base_classes(node, source);

            result.symbols.push(Symbol {
                name: name.to_string(),
                qualified: qualified.clone(),
                kind: SymbolKind::Class,
                location: node_to_location(file, &name_node),
                visibility: Visibility::Public,
                language: "cpp".to_string(),
                parent: None,
                mixins: None,
                attributes: if is_template {
                    Some(vec!["template".to_string()])
                } else {
                    None
                },
                implements: if bases.is_empty() { None } else { Some(bases) },
                doc,
                signature: None,
            });

            // Extract class members
            if let Some(body) = node.child_by_field_name("body") {
                extract_class_members(
                    &body, source, file, result, &qualified, max_depth, is_struct,
                );
            }
        }
    }
}

/// Extract base classes from a class specifier
fn extract_base_classes(node: &tree_sitter::Node, source: &[u8]) -> Vec<String> {
    let mut bases = Vec::new();

    // Look for base_class_clause
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            if child.kind() == "base_class_clause" {
                for j in 0..child.child_count() {
                    if let Some(base_spec) = child.child(j) {
                        if let Ok(text) = base_spec.utf8_text(source) {
                            let base = text
                                .trim_start_matches("public")
                                .trim_start_matches("protected")
                                .trim_start_matches("private")
                                .trim_start_matches("virtual")
                                .trim()
                                .to_string();
                            if !base.is_empty() && base != ":" && base != "," {
                                bases.push(base);
                            }
                        }
                    }
                }
            }
        }
    }

    bases
}

/// Extract class/struct members
fn extract_class_members(
    body: &tree_sitter::Node,
    source: &[u8],
    file: &Path,
    result: &mut ParseResult,
    parent_qualified: &str,
    max_depth: usize,
    is_struct: bool,
) {
    // struct defaults to public, class defaults to private
    let mut current_visibility = if is_struct {
        Visibility::Public
    } else {
        Visibility::Private
    };

    for i in 0..body.child_count() {
        if let Some(child) = body.child(i) {
            match child.kind() {
                "access_specifier" => {
                    current_visibility = extract_visibility_from_specifier(&child, source);
                }

                "function_definition" => {
                    if let Some(declarator) = child.child_by_field_name("declarator") {
                        if let Some(name) = extract_declarator_name(&declarator, source) {
                            let qualified = format!("{}::{}", parent_qualified, name);
                            let doc = extract_doc_comments(&child, source);
                            let signature = extract_function_signature(&child, source);

                            result.symbols.push(Symbol {
                                name,
                                qualified,
                                kind: SymbolKind::Function,
                                location: node_to_location(file, &declarator),
                                visibility: current_visibility,
                                language: "cpp".to_string(),
                                parent: Some(parent_qualified.to_string()),
                                mixins: None,
                                attributes: None,
                                implements: None,
                                doc,
                                signature,
                            });
                        }
                    }
                }

                "field_declaration" => {
                    if let Some(declarator) = child.child_by_field_name("declarator") {
                        if let Some(name) = extract_declarator_name(&declarator, source) {
                            let qualified = format!("{}::{}", parent_qualified, name);
                            // Check if it's a function declaration
                            let kind = if is_function_declarator(&declarator) {
                                SymbolKind::Function
                            } else {
                                SymbolKind::Member
                            };

                            result.symbols.push(Symbol {
                                name,
                                qualified,
                                kind,
                                location: node_to_location(file, &declarator),
                                visibility: current_visibility,
                                language: "cpp".to_string(),
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

                "declaration" => {
                    // Method declarations: void draw();
                    if let Some(declarator) = child.child_by_field_name("declarator") {
                        if let Some(name) = extract_declarator_name(&declarator, source) {
                            let qualified = format!("{}::{}", parent_qualified, name);
                            let kind = if is_function_declarator(&declarator) {
                                SymbolKind::Function
                            } else {
                                SymbolKind::Member
                            };

                            result.symbols.push(Symbol {
                                name,
                                qualified,
                                kind,
                                location: node_to_location(file, &declarator),
                                visibility: current_visibility,
                                language: "cpp".to_string(),
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

                // Nested class/struct
                "class_specifier" => {
                    extract_class_or_struct(
                        &child,
                        source,
                        file,
                        result,
                        Some(parent_qualified),
                        max_depth,
                        false,
                        false,
                    );
                }
                "struct_specifier" => {
                    extract_class_or_struct(
                        &child,
                        source,
                        file,
                        result,
                        Some(parent_qualified),
                        max_depth,
                        false,
                        true,
                    );
                }

                _ => {}
            }
        }
    }
}

/// Extract the name from a declarator
fn extract_declarator_name(node: &tree_sitter::Node, source: &[u8]) -> Option<String> {
    match node.kind() {
        "identifier" | "field_identifier" => node.utf8_text(source).ok().map(|s| s.to_string()),
        "destructor_name" | "operator_name" => node.utf8_text(source).ok().map(|s| s.to_string()),
        "function_declarator" => node
            .child_by_field_name("declarator")
            .and_then(|d| extract_declarator_name(&d, source)),
        "pointer_declarator" | "reference_declarator" => node
            .child_by_field_name("declarator")
            .and_then(|d| extract_declarator_name(&d, source)),
        "qualified_identifier" => {
            // Get the last part of qualified name for the symbol name
            if let Some(name_node) = node.child_by_field_name("name") {
                extract_declarator_name(&name_node, source)
            } else {
                None
            }
        }
        "template_function" => node
            .child_by_field_name("name")
            .and_then(|n| extract_declarator_name(&n, source)),
        "parenthesized_declarator" => {
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
        "pointer_declarator" | "reference_declarator" | "parenthesized_declarator" => {
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
                            language: "cpp".to_string(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::LanguageParser;

    #[test]
    fn extracts_cpp_function() {
        let source = r#"
int add(int a, int b) {
    return a + b;
}
"#;
        let parser = CppParser;
        let result = parser.extract_symbols(Path::new("test.cpp"), source, 100);

        assert_eq!(result.symbols.len(), 1);
        assert_eq!(result.symbols[0].name, "add");
        assert_eq!(result.symbols[0].kind, SymbolKind::Function);
    }

    #[test]
    fn extracts_cpp_class() {
        let source = r#"
class Widget {
public:
    void draw();
    int width;
private:
    int height;
};
"#;
        let parser = CppParser;
        let result = parser.extract_symbols(Path::new("test.cpp"), source, 100);

        let widget = result.symbols.iter().find(|s| s.name == "Widget").unwrap();
        assert_eq!(widget.kind, SymbolKind::Class);

        let draw = result.symbols.iter().find(|s| s.name == "draw");
        assert!(draw.is_some());

        let width = result.symbols.iter().find(|s| s.name == "width").unwrap();
        assert_eq!(width.visibility, Visibility::Public);

        let height = result.symbols.iter().find(|s| s.name == "height").unwrap();
        assert_eq!(height.visibility, Visibility::Private);
    }

    #[test]
    fn extracts_cpp_namespace() {
        let source = r#"
namespace utils {
    void helper() {}
}
"#;
        let parser = CppParser;
        let result = parser.extract_symbols(Path::new("test.cpp"), source, 100);

        let ns = result.symbols.iter().find(|s| s.name == "utils").unwrap();
        assert_eq!(ns.kind, SymbolKind::Module);

        let helper = result.symbols.iter().find(|s| s.name == "helper").unwrap();
        assert_eq!(helper.qualified, "utils::helper");
    }

    #[test]
    fn extracts_cpp_enum_class() {
        let source = r#"
enum class Color {
    RED,
    GREEN,
    BLUE
};
"#;
        let parser = CppParser;
        let result = parser.extract_symbols(Path::new("test.cpp"), source, 100);

        let color = result.symbols.iter().find(|s| s.name == "Color").unwrap();
        assert_eq!(color.kind, SymbolKind::Union);
        assert!(color
            .attributes
            .as_ref()
            .is_some_and(|a| a.contains(&"scoped".to_string())));

        let red = result.symbols.iter().find(|s| s.name == "RED").unwrap();
        assert_eq!(red.qualified, "Color::RED");
    }

    #[test]
    fn extracts_cpp_template_class() {
        let source = r#"
template<typename T>
class Container {
public:
    T value;
};
"#;
        let parser = CppParser;
        let result = parser.extract_symbols(Path::new("test.cpp"), source, 100);

        let container = result
            .symbols
            .iter()
            .find(|s| s.name == "Container")
            .unwrap();
        assert_eq!(container.kind, SymbolKind::Class);
        assert!(container
            .attributes
            .as_ref()
            .is_some_and(|a| a.contains(&"template".to_string())));
    }

    #[test]
    fn extracts_cpp_inheritance() {
        let source = r#"
class Base {};

class Derived : public Base {
};
"#;
        let parser = CppParser;
        let result = parser.extract_symbols(Path::new("test.cpp"), source, 100);

        let derived = result.symbols.iter().find(|s| s.name == "Derived").unwrap();
        assert!(derived.implements.is_some());
    }

    #[test]
    fn extracts_cpp_includes() {
        let source = r#"
#include <iostream>
#include "myheader.hpp"
"#;
        let parser = CppParser;
        let result = parser.extract_symbols(Path::new("test.cpp"), source, 100);

        assert!(result.opens.contains(&"iostream".to_string()));
        assert!(result.opens.contains(&"myheader.hpp".to_string()));
    }

    #[test]
    fn extracts_cpp_struct() {
        let source = r#"
struct Point {
    int x;
    int y;
};
"#;
        let parser = CppParser;
        let result = parser.extract_symbols(Path::new("test.cpp"), source, 100);

        let point = result.symbols.iter().find(|s| s.name == "Point").unwrap();
        assert_eq!(point.kind, SymbolKind::Class);

        let x = result.symbols.iter().find(|s| s.name == "x").unwrap();
        assert_eq!(x.qualified, "Point::x");
    }

    #[test]
    fn struct_members_default_to_public() {
        let source = r#"
struct Data {
    int value;      // should be public (struct default)
    void process(); // should be public
private:
    int secret;     // should be private
};
"#;
        let parser = CppParser;
        let result = parser.extract_symbols(Path::new("test.cpp"), source, 100);

        let value = result.symbols.iter().find(|s| s.name == "value").unwrap();
        assert_eq!(value.visibility, Visibility::Public); // struct defaults to public

        let process = result.symbols.iter().find(|s| s.name == "process").unwrap();
        assert_eq!(process.visibility, Visibility::Public);

        let secret = result.symbols.iter().find(|s| s.name == "secret").unwrap();
        assert_eq!(secret.visibility, Visibility::Private);
    }

    #[test]
    fn class_members_default_to_private() {
        let source = r#"
class Widget {
    int hidden;     // should be private (class default)
public:
    int visible;    // should be public
};
"#;
        let parser = CppParser;
        let result = parser.extract_symbols(Path::new("test.cpp"), source, 100);

        let hidden = result.symbols.iter().find(|s| s.name == "hidden").unwrap();
        assert_eq!(hidden.visibility, Visibility::Private); // class defaults to private

        let visible = result.symbols.iter().find(|s| s.name == "visible").unwrap();
        assert_eq!(visible.visibility, Visibility::Public);
    }
}
