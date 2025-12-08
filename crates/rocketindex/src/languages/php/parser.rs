//! Symbol extraction from PHP source files using tree-sitter.

use std::cell::RefCell;
use std::path::Path;

use crate::parse::{find_child_by_kind, node_to_location, LanguageParser, ParseResult};
use crate::{Symbol, SymbolKind, Visibility};

// Thread-local parser reuse - avoids creating a new parser per file
thread_local! {
    static PHP_PARSER: RefCell<tree_sitter::Parser> = RefCell::new({
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_php::LANGUAGE_PHP.into())
            .expect("tree-sitter-php grammar incompatible with tree-sitter version");
        parser
    });
}

pub struct PhpParser;

impl LanguageParser for PhpParser {
    fn extract_symbols(&self, file: &Path, source: &str, max_depth: usize) -> ParseResult {
        PHP_PARSER.with(|parser| {
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

            extract_recursive(
                &root,
                source.as_bytes(),
                file,
                &mut result,
                None, // namespace
                max_depth,
            );

            result
        })
    }
}

/// Determine visibility from PHP modifiers
fn extract_visibility(node: &tree_sitter::Node, source: &[u8]) -> Visibility {
    // Look for visibility_modifier child
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            if child.kind() == "visibility_modifier" {
                if let Ok(text) = child.utf8_text(source) {
                    return match text {
                        "public" => Visibility::Public,
                        "protected" => Visibility::Internal,
                        "private" => Visibility::Private,
                        _ => Visibility::Public,
                    };
                }
            }
        }
    }
    // Default visibility is public in PHP
    Visibility::Public
}

/// Build a qualified name with namespace prefix
fn qualified_name(name: &str, namespace: Option<&str>) -> String {
    match namespace {
        Some(ns) => format!("{}\\{}", ns, name),
        None => name.to_string(),
    }
}

/// Extract PHPDoc comments
fn extract_doc_comments(node: &tree_sitter::Node, source: &[u8]) -> Option<String> {
    // Look for preceding comment that starts with /**
    let mut prev = node.prev_sibling();
    while let Some(sib) = prev {
        if sib.kind() == "comment" {
            if let Ok(text) = sib.utf8_text(source) {
                if text.starts_with("/**") {
                    // Clean up the PHPDoc
                    let cleaned = text
                        .trim_start_matches("/**")
                        .trim_end_matches("*/")
                        .lines()
                        .map(|line| line.trim().trim_start_matches('*').trim())
                        .filter(|line| !line.is_empty() && !line.starts_with('@'))
                        .collect::<Vec<_>>()
                        .join("\n");
                    if !cleaned.is_empty() {
                        return Some(cleaned);
                    }
                }
            }
        } else if sib.kind() != "comment" {
            // Stop at first non-comment
            break;
        }
        prev = sib.prev_sibling();
    }
    None
}

/// Extract PHP attributes (PHP 8+)
fn extract_attributes(node: &tree_sitter::Node, source: &[u8]) -> Option<Vec<String>> {
    let mut attributes = Vec::new();

    // Look for attribute_list children
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            if child.kind() == "attribute_list" {
                if let Ok(text) = child.utf8_text(source) {
                    attributes.push(text.to_string());
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

/// Extract method/function signature
fn extract_signature(node: &tree_sitter::Node, source: &[u8], name: &str) -> Option<String> {
    let mut sig = String::new();

    sig.push_str(name);

    // Get parameters
    if let Some(params) = find_child_by_kind(node, "formal_parameters") {
        if let Ok(params_text) = params.utf8_text(source) {
            sig.push_str(params_text);
        }
    }

    // Get return type
    if let Some(return_type) = node.child_by_field_name("return_type") {
        if let Ok(rt) = return_type.utf8_text(source) {
            sig.push_str(": ");
            sig.push_str(rt);
        }
    }

    Some(sig)
}

/// Extract namespace name from namespace_definition
fn extract_namespace_name(node: &tree_sitter::Node, source: &[u8]) -> Option<String> {
    // Look for namespace_name child
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            if child.kind() == "namespace_name" {
                if let Ok(text) = child.utf8_text(source) {
                    return Some(text.to_string());
                }
            }
        }
    }
    None
}

/// Extract parent class (extends)
fn extract_parent_class(node: &tree_sitter::Node, source: &[u8]) -> Option<String> {
    if let Some(base_clause) = find_child_by_kind(node, "base_clause") {
        // Get the first name in base_clause
        for i in 0..base_clause.child_count() {
            if let Some(child) = base_clause.child(i) {
                if child.kind() == "name" || child.kind() == "qualified_name" {
                    if let Ok(text) = child.utf8_text(source) {
                        return Some(text.to_string());
                    }
                }
            }
        }
    }
    None
}

/// Extract implemented interfaces
fn extract_interfaces(node: &tree_sitter::Node, source: &[u8]) -> Option<Vec<String>> {
    let mut interfaces = Vec::new();

    if let Some(impl_clause) = find_child_by_kind(node, "class_interface_clause") {
        for i in 0..impl_clause.child_count() {
            if let Some(child) = impl_clause.child(i) {
                if child.kind() == "name" || child.kind() == "qualified_name" {
                    if let Ok(text) = child.utf8_text(source) {
                        interfaces.push(text.to_string());
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

/// Extract traits used by a class
fn extract_traits(node: &tree_sitter::Node, source: &[u8]) -> Option<Vec<String>> {
    let mut traits = Vec::new();

    // Look for use_declaration in class body
    if let Some(body) = find_child_by_kind(node, "declaration_list") {
        for i in 0..body.child_count() {
            if let Some(child) = body.child(i) {
                if child.kind() == "use_declaration" {
                    // Get trait names
                    for j in 0..child.child_count() {
                        if let Some(name_child) = child.child(j) {
                            if name_child.kind() == "name" || name_child.kind() == "qualified_name"
                            {
                                if let Ok(text) = name_child.utf8_text(source) {
                                    traits.push(text.to_string());
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    if traits.is_empty() {
        None
    } else {
        Some(traits)
    }
}

fn extract_recursive(
    node: &tree_sitter::Node,
    source: &[u8],
    file: &Path,
    result: &mut ParseResult,
    namespace: Option<&str>,
    max_depth: usize,
) {
    if max_depth == 0 {
        return;
    }

    match node.kind() {
        "namespace_definition" => {
            let ns_name = extract_namespace_name(node, source);

            // Record namespace as module path
            if let Some(ref ns) = ns_name {
                result.module_path = Some(ns.clone());
            }

            // Recurse into namespace body
            if let Some(body) = find_child_by_kind(node, "compound_statement") {
                for i in 0..body.child_count() {
                    if let Some(child) = body.child(i) {
                        extract_recursive(
                            &child,
                            source,
                            file,
                            result,
                            ns_name.as_deref(),
                            max_depth - 1,
                        );
                    }
                }
                return;
            }

            // For non-braced namespaces, continue with the rest of the file
            // but update the namespace context
            for i in 0..node.parent().map_or(0, |p| p.child_count()) {
                if let Some(sibling) = node.parent().and_then(|p| p.child(i)) {
                    if sibling.start_byte() > node.end_byte() {
                        extract_recursive(
                            &sibling,
                            source,
                            file,
                            result,
                            ns_name.as_deref(),
                            max_depth - 1,
                        );
                    }
                }
            }
            return;
        }

        "class_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Ok(name) = name_node.utf8_text(source) {
                    let qualified = qualified_name(name, namespace);
                    let visibility = Visibility::Public; // Classes are public by default
                    let doc = extract_doc_comments(node, source);
                    let attributes = extract_attributes(node, source);
                    let parent = extract_parent_class(node, source);
                    let interfaces = extract_interfaces(node, source);
                    let traits = extract_traits(node, source);

                    result.symbols.push(Symbol {
                        name: name.to_string(),
                        qualified: qualified.clone(),
                        kind: SymbolKind::Class,
                        location: node_to_location(file, &name_node),
                        visibility,
                        language: "php".to_string(),
                        parent,
                        mixins: traits,
                        attributes,
                        implements: interfaces,
                        doc,
                        signature: None,
                    });

                    // Recurse into class body
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

        "interface_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Ok(name) = name_node.utf8_text(source) {
                    let qualified = qualified_name(name, namespace);
                    let doc = extract_doc_comments(node, source);
                    let attributes = extract_attributes(node, source);

                    // Get extended interfaces
                    let parent_interfaces = extract_parent_interfaces(node, source);

                    result.symbols.push(Symbol {
                        name: name.to_string(),
                        qualified: qualified.clone(),
                        kind: SymbolKind::Interface,
                        location: node_to_location(file, &name_node),
                        visibility: Visibility::Public,
                        language: "php".to_string(),
                        parent: None,
                        mixins: None,
                        attributes,
                        implements: parent_interfaces,
                        doc,
                        signature: None,
                    });

                    // Recurse into interface body
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

        "trait_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Ok(name) = name_node.utf8_text(source) {
                    let qualified = qualified_name(name, namespace);
                    let doc = extract_doc_comments(node, source);
                    let attributes = extract_attributes(node, source);

                    result.symbols.push(Symbol {
                        name: name.to_string(),
                        qualified: qualified.clone(),
                        kind: SymbolKind::Module, // Traits are like mixins
                        location: node_to_location(file, &name_node),
                        visibility: Visibility::Public,
                        language: "php".to_string(),
                        parent: None,
                        mixins: None,
                        attributes,
                        implements: None,
                        doc,
                        signature: None,
                    });

                    // Recurse into trait body
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

        "enum_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Ok(name) = name_node.utf8_text(source) {
                    let qualified = qualified_name(name, namespace);
                    let doc = extract_doc_comments(node, source);
                    let attributes = extract_attributes(node, source);

                    result.symbols.push(Symbol {
                        name: name.to_string(),
                        qualified: qualified.clone(),
                        kind: SymbolKind::Union, // Using Union for enums
                        location: node_to_location(file, &name_node),
                        visibility: Visibility::Public,
                        language: "php".to_string(),
                        parent: None,
                        mixins: None,
                        attributes,
                        implements: None,
                        doc,
                        signature: None,
                    });

                    // Recurse into enum body for cases
                    if let Some(body) = find_child_by_kind(node, "enum_declaration_list") {
                        for i in 0..body.child_count() {
                            if let Some(child) = body.child(i) {
                                if child.kind() == "enum_case" {
                                    extract_enum_case(&child, source, file, result, &qualified);
                                } else {
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

        "function_definition" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Ok(name) = name_node.utf8_text(source) {
                    let qualified = qualified_name(name, namespace);
                    let doc = extract_doc_comments(node, source);
                    let attributes = extract_attributes(node, source);
                    let signature = extract_signature(node, source, name);

                    result.symbols.push(Symbol {
                        name: name.to_string(),
                        qualified,
                        kind: SymbolKind::Function,
                        location: node_to_location(file, &name_node),
                        visibility: Visibility::Public,
                        language: "php".to_string(),
                        parent: None,
                        mixins: None,
                        attributes,
                        implements: None,
                        doc,
                        signature,
                    });
                }
            }
        }

        "method_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Ok(name) = name_node.utf8_text(source) {
                    let qualified = qualified_name(name, namespace);
                    let visibility = extract_visibility(node, source);
                    let doc = extract_doc_comments(node, source);
                    let attributes = extract_attributes(node, source);
                    let signature = extract_signature(node, source, name);

                    result.symbols.push(Symbol {
                        name: name.to_string(),
                        qualified,
                        kind: SymbolKind::Function,
                        location: node_to_location(file, &name_node),
                        visibility,
                        language: "php".to_string(),
                        parent: None,
                        mixins: None,
                        attributes,
                        implements: None,
                        doc,
                        signature,
                    });
                }
            }
        }

        "property_declaration" => {
            // Properties can have multiple declarators
            for i in 0..node.child_count() {
                if let Some(child) = node.child(i) {
                    if child.kind() == "property_element" {
                        if let Some(var_node) = find_child_by_kind(&child, "variable_name") {
                            if let Ok(var_text) = var_node.utf8_text(source) {
                                // Remove the $ prefix for the name
                                let name = var_text.trim_start_matches('$');
                                let qualified = qualified_name(name, namespace);
                                let visibility = extract_visibility(node, source);
                                let doc = extract_doc_comments(node, source);
                                let attributes = extract_attributes(node, source);

                                result.symbols.push(Symbol {
                                    name: name.to_string(),
                                    qualified,
                                    kind: SymbolKind::Value,
                                    location: node_to_location(file, &var_node),
                                    visibility,
                                    language: "php".to_string(),
                                    parent: None,
                                    mixins: None,
                                    attributes,
                                    implements: None,
                                    doc,
                                    signature: None,
                                });
                            }
                        }
                    }
                }
            }
        }

        "const_declaration" => {
            // Class constants
            for i in 0..node.child_count() {
                if let Some(child) = node.child(i) {
                    if child.kind() == "const_element" {
                        // "name" is a child type, not a field
                        if let Some(name_node) = find_child_by_kind(&child, "name") {
                            if let Ok(name) = name_node.utf8_text(source) {
                                let qualified = qualified_name(name, namespace);
                                let visibility = extract_visibility(node, source);
                                let doc = extract_doc_comments(node, source);

                                result.symbols.push(Symbol {
                                    name: name.to_string(),
                                    qualified,
                                    kind: SymbolKind::Value,
                                    location: node_to_location(file, &name_node),
                                    visibility,
                                    language: "php".to_string(),
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
        }

        "namespace_use_declaration" => {
            // use statements
            for i in 0..node.child_count() {
                if let Some(child) = node.child(i) {
                    if child.kind() == "namespace_use_clause" {
                        if let Some(name) = find_child_by_kind(&child, "qualified_name") {
                            if let Ok(text) = name.utf8_text(source) {
                                result.opens.push(text.to_string());
                            }
                        } else if let Some(name) = find_child_by_kind(&child, "name") {
                            if let Ok(text) = name.utf8_text(source) {
                                result.opens.push(text.to_string());
                            }
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
            extract_recursive(&child, source, file, result, namespace, max_depth - 1);
        }
    }
}

/// Extract parent interfaces for an interface (extends)
fn extract_parent_interfaces(node: &tree_sitter::Node, source: &[u8]) -> Option<Vec<String>> {
    let mut interfaces = Vec::new();

    if let Some(base_clause) = find_child_by_kind(node, "base_clause") {
        for i in 0..base_clause.child_count() {
            if let Some(child) = base_clause.child(i) {
                if child.kind() == "name" || child.kind() == "qualified_name" {
                    if let Ok(text) = child.utf8_text(source) {
                        interfaces.push(text.to_string());
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

/// Extract enum case
fn extract_enum_case(
    node: &tree_sitter::Node,
    source: &[u8],
    file: &Path,
    result: &mut ParseResult,
    enum_path: &str,
) {
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            if child.kind() == "name" {
                if let Ok(name) = child.utf8_text(source) {
                    let qualified = format!("{}\\{}", enum_path, name);

                    result.symbols.push(Symbol {
                        name: name.to_string(),
                        qualified,
                        kind: SymbolKind::Member,
                        location: node_to_location(file, &child),
                        visibility: Visibility::Public,
                        language: "php".to_string(),
                        parent: Some(enum_path.to_string()),
                        mixins: None,
                        attributes: None,
                        implements: None,
                        doc: extract_doc_comments(node, source),
                        signature: None,
                    });
                    break;
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
    fn extracts_php_class() {
        let source = r#"<?php
namespace App\Models;

/**
 * Represents a user in the system.
 */
class User {
    private string $name;
}
"#;
        let parser = PhpParser;
        let result = parser.extract_symbols(std::path::Path::new("User.php"), source, 100);

        let class_sym = result
            .symbols
            .iter()
            .find(|s| s.name == "User")
            .expect("Should find User class");
        assert_eq!(class_sym.kind, SymbolKind::Class);
        assert_eq!(class_sym.qualified, "App\\Models\\User");
        assert!(class_sym.doc.is_some());
        assert!(class_sym.doc.as_ref().unwrap().contains("user"));
    }

    #[test]
    fn extracts_php_interface() {
        let source = r#"<?php
namespace App\Contracts;

interface Repository {
    public function find(int $id): ?object;
    public function save(object $entity): void;
}
"#;
        let parser = PhpParser;
        let result = parser.extract_symbols(std::path::Path::new("Repository.php"), source, 100);

        let iface = result
            .symbols
            .iter()
            .find(|s| s.name == "Repository")
            .expect("Should find Repository interface");
        assert_eq!(iface.kind, SymbolKind::Interface);
        assert_eq!(iface.qualified, "App\\Contracts\\Repository");
    }

    #[test]
    fn extracts_php_trait() {
        let source = r#"<?php
namespace App\Traits;

trait Loggable {
    public function log(string $message): void {
        echo $message;
    }
}
"#;
        let parser = PhpParser;
        let result = parser.extract_symbols(std::path::Path::new("Loggable.php"), source, 100);

        let trait_sym = result
            .symbols
            .iter()
            .find(|s| s.name == "Loggable")
            .expect("Should find Loggable trait");
        assert_eq!(trait_sym.kind, SymbolKind::Module);
        assert_eq!(trait_sym.qualified, "App\\Traits\\Loggable");
    }

    #[test]
    fn extracts_php_function() {
        let source = r#"<?php
namespace App\Helpers;

/**
 * Formats a date for display.
 */
function format_date(DateTime $date): string {
    return $date->format('Y-m-d');
}
"#;
        let parser = PhpParser;
        let result = parser.extract_symbols(std::path::Path::new("helpers.php"), source, 100);

        let func = result
            .symbols
            .iter()
            .find(|s| s.name == "format_date")
            .expect("Should find format_date function");
        assert_eq!(func.kind, SymbolKind::Function);
        assert_eq!(func.qualified, "App\\Helpers\\format_date");
        assert!(func.signature.is_some());
    }

    #[test]
    fn extracts_php_method() {
        let source = r#"<?php
namespace App\Services;

class Calculator {
    /**
     * Adds two numbers.
     */
    public function add(int $a, int $b): int {
        return $a + $b;
    }
}
"#;
        let parser = PhpParser;
        let result = parser.extract_symbols(std::path::Path::new("Calculator.php"), source, 100);

        let method = result
            .symbols
            .iter()
            .find(|s| s.name == "add")
            .expect("Should find add method");
        assert_eq!(method.kind, SymbolKind::Function);
        assert_eq!(method.qualified, "App\\Services\\Calculator\\add");
        assert_eq!(method.visibility, Visibility::Public);
    }

    #[test]
    fn extracts_php_enum() {
        let source = r#"<?php
namespace App\Enums;

enum Status: string {
    case Pending = 'pending';
    case Active = 'active';
    case Completed = 'completed';
}
"#;
        let parser = PhpParser;
        let result = parser.extract_symbols(std::path::Path::new("Status.php"), source, 100);

        let enum_sym = result
            .symbols
            .iter()
            .find(|s| s.name == "Status")
            .expect("Should find Status enum");
        assert_eq!(enum_sym.kind, SymbolKind::Union);

        let pending = result
            .symbols
            .iter()
            .find(|s| s.name == "Pending")
            .expect("Should find Pending case");
        assert_eq!(pending.kind, SymbolKind::Member);
        assert_eq!(pending.qualified, "App\\Enums\\Status\\Pending");
    }

    #[test]
    fn extracts_php_property() {
        let source = r#"<?php
namespace App\Models;

class User {
    private string $name;
    protected int $age;
    public bool $active = true;
}
"#;
        let parser = PhpParser;
        let result = parser.extract_symbols(std::path::Path::new("User.php"), source, 100);

        let name_prop = result
            .symbols
            .iter()
            .find(|s| s.name == "name")
            .expect("Should find name property");
        assert_eq!(name_prop.kind, SymbolKind::Value);
        assert_eq!(name_prop.visibility, Visibility::Private);

        let age_prop = result
            .symbols
            .iter()
            .find(|s| s.name == "age")
            .expect("Should find age property");
        assert_eq!(age_prop.visibility, Visibility::Internal);
    }

    #[test]
    fn extracts_php_use_statements() {
        let source = r#"<?php
namespace App\Controllers;

use App\Models\User;
use App\Services\UserService;
use Illuminate\Http\Request;

class UserController {
}
"#;
        let parser = PhpParser;
        let result =
            parser.extract_symbols(std::path::Path::new("UserController.php"), source, 100);

        assert!(result.opens.contains(&"App\\Models\\User".to_string()));
        assert!(result
            .opens
            .contains(&"App\\Services\\UserService".to_string()));
        assert!(result
            .opens
            .contains(&"Illuminate\\Http\\Request".to_string()));
    }

    #[test]
    fn extracts_php_class_inheritance() {
        let source = r#"<?php
namespace App\Models;

class Admin extends User implements Authenticatable, Authorizable {
    use HasRoles;
}
"#;
        let parser = PhpParser;
        let result = parser.extract_symbols(std::path::Path::new("Admin.php"), source, 100);

        let class_sym = result
            .symbols
            .iter()
            .find(|s| s.name == "Admin")
            .expect("Should find Admin class");
        assert_eq!(class_sym.parent, Some("User".to_string()));
        assert!(class_sym.implements.is_some());
        let interfaces = class_sym.implements.as_ref().unwrap();
        assert!(interfaces.contains(&"Authenticatable".to_string()));
        assert!(interfaces.contains(&"Authorizable".to_string()));
        assert!(class_sym.mixins.is_some());
        let traits = class_sym.mixins.as_ref().unwrap();
        assert!(traits.contains(&"HasRoles".to_string()));
    }

    #[test]
    fn handles_visibility_modifiers() {
        let source = r#"<?php
class Example {
    public function publicMethod() {}
    protected function protectedMethod() {}
    private function privateMethod() {}
}
"#;
        let parser = PhpParser;
        let result = parser.extract_symbols(std::path::Path::new("Example.php"), source, 100);

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
    }

    #[test]
    fn extracts_class_constants() {
        let source = r#"<?php
namespace App\Models;

class User {
    public const STATUS_ACTIVE = 'active';
    private const MAX_LOGIN_ATTEMPTS = 5;
}
"#;
        let parser = PhpParser;
        let result = parser.extract_symbols(std::path::Path::new("User.php"), source, 100);

        let const_active = result
            .symbols
            .iter()
            .find(|s| s.name == "STATUS_ACTIVE")
            .expect("Should find STATUS_ACTIVE constant");
        assert_eq!(const_active.kind, SymbolKind::Value);
        assert_eq!(const_active.visibility, Visibility::Public);

        let const_max = result
            .symbols
            .iter()
            .find(|s| s.name == "MAX_LOGIN_ATTEMPTS")
            .expect("Should find MAX_LOGIN_ATTEMPTS constant");
        assert_eq!(const_max.visibility, Visibility::Private);
    }

    #[test]
    fn extracts_php8_attributes() {
        let source = r#"<?php
namespace App\Controllers;

#[Route('/api')]
#[Middleware('auth')]
class ApiController {
    #[Get('/users')]
    public function index(): array {
        return [];
    }
}
"#;
        let parser = PhpParser;
        let result = parser.extract_symbols(std::path::Path::new("ApiController.php"), source, 100);

        let class_sym = result
            .symbols
            .iter()
            .find(|s| s.name == "ApiController")
            .expect("Should find ApiController");
        assert!(class_sym.attributes.is_some());
        let attrs = class_sym.attributes.as_ref().unwrap();
        assert!(attrs.iter().any(|a| a.contains("Route")));
    }
}
