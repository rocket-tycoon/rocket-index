//! Symbol extraction from Python source files using tree-sitter.

use std::cell::RefCell;
use std::path::Path;

use crate::parse::{find_child_by_kind, node_to_location, LanguageParser, ParseResult};
use crate::{Reference, Symbol, SymbolKind, Visibility};

// Thread-local parser reuse - avoids creating a new parser per file
thread_local! {
    static PYTHON_PARSER: RefCell<tree_sitter::Parser> = RefCell::new({
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_python::LANGUAGE.into())
            .expect("tree-sitter-python grammar incompatible with tree-sitter version");
        parser
    });
}

pub struct PythonParser;

impl LanguageParser for PythonParser {
    fn extract_symbols(&self, file: &Path, source: &str, max_depth: usize) -> ParseResult {
        PYTHON_PARSER.with(|parser| {
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

/// Extract decorators from a decorated_definition node
fn extract_decorators(node: &tree_sitter::Node, source: &[u8]) -> Vec<String> {
    let mut decorators = Vec::new();

    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            if child.kind() == "decorator" {
                // The decorator text is after the @ symbol
                if let Ok(text) = child.utf8_text(source) {
                    // Remove the @ prefix and trim whitespace
                    let decorator = text.trim_start_matches('@').trim().to_string();
                    decorators.push(decorator);
                }
            }
        }
    }

    decorators
}

/// Extract base classes from a class definition's argument_list
fn extract_base_classes(node: &tree_sitter::Node, source: &[u8]) -> Vec<String> {
    let mut bases = Vec::new();

    if let Some(args) = node.child_by_field_name("superclasses") {
        for i in 0..args.child_count() {
            if let Some(child) = args.child(i) {
                let kind = child.kind();
                // Skip parentheses and commas
                if kind == "(" || kind == ")" || kind == "," {
                    continue;
                }
                // Handle identifier, attribute, or subscript (for generics)
                if kind == "identifier" || kind == "attribute" || kind == "subscript" {
                    if let Ok(base) = child.utf8_text(source) {
                        bases.push(base.to_string());
                    }
                }
            }
        }
    }

    bases
}

/// Extract the function signature from a function definition
fn extract_function_signature(
    node: &tree_sitter::Node,
    source: &[u8],
    name: &str,
) -> Option<String> {
    let mut sig = format!("def {}", name);

    // Get parameters
    if let Some(params) = node.child_by_field_name("parameters") {
        if let Ok(params_text) = params.utf8_text(source) {
            sig.push_str(params_text);
        }
    }

    // Get return type if present
    if let Some(return_type) = node.child_by_field_name("return_type") {
        if let Ok(type_text) = return_type.utf8_text(source) {
            sig.push_str(" -> ");
            sig.push_str(type_text);
        }
    }

    Some(sig)
}

/// Extract class signature from __init__ method.
/// Returns signatures like "(name: str, age: int)" excluding self.
fn extract_class_signature(node: &tree_sitter::Node, source: &[u8]) -> Option<String> {
    // Find the class body
    let body = node.child_by_field_name("body")?;

    // Look for __init__ method in the class body
    for i in 0..body.child_count() {
        if let Some(child) = body.child(i) {
            let def_node = if child.kind() == "decorated_definition" {
                // If decorated, find the actual function_definition inside
                find_child_by_kind(&child, "function_definition")
            } else if child.kind() == "function_definition" {
                Some(child)
            } else {
                None
            };

            if let Some(func) = def_node {
                if let Some(name_node) = func.child_by_field_name("name") {
                    if let Ok(name) = name_node.utf8_text(source) {
                        if name == "__init__" {
                            // Get parameters
                            if let Some(params) = func.child_by_field_name("parameters") {
                                if let Ok(params_text) = params.utf8_text(source) {
                                    // Remove 'self' from params
                                    let cleaned = params_text
                                        .trim_start_matches('(')
                                        .trim_end_matches(')')
                                        .split(',')
                                        .filter(|p| !p.trim().starts_with("self"))
                                        .collect::<Vec<_>>()
                                        .join(", ")
                                        .trim()
                                        .to_string();

                                    if cleaned.is_empty() {
                                        return Some("()".to_string());
                                    }
                                    return Some(format!("({})", cleaned));
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    None
}

/// Extract docstring from a function or class body
fn extract_docstring(node: &tree_sitter::Node, source: &[u8]) -> Option<String> {
    // Find the block/body child
    let body = node.child_by_field_name("body")?;

    // The first statement in the body might be a docstring
    let first_stmt = body.child(0)?;

    if first_stmt.kind() == "expression_statement" {
        if let Some(expr) = first_stmt.child(0) {
            if expr.kind() == "string" {
                if let Ok(text) = expr.utf8_text(source) {
                    // Remove triple quotes and clean up the docstring
                    let trimmed = text
                        .trim_start_matches("\"\"\"")
                        .trim_start_matches("'''")
                        .trim_end_matches("\"\"\"")
                        .trim_end_matches("'''")
                        .trim();
                    return Some(trimmed.to_string());
                }
            }
        }
    }

    None
}

/// Extract class-level attribute from an expression_statement in a class body
///
/// Handles:
/// - `name = value` (Django-style field assignments)
/// - `name: type` (dataclass type annotations)
/// - `name: type = value` (annotated assignments)
fn extract_class_attribute(
    expr_stmt: &tree_sitter::Node,
    source: &[u8],
    file: &Path,
    result: &mut ParseResult,
    class_path: &str,
) {
    // Look for assignment child
    if let Some(assignment) = find_child_by_kind(expr_stmt, "assignment") {
        // Get the left side - should be identifier for simple assignments
        // Structure: assignment -> identifier (left) OR assignment -> identifier : type (left)
        if let Some(left) = assignment.child_by_field_name("left") {
            let name_node = if left.kind() == "identifier" {
                Some(left)
            } else {
                // Could be other patterns, skip for now
                None
            };

            if let Some(name_node) = name_node {
                if let Ok(name) = name_node.utf8_text(source) {
                    // Skip if it looks like an internal attribute assignment (skip _protected, __private)
                    // but keep class constants like NAME
                    let visibility = if name.starts_with("__") && !name.ends_with("__") {
                        Visibility::Private
                    } else if name.starts_with('_') {
                        Visibility::Internal
                    } else {
                        Visibility::Public
                    };

                    let qualified = format!("{}.{}", class_path, name);
                    result.symbols.push(Symbol {
                        name: name.to_string(),
                        qualified,
                        kind: SymbolKind::Member,
                        location: node_to_location(file, &name_node),
                        visibility,
                        language: "python".to_string(),
                        parent: Some(class_path.to_string()),
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

fn extract_recursive(
    node: &tree_sitter::Node,
    source: &[u8],
    file: &Path,
    result: &mut ParseResult,
    current_module: Option<&str>,
    max_depth: usize,
) {
    if max_depth == 0 {
        return;
    }

    match node.kind() {
        "decorated_definition" => {
            // Extract decorators and then process the actual definition
            let decorators = extract_decorators(node, source);

            // Find the class_definition or function_definition child
            for i in 0..node.child_count() {
                if let Some(child) = node.child(i) {
                    let kind = child.kind();
                    if kind == "class_definition" {
                        extract_definition_with_decorators(
                            &child,
                            source,
                            file,
                            result,
                            current_module,
                            max_depth,
                            Some(&decorators),
                        );
                        return; // Class handles its own recursion
                    } else if kind == "function_definition" {
                        extract_definition_with_decorators(
                            &child,
                            source,
                            file,
                            result,
                            current_module,
                            max_depth,
                            Some(&decorators),
                        );
                        // Continue to recurse for references (don't return)
                        break;
                    }
                }
            }
        }

        "class_definition" => {
            extract_definition_with_decorators(
                node,
                source,
                file,
                result,
                current_module,
                max_depth,
                None,
            );
            return;
        }

        "function_definition" => {
            extract_definition_with_decorators(
                node,
                source,
                file,
                result,
                current_module,
                max_depth,
                None,
            );
            // Don't return - continue to recurse into function body for references
        }

        "assignment" => {
            // Handle module-level assignments (constants/variables)
            // Only extract if we're at module level (no current_module with a class)
            if let Some(left) = node.child_by_field_name("left") {
                if left.kind() == "identifier" {
                    if let Ok(name) = left.utf8_text(source) {
                        // Check if it looks like a constant (ALL_CAPS)
                        let is_constant = name.chars().all(|c| c.is_uppercase() || c == '_');
                        if is_constant {
                            let qualified = qualified_name(name, current_module);
                            result.symbols.push(Symbol {
                                name: name.to_string(),
                                qualified,
                                kind: SymbolKind::Value,
                                location: node_to_location(file, &left),
                                visibility: Visibility::Public,
                                language: "python".to_string(),
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
            }
        }

        "import_statement" => {
            // import a, b or import a.b.c
            for i in 0..node.child_count() {
                if let Some(child) = node.child(i) {
                    if child.kind() == "dotted_name" || child.kind() == "aliased_import" {
                        if let Ok(import_name) = child.utf8_text(source) {
                            // For aliased imports like "import a as b", extract the original name
                            let name = if child.kind() == "aliased_import" {
                                if let Some(dotted) = find_child_by_kind(&child, "dotted_name") {
                                    dotted.utf8_text(source).ok()
                                } else {
                                    Some(import_name)
                                }
                            } else {
                                Some(import_name)
                            };

                            if let Some(name) = name {
                                result.opens.push(name.to_string());
                            }
                        }
                    }
                }
            }
        }

        "import_from_statement" => {
            // from a import b, c or from a.b import c
            // Extract the module being imported from
            let module_name = node
                .child_by_field_name("module_name")
                .and_then(|n| n.utf8_text(source).ok())
                .map(|s| s.to_string());

            if let Some(module) = module_name {
                result.opens.push(module);
            } else {
                // Try to find relative_import
                for i in 0..node.child_count() {
                    if let Some(child) = node.child(i) {
                        if child.kind() == "relative_import" {
                            if let Ok(text) = child.utf8_text(source) {
                                result.opens.push(text.to_string());
                            }
                        } else if child.kind() == "dotted_name"
                            && i < node.child_count().saturating_sub(1)
                        {
                            // This might be the module name before 'import'
                            if let Ok(text) = child.utf8_text(source) {
                                result.opens.push(text.to_string());
                                break;
                            }
                        }
                    }
                }
            }
        }

        // Extract references from identifiers
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

        // Extract references from attributes (like obj.attr)
        "attribute" => {
            // The whole attribute expression is a reference
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
            extract_recursive(&child, source, file, result, current_module, max_depth - 1);
        }
    }
}

fn extract_definition_with_decorators(
    node: &tree_sitter::Node,
    source: &[u8],
    file: &Path,
    result: &mut ParseResult,
    current_module: Option<&str>,
    max_depth: usize,
    decorators: Option<&Vec<String>>,
) {
    match node.kind() {
        "class_definition" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Ok(name) = name_node.utf8_text(source) {
                    let qualified = qualified_name(name, current_module);

                    // Extract base classes
                    let bases = extract_base_classes(node, source);
                    let parent = bases.first().cloned();
                    let implements = if bases.len() > 1 {
                        Some(bases[1..].to_vec())
                    } else {
                        None
                    };

                    // Extract docstring
                    let doc = extract_docstring(node, source);

                    // Extract constructor signature from __init__
                    let signature = extract_class_signature(node, source);

                    result.symbols.push(Symbol {
                        name: name.to_string(),
                        qualified: qualified.clone(),
                        kind: SymbolKind::Class,
                        location: node_to_location(file, &name_node),
                        visibility: Visibility::Public,
                        language: "python".to_string(),
                        parent,
                        mixins: None,
                        attributes: decorators.cloned(),
                        implements,
                        doc,
                        signature,
                    });

                    // Process class body
                    if let Some(body) = node.child_by_field_name("body") {
                        for i in 0..body.child_count() {
                            if let Some(child) = body.child(i) {
                                // Handle expression_statement containing assignments as class members
                                if child.kind() == "expression_statement" {
                                    extract_class_attribute(
                                        &child, source, file, result, &qualified,
                                    );
                                }
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

        "function_definition" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Ok(name) = name_node.utf8_text(source) {
                    // Check visibility based on name prefix
                    // Dunder methods (__name__) are public, __private are private, _protected are internal
                    let visibility = if name.starts_with("__") && name.ends_with("__") {
                        Visibility::Public // Dunder methods like __init__, __str__
                    } else if name.starts_with("__") {
                        Visibility::Private // Name-mangled private like __private
                    } else if name.starts_with('_') {
                        Visibility::Internal // Python convention for "protected"
                    } else {
                        Visibility::Public
                    };

                    // Use dot separator for methods, like Python conventions
                    let separator = ".";
                    let qualified = match current_module {
                        Some(m) => format!("{}{}{}", m, separator, name),
                        None => name.to_string(),
                    };

                    // Extract signature and docstring
                    let signature = extract_function_signature(node, source, name);
                    let doc = extract_docstring(node, source);

                    result.symbols.push(Symbol {
                        name: name.to_string(),
                        qualified,
                        kind: SymbolKind::Function,
                        location: node_to_location(file, &name_node),
                        visibility,
                        language: "python".to_string(),
                        parent: None,
                        mixins: None,
                        attributes: decorators.cloned(),
                        implements: None,
                        doc,
                        signature,
                    });
                }
            }
        }

        _ => {}
    }
}

/// Maximum recursion depth for helper functions to prevent stack overflow
const MAX_HELPER_DEPTH: usize = 50;

/// Check if an identifier node is in a reference context (not a definition).
/// Returns true if the identifier is being used/referenced, false if it's being defined.
fn is_reference_context(node: &tree_sitter::Node) -> bool {
    is_reference_context_with_depth(node, 0)
}

fn is_reference_context_with_depth(node: &tree_sitter::Node, depth: usize) -> bool {
    // Prevent stack overflow on deeply nested parent chains
    if depth > MAX_HELPER_DEPTH {
        return false; // Conservative: treat unknown deep context as definition
    }

    if let Some(parent) = node.parent() {
        match parent.kind() {
            // Definition contexts - these are NOT references
            "function_definition" => {
                // Check if this is the function name being defined
                if let Some(name_node) = parent.child_by_field_name("name") {
                    if name_node.id() == node.id() {
                        return false;
                    }
                }
                // Otherwise it's a reference (in parameters, return type, or body)
                is_reference_context_with_depth(&parent, depth + 1)
            }
            "class_definition" => {
                // Check if this is the class name being defined
                if let Some(name_node) = parent.child_by_field_name("name") {
                    if name_node.id() == node.id() {
                        return false;
                    }
                }
                // Superclasses and body are references
                is_reference_context_with_depth(&parent, depth + 1)
            }
            "parameters" | "typed_parameter" | "default_parameter" => {
                // Parameter names are definitions
                // But type annotations within parameters are references
                // The direct child identifier of a parameter is the binding
                if parent.kind() == "parameters" {
                    // Parameters is a container - check what the direct parent is
                    is_reference_context_with_depth(&parent, depth + 1)
                } else {
                    // typed_parameter or default_parameter
                    // First identifier is usually the binding name
                    if let Some(first_child) = parent.child(0) {
                        if first_child.id() == node.id() {
                            return false; // This is the parameter name
                        }
                    }
                    // Otherwise it's a type annotation
                    true
                }
            }
            "for_statement" => {
                // The loop variable is a definition
                if let Some(left) = parent.child_by_field_name("left") {
                    if left.id() == node.id() || is_descendant_of(node, &left) {
                        return false;
                    }
                }
                // The iterable is a reference
                true
            }
            "assignment" => {
                // The left side is a definition (usually)
                if let Some(left) = parent.child_by_field_name("left") {
                    if left.id() == node.id() || is_descendant_of(node, &left) {
                        return false;
                    }
                }
                // The right side is a reference
                true
            }
            "except_clause" => {
                // The alias in "except Error as e" is a definition
                if let Some(alias) = parent.child_by_field_name("alias") {
                    if alias.id() == node.id() {
                        return false;
                    }
                }
                // The exception type is a reference
                true
            }
            "import_statement" | "import_from_statement" => {
                // Import names are references to modules
                true
            }
            "with_item" => {
                // The alias in "with x as y" is a definition
                if let Some(alias) = parent.child_by_field_name("alias") {
                    if alias.id() == node.id() || is_descendant_of(node, &alias) {
                        return false;
                    }
                }
                true
            }
            "decorated_definition" => {
                // Decorators are references
                true
            }
            // Clear reference contexts
            "call" | "argument_list" | "subscript" | "generic_type" => true,
            "binary_operator" | "unary_operator" | "boolean_operator" | "not_operator" => true,
            "comparison_operator" => true,
            "return_statement" | "yield" => true,
            "if_statement" | "while_statement" | "conditional_expression" => true,
            "list" | "tuple" | "dictionary" | "set" => true,
            "list_comprehension" | "dictionary_comprehension" | "set_comprehension" => true,
            "generator_expression" => true,
            "expression_statement" => true,
            "assert_statement" | "raise_statement" => true,
            "parenthesized_expression" => true,
            "type" => true, // Type annotations are references
            "block" => true,

            // Continue checking parent for ambiguous contexts
            _ => is_reference_context_with_depth(&parent, depth + 1),
        }
    } else {
        // No parent - likely top-level, not a reference
        false
    }
}

/// Check if a node is a descendant of another node
fn is_descendant_of(node: &tree_sitter::Node, ancestor: &tree_sitter::Node) -> bool {
    let mut current = node.parent();
    while let Some(parent) = current {
        if parent.id() == ancestor.id() {
            return true;
        }
        current = parent.parent();
    }
    false
}

fn qualified_name(name: &str, current_module: Option<&str>) -> String {
    match current_module {
        Some(m) => format!("{}.{}", m, name),
        None => name.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::extract_symbols;

    #[test]
    fn extracts_python_class() {
        let source = r#"
class MyClass:
    """A simple class."""
    def __init__(self):
        pass
"#;
        let result = extract_symbols(std::path::Path::new("test.py"), source, 100);

        assert!(
            !result.symbols.is_empty(),
            "Should extract at least one symbol"
        );
        let class_sym = result
            .symbols
            .iter()
            .find(|s| s.name == "MyClass")
            .expect("Should find MyClass");
        assert_eq!(class_sym.kind, SymbolKind::Class);
        assert_eq!(class_sym.qualified, "MyClass");
        assert_eq!(class_sym.doc, Some("A simple class.".to_string()));
    }

    #[test]
    fn extracts_python_function() {
        let source = r#"
def my_function(x: int, y: str) -> bool:
    """Does something useful."""
    return True
"#;
        let result = extract_symbols(std::path::Path::new("test.py"), source, 100);

        let func = result
            .symbols
            .iter()
            .find(|s| s.name == "my_function")
            .expect("Should find my_function");
        assert_eq!(func.kind, SymbolKind::Function);
        assert_eq!(func.qualified, "my_function");
        assert!(func.signature.is_some());
        assert!(func.signature.as_ref().unwrap().contains("x: int"));
        assert!(func.signature.as_ref().unwrap().contains("-> bool"));
        assert_eq!(func.doc, Some("Does something useful.".to_string()));
    }

    #[test]
    fn extracts_python_method() {
        let source = r#"
class Calculator:
    def add(self, a: int, b: int) -> int:
        return a + b
"#;
        let result = extract_symbols(std::path::Path::new("test.py"), source, 100);

        let method = result
            .symbols
            .iter()
            .find(|s| s.name == "add")
            .expect("Should find add method");
        assert_eq!(method.kind, SymbolKind::Function);
        assert_eq!(method.qualified, "Calculator.add");
    }

    #[test]
    fn extracts_python_decorated_class() {
        let source = r#"
@dataclass
@frozen
class Point:
    x: int
    y: int
"#;
        let result = extract_symbols(std::path::Path::new("test.py"), source, 100);

        let class_sym = result
            .symbols
            .iter()
            .find(|s| s.name == "Point")
            .expect("Should find Point");
        assert_eq!(class_sym.kind, SymbolKind::Class);
        assert!(class_sym.attributes.is_some());
        let attrs = class_sym.attributes.as_ref().unwrap();
        assert!(attrs.iter().any(|a| a.contains("dataclass")));
        assert!(attrs.iter().any(|a| a.contains("frozen")));
    }

    #[test]
    fn extracts_python_decorated_function() {
        let source = r#"
@staticmethod
def helper():
    pass

@property
def name(self):
    return self._name
"#;
        let result = extract_symbols(std::path::Path::new("test.py"), source, 100);

        let helper = result
            .symbols
            .iter()
            .find(|s| s.name == "helper")
            .expect("Should find helper");
        assert!(helper.attributes.is_some());
        assert!(helper
            .attributes
            .as_ref()
            .unwrap()
            .iter()
            .any(|a| a.contains("staticmethod")));

        let name_prop = result
            .symbols
            .iter()
            .find(|s| s.name == "name")
            .expect("Should find name property");
        assert!(name_prop.attributes.is_some());
        assert!(name_prop
            .attributes
            .as_ref()
            .unwrap()
            .iter()
            .any(|a| a.contains("property")));
    }

    #[test]
    fn extracts_python_inheritance() {
        let source = r#"
class Animal:
    pass

class Dog(Animal):
    pass

class MultiInherit(Base1, Base2, Mixin):
    pass
"#;
        let result = extract_symbols(std::path::Path::new("test.py"), source, 100);

        let dog = result
            .symbols
            .iter()
            .find(|s| s.name == "Dog")
            .expect("Should find Dog");
        assert_eq!(dog.parent, Some("Animal".to_string()));

        let multi = result
            .symbols
            .iter()
            .find(|s| s.name == "MultiInherit")
            .expect("Should find MultiInherit");
        assert_eq!(multi.parent, Some("Base1".to_string()));
        assert!(multi.implements.is_some());
        let impls = multi.implements.as_ref().unwrap();
        assert!(impls.contains(&"Base2".to_string()));
        assert!(impls.contains(&"Mixin".to_string()));
    }

    #[test]
    fn extracts_python_imports() {
        let source = r#"
import os
import sys
import json as j
from pathlib import Path
from typing import List, Dict
from . import utils
from ..models import User
"#;
        let result = extract_symbols(std::path::Path::new("test.py"), source, 100);

        assert!(result.opens.contains(&"os".to_string()));
        assert!(result.opens.contains(&"sys".to_string()));
        assert!(result.opens.contains(&"json".to_string()));
        assert!(result.opens.contains(&"pathlib".to_string()));
        assert!(result.opens.contains(&"typing".to_string()));
    }

    #[test]
    fn extracts_python_constants() {
        let source = r#"
MAX_RETRIES = 5
DEFAULT_TIMEOUT = 30
API_KEY = "secret"
"#;
        let result = extract_symbols(std::path::Path::new("test.py"), source, 100);

        let max_retries = result
            .symbols
            .iter()
            .find(|s| s.name == "MAX_RETRIES")
            .expect("Should find MAX_RETRIES");
        assert_eq!(max_retries.kind, SymbolKind::Value);

        let timeout = result
            .symbols
            .iter()
            .find(|s| s.name == "DEFAULT_TIMEOUT")
            .expect("Should find DEFAULT_TIMEOUT");
        assert_eq!(timeout.kind, SymbolKind::Value);
    }

    #[test]
    fn extracts_python_private_methods() {
        let source = r#"
class MyClass:
    def public_method(self):
        pass

    def _protected_method(self):
        pass

    def __private_method(self):
        pass

    def __dunder__(self):
        pass
"#;
        let result = extract_symbols(std::path::Path::new("test.py"), source, 100);

        let public = result
            .symbols
            .iter()
            .find(|s| s.name == "public_method")
            .expect("Should find public_method");
        assert_eq!(public.visibility, Visibility::Public);

        let protected = result
            .symbols
            .iter()
            .find(|s| s.name == "_protected_method")
            .expect("Should find _protected_method");
        assert_eq!(protected.visibility, Visibility::Internal);

        let private = result
            .symbols
            .iter()
            .find(|s| s.name == "__private_method")
            .expect("Should find __private_method");
        assert_eq!(private.visibility, Visibility::Private);

        // Dunder methods are public
        let dunder = result
            .symbols
            .iter()
            .find(|s| s.name == "__dunder__")
            .expect("Should find __dunder__");
        assert_eq!(dunder.visibility, Visibility::Public);
    }

    #[test]
    fn extracts_nested_classes() {
        let source = r#"
class Outer:
    class Inner:
        def inner_method(self):
            pass
"#;
        let result = extract_symbols(std::path::Path::new("test.py"), source, 100);

        let outer = result
            .symbols
            .iter()
            .find(|s| s.name == "Outer")
            .expect("Should find Outer");
        assert_eq!(outer.qualified, "Outer");

        let inner = result
            .symbols
            .iter()
            .find(|s| s.name == "Inner")
            .expect("Should find Inner");
        assert_eq!(inner.qualified, "Outer.Inner");

        let method = result
            .symbols
            .iter()
            .find(|s| s.name == "inner_method")
            .expect("Should find inner_method");
        assert_eq!(method.qualified, "Outer.Inner.inner_method");
    }

    #[test]
    fn extracts_async_functions() {
        let source = r#"
async def fetch_data(url: str) -> dict:
    """Fetch data from URL."""
    pass

class ApiClient:
    async def get(self, endpoint: str):
        pass
"#;
        let result = extract_symbols(std::path::Path::new("test.py"), source, 100);

        let fetch = result
            .symbols
            .iter()
            .find(|s| s.name == "fetch_data")
            .expect("Should find fetch_data");
        assert_eq!(fetch.kind, SymbolKind::Function);

        let get = result
            .symbols
            .iter()
            .find(|s| s.name == "get")
            .expect("Should find get");
        assert_eq!(get.qualified, "ApiClient.get");
    }

    #[test]
    fn handles_complex_decorators() {
        let source = r#"
@app.route("/api/users", methods=["GET", "POST"])
def list_users():
    pass

@pytest.mark.parametrize("input,expected", [(1, 2), (2, 4)])
def test_double(input, expected):
    pass
"#;
        let result = extract_symbols(std::path::Path::new("test.py"), source, 100);

        let list_users = result
            .symbols
            .iter()
            .find(|s| s.name == "list_users")
            .expect("Should find list_users");
        assert!(list_users.attributes.is_some());
        assert!(list_users
            .attributes
            .as_ref()
            .unwrap()
            .iter()
            .any(|a| a.contains("app.route")));

        let test = result
            .symbols
            .iter()
            .find(|s| s.name == "test_double")
            .expect("Should find test_double");
        assert!(test.attributes.is_some());
        assert!(test
            .attributes
            .as_ref()
            .unwrap()
            .iter()
            .any(|a| a.contains("pytest.mark.parametrize")));
    }

    #[test]
    fn extracts_python_class_constructor_signature() {
        let source = r#"
class Person:
    def __init__(self, name: str, age: int):
        self.name = name
        self.age = age
"#;
        let result = extract_symbols(std::path::Path::new("test.py"), source, 100);

        let class_sym = result
            .symbols
            .iter()
            .find(|s| s.name == "Person")
            .expect("Should find Person");

        assert_eq!(class_sym.kind, SymbolKind::Class);
        let sig = class_sym
            .signature
            .as_ref()
            .expect("Class should have signature");
        assert!(
            sig.contains("name: str"),
            "Should contain name param: {}",
            sig
        );
        assert!(
            sig.contains("age: int"),
            "Should contain age param: {}",
            sig
        );
        assert!(!sig.contains("self"), "Should not contain self: {}", sig);
    }

    #[test]
    fn extracts_python_class_empty_constructor() {
        let source = r#"
class Empty:
    def __init__(self):
        pass
"#;
        let result = extract_symbols(std::path::Path::new("test.py"), source, 100);

        let class_sym = result
            .symbols
            .iter()
            .find(|s| s.name == "Empty")
            .expect("Should find Empty");

        assert_eq!(class_sym.kind, SymbolKind::Class);
        let sig = class_sym
            .signature
            .as_ref()
            .expect("Class should have signature");
        assert_eq!(sig, "()", "Empty constructor should be (): {}", sig);
    }

    #[test]
    fn extracts_python_class_no_constructor() {
        let source = r#"
class NoInit:
    pass
"#;
        let result = extract_symbols(std::path::Path::new("test.py"), source, 100);

        let class_sym = result
            .symbols
            .iter()
            .find(|s| s.name == "NoInit")
            .expect("Should find NoInit");

        assert_eq!(class_sym.kind, SymbolKind::Class);
        assert!(
            class_sym.signature.is_none(),
            "Class without __init__ should have no signature"
        );
    }

    #[test]
    fn extracts_class_level_assignments() {
        let source = r#"
class Author:
    name = models.CharField(max_length=100)
    slug = models.SlugField()

    def __str__(self):
        return self.name

@dataclass
class Task:
    priority: int
    func: Callable
"#;
        let result = extract_symbols(std::path::Path::new("test.py"), source, 100);

        // Django-style field assignments should be extracted as members
        let name = result
            .symbols
            .iter()
            .find(|s| s.name == "name" && s.qualified == "Author.name")
            .expect("Should find Author.name");
        assert_eq!(name.kind, SymbolKind::Member);
        assert_eq!(name.parent, Some("Author".to_string()));

        let slug = result
            .symbols
            .iter()
            .find(|s| s.name == "slug" && s.qualified == "Author.slug")
            .expect("Should find Author.slug");
        assert_eq!(slug.kind, SymbolKind::Member);

        // Dataclass type annotations should be extracted as members
        let priority = result
            .symbols
            .iter()
            .find(|s| s.name == "priority" && s.qualified == "Task.priority")
            .expect("Should find Task.priority");
        assert_eq!(priority.kind, SymbolKind::Member);
        assert_eq!(priority.parent, Some("Task".to_string()));

        let func = result
            .symbols
            .iter()
            .find(|s| s.name == "func" && s.qualified == "Task.func")
            .expect("Should find Task.func");
        assert_eq!(func.kind, SymbolKind::Member);
    }

    #[test]
    fn extracts_python_references() {
        let source = r#"
from typing import List

class User:
    name: str

def process(users: List[User]) -> int:
    result = len(users)
    for user in users:
        print(user.name)
    return result

def main():
    users = [User()]
    process(users)
"#;
        let result = extract_symbols(std::path::Path::new("test.py"), source, 100);

        assert!(
            !result.references.is_empty(),
            "Should extract references from Python code"
        );
        let ref_names: Vec<_> = result.references.iter().map(|r| r.name.as_str()).collect();
        // Should have reference to User (in type annotation and instantiation)
        assert!(
            ref_names.contains(&"User"),
            "Should have reference to User: {:?}",
            ref_names
        );
        // Should have reference to process (function call)
        assert!(
            ref_names.contains(&"process"),
            "Should have reference to process: {:?}",
            ref_names
        );
    }
}
