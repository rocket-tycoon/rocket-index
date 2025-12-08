//! Symbol extraction from TypeScript source files using tree-sitter.

use std::cell::RefCell;
use std::path::Path;

use crate::parse::{find_child_by_kind, node_to_location, LanguageParser, ParseResult};
use crate::{Symbol, SymbolKind, Visibility};

// Thread-local parser reuse - avoids creating a new parser per file
thread_local! {
    static TS_PARSER: RefCell<Option<tree_sitter::Parser>> = const { RefCell::new(None) };
    static TSX_PARSER: RefCell<Option<tree_sitter::Parser>> = const { RefCell::new(None) };
}

pub struct TypeScriptParser;

impl LanguageParser for TypeScriptParser {
    fn extract_symbols(&self, file: &Path, source: &str, max_depth: usize) -> ParseResult {
        let is_tsx = file.extension().is_some_and(|ext| ext == "tsx");

        let parser_cell = if is_tsx { &TSX_PARSER } else { &TS_PARSER };

        parser_cell.with(|parser_opt| {
            let mut parser_ref = parser_opt.borrow_mut();

            // Lazy initialization
            if parser_ref.is_none() {
                let mut parser = tree_sitter::Parser::new();
                let language = if is_tsx {
                    tree_sitter_typescript::LANGUAGE_TSX.into()
                } else {
                    tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()
                };

                match parser.set_language(&language) {
                    Ok(_) => *parser_ref = Some(parser),
                    Err(e) => {
                        tracing::error!(
                            "Failed to load {} grammar: {}",
                            if is_tsx { "TSX" } else { "TypeScript" },
                            e
                        );
                        return ParseResult::default();
                    }
                }
            }

            // At this point we should have a parser unless initialization failed previously
            let parser = match parser_ref.as_mut() {
                Some(p) => p,
                None => return ParseResult::default(),
            };

            let tree = match parser.parse(source, None) {
                Some(tree) => tree,
                None => {
                    tracing::warn!("Failed to parse file: {:?}", file);
                    return ParseResult::default();
                }
            };

            let mut result = ParseResult::default();
            let root = tree.root_node();

            // Check for syntax errors
            if root.has_error() {
                // Simple error traversal
                let mut cursor = root.walk();
                let mut recurse = true;
                while recurse {
                    let node = cursor.node();
                    if node.is_error() || node.is_missing() {
                        result.errors.push(crate::parse::SyntaxError {
                            message: format!("Syntax error at {:?}", node.range()),
                            location: node_to_location(file, &node),
                        });
                    }
                    if cursor.goto_first_child() {
                        continue;
                    }
                    while !cursor.goto_next_sibling() {
                        if !cursor.goto_parent() {
                            recurse = false;
                            break;
                        }
                    }
                }
            }

            extract_recursive(&root, source.as_bytes(), file, &mut result, None, max_depth);

            result
        })
    }
}

/// Extract export visibility from node
fn extract_visibility(node: &tree_sitter::Node, source: &[u8]) -> Visibility {
    // Check if this node or its parent has an export keyword
    // In tree-sitter-typescript, export_statement wraps the declaration
    if let Some(parent) = node.parent() {
        if parent.kind() == "export_statement" {
            return Visibility::Public;
        }
    }

    // Check for export keyword as sibling (for cases like "export class Foo")
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

    // For nodes inside export_statement, check parent's siblings too
    let check_node = if let Some(parent) = node.parent() {
        if parent.kind() == "export_statement" {
            parent
        } else {
            *node
        }
    } else {
        *node
    };

    // Look for preceding comment nodes
    let mut prev = check_node.prev_sibling();
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

/// Extract decorators from preceding siblings
fn extract_decorators(node: &tree_sitter::Node, source: &[u8]) -> Option<Vec<String>> {
    let mut decorators = Vec::new();

    let mut prev = node.prev_sibling();
    while let Some(sib) = prev {
        if sib.kind() == "decorator" {
            if let Ok(text) = sib.utf8_text(source) {
                // Remove @ prefix for consistency
                decorators.insert(0, text.trim_start_matches('@').to_string());
            }
            prev = sib.prev_sibling();
        } else if sib.kind() == "comment" {
            // Skip comments when looking for decorators
            prev = sib.prev_sibling();
        } else {
            break;
        }
    }

    if decorators.is_empty() {
        None
    } else {
        Some(decorators)
    }
}

/// Extract function/method signature
fn extract_function_signature(node: &tree_sitter::Node, source: &[u8]) -> Option<String> {
    // Get text up to the body (statement_block)
    let start = node.start_byte();
    if let Some(body) = find_child_by_kind(node, "statement_block") {
        let end = body.start_byte();
        if end > start {
            if let Ok(sig) = std::str::from_utf8(&source[start..end]) {
                return Some(sig.trim().to_string());
            }
        }
    }
    // For arrow functions or declarations without body
    if let Ok(full) = node.utf8_text(source) {
        // Take first line or up to first {
        let sig = full.lines().next().unwrap_or(full);
        if let Some(brace) = sig.find('{') {
            return Some(sig[..brace].trim().to_string());
        }
        return Some(sig.trim().to_string());
    }
    None
}

/// Build qualified name with . separator (JS/TS convention)
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
        "class_declaration" | "abstract_class_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Ok(name) = name_node.utf8_text(source) {
                    let qualified = qualified_name(name, parent_path);
                    let visibility = extract_visibility(node, source);
                    let doc = extract_doc_comments(node, source);
                    let decorators = extract_decorators(node, source);

                    // Extract implements clause
                    let implements = extract_implements(node, source);

                    // Extract extends (parent class)
                    let parent = extract_extends(node, source);

                    result.symbols.push(Symbol {
                        name: name.to_string(),
                        qualified: qualified.clone(),
                        kind: SymbolKind::Class,
                        location: node_to_location(file, &name_node),
                        visibility,
                        language: "typescript".to_string(),
                        parent,
                        mixins: None,
                        attributes: decorators,
                        implements,
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

        "interface_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Ok(name) = name_node.utf8_text(source) {
                    let qualified = qualified_name(name, parent_path);
                    let visibility = extract_visibility(node, source);
                    let doc = extract_doc_comments(node, source);

                    // Extract extends (for interface inheritance)
                    let extends = extract_interface_extends(node, source);

                    result.symbols.push(Symbol {
                        name: name.to_string(),
                        qualified: qualified.clone(),
                        kind: SymbolKind::Interface,
                        location: node_to_location(file, &name_node),
                        visibility,
                        language: "typescript".to_string(),
                        parent: extends,
                        mixins: None,
                        attributes: None,
                        implements: None,
                        doc,
                        signature: None,
                    });

                    // Recurse into interface body for method signatures
                    if let Some(body) = find_child_by_kind(node, "interface_body") {
                        extract_interface_body(
                            &body,
                            source,
                            file,
                            result,
                            &qualified,
                            max_depth - 1,
                        );
                    }
                    return;
                }
            }
        }

        "type_alias_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Ok(name) = name_node.utf8_text(source) {
                    let qualified = qualified_name(name, parent_path);
                    let visibility = extract_visibility(node, source);
                    let doc = extract_doc_comments(node, source);

                    result.symbols.push(Symbol {
                        name: name.to_string(),
                        qualified,
                        kind: SymbolKind::Type,
                        location: node_to_location(file, &name_node),
                        visibility,
                        language: "typescript".to_string(),
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

        "enum_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Ok(name) = name_node.utf8_text(source) {
                    let qualified = qualified_name(name, parent_path);
                    let visibility = extract_visibility(node, source);
                    let doc = extract_doc_comments(node, source);

                    result.symbols.push(Symbol {
                        name: name.to_string(),
                        qualified: qualified.clone(),
                        kind: SymbolKind::Union, // Using Union for enums
                        location: node_to_location(file, &name_node),
                        visibility,
                        language: "typescript".to_string(),
                        parent: None,
                        mixins: None,
                        attributes: None,
                        implements: None,
                        doc,
                        signature: None,
                    });

                    // Extract enum members
                    if let Some(body) = find_child_by_kind(node, "enum_body") {
                        extract_enum_members(&body, source, file, result, &qualified);
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
                        language: "typescript".to_string(),
                        parent: None,
                        mixins: None,
                        attributes: extract_decorators(node, source),
                        implements: None,
                        doc,
                        signature,
                    });
                }
            }
        }

        "lexical_declaration" | "variable_declaration" => {
            // const foo = ..., let bar = ..., var baz = ...
            extract_variable_declarations(node, source, file, result, parent_path);
        }

        "export_statement" => {
            // Handle export { ... } and export default
            // Recurse into the declaration if present
            for i in 0..node.child_count() {
                if let Some(child) = node.child(i) {
                    let kind = child.kind();
                    // Include _declaration, lexical_declaration, and module types (namespace)
                    if kind.ends_with("_declaration")
                        || kind == "lexical_declaration"
                        || kind == "internal_module"
                        || kind == "module"
                    {
                        extract_recursive(&child, source, file, result, parent_path, max_depth);
                    }
                }
            }
            return; // Don't recurse normally
        }

        "import_statement" => {
            // Track imports for resolution
            extract_import_statement(node, source, result);
        }

        "module" | "internal_module" | "ambient_declaration" => {
            // namespace or module declaration
            // In tree-sitter-typescript, namespaces are parsed as internal_module
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
                        language: "typescript".to_string(),
                        parent: None,
                        mixins: None,
                        attributes: None,
                        implements: None,
                        doc,
                        signature: None,
                    });

                    // Recurse into module body
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

        _ => {}
    }

    // Recurse into children
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            extract_recursive(&child, source, file, result, parent_path, max_depth - 1);
        }
    }
}

/// Extract class body members (methods, properties)
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
                "method_definition" | "method_signature" | "abstract_method_signature" => {
                    if let Some(name_node) = child.child_by_field_name("name") {
                        if let Ok(name) = name_node.utf8_text(source) {
                            let qualified = format!("{}.{}", class_path, name);
                            let visibility = extract_method_visibility(&child, source);
                            let doc = extract_doc_comments(&child, source);
                            let signature = extract_function_signature(&child, source);

                            result.symbols.push(Symbol {
                                name: name.to_string(),
                                qualified,
                                kind: SymbolKind::Function,
                                location: node_to_location(file, &name_node),
                                visibility,
                                language: "typescript".to_string(),
                                parent: Some(class_path.to_string()),
                                mixins: None,
                                attributes: extract_decorators(&child, source),
                                implements: None,
                                doc,
                                signature,
                            });

                            // For constructors, extract parameter properties
                            if name == "constructor" {
                                extract_constructor_parameter_properties(
                                    &child, source, file, result, class_path,
                                );
                            }
                        }
                    }
                }

                "public_field_definition" | "property_signature" => {
                    if let Some(name_node) = child.child_by_field_name("name") {
                        if let Ok(name) = name_node.utf8_text(source) {
                            let qualified = format!("{}.{}", class_path, name);
                            let visibility = extract_method_visibility(&child, source);
                            let doc = extract_doc_comments(&child, source);

                            result.symbols.push(Symbol {
                                name: name.to_string(),
                                qualified,
                                kind: SymbolKind::Member,
                                location: node_to_location(file, &name_node),
                                visibility,
                                language: "typescript".to_string(),
                                parent: Some(class_path.to_string()),
                                mixins: None,
                                attributes: extract_decorators(&child, source),
                                implements: None,
                                doc,
                                signature: None,
                            });
                        }
                    }
                }

                _ => {
                    // Recurse for nested classes, etc.
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

/// Extract interface body members
fn extract_interface_body(
    body: &tree_sitter::Node,
    source: &[u8],
    file: &Path,
    result: &mut ParseResult,
    interface_path: &str,
    _max_depth: usize,
) {
    for i in 0..body.child_count() {
        if let Some(child) = body.child(i) {
            match child.kind() {
                "method_signature" | "call_signature" => {
                    if let Some(name_node) = child.child_by_field_name("name") {
                        if let Ok(name) = name_node.utf8_text(source) {
                            let qualified = format!("{}.{}", interface_path, name);
                            let doc = extract_doc_comments(&child, source);
                            let signature = extract_function_signature(&child, source);

                            result.symbols.push(Symbol {
                                name: name.to_string(),
                                qualified,
                                kind: SymbolKind::Function,
                                location: node_to_location(file, &name_node),
                                visibility: Visibility::Public, // Interface members are always public
                                language: "typescript".to_string(),
                                parent: Some(interface_path.to_string()),
                                mixins: None,
                                attributes: None,
                                implements: None,
                                doc,
                                signature,
                            });
                        }
                    }
                }

                "property_signature" => {
                    if let Some(name_node) = child.child_by_field_name("name") {
                        if let Ok(name) = name_node.utf8_text(source) {
                            let qualified = format!("{}.{}", interface_path, name);
                            let doc = extract_doc_comments(&child, source);

                            result.symbols.push(Symbol {
                                name: name.to_string(),
                                qualified,
                                kind: SymbolKind::Member,
                                location: node_to_location(file, &name_node),
                                visibility: Visibility::Public,
                                language: "typescript".to_string(),
                                parent: Some(interface_path.to_string()),
                                mixins: None,
                                attributes: None,
                                implements: None,
                                doc,
                                signature: None,
                            });
                        }
                    }
                }

                _ => {}
            }
        }
    }
}

/// Extract enum members
fn extract_enum_members(
    body: &tree_sitter::Node,
    source: &[u8],
    file: &Path,
    result: &mut ParseResult,
    enum_path: &str,
) {
    for i in 0..body.child_count() {
        if let Some(child) = body.child(i) {
            if child.kind() == "enum_assignment" || child.kind() == "property_identifier" {
                // Try to get name from the child
                let name_node = if child.kind() == "enum_assignment" {
                    child.child_by_field_name("name")
                } else {
                    Some(child)
                };

                if let Some(name_node) = name_node {
                    if let Ok(name) = name_node.utf8_text(source) {
                        let qualified = format!("{}.{}", enum_path, name);

                        result.symbols.push(Symbol {
                            name: name.to_string(),
                            qualified,
                            kind: SymbolKind::Member,
                            location: node_to_location(file, &name_node),
                            visibility: Visibility::Public,
                            language: "typescript".to_string(),
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

/// Extract constructor parameter properties (TypeScript shorthand for declaring class members)
///
/// In TypeScript, `constructor(private readonly foo: Bar)` is shorthand for declaring
/// a private readonly class member `foo` of type `Bar`.
fn extract_constructor_parameter_properties(
    method_node: &tree_sitter::Node,
    source: &[u8],
    file: &Path,
    result: &mut ParseResult,
    class_path: &str,
) {
    // Find formal_parameters child
    if let Some(params) = find_child_by_kind(method_node, "formal_parameters") {
        for i in 0..params.child_count() {
            if let Some(param) = params.child(i) {
                // Check for required_parameter with accessibility_modifier
                if param.kind() == "required_parameter" {
                    // If it has an accessibility modifier, it's a parameter property
                    if has_accessibility_modifier(&param) {
                        // Get the parameter name (identifier)
                        if let Some(name_node) = find_child_by_kind(&param, "identifier") {
                            if let Ok(name) = name_node.utf8_text(source) {
                                let qualified = format!("{}.{}", class_path, name);
                                let visibility = extract_parameter_visibility(&param, source);

                                result.symbols.push(Symbol {
                                    name: name.to_string(),
                                    qualified,
                                    kind: SymbolKind::Member,
                                    location: node_to_location(file, &name_node),
                                    visibility,
                                    language: "typescript".to_string(),
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
        }
    }
}

/// Check if a parameter node has an accessibility modifier (public/private/protected)
fn has_accessibility_modifier(param: &tree_sitter::Node) -> bool {
    for i in 0..param.child_count() {
        if let Some(child) = param.child(i) {
            if child.kind() == "accessibility_modifier" {
                return true;
            }
        }
    }
    false
}

/// Extract visibility from a constructor parameter
fn extract_parameter_visibility(param: &tree_sitter::Node, source: &[u8]) -> Visibility {
    for i in 0..param.child_count() {
        if let Some(child) = param.child(i) {
            if child.kind() == "accessibility_modifier" {
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
    Visibility::Public
}

/// Extract variable declarations (const, let, var)
fn extract_variable_declarations(
    node: &tree_sitter::Node,
    source: &[u8],
    file: &Path,
    result: &mut ParseResult,
    parent_path: Option<&str>,
) {
    let visibility = extract_visibility(node, source);
    let doc = extract_doc_comments(node, source);

    // Find variable_declarator children
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            if child.kind() == "variable_declarator" {
                if let Some(name_node) = child.child_by_field_name("name") {
                    // Could be identifier or destructuring pattern
                    if name_node.kind() == "identifier" {
                        if let Ok(name) = name_node.utf8_text(source) {
                            let qualified = qualified_name(name, parent_path);

                            // Check if it's a function (arrow function or function expression)
                            let (kind, signature) = if let Some(value) =
                                child.child_by_field_name("value")
                            {
                                if value.kind() == "arrow_function" || value.kind() == "function" {
                                    (
                                        SymbolKind::Function,
                                        extract_function_signature(&value, source),
                                    )
                                } else {
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
                                language: "typescript".to_string(),
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

/// Extract method visibility (public/private/protected)
fn extract_method_visibility(node: &tree_sitter::Node, source: &[u8]) -> Visibility {
    // Look for accessibility_modifier child
    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            let child = cursor.node();
            if child.kind() == "accessibility_modifier" {
                if let Ok(text) = child.utf8_text(source) {
                    return match text {
                        "public" => Visibility::Public,
                        "protected" => Visibility::Internal,
                        "private" => Visibility::Private,
                        _ => Visibility::Public, // default to public in TS
                    };
                }
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
    Visibility::Public // default for class members in TS
}

/// Extract implements clause from class
fn extract_implements(node: &tree_sitter::Node, source: &[u8]) -> Option<Vec<String>> {
    let mut implements = Vec::new();

    // Look through all children for class_heritage which contains implements
    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            let child = cursor.node();
            if child.kind() == "class_heritage" {
                // Look for implements_clause inside class_heritage
                let mut inner = child.walk();
                if inner.goto_first_child() {
                    loop {
                        let inner_child = inner.node();
                        if inner_child.kind() == "implements_clause" {
                            // Extract type identifiers from implements clause
                            let mut impl_cursor = inner_child.walk();
                            if impl_cursor.goto_first_child() {
                                loop {
                                    let impl_child = impl_cursor.node();
                                    if impl_child.kind() == "type_identifier"
                                        || impl_child.kind() == "generic_type"
                                    {
                                        if let Ok(text) = impl_child.utf8_text(source) {
                                            implements.push(text.to_string());
                                        }
                                    }
                                    if !impl_cursor.goto_next_sibling() {
                                        break;
                                    }
                                }
                            }
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
    if implements.is_empty() {
        None
    } else {
        Some(implements)
    }
}

/// Extract extends clause from class (parent class)
fn extract_extends(node: &tree_sitter::Node, source: &[u8]) -> Option<String> {
    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            let child = cursor.node();
            if child.kind() == "class_heritage" {
                // Look for extends_clause
                let mut inner = child.walk();
                if inner.goto_first_child() {
                    loop {
                        let inner_child = inner.node();
                        if inner_child.kind() == "extends_clause" {
                            // Get the type name - could be type_identifier, identifier, or generic_type
                            if let Some(type_node) =
                                find_child_by_kind(&inner_child, "type_identifier")
                            {
                                return type_node.utf8_text(source).ok().map(|s| s.to_string());
                            }
                            if let Some(type_node) = find_child_by_kind(&inner_child, "identifier")
                            {
                                return type_node.utf8_text(source).ok().map(|s| s.to_string());
                            }
                            if let Some(type_node) =
                                find_child_by_kind(&inner_child, "generic_type")
                            {
                                return type_node.utf8_text(source).ok().map(|s| s.to_string());
                            }
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

/// Extract extends from interface (interface inheritance)
fn extract_interface_extends(node: &tree_sitter::Node, source: &[u8]) -> Option<String> {
    // For interfaces, we just track the first extends as parent
    // (interfaces can extend multiple, but we track one for simplicity)
    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            let child = cursor.node();
            if child.kind() == "extends_type_clause" {
                let mut inner = child.walk();
                if inner.goto_first_child() {
                    loop {
                        let inner_child = inner.node();
                        if inner_child.kind() == "type_identifier" {
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

/// Extract import statements for opens
fn extract_import_statement(node: &tree_sitter::Node, source: &[u8], result: &mut ParseResult) {
    // import { foo, bar } from 'module'
    // import * as ns from 'module'
    // import defaultExport from 'module'

    // Get the module path
    if let Some(source_node) = node.child_by_field_name("source") {
        if let Ok(module_path) = source_node.utf8_text(source) {
            // Remove quotes
            let module_path = module_path.trim_matches(|c| c == '"' || c == '\'');
            result.opens.push(module_path.to_string());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::extract_symbols;

    #[test]
    fn extracts_typescript_class() {
        let source = r#"
/** A simple user class */
export class User {
    public name: string;
    private age: number;

    constructor(name: string, age: number) {
        this.name = name;
        this.age = age;
    }

    public greet(): string {
        return `Hello, ${this.name}`;
    }
}
"#;
        let result = extract_symbols(std::path::Path::new("test.ts"), source, 100);

        let class = result
            .symbols
            .iter()
            .find(|s| s.name == "User")
            .expect("Should find User");
        assert_eq!(class.kind, SymbolKind::Class);
        assert_eq!(class.visibility, Visibility::Public);
        assert!(class.doc.is_some());

        let greet = result
            .symbols
            .iter()
            .find(|s| s.name == "greet")
            .expect("Should find greet");
        assert_eq!(greet.kind, SymbolKind::Function);
        assert_eq!(greet.qualified, "User.greet");
    }

    #[test]
    fn extracts_typescript_interface() {
        let source = r#"
export interface Printable {
    print(): void;
    format(options: FormatOptions): string;
}
"#;
        let result = extract_symbols(std::path::Path::new("test.ts"), source, 100);

        let iface = result
            .symbols
            .iter()
            .find(|s| s.name == "Printable")
            .expect("Should find Printable");
        assert_eq!(iface.kind, SymbolKind::Interface);

        let print = result
            .symbols
            .iter()
            .find(|s| s.name == "print")
            .expect("Should find print");
        assert_eq!(print.qualified, "Printable.print");
    }

    #[test]
    fn extracts_typescript_function() {
        let source = r#"
/** Adds two numbers */
export function add(a: number, b: number): number {
    return a + b;
}
"#;
        let result = extract_symbols(std::path::Path::new("test.ts"), source, 100);

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
    fn extracts_typescript_enum() {
        let source = r#"
export enum Color {
    Red,
    Green,
    Blue
}
"#;
        let result = extract_symbols(std::path::Path::new("test.ts"), source, 100);

        let enum_sym = result
            .symbols
            .iter()
            .find(|s| s.name == "Color")
            .expect("Should find Color");
        assert_eq!(enum_sym.kind, SymbolKind::Union);

        // Check for enum members
        assert!(result
            .symbols
            .iter()
            .any(|s| s.name == "Red" && s.qualified == "Color.Red"));
    }

    #[test]
    fn extracts_typescript_type_alias() {
        let source = r#"
export type UserId = string;
export type Result<T> = { ok: true; value: T } | { ok: false; error: Error };
"#;
        let result = extract_symbols(std::path::Path::new("test.ts"), source, 100);

        let user_id = result
            .symbols
            .iter()
            .find(|s| s.name == "UserId")
            .expect("Should find UserId");
        assert_eq!(user_id.kind, SymbolKind::Type);
    }

    #[test]
    fn extracts_arrow_function() {
        let source = r#"
export const multiply = (a: number, b: number): number => a * b;
"#;
        let result = extract_symbols(std::path::Path::new("test.ts"), source, 100);

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
import * as lodash from 'lodash';
"#;
        let result = extract_symbols(std::path::Path::new("test.ts"), source, 100);

        assert!(result.opens.contains(&"./utils".to_string()));
        assert!(result.opens.contains(&"lodash".to_string()));
    }

    #[test]
    fn extracts_class_with_implements() {
        let source = r#"
interface Serializable {
    serialize(): string;
}

class User implements Serializable {
    serialize(): string {
        return JSON.stringify(this);
    }
}
"#;
        let result = extract_symbols(std::path::Path::new("test.ts"), source, 100);

        let user = result
            .symbols
            .iter()
            .find(|s| s.name == "User")
            .expect("Should find User");
        assert!(user.implements.is_some());
        assert!(user
            .implements
            .as_ref()
            .unwrap()
            .contains(&"Serializable".to_string()));
    }

    #[test]
    fn extracts_class_with_extends() {
        let source = r#"
class Animal {
    speak(): void {}
}

class Dog extends Animal {
    bark(): void {}
}
"#;
        let result = extract_symbols(std::path::Path::new("test.ts"), source, 100);

        let dog = result
            .symbols
            .iter()
            .find(|s| s.name == "Dog")
            .expect("Should find Dog");
        assert_eq!(dog.kind, SymbolKind::Class);
        assert_eq!(dog.parent, Some("Animal".to_string()));

        let bark = result
            .symbols
            .iter()
            .find(|s| s.name == "bark")
            .expect("Should find bark");
        assert_eq!(bark.qualified, "Dog.bark");
    }

    #[test]
    fn extracts_namespace() {
        let source = r#"
export namespace Utils {
    export function helper(): void {}
    export const VERSION = "1.0";
}
"#;
        let result = extract_symbols(std::path::Path::new("test.ts"), source, 100);

        let ns = result
            .symbols
            .iter()
            .find(|s| s.name == "Utils")
            .expect("Should find Utils namespace");
        assert_eq!(ns.kind, SymbolKind::Module);

        let helper = result
            .symbols
            .iter()
            .find(|s| s.name == "helper")
            .expect("Should find helper");
        assert_eq!(helper.qualified, "Utils.helper");
    }

    #[test]
    fn extracts_visibility_modifiers() {
        let source = r#"
class Service {
    public publicMethod(): void {}
    protected protectedMethod(): void {}
    private privateMethod(): void {}
}
"#;
        let result = extract_symbols(std::path::Path::new("test.ts"), source, 100);

        let public_m = result
            .symbols
            .iter()
            .find(|s| s.name == "publicMethod")
            .expect("Should find publicMethod");
        assert_eq!(public_m.visibility, Visibility::Public);

        let protected_m = result
            .symbols
            .iter()
            .find(|s| s.name == "protectedMethod")
            .expect("Should find protectedMethod");
        assert_eq!(protected_m.visibility, Visibility::Internal);

        let private_m = result
            .symbols
            .iter()
            .find(|s| s.name == "privateMethod")
            .expect("Should find privateMethod");
        assert_eq!(private_m.visibility, Visibility::Private);
    }

    #[test]
    fn extracts_abstract_class() {
        let source = r#"
export abstract class Shape {
    abstract area(): number;

    describe(): string {
        return "A shape";
    }
}
"#;
        let result = extract_symbols(std::path::Path::new("test.ts"), source, 100);

        let shape = result
            .symbols
            .iter()
            .find(|s| s.name == "Shape")
            .expect("Should find Shape");
        assert_eq!(shape.kind, SymbolKind::Class);
        assert_eq!(shape.visibility, Visibility::Public);

        // Both abstract and concrete methods should be extracted
        assert!(result.symbols.iter().any(|s| s.name == "area"));
        assert!(result.symbols.iter().any(|s| s.name == "describe"));
    }

    #[test]
    fn extracts_static_members() {
        let source = r#"
class Counter {
    static count: number = 0;
    static increment(): void {
        Counter.count++;
    }
}
"#;
        let result = extract_symbols(std::path::Path::new("test.ts"), source, 100);

        // Static members should be extracted
        assert!(result
            .symbols
            .iter()
            .any(|s| s.name == "count" && s.qualified == "Counter.count"));
        assert!(result
            .symbols
            .iter()
            .any(|s| s.name == "increment" && s.qualified == "Counter.increment"));
    }

    #[test]
    fn extracts_constructor_parameter_properties() {
        let source = r#"
class MyService {
    constructor(private readonly dependency: OtherService) {}
}
"#;
        let result = extract_symbols(std::path::Path::new("test.ts"), source, 100);

        // Constructor parameter property should be indexed as a class member
        let dep = result
            .symbols
            .iter()
            .find(|s| s.name == "dependency")
            .expect("Should find dependency as a class member");
        assert_eq!(dep.kind, SymbolKind::Member);
        assert_eq!(dep.qualified, "MyService.dependency");
        assert_eq!(dep.visibility, Visibility::Private);
        assert_eq!(dep.parent, Some("MyService".to_string()));
    }

    #[test]
    fn extracts_multiple_constructor_parameter_properties() {
        let source = r#"
class Controller {
    constructor(
        private readonly service: Service,
        public readonly name: string,
        protected config: Config
    ) {}
}
"#;
        let result = extract_symbols(std::path::Path::new("test.ts"), source, 100);

        // All three should be indexed
        let service = result
            .symbols
            .iter()
            .find(|s| s.name == "service")
            .expect("Should find service");
        assert_eq!(service.visibility, Visibility::Private);
        assert_eq!(service.qualified, "Controller.service");

        let name = result
            .symbols
            .iter()
            .find(|s| s.name == "name")
            .expect("Should find name");
        assert_eq!(name.visibility, Visibility::Public);
        assert_eq!(name.qualified, "Controller.name");

        let config = result
            .symbols
            .iter()
            .find(|s| s.name == "config")
            .expect("Should find config");
        assert_eq!(config.visibility, Visibility::Internal); // protected maps to Internal
        assert_eq!(config.qualified, "Controller.config");
    }
    #[test]
    fn extracts_tsx_component() {
        let source = r#"
export const App = () => {
    return (
        <div className="app">
            <h1>Hello World</h1>
        </div>
    );
};
"#;
        // Note: passing .tsx extension
        let result = extract_symbols(std::path::Path::new("App.tsx"), source, 100);

        let app = result
            .symbols
            .iter()
            .find(|s| s.name == "App")
            .expect("Should find App component");
        assert_eq!(app.kind, SymbolKind::Function);

        // Should produce no syntax errors if parsed correctly as TSX
        assert!(
            result.errors.is_empty(),
            "Expected no syntax errors, found: {:?}",
            result.errors
        );
    }
}
