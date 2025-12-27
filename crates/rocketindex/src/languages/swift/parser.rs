//! Symbol extraction from Swift source files using tree-sitter.

use std::cell::RefCell;
use std::path::Path;

use crate::parse::{find_child_by_kind, node_to_location, LanguageParser, ParseResult};
use crate::{Reference, Symbol, SymbolKind, Visibility};

// Thread-local parser reuse - avoids creating a new parser per file
thread_local! {
    static SWIFT_PARSER: RefCell<tree_sitter::Parser> = RefCell::new({
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_swift::LANGUAGE.into())
            .expect("tree-sitter-swift grammar incompatible with tree-sitter version");
        parser
    });
}

pub struct SwiftParser;

impl LanguageParser for SwiftParser {
    fn extract_symbols(&self, file: &Path, source: &str, max_depth: usize) -> ParseResult {
        SWIFT_PARSER.with(|parser| {
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

/// Determine visibility from Swift modifiers
fn extract_visibility(node: &tree_sitter::Node, source: &[u8]) -> Visibility {
    if let Some(modifiers) = find_child_by_kind(node, "modifiers") {
        let mut cursor = modifiers.walk();
        if cursor.goto_first_child() {
            loop {
                let child = cursor.node();
                if child.kind() == "visibility_modifier" {
                    if let Ok(text) = child.utf8_text(source) {
                        return match text {
                            "public" | "open" => Visibility::Public,
                            "private" | "fileprivate" => Visibility::Private,
                            "internal" => Visibility::Internal,
                            _ => Visibility::Internal,
                        };
                    }
                }
                if !cursor.goto_next_sibling() {
                    break;
                }
            }
        }
    }
    // Swift defaults to internal visibility
    Visibility::Internal
}

/// Build a qualified name with parent prefix
fn qualified_name(name: &str, parent: Option<&str>) -> String {
    match parent {
        Some(p) => format!("{}.{}", p, name),
        None => name.to_string(),
    }
}

/// Extract documentation comments (/// or /** ... */)
fn extract_doc_comments(node: &tree_sitter::Node, source: &[u8]) -> Option<String> {
    let mut comments = Vec::new();
    let mut prev = node.prev_sibling();

    while let Some(sib) = prev {
        match sib.kind() {
            "comment" => {
                if let Ok(text) = sib.utf8_text(source) {
                    if text.starts_with("///") {
                        let line = text.trim_start_matches("///").trim();
                        comments.push(line.to_string());
                    } else if text.starts_with("/**") {
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
            }
            "multiline_comment" => {
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
                break;
            }
            _ => break,
        }
        prev = sib.prev_sibling();
    }

    if comments.is_empty() {
        None
    } else {
        comments.reverse();
        Some(comments.join("\n"))
    }
}

/// Extract attributes (@available, @objc, etc.)
fn extract_attributes(node: &tree_sitter::Node, source: &[u8]) -> Option<Vec<String>> {
    let mut attributes = Vec::new();

    if let Some(modifiers) = find_child_by_kind(node, "modifiers") {
        let mut cursor = modifiers.walk();
        if cursor.goto_first_child() {
            loop {
                let child = cursor.node();
                if child.kind() == "attribute" {
                    if let Ok(text) = child.utf8_text(source) {
                        attributes.push(text.to_string());
                    }
                }
                if !cursor.goto_next_sibling() {
                    break;
                }
            }
        }
    }

    if attributes.is_empty() {
        None
    } else {
        Some(attributes)
    }
}

/// Extract function signature
fn extract_function_signature(
    node: &tree_sitter::Node,
    source: &[u8],
    name: &str,
) -> Option<String> {
    let mut sig = String::new();
    sig.push_str("func ");
    sig.push_str(name);

    // Get parameters
    if let Some(params) = find_child_by_kind(node, "function_value_parameters") {
        if let Ok(params_text) = params.utf8_text(source) {
            sig.push_str(params_text);
        }
    }

    // Get return type
    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        let mut found_arrow = false;
        loop {
            let child = cursor.node();
            if child.kind() == "->" || child.kind() == "arrow_operator" {
                found_arrow = true;
            } else if found_arrow && child.kind().contains("type") {
                if let Ok(rt) = child.utf8_text(source) {
                    sig.push_str(" -> ");
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

/// Kind of Swift type declaration
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeclarationKind {
    Struct,
    Enum,
    Class,
    Protocol,
}

/// Determine the actual declaration kind by looking for struct/enum/class child
fn determine_declaration_kind(node: &tree_sitter::Node) -> DeclarationKind {
    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            match cursor.node().kind() {
                "struct" => return DeclarationKind::Struct,
                "enum" => return DeclarationKind::Enum,
                "class" => return DeclarationKind::Class,
                "protocol" => return DeclarationKind::Protocol,
                _ => {}
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
    // Default to class if we can't determine
    DeclarationKind::Class
}

/// Extract the name from a declaration node
fn extract_name<'a>(node: &tree_sitter::Node<'a>, source: &'a [u8]) -> Option<&'a str> {
    // Try common field names
    for field in ["name", "identifier"] {
        if let Some(name_node) = node.child_by_field_name(field) {
            if let Ok(name) = name_node.utf8_text(source) {
                return Some(name);
            }
        }
    }

    // Fall back to finding identifier child
    if let Some(id_node) = find_child_by_kind(node, "simple_identifier") {
        if let Ok(name) = id_node.utf8_text(source) {
            return Some(name);
        }
    }

    if let Some(id_node) = find_child_by_kind(node, "identifier") {
        if let Ok(name) = id_node.utf8_text(source) {
            return Some(name);
        }
    }

    None
}

fn extract_recursive(
    node: &tree_sitter::Node,
    source: &[u8],
    file: &Path,
    result: &mut ParseResult,
    parent: Option<&str>,
    max_depth: usize,
) {
    if max_depth == 0 {
        return;
    }

    match node.kind() {
        // In tree-sitter-swift, struct/enum/class all use "class_declaration"
        // with a child node indicating the actual type
        "class_declaration" => {
            // Determine the actual kind by checking for struct/enum/class child
            let actual_kind = determine_declaration_kind(node);

            if let Some(name) = extract_name(node, source) {
                let qualified = qualified_name(name, parent);
                let visibility = extract_visibility(node, source);
                let doc = extract_doc_comments(node, source);
                let attributes = extract_attributes(node, source);

                let symbol_kind = match actual_kind {
                    DeclarationKind::Struct => SymbolKind::Record,
                    DeclarationKind::Enum => SymbolKind::Union,
                    DeclarationKind::Class => SymbolKind::Class,
                    DeclarationKind::Protocol => SymbolKind::Interface,
                };

                result.symbols.push(Symbol {
                    name: name.to_string(),
                    qualified: qualified.clone(),
                    kind: symbol_kind,
                    location: node_to_location(file, node),
                    visibility,
                    language: "swift".to_string(),
                    parent: parent.map(|s| s.to_string()),
                    mixins: None,
                    attributes,
                    implements: None,
                    doc,
                    signature: None,
                });

                // Extract enum cases if this is an enum
                if actual_kind == DeclarationKind::Enum {
                    if let Some(body) = find_child_by_kind(node, "enum_class_body") {
                        extract_enum_cases(&body, source, file, result, &qualified);
                    }
                }

                // Recurse into body (class_body or enum_class_body)
                let body = find_child_by_kind(node, "class_body")
                    .or_else(|| find_child_by_kind(node, "enum_class_body"));
                if let Some(body) = body {
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

        "protocol_declaration" => {
            if let Some(name) = extract_name(node, source) {
                let qualified = qualified_name(name, parent);
                let visibility = extract_visibility(node, source);
                let doc = extract_doc_comments(node, source);
                let attributes = extract_attributes(node, source);

                result.symbols.push(Symbol {
                    name: name.to_string(),
                    qualified: qualified.clone(),
                    kind: SymbolKind::Interface,
                    location: node_to_location(file, node),
                    visibility,
                    language: "swift".to_string(),
                    parent: parent.map(|s| s.to_string()),
                    mixins: None,
                    attributes,
                    implements: None,
                    doc,
                    signature: None,
                });

                // Recurse into protocol body
                if let Some(body) = find_child_by_kind(node, "protocol_body") {
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

        "function_declaration" => {
            if let Some(name) = extract_name(node, source) {
                let qualified = qualified_name(name, parent);
                let visibility = extract_visibility(node, source);
                let doc = extract_doc_comments(node, source);
                let attributes = extract_attributes(node, source);
                let signature = extract_function_signature(node, source, name);

                result.symbols.push(Symbol {
                    name: name.to_string(),
                    qualified,
                    kind: SymbolKind::Function,
                    location: node_to_location(file, node),
                    visibility,
                    language: "swift".to_string(),
                    parent: parent.map(|s| s.to_string()),
                    mixins: None,
                    attributes,
                    implements: None,
                    doc,
                    signature,
                });
            }
        }

        "property_declaration" => {
            if let Some(name) = extract_name(node, source) {
                let qualified = qualified_name(name, parent);
                let visibility = extract_visibility(node, source);
                let doc = extract_doc_comments(node, source);
                let attributes = extract_attributes(node, source);

                result.symbols.push(Symbol {
                    name: name.to_string(),
                    qualified,
                    kind: SymbolKind::Value,
                    location: node_to_location(file, node),
                    visibility,
                    language: "swift".to_string(),
                    parent: parent.map(|s| s.to_string()),
                    mixins: None,
                    attributes,
                    implements: None,
                    doc,
                    signature: None,
                });
            }
        }

        "typealias_declaration" => {
            if let Some(name) = extract_name(node, source) {
                let qualified = qualified_name(name, parent);
                let visibility = extract_visibility(node, source);
                let doc = extract_doc_comments(node, source);

                result.symbols.push(Symbol {
                    name: name.to_string(),
                    qualified,
                    kind: SymbolKind::Type,
                    location: node_to_location(file, node),
                    visibility,
                    language: "swift".to_string(),
                    parent: parent.map(|s| s.to_string()),
                    mixins: None,
                    attributes: None,
                    implements: None,
                    doc,
                    signature: None,
                });
            }
        }

        "extension_declaration" => {
            // Extensions add methods to existing types
            // We extract the extended type name and use it as the parent
            if let Some(type_node) = node.child_by_field_name("name") {
                if let Ok(type_name) = type_node.utf8_text(source) {
                    // Recurse into extension body with the extended type as parent
                    if let Some(body) = find_child_by_kind(node, "class_body") {
                        let mut cursor = body.walk();
                        if cursor.goto_first_child() {
                            loop {
                                extract_recursive(
                                    &cursor.node(),
                                    source,
                                    file,
                                    result,
                                    Some(type_name),
                                    max_depth - 1,
                                );
                                if !cursor.goto_next_sibling() {
                                    break;
                                }
                            }
                        }
                    }
                }
            }
            return;
        }

        "import_declaration" => {
            // Extract import path
            let mut cursor = node.walk();
            if cursor.goto_first_child() {
                loop {
                    let child = cursor.node();
                    if child.kind() == "identifier" || child.kind() == "simple_identifier" {
                        if let Ok(text) = child.utf8_text(source) {
                            result.opens.push(text.to_string());
                        }
                    }
                    if !cursor.goto_next_sibling() {
                        break;
                    }
                }
            }
        }

        // Extract type references
        "user_type" | "type_identifier" => {
            if let Some(id) = find_child_by_kind(node, "simple_identifier") {
                if let Ok(name) = id.utf8_text(source) {
                    result.references.push(Reference {
                        name: name.to_string(),
                        location: node_to_location(file, &id),
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
            extract_recursive(&cursor.node(), source, file, result, parent, max_depth - 1);
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
}

/// Extract enum cases
fn extract_enum_cases(
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
                if let Some(name) = extract_name(&child, source) {
                    let qualified = format!("{}.{}", enum_qualified, name);
                    result.symbols.push(Symbol {
                        name: name.to_string(),
                        qualified,
                        kind: SymbolKind::Member,
                        location: node_to_location(file, &child),
                        visibility: Visibility::Public,
                        language: "swift".to_string(),
                        parent: Some(enum_qualified.to_string()),
                        mixins: None,
                        attributes: None,
                        implements: None,
                        doc: extract_doc_comments(&child, source),
                        signature: None,
                    });
                }
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::LanguageParser;

    #[test]
    #[ignore]
    fn debug_swift_ast() {
        let source = r#"
struct Point {
    var x: Int
}

enum Status {
    case pending
    case active
}

class User {
    var name: String = ""
}
"#;
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_swift::LANGUAGE.into())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();

        fn print_tree(node: &tree_sitter::Node, indent: usize) {
            let prefix = " ".repeat(indent);
            println!("{}kind={:?}", prefix, node.kind());
            let mut cursor = node.walk();
            if cursor.goto_first_child() {
                loop {
                    print_tree(&cursor.node(), indent + 2);
                    if !cursor.goto_next_sibling() {
                        break;
                    }
                }
            }
        }
        print_tree(&tree.root_node(), 0);
    }

    #[test]
    fn extracts_swift_class() {
        let source = r#"
/// A simple user class.
class User {
    var name: String = ""
}
"#;
        let parser = SwiftParser;
        let result = parser.extract_symbols(std::path::Path::new("User.swift"), source, 100);

        let class_sym = result
            .symbols
            .iter()
            .find(|s| s.name == "User")
            .expect("Should find User class");
        assert_eq!(class_sym.kind, SymbolKind::Class);
        assert_eq!(class_sym.qualified, "User");
    }

    #[test]
    fn extracts_swift_struct() {
        let source = r#"
struct Point {
    var x: Int
    var y: Int
}
"#;
        let parser = SwiftParser;
        let result = parser.extract_symbols(std::path::Path::new("Point.swift"), source, 100);

        let struct_sym = result
            .symbols
            .iter()
            .find(|s| s.name == "Point")
            .expect("Should find Point struct");
        assert_eq!(struct_sym.kind, SymbolKind::Record);
    }

    #[test]
    fn extracts_swift_protocol() {
        let source = r#"
protocol Repository {
    func findById(id: Int) -> Any?
}
"#;
        let parser = SwiftParser;
        let result = parser.extract_symbols(std::path::Path::new("Repository.swift"), source, 100);

        let protocol = result
            .symbols
            .iter()
            .find(|s| s.name == "Repository")
            .expect("Should find Repository protocol");
        assert_eq!(protocol.kind, SymbolKind::Interface);
    }

    #[test]
    fn extracts_swift_function() {
        let source = r#"
/// Adds two numbers.
func add(a: Int, b: Int) -> Int {
    return a + b
}
"#;
        let parser = SwiftParser;
        let result = parser.extract_symbols(std::path::Path::new("Math.swift"), source, 100);

        let func = result
            .symbols
            .iter()
            .find(|s| s.name == "add")
            .expect("Should find add function");
        assert_eq!(func.kind, SymbolKind::Function);
    }

    #[test]
    fn extracts_swift_enum() {
        let source = r#"
enum Status {
    case pending
    case active
    case completed
}
"#;
        let parser = SwiftParser;
        let result = parser.extract_symbols(std::path::Path::new("Status.swift"), source, 100);

        let enum_sym = result
            .symbols
            .iter()
            .find(|s| s.name == "Status")
            .expect("Should find Status enum");
        assert_eq!(enum_sym.kind, SymbolKind::Union);
    }

    #[test]
    fn extracts_swift_property() {
        let source = r#"
var globalCounter: Int = 0
"#;
        let parser = SwiftParser;
        let result = parser.extract_symbols(std::path::Path::new("Config.swift"), source, 100);

        let prop = result
            .symbols
            .iter()
            .find(|s| s.name == "globalCounter")
            .expect("Should find globalCounter property");
        assert_eq!(prop.kind, SymbolKind::Value);
    }

    #[test]
    fn extracts_swift_typealias() {
        let source = r#"
typealias StringList = [String]
"#;
        let parser = SwiftParser;
        let result = parser.extract_symbols(std::path::Path::new("Types.swift"), source, 100);

        let alias = result
            .symbols
            .iter()
            .find(|s| s.name == "StringList")
            .expect("Should find StringList typealias");
        assert_eq!(alias.kind, SymbolKind::Type);
    }
}
