//! Symbol extraction from Java source files using tree-sitter.

use std::cell::RefCell;
use std::path::Path;

use crate::parse::{find_child_by_kind, node_to_location, LanguageParser, ParseResult};
use crate::{Symbol, SymbolKind, Visibility};

// Thread-local parser reuse - avoids creating a new parser per file
thread_local! {
    static JAVA_PARSER: RefCell<tree_sitter::Parser> = RefCell::new({
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_java::LANGUAGE.into())
            .expect("tree-sitter-java grammar incompatible with tree-sitter version");
        parser
    });
}

pub struct JavaParser;

impl LanguageParser for JavaParser {
    fn extract_symbols(&self, file: &Path, source: &str, max_depth: usize) -> ParseResult {
        JAVA_PARSER.with(|parser| {
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

            // Extract package name from compilation_unit
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

/// Extract package name from a compilation unit
fn extract_package_name(root: &tree_sitter::Node, source: &[u8]) -> Option<String> {
    for i in 0..root.child_count() {
        if let Some(child) = root.child(i) {
            if child.kind() == "package_declaration" {
                // Get the package name (scoped_identifier or identifier)
                for j in 0..child.child_count() {
                    if let Some(name_child) = child.child(j) {
                        if name_child.kind() == "scoped_identifier"
                            || name_child.kind() == "identifier"
                        {
                            if let Ok(name) = name_child.utf8_text(source) {
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

/// Determine visibility from Java modifiers
fn extract_visibility(node: &tree_sitter::Node, source: &[u8]) -> Visibility {
    if let Some(modifiers) = find_child_by_kind(node, "modifiers") {
        for i in 0..modifiers.child_count() {
            if let Some(child) = modifiers.child(i) {
                if let Ok(text) = child.utf8_text(source) {
                    match text {
                        "public" => return Visibility::Public,
                        "protected" => return Visibility::Internal,
                        "private" => return Visibility::Private,
                        _ => {}
                    }
                }
            }
        }
    }
    // Default (package-private) is internal in our model
    Visibility::Internal
}

/// Build a qualified name with package prefix
fn qualified_name(name: &str, package: Option<&str>) -> String {
    match package {
        Some(p) => format!("{}.{}", p, name),
        None => name.to_string(),
    }
}

/// Extract Javadoc comments
fn extract_doc_comments(node: &tree_sitter::Node, source: &[u8]) -> Option<String> {
    // Look for preceding block_comment that starts with /**
    let mut prev = node.prev_sibling();
    while let Some(sib) = prev {
        if sib.kind() == "block_comment" {
            if let Ok(text) = sib.utf8_text(source) {
                if text.starts_with("/**") {
                    // Clean up the Javadoc
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
            // Stop at first non-comment
            break;
        }
        prev = sib.prev_sibling();
    }
    None
}

/// Extract annotations (e.g., @Override, @Service)
fn extract_annotations(node: &tree_sitter::Node, source: &[u8]) -> Option<Vec<String>> {
    let mut annotations = Vec::new();

    if let Some(modifiers) = find_child_by_kind(node, "modifiers") {
        for i in 0..modifiers.child_count() {
            if let Some(child) = modifiers.child(i) {
                if child.kind() == "marker_annotation" || child.kind() == "annotation" {
                    if let Ok(text) = child.utf8_text(source) {
                        annotations.push(text.to_string());
                    }
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

/// Extract method signature
fn extract_method_signature(node: &tree_sitter::Node, source: &[u8], name: &str) -> Option<String> {
    let mut sig = String::new();

    // Get return type
    if let Some(return_type) = node.child_by_field_name("type") {
        if let Ok(rt) = return_type.utf8_text(source) {
            sig.push_str(rt);
            sig.push(' ');
        }
    }

    sig.push_str(name);

    // Get parameters
    if let Some(params) = node.child_by_field_name("parameters") {
        if let Ok(params_text) = params.utf8_text(source) {
            sig.push_str(params_text);
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
                    let annotations = extract_annotations(node, source);

                    // Get superclass
                    let parent = node
                        .child_by_field_name("superclass")
                        .and_then(|sc| find_child_by_kind(&sc, "type_identifier"))
                        .and_then(|ti| ti.utf8_text(source).ok())
                        .map(|s| s.to_string());

                    // Get interfaces
                    let implements = extract_interfaces(node, source);

                    result.symbols.push(Symbol {
                        name: name.to_string(),
                        qualified: qualified.clone(),
                        kind: SymbolKind::Class,
                        location: node_to_location(file, &name_node),
                        visibility,
                        language: "java".to_string(),
                        parent,
                        mixins: None,
                        attributes: annotations,
                        implements,
                        doc,
                        signature: None,
                    });

                    // Recurse into class body
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

        "interface_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Ok(name) = name_node.utf8_text(source) {
                    let qualified = qualified_name(name, package);
                    let visibility = extract_visibility(node, source);
                    let doc = extract_doc_comments(node, source);
                    let annotations = extract_annotations(node, source);

                    result.symbols.push(Symbol {
                        name: name.to_string(),
                        qualified: qualified.clone(),
                        kind: SymbolKind::Interface,
                        location: node_to_location(file, &name_node),
                        visibility,
                        language: "java".to_string(),
                        parent: None,
                        mixins: None,
                        attributes: annotations,
                        implements: None,
                        doc,
                        signature: None,
                    });

                    // Recurse into interface body
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

        "enum_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Ok(name) = name_node.utf8_text(source) {
                    let qualified = qualified_name(name, package);
                    let visibility = extract_visibility(node, source);
                    let doc = extract_doc_comments(node, source);
                    let annotations = extract_annotations(node, source);

                    result.symbols.push(Symbol {
                        name: name.to_string(),
                        qualified: qualified.clone(),
                        kind: SymbolKind::Union, // Using Union for enums
                        location: node_to_location(file, &name_node),
                        visibility,
                        language: "java".to_string(),
                        parent: None,
                        mixins: None,
                        attributes: annotations,
                        implements: None,
                        doc,
                        signature: None,
                    });

                    // Extract enum constants
                    if let Some(body) = node.child_by_field_name("body") {
                        extract_enum_constants(&body, source, file, result, &qualified);

                        // Also process methods in enum
                        for i in 0..body.child_count() {
                            if let Some(child) = body.child(i) {
                                if child.kind() == "method_declaration" {
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
                    return;
                }
            }
        }

        "annotation_type_declaration" => {
            // Java annotation definitions: public @interface Service { ... }
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Ok(name) = name_node.utf8_text(source) {
                    let qualified = qualified_name(name, package);
                    let visibility = extract_visibility(node, source);
                    let doc = extract_doc_comments(node, source);
                    let annotations = extract_annotations(node, source);

                    result.symbols.push(Symbol {
                        name: name.to_string(),
                        qualified: qualified.clone(),
                        kind: SymbolKind::Interface, // Annotations are interfaces in Java
                        location: node_to_location(file, &name_node),
                        visibility,
                        language: "java".to_string(),
                        parent: None,
                        mixins: None,
                        attributes: annotations,
                        implements: None,
                        doc,
                        signature: None,
                    });

                    // Extract annotation elements (methods)
                    if let Some(body) = node.child_by_field_name("body") {
                        for i in 0..body.child_count() {
                            if let Some(child) = body.child(i) {
                                if child.kind() == "annotation_type_element_declaration" {
                                    // Extract the element name (it's an identifier child)
                                    for j in 0..child.child_count() {
                                        if let Some(elem_child) = child.child(j) {
                                            if elem_child.kind() == "identifier" {
                                                if let Ok(elem_name) = elem_child.utf8_text(source)
                                                {
                                                    let elem_qualified =
                                                        format!("{}.{}", qualified, elem_name);
                                                    result.symbols.push(Symbol {
                                                        name: elem_name.to_string(),
                                                        qualified: elem_qualified,
                                                        kind: SymbolKind::Function,
                                                        location: node_to_location(
                                                            file,
                                                            &elem_child,
                                                        ),
                                                        visibility: Visibility::Public,
                                                        language: "java".to_string(),
                                                        parent: Some(qualified.clone()),
                                                        mixins: None,
                                                        attributes: None,
                                                        implements: None,
                                                        doc: None,
                                                        signature: None,
                                                    });
                                                }
                                                break;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    return;
                }
            }
        }

        "record_declaration" => {
            // Java 16+ records: public record Point(int x, int y) { ... }
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Ok(name) = name_node.utf8_text(source) {
                    let qualified = qualified_name(name, package);
                    let visibility = extract_visibility(node, source);
                    let doc = extract_doc_comments(node, source);
                    let annotations = extract_annotations(node, source);

                    result.symbols.push(Symbol {
                        name: name.to_string(),
                        qualified: qualified.clone(),
                        kind: SymbolKind::Class, // Records are class-like
                        location: node_to_location(file, &name_node),
                        visibility,
                        language: "java".to_string(),
                        parent: None,
                        mixins: None,
                        attributes: annotations,
                        implements: None,
                        doc,
                        signature: None,
                    });

                    // Extract record components from formal_parameters
                    if let Some(params) = node.child_by_field_name("parameters") {
                        for i in 0..params.child_count() {
                            if let Some(param) = params.child(i) {
                                if param.kind() == "formal_parameter" {
                                    if let Some(param_name) = param.child_by_field_name("name") {
                                        if let Ok(pname) = param_name.utf8_text(source) {
                                            let param_qualified =
                                                format!("{}.{}", qualified, pname);
                                            result.symbols.push(Symbol {
                                                name: pname.to_string(),
                                                qualified: param_qualified,
                                                kind: SymbolKind::Member,
                                                location: node_to_location(file, &param_name),
                                                visibility: Visibility::Public,
                                                language: "java".to_string(),
                                                parent: Some(qualified.clone()),
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

                    // Extract methods from class_body
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

        "method_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Ok(name) = name_node.utf8_text(source) {
                    let qualified = qualified_name(name, package);
                    let visibility = extract_visibility(node, source);
                    let doc = extract_doc_comments(node, source);
                    let annotations = extract_annotations(node, source);
                    let signature = extract_method_signature(node, source, name);

                    result.symbols.push(Symbol {
                        name: name.to_string(),
                        qualified,
                        kind: SymbolKind::Function,
                        location: node_to_location(file, &name_node),
                        visibility,
                        language: "java".to_string(),
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

        "constructor_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Ok(name) = name_node.utf8_text(source) {
                    let qualified = qualified_name(name, package);
                    let visibility = extract_visibility(node, source);
                    let doc = extract_doc_comments(node, source);
                    let annotations = extract_annotations(node, source);

                    // Get constructor parameters for signature
                    let signature = if let Some(params) = node.child_by_field_name("parameters") {
                        params
                            .utf8_text(source)
                            .ok()
                            .map(|p| format!("{}{}", name, p))
                    } else {
                        Some(format!("{}()", name))
                    };

                    result.symbols.push(Symbol {
                        name: name.to_string(),
                        qualified,
                        kind: SymbolKind::Function,
                        location: node_to_location(file, &name_node),
                        visibility,
                        language: "java".to_string(),
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

        "field_declaration" => {
            // Fields can have multiple declarators
            if let Some(declarator_list) = find_child_by_kind(node, "variable_declarator") {
                if let Some(name_node) = declarator_list.child_by_field_name("name") {
                    if let Ok(name) = name_node.utf8_text(source) {
                        let qualified = qualified_name(name, package);
                        let visibility = extract_visibility(node, source);
                        let doc = extract_doc_comments(node, source);
                        let annotations = extract_annotations(node, source);

                        result.symbols.push(Symbol {
                            name: name.to_string(),
                            qualified,
                            kind: SymbolKind::Value,
                            location: node_to_location(file, &name_node),
                            visibility,
                            language: "java".to_string(),
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

        "import_declaration" => {
            // Extract import path
            for i in 0..node.child_count() {
                if let Some(child) = node.child(i) {
                    if child.kind() == "scoped_identifier" || child.kind() == "identifier" {
                        if let Ok(text) = child.utf8_text(source) {
                            result.opens.push(text.to_string());
                        }
                    }
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

/// Extract interfaces from an implements clause
fn extract_interfaces(node: &tree_sitter::Node, source: &[u8]) -> Option<Vec<String>> {
    let mut interfaces = Vec::new();

    if let Some(impl_clause) = find_child_by_kind(node, "super_interfaces") {
        for i in 0..impl_clause.child_count() {
            if let Some(child) = impl_clause.child(i) {
                if child.kind() == "type_list" {
                    for j in 0..child.child_count() {
                        if let Some(type_child) = child.child(j) {
                            if type_child.kind() == "type_identifier"
                                || type_child.kind() == "generic_type"
                            {
                                if let Ok(name) = type_child.utf8_text(source) {
                                    interfaces.push(name.to_string());
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    if interfaces.is_empty() {
        None
    } else {
        Some(interfaces)
    }
}

/// Extract enum constants
fn extract_enum_constants(
    body: &tree_sitter::Node,
    source: &[u8],
    file: &Path,
    result: &mut ParseResult,
    enum_path: &str,
) {
    for i in 0..body.child_count() {
        if let Some(child) = body.child(i) {
            if child.kind() == "enum_constant" {
                if let Some(name_node) = child.child_by_field_name("name") {
                    if let Ok(name) = name_node.utf8_text(source) {
                        let qualified = format!("{}.{}", enum_path, name);

                        result.symbols.push(Symbol {
                            name: name.to_string(),
                            qualified,
                            kind: SymbolKind::Member,
                            location: node_to_location(file, &name_node),
                            visibility: Visibility::Public,
                            language: "java".to_string(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::LanguageParser;

    #[test]
    fn extracts_java_class() {
        let source = r#"
package com.example;

/**
 * A simple user class.
 */
public class User {
    private String name;
}
"#;
        let parser = JavaParser;
        let result = parser.extract_symbols(std::path::Path::new("User.java"), source, 100);

        let class_sym = result
            .symbols
            .iter()
            .find(|s| s.name == "User")
            .expect("Should find User class");
        assert_eq!(class_sym.kind, SymbolKind::Class);
        assert_eq!(class_sym.qualified, "com.example.User");
        assert_eq!(class_sym.visibility, Visibility::Public);
        assert!(class_sym.doc.is_some());
        assert!(class_sym.doc.as_ref().unwrap().contains("simple user"));
    }

    #[test]
    fn extracts_java_interface() {
        let source = r#"
package com.example;

public interface Repository<T> {
    T findById(int id);
    void save(T entity);
}
"#;
        let parser = JavaParser;
        let result = parser.extract_symbols(std::path::Path::new("Repository.java"), source, 100);

        let iface = result
            .symbols
            .iter()
            .find(|s| s.name == "Repository")
            .expect("Should find Repository interface");
        assert_eq!(iface.kind, SymbolKind::Interface);
        assert_eq!(iface.visibility, Visibility::Public);
    }

    #[test]
    fn extracts_java_method() {
        let source = r#"
package com.example;

public class Calculator {
    /**
     * Adds two numbers.
     */
    public int add(int a, int b) {
        return a + b;
    }
}
"#;
        let parser = JavaParser;
        let result = parser.extract_symbols(std::path::Path::new("Calculator.java"), source, 100);

        let method = result
            .symbols
            .iter()
            .find(|s| s.name == "add")
            .expect("Should find add method");
        assert_eq!(method.kind, SymbolKind::Function);
        assert_eq!(method.qualified, "com.example.Calculator.add");
        assert!(method.signature.is_some());
        assert!(method.signature.as_ref().unwrap().contains("int a"));
    }

    #[test]
    fn extracts_java_enum() {
        let source = r#"
package com.example;

public enum Status {
    PENDING,
    ACTIVE,
    COMPLETED
}
"#;
        let parser = JavaParser;
        let result = parser.extract_symbols(std::path::Path::new("Status.java"), source, 100);

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
            .expect("Should find PENDING constant");
        assert_eq!(pending.kind, SymbolKind::Member);
        assert_eq!(pending.qualified, "com.example.Status.PENDING");
    }

    #[test]
    fn extracts_java_annotations() {
        let source = r#"
package com.example;

@Service
@Transactional
public class UserService {
    @Override
    public void doSomething() {}
}
"#;
        let parser = JavaParser;
        let result = parser.extract_symbols(std::path::Path::new("UserService.java"), source, 100);

        let class_sym = result
            .symbols
            .iter()
            .find(|s| s.name == "UserService")
            .expect("Should find UserService");

        assert!(class_sym.attributes.is_some());
        let attrs = class_sym.attributes.as_ref().unwrap();
        assert!(attrs.iter().any(|a| a.contains("Service")));
        assert!(attrs.iter().any(|a| a.contains("Transactional")));

        let method = result
            .symbols
            .iter()
            .find(|s| s.name == "doSomething")
            .expect("Should find doSomething");
        assert!(method.attributes.is_some());
        assert!(method
            .attributes
            .as_ref()
            .unwrap()
            .iter()
            .any(|a| a.contains("Override")));
    }

    #[test]
    fn extracts_java_inheritance() {
        let source = r#"
package com.example;

public class Dog extends Animal implements Runnable, Comparable<Dog> {
}
"#;
        let parser = JavaParser;
        let result = parser.extract_symbols(std::path::Path::new("Dog.java"), source, 100);

        let class_sym = result
            .symbols
            .iter()
            .find(|s| s.name == "Dog")
            .expect("Should find Dog");
        assert_eq!(class_sym.parent, Some("Animal".to_string()));
        // Note: interfaces in implements clause would be in `implements` field
    }

    #[test]
    fn extracts_java_imports() {
        let source = r#"
package com.example;

import java.util.List;
import java.util.Map;
import com.example.service.UserService;

public class App {}
"#;
        let parser = JavaParser;
        let result = parser.extract_symbols(std::path::Path::new("App.java"), source, 100);

        assert!(result.opens.contains(&"java.util.List".to_string()));
        assert!(result.opens.contains(&"java.util.Map".to_string()));
        assert!(result
            .opens
            .contains(&"com.example.service.UserService".to_string()));
    }

    #[test]
    fn handles_visibility_modifiers() {
        let source = r#"
package com.example;

public class Example {
    public void publicMethod() {}
    protected void protectedMethod() {}
    private void privateMethod() {}
    void packageMethod() {}
}
"#;
        let parser = JavaParser;
        let result = parser.extract_symbols(std::path::Path::new("Example.java"), source, 100);

        let public = result
            .symbols
            .iter()
            .find(|s| s.name == "publicMethod")
            .unwrap();
        assert_eq!(public.visibility, Visibility::Public);

        let protected = result
            .symbols
            .iter()
            .find(|s| s.name == "protectedMethod")
            .unwrap();
        assert_eq!(protected.visibility, Visibility::Internal);

        let private = result
            .symbols
            .iter()
            .find(|s| s.name == "privateMethod")
            .unwrap();
        assert_eq!(private.visibility, Visibility::Private);

        let package = result
            .symbols
            .iter()
            .find(|s| s.name == "packageMethod")
            .unwrap();
        assert_eq!(package.visibility, Visibility::Internal);
    }

    #[test]
    fn extracts_constructor() {
        let source = r#"
package com.example;

public class Person {
    public Person(String name, int age) {}
}
"#;
        let parser = JavaParser;
        let result = parser.extract_symbols(std::path::Path::new("Person.java"), source, 100);

        let ctor = result
            .symbols
            .iter()
            .find(|s| s.name == "Person" && s.kind == SymbolKind::Function)
            .expect("Should find constructor");
        assert!(ctor.signature.is_some());
        assert!(ctor.signature.as_ref().unwrap().contains("String name"));
    }

    #[test]
    fn extracts_field() {
        let source = r#"
package com.example;

public class Counter {
    private int count = 0;
}
"#;
        let parser = JavaParser;
        let result = parser.extract_symbols(std::path::Path::new("Counter.java"), source, 100);

        let field = result
            .symbols
            .iter()
            .find(|s| s.name == "count")
            .expect("Should find count field");
        assert_eq!(field.kind, SymbolKind::Value);
        assert_eq!(field.visibility, Visibility::Private);
    }

    // ============================================================
    // QUIRK TESTS: These test known indexing quirks/gaps
    // ============================================================

    #[test]
    fn extracts_annotation_definitions() {
        // QUIRK: Annotation definitions (@interface) are not indexed at all
        let source = r#"
package com.example;

import java.lang.annotation.Retention;
import java.lang.annotation.RetentionPolicy;

/**
 * Marks a class as a service component.
 */
@Retention(RetentionPolicy.RUNTIME)
public @interface Service {
    String value() default "";
    boolean lazy() default false;
}
"#;
        let parser = JavaParser;
        let result = parser.extract_symbols(std::path::Path::new("Service.java"), source, 100);

        // The annotation definition should be indexed
        let annotation = result.symbols.iter().find(|s| s.name == "Service");
        assert!(
            annotation.is_some(),
            "@interface Service should be indexed as a type"
        );

        let annotation = annotation.unwrap();
        assert_eq!(
            annotation.kind,
            SymbolKind::Interface,
            "Annotation definitions should be indexed as Interface"
        );
        assert_eq!(annotation.qualified, "com.example.Service");

        // Annotation methods should also be indexed
        let value_method = result.symbols.iter().find(|s| s.name == "value");
        assert!(
            value_method.is_some(),
            "Annotation method 'value' should be indexed"
        );

        let lazy_method = result.symbols.iter().find(|s| s.name == "lazy");
        assert!(
            lazy_method.is_some(),
            "Annotation method 'lazy' should be indexed"
        );
    }

    #[test]
    fn extracts_java_records() {
        // QUIRK: Java records (Java 16+) are not indexed at all
        let source = r#"
package com.example;

/**
 * Represents a point in 2D space.
 */
public record Point(int x, int y) {
    public double distanceFromOrigin() {
        return Math.sqrt(x * x + y * y);
    }
}
"#;
        let parser = JavaParser;
        let result = parser.extract_symbols(std::path::Path::new("Point.java"), source, 100);

        // The record should be indexed as a class-like type
        let record = result.symbols.iter().find(|s| s.name == "Point");
        assert!(record.is_some(), "record Point should be indexed");

        let record = record.unwrap();
        assert_eq!(
            record.kind,
            SymbolKind::Class,
            "Records should be indexed as Class"
        );
        assert_eq!(record.qualified, "com.example.Point");

        // Record components (x, y) should be indexed as members
        let x_field = result.symbols.iter().find(|s| s.name == "x");
        assert!(x_field.is_some(), "Record component 'x' should be indexed");
        assert_eq!(x_field.unwrap().qualified, "com.example.Point.x");

        let y_field = result.symbols.iter().find(|s| s.name == "y");
        assert!(y_field.is_some(), "Record component 'y' should be indexed");

        // Methods defined in records should also be indexed
        let method = result
            .symbols
            .iter()
            .find(|s| s.name == "distanceFromOrigin");
        assert!(
            method.is_some(),
            "Record method 'distanceFromOrigin' should be indexed"
        );
        assert_eq!(
            method.unwrap().qualified,
            "com.example.Point.distanceFromOrigin"
        );
    }
}
