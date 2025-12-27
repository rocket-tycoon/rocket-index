//! Symbol extraction from Objective-C source files using tree-sitter.

use std::cell::RefCell;
use std::path::Path;

use crate::parse::{find_child_by_kind, node_to_location, LanguageParser, ParseResult};
use crate::{Reference, Symbol, SymbolKind, Visibility};

thread_local! {
    static OBJC_PARSER: RefCell<tree_sitter::Parser> = RefCell::new({
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_objc::LANGUAGE.into())
            .expect("tree-sitter-objc grammar incompatible with tree-sitter version");
        parser
    });
}

pub struct ObjCParser;

impl LanguageParser for ObjCParser {
    fn extract_symbols(&self, file: &Path, source: &str, max_depth: usize) -> ParseResult {
        OBJC_PARSER.with(|parser| {
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

/// Build a qualified name
fn qualified_name(name: &str, parent: Option<&str>) -> String {
    match parent {
        Some(p) => format!("{}.{}", p, name),
        None => name.to_string(),
    }
}

/// Extract doc comments (/** ... */ or ///)
fn extract_doc_comments(node: &tree_sitter::Node, source: &[u8]) -> Option<String> {
    let mut prev = node.prev_sibling();
    while let Some(sib) = prev {
        let kind = sib.kind();
        if kind == "comment" {
            if let Ok(text) = sib.utf8_text(source) {
                if text.starts_with("///") || text.starts_with("/**") {
                    let cleaned = text
                        .lines()
                        .map(|line| {
                            line.trim()
                                .trim_start_matches("///")
                                .trim_start_matches("/**")
                                .trim_start_matches("*/")
                                .trim_start_matches('*')
                                .trim()
                        })
                        .filter(|line| !line.is_empty())
                        .collect::<Vec<_>>()
                        .join("\n");
                    if !cleaned.is_empty() {
                        return Some(cleaned);
                    }
                }
            }
        } else {
            break;
        }
        prev = sib.prev_sibling();
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
        // @interface ClassName : ParentClass
        "class_interface" => {
            if let Some(name_node) = find_child_by_kind(node, "identifier") {
                if let Ok(name) = name_node.utf8_text(source) {
                    let qualified = qualified_name(name, None);
                    let doc = extract_doc_comments(node, source);

                    result.symbols.push(Symbol {
                        name: name.to_string(),
                        qualified: qualified.clone(),
                        kind: SymbolKind::Class,
                        location: node_to_location(file, &name_node),
                        visibility: Visibility::Public,
                        language: "objc".to_string(),
                        parent: None,
                        mixins: None,
                        attributes: None,
                        implements: None,
                        doc,
                        signature: None,
                    });

                    // Recurse into body to find methods/properties
                    extract_class_body(node, source, file, result, &qualified, max_depth - 1);
                    return;
                }
            }
        }

        // @implementation ClassName
        "class_implementation" => {
            if let Some(name_node) = find_child_by_kind(node, "identifier") {
                if let Ok(name) = name_node.utf8_text(source) {
                    let qualified = qualified_name(name, None);

                    // Implementation provides method bodies
                    extract_class_body(node, source, file, result, &qualified, max_depth - 1);
                    return;
                }
            }
        }

        // @protocol ProtocolName
        "protocol_declaration" => {
            if let Some(name_node) = find_child_by_kind(node, "identifier") {
                if let Ok(name) = name_node.utf8_text(source) {
                    let qualified = qualified_name(name, None);
                    let doc = extract_doc_comments(node, source);

                    result.symbols.push(Symbol {
                        name: name.to_string(),
                        qualified: qualified.clone(),
                        kind: SymbolKind::Interface,
                        location: node_to_location(file, &name_node),
                        visibility: Visibility::Public,
                        language: "objc".to_string(),
                        parent: None,
                        mixins: None,
                        attributes: None,
                        implements: None,
                        doc,
                        signature: None,
                    });

                    extract_class_body(node, source, file, result, &qualified, max_depth - 1);
                    return;
                }
            }
        }

        // @interface ClassName (CategoryName)
        "category_interface" => {
            // Get class name and category name
            let mut identifiers = Vec::new();
            let mut cursor = node.walk();
            if cursor.goto_first_child() {
                loop {
                    if cursor.node().kind() == "identifier" {
                        if let Ok(name) = cursor.node().utf8_text(source) {
                            identifiers.push((name.to_string(), cursor.node()));
                        }
                    }
                    if !cursor.goto_next_sibling() {
                        break;
                    }
                }
            }

            if identifiers.len() >= 2 {
                let class_name = &identifiers[0].0;
                let category_name = &identifiers[1].0;
                let qualified = format!("{}({})", class_name, category_name);

                result.symbols.push(Symbol {
                    name: category_name.clone(),
                    qualified: qualified.clone(),
                    kind: SymbolKind::Class,
                    location: node_to_location(file, &identifiers[1].1),
                    visibility: Visibility::Public,
                    language: "objc".to_string(),
                    parent: Some(class_name.clone()),
                    mixins: None,
                    attributes: None,
                    implements: None,
                    doc: extract_doc_comments(node, source),
                    signature: None,
                });

                extract_class_body(node, source, file, result, &qualified, max_depth - 1);
                return;
            }
        }

        // #import "file.h" or #import <Framework/Header.h>
        "preproc_include" | "preproc_import" | "import_declaration" => {
            // Extract import path from various formats
            if let Some(string) = find_child_by_kind(node, "string_literal") {
                if let Ok(text) = string.utf8_text(source) {
                    let path = text.trim_matches('"').trim_matches('<').trim_matches('>');
                    result.opens.push(path.to_string());
                }
            }
            if let Some(system) = find_child_by_kind(node, "system_lib_string") {
                if let Ok(text) = system.utf8_text(source) {
                    let path = text.trim_matches('<').trim_matches('>');
                    result.opens.push(path.to_string());
                }
            }
        }

        // C function declarations
        "function_definition" | "declaration" => {
            // Look for function declarator
            if let Some(declarator) = find_child_by_kind(node, "function_declarator") {
                if let Some(name_node) = find_child_by_kind(&declarator, "identifier") {
                    if let Ok(name) = name_node.utf8_text(source) {
                        let qualified = qualified_name(name, parent_path);
                        let doc = extract_doc_comments(node, source);

                        result.symbols.push(Symbol {
                            name: name.to_string(),
                            qualified,
                            kind: SymbolKind::Function,
                            location: node_to_location(file, &name_node),
                            visibility: Visibility::Public,
                            language: "objc".to_string(),
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

        // typedef
        "type_definition" => {
            // Get the defined type name (last identifier usually)
            let mut last_id = None;
            let mut cursor = node.walk();
            if cursor.goto_first_child() {
                loop {
                    if cursor.node().kind() == "type_identifier"
                        || cursor.node().kind() == "identifier"
                    {
                        last_id = Some(cursor.node());
                    }
                    if !cursor.goto_next_sibling() {
                        break;
                    }
                }
            }

            if let Some(name_node) = last_id {
                if let Ok(name) = name_node.utf8_text(source) {
                    let qualified = qualified_name(name, parent_path);
                    let doc = extract_doc_comments(node, source);

                    result.symbols.push(Symbol {
                        name: name.to_string(),
                        qualified,
                        kind: SymbolKind::Type,
                        location: node_to_location(file, &name_node),
                        visibility: Visibility::Public,
                        language: "objc".to_string(),
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

        // Message expression: [receiver method:arg]
        // Extract reference to the method being called
        "message_expression" => {
            extract_message_reference(node, source, file, result);
        }

        // Function call: functionName(args)
        "call_expression" => {
            if let Some(func_node) = node.child(0) {
                if func_node.kind() == "identifier" {
                    if let Ok(name) = func_node.utf8_text(source) {
                        result.references.push(Reference {
                            name: name.to_string(),
                            location: node_to_location(file, &func_node),
                        });
                    }
                }
            }
        }

        // Identifier in reference context
        "identifier" => {
            if is_reference_context(node) {
                if let Ok(name) = node.utf8_text(source) {
                    result.references.push(Reference {
                        name: name.to_string(),
                        location: node_to_location(file, node),
                    });
                }
            }
        }

        // Type references
        "type_identifier" => {
            // Type identifiers in type contexts are references
            if let Ok(name) = node.utf8_text(source) {
                result.references.push(Reference {
                    name: name.to_string(),
                    location: node_to_location(file, node),
                });
            }
        }

        _ => {}
    }

    // Recurse into children
    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            extract_recursive(
                &cursor.node(),
                source,
                file,
                result,
                parent_path,
                max_depth - 1,
            );
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
}

/// Extract methods and properties from a class/protocol body
fn extract_class_body(
    node: &tree_sitter::Node,
    source: &[u8],
    file: &Path,
    result: &mut ParseResult,
    class_qualified: &str,
    max_depth: usize,
) {
    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            let child = cursor.node();
            match child.kind() {
                // Method declaration/definition: - (ReturnType)methodName:(Type)param
                "method_declaration"
                | "method_definition"
                | "class_method_declaration"
                | "instance_method_declaration" => {
                    extract_method(&child, source, file, result, class_qualified);
                    // Also recurse to extract type references from method signatures
                    extract_recursive(
                        &child,
                        source,
                        file,
                        result,
                        Some(class_qualified),
                        max_depth - 1,
                    );
                }

                // @property (attributes) Type propertyName;
                "property_declaration" => {
                    extract_property(&child, source, file, result, class_qualified);
                    // Also recurse to extract type references from property declarations
                    extract_recursive(
                        &child,
                        source,
                        file,
                        result,
                        Some(class_qualified),
                        max_depth - 1,
                    );
                }

                // Implementation definitions contain method_definitions
                "implementation_definition" => {
                    extract_class_body(
                        &child,
                        source,
                        file,
                        result,
                        class_qualified,
                        max_depth - 1,
                    );
                }

                // Type identifier - captures type references in inheritance clause, etc.
                "type_identifier" => {
                    if let Ok(name) = child.utf8_text(source) {
                        result.references.push(Reference {
                            name: name.to_string(),
                            location: node_to_location(file, &child),
                        });
                    }
                }

                // Identifier that's not a class/method name (e.g., superclass name)
                "identifier" => {
                    // Check if this is a superclass reference (after ":")
                    if let Some(prev) = child.prev_sibling() {
                        if prev.kind() == ":" {
                            if let Ok(name) = child.utf8_text(source) {
                                result.references.push(Reference {
                                    name: name.to_string(),
                                    location: node_to_location(file, &child),
                                });
                            }
                        }
                    }
                }

                _ => {
                    if max_depth > 0 {
                        extract_class_body(
                            &child,
                            source,
                            file,
                            result,
                            class_qualified,
                            max_depth - 1,
                        );
                    }
                }
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
}

/// Extract method from declaration
fn extract_method(
    node: &tree_sitter::Node,
    source: &[u8],
    file: &Path,
    result: &mut ParseResult,
    class_qualified: &str,
) {
    // Method selector is made up of keyword_selector or unary_selector
    // We'll get the method name from the selector
    let mut method_name = String::new();

    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            let child = cursor.node();
            if child.kind() == "selector" || child.kind() == "keyword_selector" {
                // Build method name from selector parts
                let mut sel_cursor = child.walk();
                if sel_cursor.goto_first_child() {
                    loop {
                        let part = sel_cursor.node();
                        if part.kind() == "keyword_declarator" || part.kind() == "identifier" {
                            if let Some(kw) = find_child_by_kind(&part, "identifier") {
                                if let Ok(text) = kw.utf8_text(source) {
                                    if !method_name.is_empty() {
                                        method_name.push(':');
                                    }
                                    method_name.push_str(text);
                                }
                            } else if let Ok(text) = part.utf8_text(source) {
                                if !method_name.is_empty() {
                                    method_name.push(':');
                                }
                                method_name.push_str(text.trim_end_matches(':'));
                            }
                        }
                        if !sel_cursor.goto_next_sibling() {
                            break;
                        }
                    }
                }
            } else if child.kind() == "identifier" && method_name.is_empty() {
                if let Ok(text) = child.utf8_text(source) {
                    method_name = text.to_string();
                }
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }

    if !method_name.is_empty() {
        let qualified = format!("{}.{}", class_qualified, method_name);
        let doc = extract_doc_comments(node, source);

        result.symbols.push(Symbol {
            name: method_name,
            qualified,
            kind: SymbolKind::Function,
            location: node_to_location(file, node),
            visibility: Visibility::Public,
            language: "objc".to_string(),
            parent: Some(class_qualified.to_string()),
            mixins: None,
            attributes: None,
            implements: None,
            doc,
            signature: None,
        });
    }
}

/// Recursively find an identifier in a node tree
fn find_identifier_recursive<'a>(node: &tree_sitter::Node<'a>) -> Option<tree_sitter::Node<'a>> {
    if node.kind() == "identifier" {
        return Some(*node);
    }

    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            if let Some(id) = find_identifier_recursive(&cursor.node()) {
                return Some(id);
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
    None
}

/// Check if a node is in a reference context (not a definition)
fn is_reference_context(node: &tree_sitter::Node) -> bool {
    if let Some(parent) = node.parent() {
        match parent.kind() {
            // Definition contexts - not references
            "class_interface"
            | "class_implementation"
            | "protocol_declaration"
            | "category_interface" => {
                // Check if this is the name being defined
                if let Some(name_field) = find_child_by_kind(&parent, "identifier") {
                    if name_field.id() == node.id() {
                        return false;
                    }
                }
                return true;
            }
            "method_declaration" | "method_definition" | "function_definition" => {
                return false; // Method/function names are definitions
            }
            "property_declaration" | "type_definition" => {
                return false; // Property and type names are definitions
            }
            "declaration" => {
                // Variable declarations - the declared name is not a reference
                // but type references within are
                if parent
                    .child_by_field_name("declarator")
                    .and_then(|d| find_identifier_recursive(&d))
                    .is_some_and(|id| id.id() == node.id())
                {
                    return false;
                }
                return true;
            }
            // Reference contexts
            "call_expression" | "message_expression" | "subscript_expression" => return true,
            "assignment_expression" | "binary_expression" | "unary_expression" => return true,
            "return_statement" | "expression_statement" | "if_statement" | "while_statement" => {
                return true
            }
            "argument_list" | "initializer_list" => return true,
            "field_expression" => return true, // obj.field or obj->field
            "compound_statement" | "translation_unit" => return false,
            _ => return is_reference_context(&parent),
        }
    }
    false
}

/// Extract method reference from message expression: [receiver method:arg1 key:arg2]
fn extract_message_reference(
    node: &tree_sitter::Node,
    source: &[u8],
    file: &Path,
    result: &mut ParseResult,
) {
    // Message expressions have structure: [receiver selector]
    // Simple: [obj method] → [, identifier(receiver), identifier(method), ]
    // Keyword: [obj method:arg] → [, identifier(receiver), identifier(method), :, ...]
    let mut cursor = node.walk();
    let mut found_receiver = false;

    if cursor.goto_first_child() {
        loop {
            let child = cursor.node();
            match child.kind() {
                "[" | "]" | ":" => {} // Skip brackets and colons
                "identifier" => {
                    if let Ok(text) = child.utf8_text(source) {
                        if !found_receiver {
                            // First identifier is the receiver (self, super, or variable)
                            found_receiver = true;
                            // If receiver is a variable (not self/super), it's a reference
                            if text != "self" && text != "super" {
                                result.references.push(Reference {
                                    name: text.to_string(),
                                    location: node_to_location(file, &child),
                                });
                            }
                        } else {
                            // Subsequent identifier is the method name
                            result.references.push(Reference {
                                name: text.to_string(),
                                location: node_to_location(file, &child),
                            });
                        }
                    }
                }
                // Nested message expression - receiver is another message
                "message_expression" => {
                    found_receiver = true;
                    // The nested expression will be handled by recursion in extract_recursive
                }
                // Method parameter in keyword syntax
                "method_parameter" => {
                    // This can contain the method name if it's the keyword selector part
                    if let Some(name_node) = find_child_by_kind(&child, "identifier") {
                        if let Ok(text) = name_node.utf8_text(source) {
                            // Only the first keyword is the primary method name
                            result.references.push(Reference {
                                name: text.to_string(),
                                location: node_to_location(file, &name_node),
                            });
                        }
                    }
                }
                _ => {}
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
}

/// Extract property from declaration
fn extract_property(
    node: &tree_sitter::Node,
    source: &[u8],
    file: &Path,
    result: &mut ParseResult,
    class_qualified: &str,
) {
    // Property structure: @property (attrs) Type *propertyName;
    // The identifier is nested in struct_declaration -> struct_declarator -> pointer_declarator
    // So we need to search recursively for the identifier in struct_declaration
    if let Some(struct_decl) = find_child_by_kind(node, "struct_declaration") {
        if let Some(name_node) = find_identifier_recursive(&struct_decl) {
            if let Ok(name) = name_node.utf8_text(source) {
                let qualified = format!("{}.{}", class_qualified, name);
                let doc = extract_doc_comments(node, source);

                result.symbols.push(Symbol {
                    name: name.to_string(),
                    qualified,
                    kind: SymbolKind::Member,
                    location: node_to_location(file, &name_node),
                    visibility: Visibility::Public,
                    language: "objc".to_string(),
                    parent: Some(class_qualified.to_string()),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::LanguageParser;

    #[cfg(test)]
    fn debug_print_tree(node: &tree_sitter::Node, source: &[u8], depth: usize) {
        let indent = "  ".repeat(depth);
        let text = if node.child_count() == 0 {
            format!(" = {:?}", node.utf8_text(source).unwrap_or(""))
        } else {
            String::new()
        };
        println!("{}{}{}", indent, node.kind(), text);

        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                debug_print_tree(&child, source, depth + 1);
            }
        }
    }

    #[test]
    fn debug_objc_ast() {
        let source = r#"
#import <Foundation/Foundation.h>

/// A simple user class.
@interface User : NSObject

@property (nonatomic, strong) NSString *name;

- (instancetype)initWithName:(NSString *)name;
- (NSString *)greet;

@end

@implementation User

- (instancetype)initWithName:(NSString *)name {
    self = [super init];
    if (self) {
        _name = name;
    }
    return self;
}

- (NSString *)greet {
    return [@"Hello, " stringByAppendingString:self.name];
}

@end
"#;

        OBJC_PARSER.with(|parser| {
            let mut parser = parser.borrow_mut();
            let tree = parser.parse(source, None).unwrap();
            debug_print_tree(&tree.root_node(), source.as_bytes(), 0);
        });
    }

    #[test]
    fn extracts_objc_class() {
        let source = r#"
/// A simple user class.
@interface User : NSObject
@end
"#;
        let parser = ObjCParser;
        let result = parser.extract_symbols(std::path::Path::new("User.m"), source, 100);

        let class_sym = result
            .symbols
            .iter()
            .find(|s| s.name == "User")
            .expect("Should find User class");
        assert_eq!(class_sym.kind, SymbolKind::Class);
    }

    #[test]
    fn extracts_objc_protocol() {
        let source = r#"
@protocol Repository
- (id)findById:(NSInteger)id;
@end
"#;
        let parser = ObjCParser;
        let result = parser.extract_symbols(std::path::Path::new("Repository.h"), source, 100);

        let proto = result
            .symbols
            .iter()
            .find(|s| s.name == "Repository")
            .expect("Should find Repository protocol");
        assert_eq!(proto.kind, SymbolKind::Interface);
    }

    #[test]
    fn extracts_objc_imports() {
        let source = r#"
#import <Foundation/Foundation.h>
#import "User.h"

@interface App : NSObject
@end
"#;
        let parser = ObjCParser;
        let result = parser.extract_symbols(std::path::Path::new("App.m"), source, 100);

        assert!(result.opens.iter().any(|o| o.contains("Foundation")));
        assert!(result.opens.iter().any(|o| o.contains("User.h")));
    }

    #[test]
    fn extracts_objc_methods() {
        let source = r#"
@interface User : NSObject
- (instancetype)initWithName:(NSString *)name;
- (NSString *)greet;
@end
"#;
        let parser = ObjCParser;
        let result = parser.extract_symbols(std::path::Path::new("User.h"), source, 100);

        assert!(
            result.symbols.iter().any(|s| s.name == "initWithName"),
            "Should find initWithName method"
        );
        assert!(
            result.symbols.iter().any(|s| s.name == "greet"),
            "Should find greet method"
        );
    }

    #[test]
    fn extracts_objc_properties() {
        let source = r#"
@interface User : NSObject
@property (nonatomic, strong) NSString *name;
@property (nonatomic, assign) NSInteger age;
@end
"#;
        let parser = ObjCParser;
        let result = parser.extract_symbols(std::path::Path::new("User.h"), source, 100);

        let name_prop = result
            .symbols
            .iter()
            .find(|s| s.name == "name" && s.kind == SymbolKind::Member);
        assert!(name_prop.is_some(), "Should find name property");
        assert_eq!(name_prop.unwrap().qualified, "User.name");

        let age_prop = result
            .symbols
            .iter()
            .find(|s| s.name == "age" && s.kind == SymbolKind::Member);
        assert!(age_prop.is_some(), "Should find age property");
    }

    #[test]
    fn extracts_methods_from_implementation() {
        let source = r#"
@implementation User

- (instancetype)initWithName:(NSString *)name {
    self = [super init];
    return self;
}

- (NSString *)greet {
    return @"Hello";
}

@end
"#;
        let parser = ObjCParser;
        let result = parser.extract_symbols(std::path::Path::new("User.m"), source, 100);

        assert!(
            result.symbols.iter().any(|s| s.name == "initWithName"),
            "Should find initWithName from implementation"
        );
        assert!(
            result.symbols.iter().any(|s| s.name == "greet"),
            "Should find greet from implementation"
        );
    }

    #[test]
    fn extracts_doc_comments() {
        let source = r#"
/// A simple user class.
@interface User : NSObject
@end
"#;
        let parser = ObjCParser;
        let result = parser.extract_symbols(std::path::Path::new("User.h"), source, 100);

        let class_sym = result.symbols.iter().find(|s| s.name == "User").unwrap();
        assert!(class_sym.doc.is_some());
        assert!(class_sym
            .doc
            .as_ref()
            .unwrap()
            .contains("simple user class"));
    }

    #[test]
    fn extracts_message_expression_references() {
        let source = r#"
@implementation App

- (void)doWork {
    [self performAction];
    [helper doSomething];
}

@end
"#;
        let parser = ObjCParser;
        let result = parser.extract_symbols(std::path::Path::new("App.m"), source, 100);

        // Should find method call references
        assert!(
            result.references.iter().any(|r| r.name == "performAction"),
            "Should find performAction method reference"
        );
        assert!(
            result.references.iter().any(|r| r.name == "doSomething"),
            "Should find doSomething method reference"
        );
    }

    #[test]
    fn extracts_keyword_selector_references() {
        let source = r#"
@implementation App

- (void)setup {
    [user initWithName:@"Test" age:25];
}

@end
"#;
        let parser = ObjCParser;
        let result = parser.extract_symbols(std::path::Path::new("App.m"), source, 100);

        // Should find the method reference from keyword selector
        assert!(
            result.references.iter().any(|r| r.name == "initWithName"),
            "Should find initWithName method reference from keyword selector"
        );
    }

    #[test]
    fn extracts_function_call_references() {
        let source = r#"
void process() {
    NSLog(@"Hello");
    calculateValue(42);
}
"#;
        let parser = ObjCParser;
        let result = parser.extract_symbols(std::path::Path::new("utils.m"), source, 100);

        // Should find C-style function calls
        assert!(
            result.references.iter().any(|r| r.name == "NSLog"),
            "Should find NSLog function reference"
        );
        assert!(
            result.references.iter().any(|r| r.name == "calculateValue"),
            "Should find calculateValue function reference"
        );
    }

    #[test]
    fn extracts_type_references() {
        let source = r#"
@interface App : NSObject
@property (nonatomic, strong) NSString *name;
@property (nonatomic, strong) User *user;
@end
"#;
        let parser = ObjCParser;
        let result = parser.extract_symbols(std::path::Path::new("App.h"), source, 100);

        // Should find type references
        assert!(
            result.references.iter().any(|r| r.name == "NSObject"),
            "Should find NSObject type reference"
        );
        assert!(
            result.references.iter().any(|r| r.name == "NSString"),
            "Should find NSString type reference"
        );
        assert!(
            result.references.iter().any(|r| r.name == "User"),
            "Should find User type reference"
        );
    }
}
