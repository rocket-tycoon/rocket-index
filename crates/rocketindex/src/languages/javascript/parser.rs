//! Symbol extraction from JavaScript source files using tree-sitter.

use std::cell::RefCell;
use std::path::Path;

use crate::parse::{find_child_by_kind, node_to_location, LanguageParser, ParseResult};
use crate::{Reference, Symbol, SymbolKind, Visibility};

// Thread-local parser reuse - avoids creating a new parser per file
thread_local! {
    static JS_PARSER: RefCell<tree_sitter::Parser> = RefCell::new({
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_javascript::LANGUAGE.into())
            .expect("tree-sitter-javascript grammar incompatible with tree-sitter version");
        parser
    });
}

pub struct JavaScriptParser;

impl LanguageParser for JavaScriptParser {
    fn extract_symbols(&self, file: &Path, source: &str, max_depth: usize) -> ParseResult {
        JS_PARSER.with(|parser| {
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

/// Extract export visibility from node
fn extract_visibility(node: &tree_sitter::Node, source: &[u8]) -> Visibility {
    // Check if this node or its parent has an export keyword
    if let Some(parent) = node.parent() {
        if parent.kind() == "export_statement" {
            return Visibility::Public;
        }
    }

    // Check for export keyword as sibling
    if let Some(prev) = node.prev_sibling() {
        if prev.kind() == "export" {
            return Visibility::Public;
        }
    }

    // Check if node text starts with export
    if let Ok(text) = node.utf8_text(source) {
        if text.starts_with("export ") {
            return Visibility::Public;
        }
    }

    Visibility::Private
}

/// Extract JSDoc comments from preceding siblings
fn extract_doc_comments(node: &tree_sitter::Node, source: &[u8]) -> Option<String> {
    let mut docs = Vec::new();

    let mut prev = node.prev_sibling();
    while let Some(sib) = prev {
        match sib.kind() {
            "comment" => {
                if let Ok(text) = sib.utf8_text(source) {
                    // JSDoc style: /** ... */
                    if text.starts_with("/**") {
                        let doc = text
                            .trim_start_matches("/**")
                            .trim_end_matches("*/")
                            .lines()
                            .map(|l| l.trim().trim_start_matches('*').trim())
                            .filter(|l| !l.is_empty())
                            .collect::<Vec<_>>()
                            .join("\n");
                        if !doc.is_empty() {
                            docs.insert(0, doc);
                        }
                    }
                    // Single line // comment
                    else if text.starts_with("//") {
                        let doc = text.trim_start_matches("//").trim();
                        if !doc.is_empty() {
                            docs.insert(0, doc.to_string());
                        }
                    }
                }
                prev = sib.prev_sibling();
            }
            _ => break,
        }
    }

    if docs.is_empty() {
        None
    } else {
        Some(docs.join("\n"))
    }
}

/// Extract function signature
fn extract_function_signature(node: &tree_sitter::Node, source: &[u8]) -> Option<String> {
    let start = node.start_byte();
    if let Some(body) = find_child_by_kind(node, "statement_block") {
        let end = body.start_byte();
        if end > start {
            if let Ok(sig) = std::str::from_utf8(&source[start..end]) {
                return Some(sig.trim().to_string());
            }
        }
    }
    if let Ok(full) = node.utf8_text(source) {
        let sig = full.lines().next().unwrap_or(full);
        if let Some(brace) = sig.find('{') {
            return Some(sig[..brace].trim().to_string());
        }
        return Some(sig.trim().to_string());
    }
    None
}

/// Build qualified name with . separator
fn qualified_name(name: &str, parent_path: Option<&str>) -> String {
    match parent_path {
        Some(p) => format!("{}.{}", p, name),
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
        "class_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Ok(name) = name_node.utf8_text(source) {
                    let qualified = qualified_name(name, parent_path);
                    let visibility = extract_visibility(node, source);
                    let doc = extract_doc_comments(node, source);

                    // Extract extends (parent class)
                    let parent = extract_extends(node, source);

                    result.symbols.push(Symbol {
                        name: name.to_string(),
                        qualified: qualified.clone(),
                        kind: SymbolKind::Class,
                        location: node_to_location(file, &name_node),
                        visibility,
                        language: "javascript".to_string(),
                        parent,
                        mixins: None,
                        attributes: None,
                        implements: None,
                        doc,
                        signature: None,
                    });

                    // Recurse into class body
                    if let Some(body) = node.child_by_field_name("body") {
                        extract_class_body(&body, source, file, result, &qualified, max_depth - 1);
                    }
                    return;
                }
            }
        }

        "function_declaration" => {
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
                        language: "javascript".to_string(),
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

        "lexical_declaration" | "variable_declaration" => {
            extract_variable_declarations(node, source, file, result, parent_path);
        }

        "export_statement" => {
            // Handle export { ... } and export default
            for i in 0..node.child_count() {
                if let Some(child) = node.child(i) {
                    let kind = child.kind();
                    if kind.ends_with("_declaration") || kind == "lexical_declaration" {
                        extract_recursive(&child, source, file, result, parent_path, max_depth);
                    }
                }
            }
            return;
        }

        "import_statement" => {
            extract_import_statement(node, source, result);
        }

        "expression_statement" => {
            // Look for prototype method assignments: Foo.prototype.method = function() {}
            extract_prototype_method_assignment(node, source, file, result);
        }

        // Extract references from identifiers
        "identifier" | "property_identifier" => {
            if is_reference_context(node) {
                if let Ok(name) = node.utf8_text(source) {
                    result.references.push(Reference {
                        name: name.to_string(),
                        location: node_to_location(file, node),
                    });
                }
            }
        }

        // Extract references from member expressions (like obj.method)
        "member_expression" => {
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
            extract_recursive(&child, source, file, result, parent_path, max_depth - 1);
        }
    }
}

/// Extract class body members
fn extract_class_body(
    body: &tree_sitter::Node,
    source: &[u8],
    file: &Path,
    result: &mut ParseResult,
    class_path: &str,
    max_depth: usize,
) {
    for i in 0..body.child_count() {
        if let Some(child) = body.child(i) {
            match child.kind() {
                "method_definition" => {
                    if let Some(name_node) = child.child_by_field_name("name") {
                        if let Ok(name) = name_node.utf8_text(source) {
                            let qualified = format!("{}.{}", class_path, name);
                            let doc = extract_doc_comments(&child, source);
                            let signature = extract_function_signature(&child, source);

                            result.symbols.push(Symbol {
                                name: name.to_string(),
                                qualified,
                                kind: SymbolKind::Function,
                                location: node_to_location(file, &name_node),
                                visibility: Visibility::Public, // JS doesn't have visibility modifiers
                                language: "javascript".to_string(),
                                parent: Some(class_path.to_string()),
                                mixins: None,
                                attributes: None,
                                implements: None,
                                doc,
                                signature,
                            });
                        }
                    }
                }

                "field_definition" => {
                    if let Some(name_node) = child.child_by_field_name("property") {
                        if let Ok(name) = name_node.utf8_text(source) {
                            let qualified = format!("{}.{}", class_path, name);
                            let doc = extract_doc_comments(&child, source);

                            // Check if it's a private field (starts with #)
                            let visibility = if name.starts_with('#') {
                                Visibility::Private
                            } else {
                                Visibility::Public
                            };

                            result.symbols.push(Symbol {
                                name: name.to_string(),
                                qualified,
                                kind: SymbolKind::Member,
                                location: node_to_location(file, &name_node),
                                visibility,
                                language: "javascript".to_string(),
                                parent: Some(class_path.to_string()),
                                mixins: None,
                                attributes: None,
                                implements: None,
                                doc,
                                signature: None,
                            });
                        }
                    }
                }

                _ => {
                    if max_depth > 0 {
                        extract_recursive(
                            &child,
                            source,
                            file,
                            result,
                            Some(class_path),
                            max_depth - 1,
                        );
                    }
                }
            }
        }
    }
}

/// Extract variable declarations
fn extract_variable_declarations(
    node: &tree_sitter::Node,
    source: &[u8],
    file: &Path,
    result: &mut ParseResult,
    parent_path: Option<&str>,
) {
    let visibility = extract_visibility(node, source);
    let doc = extract_doc_comments(node, source);

    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            if child.kind() == "variable_declarator" {
                if let Some(name_node) = child.child_by_field_name("name") {
                    if name_node.kind() == "identifier" {
                        if let Ok(name) = name_node.utf8_text(source) {
                            let qualified = qualified_name(name, parent_path);

                            let (kind, signature) = if let Some(value) =
                                child.child_by_field_name("value")
                            {
                                if value.kind() == "arrow_function" || value.kind() == "function" {
                                    (
                                        SymbolKind::Function,
                                        extract_function_signature(&value, source),
                                    )
                                } else {
                                    // Extract object literal properties if value is an object
                                    if value.kind() == "object" {
                                        extract_object_literal_properties(
                                            &value, source, file, result, &qualified,
                                        );
                                    }
                                    (SymbolKind::Value, None)
                                }
                            } else {
                                (SymbolKind::Value, None)
                            };

                            result.symbols.push(Symbol {
                                name: name.to_string(),
                                qualified,
                                kind,
                                location: node_to_location(file, &name_node),
                                visibility,
                                language: "javascript".to_string(),
                                parent: None,
                                mixins: None,
                                attributes: None,
                                implements: None,
                                doc: doc.clone(),
                                signature,
                            });
                        }
                    }
                }
            }
        }
    }
}

/// Extract extends from class
fn extract_extends(node: &tree_sitter::Node, source: &[u8]) -> Option<String> {
    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            let child = cursor.node();
            if child.kind() == "class_heritage" {
                // Look for the identifier after extends
                let mut inner = child.walk();
                if inner.goto_first_child() {
                    loop {
                        let inner_child = inner.node();
                        if inner_child.kind() == "identifier" {
                            return inner_child.utf8_text(source).ok().map(|s| s.to_string());
                        }
                        if !inner.goto_next_sibling() {
                            break;
                        }
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

/// Extract properties from object literals: const X = { a, b: c }
///
/// Handles both shorthand properties ({ map }) and pair properties ({ key: value }).
fn extract_object_literal_properties(
    object_node: &tree_sitter::Node,
    source: &[u8],
    file: &Path,
    result: &mut ParseResult,
    object_path: &str,
) {
    for i in 0..object_node.child_count() {
        if let Some(child) = object_node.child(i) {
            match child.kind() {
                // Shorthand property: { map } -> Children.map
                "shorthand_property_identifier" => {
                    if let Ok(name) = child.utf8_text(source) {
                        let qualified = format!("{}.{}", object_path, name);
                        result.symbols.push(Symbol {
                            name: name.to_string(),
                            qualified,
                            kind: SymbolKind::Member,
                            location: node_to_location(file, &child),
                            visibility: Visibility::Public,
                            language: "javascript".to_string(),
                            parent: Some(object_path.to_string()),
                            mixins: None,
                            attributes: None,
                            implements: None,
                            doc: None,
                            signature: None,
                        });
                    }
                }
                // Pair property: { key: value } -> Object.key
                "pair" => {
                    if let Some(key_node) = child.child_by_field_name("key") {
                        if key_node.kind() == "property_identifier" {
                            if let Ok(name) = key_node.utf8_text(source) {
                                let qualified = format!("{}.{}", object_path, name);
                                result.symbols.push(Symbol {
                                    name: name.to_string(),
                                    qualified,
                                    kind: SymbolKind::Member,
                                    location: node_to_location(file, &key_node),
                                    visibility: Visibility::Public,
                                    language: "javascript".to_string(),
                                    parent: Some(object_path.to_string()),
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
                _ => {}
            }
        }
    }
}

/// Extract prototype method assignments: Foo.prototype.method = function() {}
///
/// This is a classic JavaScript pattern for defining instance methods before ES6 classes.
fn extract_prototype_method_assignment(
    node: &tree_sitter::Node, // expression_statement
    source: &[u8],
    file: &Path,
    result: &mut ParseResult,
) {
    // Look for assignment_expression child
    if let Some(assign) = find_child_by_kind(node, "assignment_expression") {
        // Get left side (member_expression)
        if let Some(left) = assign.child_by_field_name("left") {
            if left.kind() == "member_expression" {
                // Check for pattern: Something.prototype.methodName
                if let Some((class_name, method_name, method_name_node)) =
                    extract_prototype_pattern(&left, source)
                {
                    // Get right side - should be a function_expression
                    if let Some(right) = assign.child_by_field_name("right") {
                        if right.kind() == "function_expression" || right.kind() == "arrow_function"
                        {
                            let qualified = format!("{}.{}", class_name, method_name);
                            let doc = extract_doc_comments(node, source);
                            let signature = extract_function_signature(&right, source);

                            result.symbols.push(Symbol {
                                name: method_name.to_string(),
                                qualified,
                                kind: SymbolKind::Function,
                                location: node_to_location(file, &method_name_node),
                                visibility: Visibility::Public,
                                language: "javascript".to_string(),
                                parent: Some(class_name.to_string()),
                                mixins: None,
                                attributes: None,
                                implements: None,
                                doc,
                                signature,
                            });
                        }
                    }
                }
            }
        }
    }
}

/// Extract the pattern Foo.prototype.methodName from a member_expression
/// Returns (class_name, method_name, method_name_node) if pattern matches
fn extract_prototype_pattern<'a>(
    node: &'a tree_sitter::Node<'a>,
    source: &[u8],
) -> Option<(String, String, tree_sitter::Node<'a>)> {
    // node should be: Foo.prototype.methodName
    // Structure:
    //   member_expression
    //     member_expression (Foo.prototype)
    //       identifier (Foo)
    //       property_identifier (prototype)
    //     property_identifier (methodName)

    // Get the method name (property on the right)
    let method_name_node = node.child_by_field_name("property")?;
    let method_name = method_name_node.utf8_text(source).ok()?;

    // Get the object (left side: Foo.prototype)
    let obj = node.child_by_field_name("object")?;
    if obj.kind() != "member_expression" {
        return None;
    }

    // Check that the property is "prototype"
    let proto_prop = obj.child_by_field_name("property")?;
    let proto_text = proto_prop.utf8_text(source).ok()?;
    if proto_text != "prototype" {
        return None;
    }

    // Get the class name (left side of Foo.prototype)
    let class_node = obj.child_by_field_name("object")?;
    if class_node.kind() != "identifier" {
        return None;
    }
    let class_name = class_node.utf8_text(source).ok()?;

    Some((
        class_name.to_string(),
        method_name.to_string(),
        method_name_node,
    ))
}

/// Extract import statements
fn extract_import_statement(node: &tree_sitter::Node, source: &[u8], result: &mut ParseResult) {
    if let Some(source_node) = node.child_by_field_name("source") {
        if let Ok(module_path) = source_node.utf8_text(source) {
            let module_path = module_path.trim_matches(|c| c == '"' || c == '\'');
            result.opens.push(module_path.to_string());
        }
    }
}

/// Check if a node is an ancestor of another node
fn is_descendant_of(node: &tree_sitter::Node, ancestor_kind: &str) -> bool {
    let mut current = node.parent();
    while let Some(parent) = current {
        if parent.kind() == ancestor_kind {
            return true;
        }
        current = parent.parent();
    }
    false
}

/// Determine if an identifier is in a reference context (usage, not definition)
fn is_reference_context(node: &tree_sitter::Node) -> bool {
    is_reference_context_with_depth(node, 0)
}

fn is_reference_context_with_depth(node: &tree_sitter::Node, depth: usize) -> bool {
    // Prevent infinite recursion
    if depth > 20 {
        return false;
    }

    let parent = match node.parent() {
        Some(p) => p,
        None => return false,
    };

    let parent_kind = parent.kind();

    match parent_kind {
        // Function/method definitions - name is definition
        "function_declaration" | "method_definition" => {
            if let Some(name_node) = parent.child_by_field_name("name") {
                if name_node.id() == node.id() {
                    return false;
                }
            }
        }

        // Class definitions
        "class_declaration" => {
            if let Some(name_node) = parent.child_by_field_name("name") {
                if name_node.id() == node.id() {
                    return false;
                }
            }
        }

        // Variable declarations
        "variable_declarator" => {
            if let Some(name_node) = parent.child_by_field_name("name") {
                if name_node.id() == node.id() {
                    return false;
                }
            }
        }

        // Parameter definitions
        "formal_parameters" => {
            // Parameters are definitions
            return false;
        }

        // Import statements
        "import_clause" | "import_specifier" | "named_imports" => {
            return false;
        }

        // Export specifiers
        "export_specifier" => {
            return false;
        }

        // Object/array patterns (destructuring)
        "object_pattern" | "array_pattern" => {
            return false;
        }

        // Shorthand property identifiers in patterns
        "shorthand_property_identifier_pattern" => {
            return false;
        }

        // Property definition in object literal (key is definition)
        "pair" => {
            if let Some(key) = parent.child_by_field_name("key") {
                if node.id() == key.id() {
                    return false;
                }
            }
            // Value side is a reference
            return true;
        }

        // Field definitions in class
        "field_definition" => {
            if let Some(name_node) = parent.child_by_field_name("property") {
                if name_node.id() == node.id() {
                    return false;
                }
            }
        }

        // Call expressions - function being called is a reference
        "call_expression" | "new_expression" => {
            return true;
        }

        // Member expressions
        "member_expression" => {
            return true;
        }

        // Binary/unary expressions
        "binary_expression" | "unary_expression" | "update_expression" => {
            return true;
        }

        // Return/throw statements
        "return_statement" | "throw_statement" => {
            return true;
        }

        // Assignment - both sides can be references
        "assignment_expression" => {
            return true;
        }

        // Subscript expressions (array access)
        "subscript_expression" => {
            return true;
        }

        // Conditional expressions
        "ternary_expression" | "conditional_expression" => {
            return true;
        }

        // Arguments - always references
        "arguments" => {
            return true;
        }

        // Array/object literals - values inside are references
        "array" | "object" => {
            return true;
        }

        // Template substitution
        "template_substitution" => {
            return true;
        }

        // Parenthesized expression - check parent
        "parenthesized_expression" => {
            return is_reference_context_with_depth(&parent, depth + 1);
        }

        // Statement block, expression statement - check parent
        "statement_block" | "expression_statement" | "program" => {
            return is_reference_context_with_depth(&parent, depth + 1);
        }

        _ => {}
    }

    // Default: if in expression context, likely a reference
    if is_descendant_of(node, "statement_block") {
        return true;
    }

    is_reference_context_with_depth(&parent, depth + 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::extract_symbols;

    #[test]
    fn extracts_javascript_class() {
        let source = r#"
/** A simple user class */
export class User {
    constructor(name, age) {
        this.name = name;
        this.age = age;
    }

    greet() {
        return `Hello, ${this.name}`;
    }
}
"#;
        let result = extract_symbols(std::path::Path::new("test.js"), source, 100);

        let class = result
            .symbols
            .iter()
            .find(|s| s.name == "User")
            .expect("Should find User");
        assert_eq!(class.kind, SymbolKind::Class);
        assert_eq!(class.visibility, Visibility::Public);

        let greet = result
            .symbols
            .iter()
            .find(|s| s.name == "greet")
            .expect("Should find greet");
        assert_eq!(greet.kind, SymbolKind::Function);
        assert_eq!(greet.qualified, "User.greet");
    }

    #[test]
    fn extracts_javascript_function() {
        let source = r#"
/** Adds two numbers */
export function add(a, b) {
    return a + b;
}
"#;
        let result = extract_symbols(std::path::Path::new("test.js"), source, 100);

        let func = result
            .symbols
            .iter()
            .find(|s| s.name == "add")
            .expect("Should find add");
        assert_eq!(func.kind, SymbolKind::Function);
        assert_eq!(func.visibility, Visibility::Public);
    }

    #[test]
    fn extracts_arrow_function() {
        let source = r#"
export const multiply = (a, b) => a * b;
"#;
        let result = extract_symbols(std::path::Path::new("test.js"), source, 100);

        let func = result
            .symbols
            .iter()
            .find(|s| s.name == "multiply")
            .expect("Should find multiply");
        assert_eq!(func.kind, SymbolKind::Function);
    }

    #[test]
    fn extracts_imports() {
        let source = r#"
import { foo, bar } from './utils';
import lodash from 'lodash';
"#;
        let result = extract_symbols(std::path::Path::new("test.js"), source, 100);

        assert!(result.opens.contains(&"./utils".to_string()));
        assert!(result.opens.contains(&"lodash".to_string()));
    }

    #[test]
    fn extracts_class_with_extends() {
        let source = r#"
class Animal {
    speak() {}
}

class Dog extends Animal {
    bark() {}
}
"#;
        let result = extract_symbols(std::path::Path::new("test.js"), source, 100);

        let dog = result
            .symbols
            .iter()
            .find(|s| s.name == "Dog")
            .expect("Should find Dog");
        assert_eq!(dog.parent, Some("Animal".to_string()));
    }

    #[test]
    fn extracts_private_fields() {
        let source = r#"
class Counter {
    #count = 0;

    increment() {
        this.#count++;
    }
}
"#;
        let result = extract_symbols(std::path::Path::new("test.js"), source, 100);

        let field = result
            .symbols
            .iter()
            .find(|s| s.name == "#count")
            .expect("Should find #count");
        assert_eq!(field.visibility, Visibility::Private);
    }

    #[test]
    fn extracts_static_methods() {
        let source = r#"
class MathUtils {
    static add(a, b) {
        return a + b;
    }

    static PI = 3.14159;
}
"#;
        let result = extract_symbols(std::path::Path::new("test.js"), source, 100);

        assert!(result
            .symbols
            .iter()
            .any(|s| s.name == "add" && s.qualified == "MathUtils.add"));
        assert!(result
            .symbols
            .iter()
            .any(|s| s.name == "PI" && s.qualified == "MathUtils.PI"));
    }

    #[test]
    fn extracts_getter_setter() {
        let source = r#"
class Rectangle {
    constructor(width, height) {
        this._width = width;
        this._height = height;
    }

    get area() {
        return this._width * this._height;
    }

    set width(value) {
        this._width = value;
    }
}
"#;
        let result = extract_symbols(std::path::Path::new("test.js"), source, 100);

        // Getters and setters should be extracted as functions
        assert!(result
            .symbols
            .iter()
            .any(|s| s.name == "area" && s.qualified == "Rectangle.area"));
        assert!(result
            .symbols
            .iter()
            .any(|s| s.name == "width" && s.qualified == "Rectangle.width"));
    }

    #[test]
    fn extracts_jsdoc_comments() {
        let source = r#"
/**
 * Calculates the sum of two numbers.
 * @param {number} a - First number
 * @param {number} b - Second number
 * @returns {number} The sum
 */
function add(a, b) {
    return a + b;
}
"#;
        let result = extract_symbols(std::path::Path::new("test.js"), source, 100);

        let func = result
            .symbols
            .iter()
            .find(|s| s.name == "add")
            .expect("Should find add");
        assert!(func.doc.is_some());
        assert!(func.doc.as_ref().unwrap().contains("Calculates the sum"));
    }

    #[test]
    fn extracts_const_variables() {
        let source = r#"
export const API_URL = "https://api.example.com";
export const MAX_RETRIES = 3;
"#;
        let result = extract_symbols(std::path::Path::new("test.js"), source, 100);

        let api_url = result
            .symbols
            .iter()
            .find(|s| s.name == "API_URL")
            .expect("Should find API_URL");
        assert_eq!(api_url.kind, SymbolKind::Value);
        assert_eq!(api_url.visibility, Visibility::Public);

        assert!(result.symbols.iter().any(|s| s.name == "MAX_RETRIES"));
    }

    #[test]
    fn extracts_default_export_function() {
        let source = r#"
export default function processData(data) {
    return data.map(x => x * 2);
}
"#;
        let result = extract_symbols(std::path::Path::new("test.js"), source, 100);

        let func = result
            .symbols
            .iter()
            .find(|s| s.name == "processData")
            .expect("Should find processData");
        assert_eq!(func.kind, SymbolKind::Function);
    }

    #[test]
    fn extracts_default_export_class() {
        let source = r#"
export default class DataProcessor {
    process(data) {
        return data;
    }
}
"#;
        let result = extract_symbols(std::path::Path::new("test.js"), source, 100);

        let class = result
            .symbols
            .iter()
            .find(|s| s.name == "DataProcessor")
            .expect("Should find DataProcessor");
        assert_eq!(class.kind, SymbolKind::Class);
    }

    #[test]
    fn extracts_prototype_method_assignments() {
        let source = r#"
function Component(props) {
    this.props = props;
}

Component.prototype.setState = function(partialState, callback) {
    // implementation
};

Component.prototype.forceUpdate = function(callback) {
    // implementation
};
"#;
        let result = extract_symbols(std::path::Path::new("test.js"), source, 100);

        // The constructor function should be found
        let comp = result
            .symbols
            .iter()
            .find(|s| s.name == "Component")
            .expect("Should find Component");
        assert_eq!(comp.kind, SymbolKind::Function);

        // Prototype method assignments should be indexed as methods
        let set_state = result
            .symbols
            .iter()
            .find(|s| s.name == "setState")
            .expect("Should find setState");
        assert_eq!(set_state.kind, SymbolKind::Function);
        assert_eq!(set_state.qualified, "Component.setState");
        assert_eq!(set_state.parent, Some("Component".to_string()));

        let force_update = result
            .symbols
            .iter()
            .find(|s| s.name == "forceUpdate")
            .expect("Should find forceUpdate");
        assert_eq!(force_update.kind, SymbolKind::Function);
        assert_eq!(force_update.qualified, "Component.forceUpdate");
    }

    #[test]
    fn extracts_object_literal_properties() {
        let source = r#"
const Children = {
    map,
    forEach,
    count,
};

const React = {
    createElement: createElement,
    Component: Component,
};
"#;
        let result = extract_symbols(std::path::Path::new("test.js"), source, 100);

        // Children object should be found
        let children = result
            .symbols
            .iter()
            .find(|s| s.name == "Children")
            .expect("Should find Children");
        assert_eq!(children.kind, SymbolKind::Value);

        // Shorthand properties should be indexed as members
        let map = result
            .symbols
            .iter()
            .find(|s| s.name == "map" && s.qualified == "Children.map")
            .expect("Should find Children.map");
        assert_eq!(map.kind, SymbolKind::Member);
        assert_eq!(map.parent, Some("Children".to_string()));

        let for_each = result
            .symbols
            .iter()
            .find(|s| s.name == "forEach" && s.qualified == "Children.forEach")
            .expect("Should find Children.forEach");
        assert_eq!(for_each.kind, SymbolKind::Member);

        // React object should be found
        let react = result
            .symbols
            .iter()
            .find(|s| s.name == "React")
            .expect("Should find React");
        assert_eq!(react.kind, SymbolKind::Value);

        // Pair properties should be indexed as members
        let create_element = result
            .symbols
            .iter()
            .find(|s| s.name == "createElement" && s.qualified == "React.createElement")
            .expect("Should find React.createElement");
        assert_eq!(create_element.kind, SymbolKind::Member);
        assert_eq!(create_element.parent, Some("React".to_string()));
    }

    #[test]
    fn extracts_javascript_references() {
        let source = r#"
class User {
    constructor(name) {
        this.name = name;
    }
}

function greet(user) {
    return `Hello, ${user.name}!`;
}

function main() {
    const user = new User("Alice");
    console.log(greet(user));
}
"#;
        let result = extract_symbols(std::path::Path::new("test.js"), source, 100);

        assert!(
            !result.references.is_empty(),
            "Should extract references from JavaScript code"
        );

        let ref_names: Vec<_> = result.references.iter().map(|r| r.name.as_str()).collect();

        // Should have references to User class
        assert!(
            ref_names.contains(&"User"),
            "Should have reference to User: {:?}",
            ref_names
        );

        // Should have references to greet function
        assert!(
            ref_names.contains(&"greet"),
            "Should have reference to greet: {:?}",
            ref_names
        );
    }
}
