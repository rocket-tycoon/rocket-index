//! Symbol extraction from Haxe source files using tree-sitter.

use std::cell::RefCell;
use std::path::Path;

use crate::parse::{find_child_by_kind, node_to_location, LanguageParser, ParseResult};
use crate::{Reference, Symbol, SymbolKind, Visibility};

// Thread-local parser reuse - avoids creating a new parser per file
thread_local! {
    static HAXE_PARSER: RefCell<tree_sitter::Parser> = RefCell::new({
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_haxe::LANGUAGE.into())
            .expect("tree-sitter-haxe grammar incompatible with tree-sitter version");
        parser
    });
}

pub struct HaxeParser;

impl LanguageParser for HaxeParser {
    fn extract_symbols(&self, file: &Path, source: &str, max_depth: usize) -> ParseResult {
        HAXE_PARSER.with(|parser| {
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

            // Extract package name from package_statement
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

/// Extract package name from a module (e.g., `package my.test.package;`)
fn extract_package_name(root: &tree_sitter::Node, source: &[u8]) -> Option<String> {
    let mut cursor = root.walk();
    if cursor.goto_first_child() {
        loop {
            let node = cursor.node();
            if node.kind() == "package_statement" {
                // The package name consists of multiple package_name children separated by dots
                // We need to collect all package_name nodes and join them with dots
                let mut parts = Vec::new();
                for i in 0..node.child_count() {
                    if let Some(child) = node.child(i) {
                        if child.kind() == "package_name" {
                            if let Ok(name) = child.utf8_text(source) {
                                parts.push(name.to_string());
                            }
                        }
                    }
                }
                if !parts.is_empty() {
                    return Some(parts.join("."));
                }
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
    None
}

/// Determine visibility from Haxe modifiers
fn extract_visibility(node: &tree_sitter::Node, source: &[u8]) -> Visibility {
    // Look for modifier children (public, private, etc.)
    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            let child = cursor.node();
            if let Ok(text) = child.utf8_text(source) {
                match text {
                    "public" => return Visibility::Public,
                    "private" => return Visibility::Private,
                    _ => {}
                }
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
    // Default in Haxe is private for class members
    Visibility::Private
}

/// Build a qualified name with package prefix
fn qualified_name(name: &str, package: Option<&str>) -> String {
    match package {
        Some(p) => format!("{}.{}", p, name),
        None => name.to_string(),
    }
}

/// Extract metadata annotations (e.g., @:keep, @:some_macro)
fn extract_metadata(node: &tree_sitter::Node, source: &[u8]) -> Option<Vec<String>> {
    let mut metadata = Vec::new();

    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            let child = cursor.node();
            if child.kind() == "metadata" {
                if let Ok(text) = child.utf8_text(source) {
                    metadata.push(text.to_string());
                }
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }

    if metadata.is_empty() {
        None
    } else {
        Some(metadata)
    }
}

/// Extract doc comments (/* ... */ or /// comments before a declaration)
fn extract_doc_comments(node: &tree_sitter::Node, source: &[u8]) -> Option<String> {
    let mut prev = node.prev_sibling();
    while let Some(sib) = prev {
        if sib.kind() == "comment" {
            if let Ok(text) = sib.utf8_text(source) {
                if text.starts_with("/**") || text.starts_with("///") {
                    // Clean up the comment
                    let cleaned = text
                        .trim_start_matches("/**")
                        .trim_start_matches("///")
                        .trim_end_matches("*/")
                        .lines()
                        .map(|line| line.trim().trim_start_matches('*').trim())
                        .filter(|line| !line.is_empty())
                        .collect::<Vec<_>>()
                        .join("\n");
                    return Some(cleaned);
                }
            }
        } else if sib.kind() != "metadata" {
            // Stop at first non-comment, non-metadata sibling
            break;
        }
        prev = sib.prev_sibling();
    }
    None
}

/// Extract function signature
fn extract_function_signature(
    node: &tree_sitter::Node,
    source: &[u8],
    name: &str,
) -> Option<String> {
    let mut sig = String::new();
    sig.push_str(name);
    sig.push('(');

    let mut args = Vec::new();
    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            let child = cursor.node();
            if child.kind() == "function_arg" {
                if let Some(arg_name) = child.child_by_field_name("name") {
                    if let Ok(arg_text) = arg_name.utf8_text(source) {
                        let mut arg = arg_text.to_string();

                        // Try to get the type annotation
                        for i in 0..child.child_count() {
                            if let Some(type_child) = child.child(i) {
                                if type_child.kind() == "type" {
                                    if let Ok(type_text) = type_child.utf8_text(source) {
                                        arg.push(':');
                                        arg.push_str(type_text);
                                    }
                                    break;
                                }
                            }
                        }

                        args.push(arg);
                    }
                }
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }

    sig.push_str(&args.join(", "));
    sig.push(')');

    // Add return type if present
    if let Some(return_type) = node.child_by_field_name("return_type") {
        if let Ok(rt) = return_type.utf8_text(source) {
            sig.push(':');
            sig.push_str(rt);
        }
    }

    Some(sig)
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
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Ok(name) = name_node.utf8_text(source) {
                    let qualified = qualified_name(name, package);
                    let visibility = extract_visibility(node, source);
                    let doc = extract_doc_comments(node, source);
                    let metadata = extract_metadata(node, source);

                    // Get superclass
                    let parent = node
                        .child_by_field_name("super_class_name")
                        .and_then(|sc| sc.utf8_text(source).ok())
                        .map(|s| s.to_string());

                    // Get interfaces
                    let implements = extract_interfaces(node, source);

                    result.symbols.push(Symbol {
                        name: name.to_string(),
                        qualified: qualified.clone(),
                        kind: SymbolKind::Class,
                        location: node_to_location(file, &name_node),
                        visibility,
                        language: "haxe".to_string(),
                        parent,
                        mixins: None,
                        attributes: metadata,
                        implements,
                        doc,
                        signature: None,
                    });

                    // Recurse into class body
                    if let Some(body) = node.child_by_field_name("body") {
                        extract_recursive(
                            &body,
                            source,
                            file,
                            result,
                            Some(&qualified),
                            max_depth - 1,
                        );
                    }
                    return;
                }
            }
        }

        "interface_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Ok(name) = name_node.utf8_text(source) {
                    let qualified = qualified_name(name, package);
                    let visibility = extract_visibility(node, source);
                    let doc = extract_doc_comments(node, source);
                    let metadata = extract_metadata(node, source);

                    // Get extended interfaces
                    let implements = extract_interfaces(node, source);

                    result.symbols.push(Symbol {
                        name: name.to_string(),
                        qualified: qualified.clone(),
                        kind: SymbolKind::Interface,
                        location: node_to_location(file, &name_node),
                        visibility,
                        language: "haxe".to_string(),
                        parent: None,
                        mixins: None,
                        attributes: metadata,
                        implements,
                        doc,
                        signature: None,
                    });

                    // Recurse into interface body
                    if let Some(body) = node.child_by_field_name("body") {
                        extract_recursive(
                            &body,
                            source,
                            file,
                            result,
                            Some(&qualified),
                            max_depth - 1,
                        );
                    }
                    return;
                }
            }
        }

        "typedef_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Ok(name) = name_node.utf8_text(source) {
                    let qualified = qualified_name(name, package);
                    let visibility = extract_visibility(node, source);
                    let doc = extract_doc_comments(node, source);
                    let metadata = extract_metadata(node, source);

                    result.symbols.push(Symbol {
                        name: name.to_string(),
                        qualified: qualified.clone(),
                        kind: SymbolKind::Type,
                        location: node_to_location(file, &name_node),
                        visibility,
                        language: "haxe".to_string(),
                        parent: None,
                        mixins: None,
                        attributes: metadata,
                        implements: None,
                        doc,
                        signature: None,
                    });

                    // Recurse into typedef body if it has one (struct typedef)
                    if let Some(body) = find_child_by_kind(node, "block") {
                        extract_recursive(
                            &body,
                            source,
                            file,
                            result,
                            Some(&qualified),
                            max_depth - 1,
                        );
                    }
                    return;
                }
            }
        }

        "function_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Ok(name) = name_node.utf8_text(source) {
                    // Handle constructor (new)
                    let fn_name = if name == "new" {
                        "new".to_string()
                    } else {
                        name.to_string()
                    };

                    let qualified = qualified_name(&fn_name, package);
                    let visibility = extract_visibility(node, source);
                    let doc = extract_doc_comments(node, source);
                    let metadata = extract_metadata(node, source);
                    let signature = extract_function_signature(node, source, &fn_name);

                    result.symbols.push(Symbol {
                        name: fn_name,
                        qualified,
                        kind: SymbolKind::Function,
                        location: node_to_location(file, &name_node),
                        visibility,
                        language: "haxe".to_string(),
                        parent: None,
                        mixins: None,
                        attributes: metadata,
                        implements: None,
                        doc,
                        signature,
                    });
                }
            }
        }

        "variable_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Ok(name) = name_node.utf8_text(source) {
                    let qualified = qualified_name(name, package);
                    let visibility = extract_visibility(node, source);
                    let doc = extract_doc_comments(node, source);
                    let metadata = extract_metadata(node, source);

                    result.symbols.push(Symbol {
                        name: name.to_string(),
                        qualified,
                        kind: SymbolKind::Value,
                        location: node_to_location(file, &name_node),
                        visibility,
                        language: "haxe".to_string(),
                        parent: None,
                        mixins: None,
                        attributes: metadata,
                        implements: None,
                        doc,
                        signature: None,
                    });
                }
            }
        }

        "import_statement" => {
            // Extract import path
            let mut import_parts = Vec::new();
            let mut cursor = node.walk();
            if cursor.goto_first_child() {
                loop {
                    let child = cursor.node();
                    if child.kind() == "package_name" || child.kind() == "type_name" {
                        if let Ok(text) = child.utf8_text(source) {
                            import_parts.push(text.to_string());
                        }
                    }
                    if !cursor.goto_next_sibling() {
                        break;
                    }
                }
            }
            if !import_parts.is_empty() {
                result.opens.push(import_parts.join("."));
            }
        }

        "using_statement" => {
            // Extract using path (similar to import)
            let mut using_parts = Vec::new();
            let mut cursor = node.walk();
            if cursor.goto_first_child() {
                loop {
                    let child = cursor.node();
                    if child.kind() == "package_name" || child.kind() == "type_name" {
                        if let Ok(text) = child.utf8_text(source) {
                            using_parts.push(text.to_string());
                        }
                    }
                    if !cursor.goto_next_sibling() {
                        break;
                    }
                }
            }
            if !using_parts.is_empty() {
                result.opens.push(using_parts.join("."));
            }
        }

        // Extract references from type names
        "type_name" => {
            if is_reference_context(node) {
                if let Ok(name) = node.utf8_text(source) {
                    result.references.push(Reference {
                        name: name.to_string(),
                        location: node_to_location(file, node),
                    });
                }
            }
        }

        // Extract references from identifiers that are class references
        "identifier" => {
            if is_class_reference_identifier(node, source) {
                if let Ok(name) = node.utf8_text(source) {
                    result.references.push(Reference {
                        name: name.to_string(),
                        location: node_to_location(file, node),
                    });
                }
            }
        }

        // Extract method call references
        "call_expression" => {
            // Try to get the function/method being called
            if let Some(object) = node.child_by_field_name("object") {
                if let Ok(obj_text) = object.utf8_text(source) {
                    result.references.push(Reference {
                        name: obj_text.to_string(),
                        location: node_to_location(file, &object),
                    });
                }
            }
            if let Some(constructor) = node.child_by_field_name("constructor") {
                if let Ok(ctor_text) = constructor.utf8_text(source) {
                    result.references.push(Reference {
                        name: ctor_text.to_string(),
                        location: node_to_location(file, &constructor),
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

/// Determine if a node is used as a reference (not a definition)
fn is_reference_context(node: &tree_sitter::Node) -> bool {
    let parent = match node.parent() {
        Some(p) => p,
        None => return false,
    };

    match parent.kind() {
        // These are definition contexts, not references
        "class_declaration" | "interface_declaration" | "typedef_declaration" => {
            // Check if this is the name being defined
            if let Some(name_node) = parent.child_by_field_name("name") {
                if node.id() == name_node.id() {
                    return false;
                }
            }
            // Otherwise it could be a type reference (extends, implements)
            true
        }

        // Type references in various contexts
        "type" | "function_arg" | "variable_declaration" => true,

        // Object creation
        "call_expression" => {
            // Check if this is the constructor type
            if let Some(ctor) = parent.child_by_field_name("constructor") {
                node.id() == ctor.id()
            } else {
                false
            }
        }

        _ => false,
    }
}

/// Determine if an identifier is a class reference (e.g., class name in method call)
fn is_class_reference_identifier(node: &tree_sitter::Node, source: &[u8]) -> bool {
    let name = match node.utf8_text(source) {
        Ok(n) => n,
        Err(_) => return false,
    };

    // Class names in Haxe typically start with uppercase
    let first_char = match name.chars().next() {
        Some(c) => c,
        None => return false,
    };

    if !first_char.is_uppercase() {
        return false;
    }

    let parent = match node.parent() {
        Some(p) => p,
        None => return false,
    };

    match parent.kind() {
        // Member access: MyClass.staticMethod
        "member_expression" => {
            // Check if this identifier is the first child (the class name)
            if let Some(first_child) = parent.child(0) {
                first_child.id() == node.id()
            } else {
                false
            }
        }

        _ => false,
    }
}

/// Extract interfaces from implements/extends clauses
fn extract_interfaces(node: &tree_sitter::Node, source: &[u8]) -> Option<Vec<String>> {
    let mut interfaces = Vec::new();

    // Look for interface_name fields
    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            let child = cursor.node();
            // Check field names - tree-sitter uses numeric indices for repeated fields
            if let Some(field_name) = node.field_name_for_child(child.id() as u32) {
                if field_name == "interface_name" {
                    if let Ok(name) = child.utf8_text(source) {
                        interfaces.push(name.to_string());
                    }
                }
            }
            // Also check type_name children in interface context
            if child.kind() == "type_name" {
                // Only add if we're after 'implements' or 'extends' (for interfaces)
                if let Some(prev) = child.prev_sibling() {
                    if let Ok(prev_text) = prev.utf8_text(source) {
                        if prev_text == "implements" || prev_text == "extends" {
                            if let Ok(name) = child.utf8_text(source) {
                                interfaces.push(name.to_string());
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

    if interfaces.is_empty() {
        None
    } else {
        Some(interfaces)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::LanguageParser;

    #[test]
    fn extracts_haxe_class() {
        let source = r#"
package my.test;

class User {
    public var name:String;
}
"#;
        let parser = HaxeParser;
        let result = parser.extract_symbols(std::path::Path::new("User.hx"), source, 100);

        let class_sym = result
            .symbols
            .iter()
            .find(|s| s.name == "User")
            .expect("Should find User class");
        assert_eq!(class_sym.kind, SymbolKind::Class);
        assert_eq!(class_sym.qualified, "my.test.User");
    }

    #[test]
    fn extracts_haxe_interface() {
        let source = r#"
package my.test;

interface IRepository {
    function findById(id:Int):Dynamic;
    function save(entity:Dynamic):Void;
}
"#;
        let parser = HaxeParser;
        let result = parser.extract_symbols(std::path::Path::new("IRepository.hx"), source, 100);

        let iface = result
            .symbols
            .iter()
            .find(|s| s.name == "IRepository")
            .expect("Should find IRepository interface");
        assert_eq!(iface.kind, SymbolKind::Interface);
        assert_eq!(iface.qualified, "my.test.IRepository");
    }

    #[test]
    fn extracts_haxe_function() {
        let source = r#"
package my.test;

class Calculator {
    public static function add(a:Int, b:Int):Int {
        return a + b;
    }
}
"#;
        let parser = HaxeParser;
        let result = parser.extract_symbols(std::path::Path::new("Calculator.hx"), source, 100);

        let method = result
            .symbols
            .iter()
            .find(|s| s.name == "add")
            .expect("Should find add method");
        assert_eq!(method.kind, SymbolKind::Function);
        assert_eq!(method.qualified, "my.test.Calculator.add");
    }

    #[test]
    fn extracts_haxe_typedef() {
        let source = r#"
package my.test;

typedef Point = {
    var x:Int;
    var y:Int;
}
"#;
        let parser = HaxeParser;
        let result = parser.extract_symbols(std::path::Path::new("Point.hx"), source, 100);

        let typedef = result
            .symbols
            .iter()
            .find(|s| s.name == "Point")
            .expect("Should find Point typedef");
        assert_eq!(typedef.kind, SymbolKind::Type);
        assert_eq!(typedef.qualified, "my.test.Point");
    }

    #[test]
    fn extracts_haxe_metadata() {
        let source = r#"
package my.test;

@:keep
@:native("User")
class User {
    public function new() {}
}
"#;
        let parser = HaxeParser;
        let result = parser.extract_symbols(std::path::Path::new("User.hx"), source, 100);

        let class_sym = result
            .symbols
            .iter()
            .find(|s| s.name == "User")
            .expect("Should find User");

        assert!(class_sym.attributes.is_some());
        let attrs = class_sym.attributes.as_ref().unwrap();
        assert!(attrs.iter().any(|a| a.contains("keep")));
    }

    #[test]
    fn extracts_haxe_constructor() {
        let source = r#"
package my.test;

class Person {
    public function new(name:String, age:Int) {}
}
"#;
        let parser = HaxeParser;
        let result = parser.extract_symbols(std::path::Path::new("Person.hx"), source, 100);

        let ctor = result
            .symbols
            .iter()
            .find(|s| s.name == "new" && s.kind == SymbolKind::Function)
            .expect("Should find constructor");
        assert_eq!(ctor.qualified, "my.test.Person.new");
    }

    #[test]
    fn extracts_haxe_variable() {
        let source = r#"
package my.test;

class Counter {
    private var count:Int = 0;
}
"#;
        let parser = HaxeParser;
        let result = parser.extract_symbols(std::path::Path::new("Counter.hx"), source, 100);

        let field = result
            .symbols
            .iter()
            .find(|s| s.name == "count")
            .expect("Should find count field");
        assert_eq!(field.kind, SymbolKind::Value);
        assert_eq!(field.visibility, Visibility::Private);
    }

    #[test]
    fn handles_visibility_modifiers() {
        let source = r#"
package my.test;

class Example {
    public function publicMethod() {}
    private function privateMethod() {}
    function defaultMethod() {}
}
"#;
        let parser = HaxeParser;
        let result = parser.extract_symbols(std::path::Path::new("Example.hx"), source, 100);

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

        let default = result
            .symbols
            .iter()
            .find(|s| s.name == "defaultMethod")
            .unwrap();
        // Default visibility in Haxe is private for class members
        assert_eq!(default.visibility, Visibility::Private);
    }

    #[test]
    fn extracts_without_package() {
        let source = r#"
class Main {
    static function main() {
        trace("Hello");
    }
}
"#;
        let parser = HaxeParser;
        let result = parser.extract_symbols(std::path::Path::new("Main.hx"), source, 100);

        let class_sym = result
            .symbols
            .iter()
            .find(|s| s.name == "Main")
            .expect("Should find Main class");
        assert_eq!(class_sym.qualified, "Main");
    }
}
