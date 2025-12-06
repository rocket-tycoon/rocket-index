//! Symbol extraction from F# source files using tree-sitter.

use std::path::Path;

use crate::parse::{LanguageParser, ParseResult, ParseWarning, SyntaxError};
use crate::{Location, Reference, Symbol, SymbolKind, Visibility};

pub struct FSharpParser;

impl LanguageParser for FSharpParser {
    fn extract_symbols(&self, file: &Path, source: &str, max_depth: usize) -> ParseResult {
        let mut parser = tree_sitter::Parser::new();

        // Set the F# language
        parser
            .set_language(&tree_sitter_fsharp::LANGUAGE_FSHARP.into())
            .expect("tree-sitter-fsharp grammar incompatible with tree-sitter version");

        let tree = match parser.parse(source, None) {
            Some(tree) => tree,
            None => {
                tracing::warn!("Failed to parse file: {:?}", file);
                return ParseResult::default();
            }
        };

        let mut result = ParseResult::default();
        let root = tree.root_node();

        // Extract syntax errors from the tree
        extract_syntax_errors(&root, source.as_bytes(), file, &mut result.errors);

        extract_recursive(
            &root,
            source.as_bytes(),
            file,
            &mut result,
            None, // No parent module yet
            max_depth,
        );

        result
    }
}

/// Extract syntax errors from the tree-sitter parse tree.
fn extract_syntax_errors(
    node: &tree_sitter::Node,
    source: &[u8],
    file: &Path,
    errors: &mut Vec<SyntaxError>,
) {
    extract_syntax_errors_with_depth(node, source, file, errors, 0);
}

fn extract_syntax_errors_with_depth(
    node: &tree_sitter::Node,
    source: &[u8],
    file: &Path,
    errors: &mut Vec<SyntaxError>,
    depth: usize,
) {
    // Prevent stack overflow on deeply nested error trees
    if depth > MAX_HELPER_DEPTH {
        return;
    }

    // Check if this node is an error
    if node.is_error() {
        let message = generate_error_message(node, source);
        errors.push(SyntaxError {
            message,
            location: node_to_location(file, node),
        });
        // Don't recurse into error nodes - the whole subtree is problematic
        return;
    }

    // Check if this node is missing (parser expected something that wasn't there)
    if node.is_missing() {
        let expected = node.kind();
        errors.push(SyntaxError {
            message: format!("Expected {}", expected),
            location: node_to_location(file, node),
        });
        return;
    }

    // Recurse into children
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            extract_syntax_errors_with_depth(&child, source, file, errors, depth + 1);
        }
    }
}

/// Generate a human-readable error message for an ERROR node.
fn generate_error_message(node: &tree_sitter::Node, source: &[u8]) -> String {
    // Try to get the text of the error node for context
    let error_text = node
        .utf8_text(source)
        .ok()
        .map(|s| s.chars().take(30).collect::<String>())
        .unwrap_or_default();

    // Look at parent context to provide better messages
    if let Some(parent) = node.parent() {
        match parent.kind() {
            "function_or_value_defn" => {
                return format!("Syntax error in let binding: '{}'", error_text.trim());
            }
            "type_definition" => {
                return format!("Syntax error in type definition: '{}'", error_text.trim());
            }
            "module_defn" => {
                return format!("Syntax error in module definition: '{}'", error_text.trim());
            }
            _ => {}
        }
    }

    if error_text.trim().is_empty() {
        "Syntax error".to_string()
    } else {
        format!("Syntax error near '{}'", error_text.trim())
    }
}

/// Maximum recursion depth for helper functions (more conservative).
const MAX_HELPER_DEPTH: usize = 200;

/// Recursively extract symbols from a tree-sitter node.
fn extract_recursive(
    node: &tree_sitter::Node,
    source: &[u8],
    file: &Path,
    result: &mut ParseResult,
    current_module: Option<&str>,
    max_depth: usize,
) {
    extract_recursive_with_depth(node, source, file, result, current_module, 0, max_depth);
}

/// Inner recursive function with depth tracking.
fn extract_recursive_with_depth(
    node: &tree_sitter::Node,
    source: &[u8],
    file: &Path,
    result: &mut ParseResult,
    current_module: Option<&str>,
    depth: usize,
    max_depth: usize,
) {
    if depth > max_depth {
        // Only add one warning per file (check if we already have a depth warning)
        let has_depth_warning = result
            .warnings
            .iter()
            .any(|w| w.message.contains("recursion depth"));
        if !has_depth_warning {
            result.warnings.push(ParseWarning {
                message: format!(
                    "Maximum recursion depth ({}) reached, some deeply nested code may not be indexed",
                    max_depth
                ),
                location: Some(node_to_location(file, node)),
            });
            tracing::warn!(
                "Max recursion depth ({}) reached in {:?}, skipping deeper nodes",
                max_depth,
                file
            );
        }
        return;
    }

    // Skip error nodes to avoid extracting garbage from malformed code
    // BUT: if the root is ERROR (depth == 0), still process children since they may be valid
    if node.is_error() && depth > 0 {
        tracing::debug!(
            "Skipping ERROR node at {:?}:{}:{} - malformed syntax",
            file,
            node.start_position().row + 1,
            node.start_position().column + 1
        );
        return;
    }

    // Also skip nodes marked as missing (parser recovery artifacts)
    if node.is_missing() {
        return;
    }

    match node.kind() {
        "namespace" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Ok(name) = name_node.utf8_text(source) {
                    let qualified = qualified_name(name.trim(), current_module);
                    result.module_path = Some(qualified.clone());
                    // Process children with this namespace context
                    for i in 0..node.child_count() {
                        if let Some(child) = node.child(i) {
                            if child.id() != name_node.id() {
                                extract_recursive_with_depth(
                                    &child,
                                    source,
                                    file,
                                    result,
                                    Some(&qualified),
                                    depth + 1,
                                    max_depth,
                                );
                            }
                        }
                    }
                    return;
                }
            }
        }

        "named_module" | "module_defn" => {
            // Module name can be:
            // - "name" field (for named_module)
            // - long_identifier child (for some module_defn cases)
            // - direct identifier child (for module_defn inside namespace)
            let module_name_node = node
                .child_by_field_name("name")
                .or_else(|| find_child_by_kind(node, "long_identifier"))
                .or_else(|| find_child_by_kind(node, "identifier"));
            if let Some(name_node) = module_name_node {
                if let Ok(name) = name_node.utf8_text(source) {
                    let trimmed = name.trim();
                    let short_name = trimmed
                        .split('.')
                        .next_back()
                        .unwrap_or(trimmed)
                        .to_string();
                    let qualified = qualified_name(trimmed, current_module);
                    let symbol = Symbol {
                        name: short_name,
                        qualified: qualified.clone(),
                        kind: SymbolKind::Module,
                        location: node_to_location(file, &name_node),
                        visibility: Visibility::Public,
                        language: "fsharp".to_string(),
                        parent: None,
                        mixins: None,
                        attributes: None,
                        implements: None,
                        doc: None,
                        signature: None,
                    };
                    if result.module_path.is_none() {
                        result.module_path = Some(qualified.clone());
                    }
                    result.symbols.push(symbol);
                    // Process children with this module context
                    for i in 0..node.child_count() {
                        if let Some(child) = node.child(i) {
                            if child.id() != name_node.id() {
                                extract_recursive_with_depth(
                                    &child,
                                    source,
                                    file,
                                    result,
                                    Some(&qualified),
                                    depth + 1,
                                    max_depth,
                                );
                            }
                        }
                    }
                    return;
                }
            }
        }

        "open_statement" | "import_decl" => {
            let name_node = node
                .child_by_field_name("name")
                .or_else(|| find_child_by_kind(node, "long_identifier"));
            if let Some(name_node) = name_node {
                if let Ok(name) = name_node.utf8_text(source) {
                    result.opens.push(name.trim().to_string());
                }
            }
        }

        "function_or_value_defn" => {
            handle_function_or_value_defn(node, source, file, result, current_module);
        }

        "type_definition" => {
            // Doc comments are siblings of type_definition
            let doc = extract_doc_comment(node, source);
            for i in 0..node.child_count() {
                if let Some(child) = node.child(i) {
                    extract_type_defn(&child, source, file, result, current_module, doc.as_deref());
                }
            }
        }

        "long_identifier" | "long_identifier_or_op" => {
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

    // Recurse into children - no cloning needed since current_module is &str
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            extract_recursive_with_depth(
                &child,
                source,
                file,
                result,
                current_module,
                depth + 1,
                max_depth,
            );
        }
    }
}

/// Extract documentation comment from preceding sibling nodes.
/// F# uses `/// comment` style which becomes `line_comment` nodes.
fn extract_doc_comment(node: &tree_sitter::Node, source: &[u8]) -> Option<String> {
    // Get the parent node to find siblings
    let parent = node.parent()?;

    // Find our position in the parent's children
    let mut our_index = None;
    for i in 0..parent.child_count() {
        if let Some(child) = parent.child(i) {
            if child.id() == node.id() {
                our_index = Some(i);
                break;
            }
        }
    }

    let our_index = our_index?;
    if our_index == 0 {
        return None;
    }

    // Collect preceding line_comment nodes that are doc comments (start with ///)
    let mut doc_lines = Vec::new();

    // Walk backwards from our position, collecting consecutive doc comments
    let mut i = our_index;
    while i > 0 {
        i -= 1;
        if let Some(sibling) = parent.child(i) {
            match sibling.kind() {
                "line_comment" => {
                    if let Ok(text) = sibling.utf8_text(source) {
                        let text = text.trim();
                        if text.starts_with("///") {
                            // Strip the /// prefix and any leading space
                            let doc_text = text.trim_start_matches('/').trim();
                            doc_lines.push(doc_text.to_string());
                        } else {
                            // Not a doc comment, stop looking
                            break;
                        }
                    }
                }
                "attributes" | "attribute" => {
                    // Skip over attribute blocks (single or grouped) between docs and declarations
                    continue;
                }
                _ => {
                    // Not a comment, stop looking
                    break;
                }
            }
        }
    }

    if doc_lines.is_empty() {
        return None;
    }

    // Reverse to get original order (we collected backwards)
    doc_lines.reverse();
    Some(doc_lines.join("\n"))
}

/// Extract implemented interfaces from a type definition node.
/// Looks for interface_implementation children in type_extension_elements.
fn extract_interfaces(node: &tree_sitter::Node, source: &[u8]) -> Vec<String> {
    let mut interfaces = Vec::new();

    fn find_interfaces_recursive(
        node: &tree_sitter::Node,
        source: &[u8],
        interfaces: &mut Vec<String>,
        depth: usize,
    ) {
        if depth > 20 {
            return;
        }

        if node.kind() == "interface_implementation" {
            // interface_implementation -> simple_type -> long_identifier -> identifier
            if let Some(simple_type) = find_child_by_kind(node, "simple_type") {
                if let Some(long_id) = find_child_by_kind(&simple_type, "long_identifier") {
                    if let Ok(name) = long_id.utf8_text(source) {
                        interfaces.push(name.trim().to_string());
                    }
                }
            }
        }

        // Look in type_extension_elements and other children
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                find_interfaces_recursive(&child, source, interfaces, depth + 1);
            }
        }
    }

    find_interfaces_recursive(node, source, &mut interfaces, 0);
    interfaces
}

fn extract_type_defn(
    node: &tree_sitter::Node,
    source: &[u8],
    file: &Path,
    result: &mut ParseResult,
    current_module: Option<&str>,
    doc: Option<&str>,
) {
    let kind = match node.kind() {
        "record_type_defn" => SymbolKind::Record,
        "union_type_defn" => SymbolKind::Union,
        "class_type_defn" | "anon_type_defn" => SymbolKind::Class, // anon_type_defn = class with primary constructor
        "interface_type_defn" => SymbolKind::Interface,
        "type_abbrev_defn" | "type_extension" => SymbolKind::Type,
        _ => return,
    };

    // Find the type name - structure is:
    // record_type_defn > type_name > (identifier or type_name: identifier)
    //
    // We try to find the innermost identifier within type_name children
    if let Some(type_name_node) = find_child_by_kind(node, "type_name") {
        // Look for identifier inside type_name, or use type_name's inner type_name field
        let name_node = type_name_node
            .child_by_field_name("type_name") // Some grammars nest: type_name > type_name: identifier
            .or_else(|| find_child_by_kind(&type_name_node, "identifier"))
            .or_else(|| find_child_by_kind(&type_name_node, "long_identifier"))
            .unwrap_or(type_name_node);

        if let Ok(name) = name_node.utf8_text(source) {
            let trimmed = name.trim();
            let attrs = extract_attributes(node, source);
            let interfaces = extract_interfaces(node, source);
            let symbol = Symbol {
                name: trimmed.to_string(),
                qualified: qualified_name(trimmed, current_module),
                kind,
                location: node_to_location(file, &name_node),
                visibility: Visibility::Public,
                language: "fsharp".to_string(),
                parent: None,
                mixins: None,
                attributes: if attrs.is_empty() { None } else { Some(attrs) },
                implements: if interfaces.is_empty() {
                    None
                } else {
                    Some(interfaces)
                },
                doc: doc.map(|d| d.to_string()),
                signature: None,
            };
            result.symbols.push(symbol);

            if matches!(kind, SymbolKind::Class | SymbolKind::Interface) {
                extract_members(node, source, file, result, current_module, 0);
            }
            return;
        }
    }

    // Fallback: try to find identifier directly on node
    if let Some(name_node) = find_child_by_kind(node, "identifier")
        .or_else(|| find_child_by_kind(node, "long_identifier"))
    {
        if let Ok(name) = name_node.utf8_text(source) {
            let trimmed = name.trim();
            let attrs = extract_attributes(node, source);
            let interfaces = extract_interfaces(node, source);
            let symbol = Symbol {
                name: trimmed.to_string(),
                qualified: qualified_name(trimmed, current_module),
                kind,
                location: node_to_location(file, &name_node),
                visibility: Visibility::Public,
                language: "fsharp".to_string(),
                parent: None,
                mixins: None,
                attributes: if attrs.is_empty() { None } else { Some(attrs) },
                implements: if interfaces.is_empty() {
                    None
                } else {
                    Some(interfaces)
                },
                doc: doc.map(|d| d.to_string()),
                signature: None,
            };
            result.symbols.push(symbol);
        }
    }

    if matches!(kind, SymbolKind::Class | SymbolKind::Interface) {
        extract_members(node, source, file, result, current_module, 0);
    }
}

fn extract_members(
    node: &tree_sitter::Node,
    source: &[u8],
    file: &Path,
    result: &mut ParseResult,
    current_module: Option<&str>,
    depth: usize,
) {
    // Prevent stack overflow on deeply nested class hierarchies
    if depth > MAX_HELPER_DEPTH {
        return;
    }

    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            if child.kind() == "member_defn" {
                if let Some(name_node) = find_child_by_kind(&child, "identifier") {
                    if let Ok(name) = name_node.utf8_text(source) {
                        let trimmed = name.trim();
                        let symbol = Symbol {
                            name: trimmed.to_string(),
                            qualified: qualified_name(trimmed, current_module),
                            kind: SymbolKind::Member,
                            location: node_to_location(file, &name_node),
                            visibility: extract_visibility(&child, source),
                            language: "fsharp".to_string(),
                            parent: None,
                            mixins: None,
                            attributes: None,
                            implements: None,
                            doc: None,
                            signature: None,
                        };
                        result.symbols.push(symbol);
                    }
                }
            }
            extract_members(&child, source, file, result, current_module, depth + 1);
        }
    }
}

/// Extract type signature from a function or value definition.
/// Returns signatures like "int -> int -> int" for functions or "int" for values.
fn extract_signature(node: &tree_sitter::Node, source: &[u8]) -> Option<String> {
    let mut param_types = Vec::new();
    let mut return_type = None;

    // Look for function_declaration_left with argument_patterns
    if let Some(func_decl) = find_child_by_kind(node, "function_declaration_left") {
        if let Some(args) = find_child_by_kind(&func_decl, "argument_patterns") {
            for i in 0..args.child_count() {
                if let Some(child) = args.child(i) {
                    if child.kind() == "typed_pattern" {
                        // Find simple_type in typed_pattern
                        if let Some(type_node) = find_child_by_kind(&child, "simple_type") {
                            if let Ok(type_text) = type_node.utf8_text(source) {
                                param_types.push(type_text.trim().to_string());
                            }
                        }
                    }
                }
            }
        }
    }

    // Look for return type (simple_type directly under function_or_value_defn)
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            if child.kind() == "simple_type" {
                if let Ok(type_text) = child.utf8_text(source) {
                    return_type = Some(type_text.trim().to_string());
                    break;
                }
            }
        }
    }

    // Build the signature
    if param_types.is_empty() {
        // Value or function without typed parameters
        return_type
    } else {
        // Function with typed parameters
        let sig = if let Some(ret) = return_type {
            format!("{} -> {}", param_types.join(" -> "), ret)
        } else {
            param_types.join(" -> ")
        };
        Some(sig)
    }
}

fn handle_function_or_value_defn(
    node: &tree_sitter::Node,
    source: &[u8],
    file: &Path,
    result: &mut ParseResult,
    current_module: Option<&str>,
) {
    let mut handled = false;

    // Doc comments are siblings of the parent value_declaration, not function_or_value_defn
    let doc = node.parent().and_then(|p| extract_doc_comment(&p, source));

    // Extract type signature if present
    let signature = extract_signature(node, source);

    if let Some(decl) = find_child_by_kind(node, "function_declaration_left") {
        if let Some(name_node) = find_child_by_kind(&decl, "identifier") {
            if let Ok(name) = name_node.utf8_text(source) {
                let trimmed = name.trim();
                let qualified = qualified_name(trimmed, current_module);
                let has_args = find_child_by_kind(&decl, "argument_patterns").is_some();
                let kind = if has_args {
                    SymbolKind::Function
                } else {
                    SymbolKind::Value
                };

                let attrs = extract_attributes(node, source);
                let symbol = Symbol {
                    name: trimmed.to_string(),
                    qualified,
                    kind,
                    location: node_to_location(file, &name_node),
                    visibility: extract_visibility(node, source),
                    language: "fsharp".to_string(),
                    parent: None,
                    mixins: None,
                    attributes: if attrs.is_empty() { None } else { Some(attrs) },
                    implements: None,
                    doc: doc.clone(),
                    signature: signature.clone(),
                };
                result.symbols.push(symbol);
                handled = true;
            }
        }
    }

    if !handled {
        if let Some(decl) = find_child_by_kind(node, "value_declaration_left") {
            // Value declaration identifier can be:
            // - Direct identifier child
            // - identifier_pattern -> long_identifier_or_op -> identifier (in namespace modules)
            // Use find_first_identifier to handle nested patterns
            let name_node = find_first_identifier(&decl);

            if let Some(name_node) = name_node {
                if let Ok(name) = name_node.utf8_text(source) {
                    let trimmed = name.trim();
                    let attrs = extract_attributes(node, source);
                    let symbol = Symbol {
                        name: trimmed.to_string(),
                        qualified: qualified_name(trimmed, current_module),
                        kind: SymbolKind::Value,
                        location: node_to_location(file, &name_node),
                        visibility: extract_visibility(node, source),
                        language: "fsharp".to_string(),
                        parent: None,
                        mixins: None,
                        attributes: if attrs.is_empty() { None } else { Some(attrs) },
                        implements: None,
                        doc,
                        signature,
                    };
                    result.symbols.push(symbol);
                }
            }
        }
    }
}

/// Recursively find the first identifier node within a subtree.
/// Useful for extracting the name from nested patterns.
fn find_first_identifier<'a>(node: &tree_sitter::Node<'a>) -> Option<tree_sitter::Node<'a>> {
    find_first_identifier_with_depth(node, 0)
}

fn find_first_identifier_with_depth<'a>(
    node: &tree_sitter::Node<'a>,
    depth: usize,
) -> Option<tree_sitter::Node<'a>> {
    // Prevent stack overflow on deeply nested patterns
    if depth > MAX_HELPER_DEPTH {
        return None;
    }

    if node.kind() == "identifier" {
        return Some(*node);
    }
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            if let Some(id) = find_first_identifier_with_depth(&child, depth + 1) {
                return Some(id);
            }
        }
    }
    None
}

fn qualified_name(name: &str, current_module: Option<&str>) -> String {
    let trimmed = name.trim().trim_matches('.');
    if trimmed.is_empty() {
        return current_module.unwrap_or_default().to_string();
    }

    match current_module {
        Some(prefix) if !prefix.trim().is_empty() => {
            let normalized = prefix.trim().trim_matches('.');
            if normalized.is_empty() || trimmed.starts_with(normalized) {
                trimmed.to_string()
            } else {
                format!("{}.{}", normalized, trimmed)
            }
        }
        _ => trimmed.to_string(),
    }
}

/// Check if a node is in a reference context (as opposed to a definition).
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
            // These are definition contexts, not references
            "function_declaration_left"
            | "value_declaration_left"
            | "module_defn"
            | "type_definition"
            | "record_type_defn"
            | "union_type_defn"
            | "class_type_defn"
            | "interface_type_defn"
            | "argument_patterns" => false,

            // Application expressions are references
            "application_expression" | "infix_expression" | "prefix_expression" => true,

            // Check parent's parent for more context
            _ => is_reference_context_with_depth(&parent, depth + 1),
        }
    } else {
        false
    }
}

/// Extract visibility from a node's access modifiers.
fn extract_visibility(node: &tree_sitter::Node, source: &[u8]) -> Visibility {
    // First, look for access_modifier directly on this node
    if let Some(modifier) = find_child_by_kind(node, "access_modifier") {
        if let Ok(text) = modifier.utf8_text(source) {
            return match text.trim() {
                "private" => Visibility::Private,
                "internal" => Visibility::Internal,
                "public" => Visibility::Public,
                _ => Visibility::Public,
            };
        }
    }

    // For function_or_value_defn, the access_modifier is inside function_declaration_left
    if let Some(decl) = find_child_by_kind(node, "function_declaration_left") {
        if let Some(modifier) = find_child_by_kind(&decl, "access_modifier") {
            if let Ok(text) = modifier.utf8_text(source) {
                return match text.trim() {
                    "private" => Visibility::Private,
                    "internal" => Visibility::Internal,
                    "public" => Visibility::Public,
                    _ => Visibility::Public,
                };
            }
        }
    }

    // Similarly for value_declaration_left
    if let Some(decl) = find_child_by_kind(node, "value_declaration_left") {
        if let Some(modifier) = find_child_by_kind(&decl, "access_modifier") {
            if let Ok(text) = modifier.utf8_text(source) {
                return match text.trim() {
                    "private" => Visibility::Private,
                    "internal" => Visibility::Internal,
                    "public" => Visibility::Public,
                    _ => Visibility::Public,
                };
            }
        }
    }

    Visibility::Public // Default visibility in F#
}

/// Find a child node by its kind.
fn find_child_by_kind<'a>(
    node: &'a tree_sitter::Node<'a>,
    kind: &str,
) -> Option<tree_sitter::Node<'a>> {
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            if child.kind() == kind {
                return Some(child);
            }
        }
    }
    None
}

/// Extract F# attributes from a node.
/// Looks for `attributes` sibling in the parent node (for value_declaration, type_definition, etc.)
fn extract_attributes(node: &tree_sitter::Node, source: &[u8]) -> Vec<String> {
    let mut attrs = Vec::new();

    // Look in parent node for `attributes` sibling
    if let Some(parent) = node.parent() {
        if let Some(attributes_node) = find_child_by_kind(&parent, "attributes") {
            extract_attrs_from_node(&attributes_node, source, &mut attrs);
        }
    }

    attrs
}

/// Recursively extract attribute names from an attributes node.
fn extract_attrs_from_node(node: &tree_sitter::Node, source: &[u8], attrs: &mut Vec<String>) {
    if node.kind() == "attribute" {
        // Find the simple_type -> long_identifier -> identifier
        if let Some(simple_type) = find_child_by_kind(node, "simple_type") {
            if let Some(long_id) = find_child_by_kind(&simple_type, "long_identifier") {
                // Get the first identifier (the attribute name)
                if let Some(id_node) = find_child_by_kind(&long_id, "identifier") {
                    if let Ok(name) = id_node.utf8_text(source) {
                        attrs.push(name.to_string());
                    }
                }
            }
        }
    }

    // Recurse into children
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            extract_attrs_from_node(&child, source, attrs);
        }
    }
}

/// Convert a tree-sitter node position to our Location type.
fn node_to_location(file: &Path, node: &tree_sitter::Node) -> Location {
    let start = node.start_position();
    let end = node.end_position();
    Location::with_end(
        file.to_path_buf(),
        (start.row + 1) as u32,    // Convert to 1-indexed
        (start.column + 1) as u32, // Convert to 1-indexed
        (end.row + 1) as u32,      // Convert to 1-indexed
        (end.column + 1) as u32,   // Convert to 1-indexed
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::extract_symbols;

    #[test]
    fn extracts_let_binding() {
        let source = r#"
module Test

let add x y = x + y
"#;
        let result = extract_symbols(Path::new("test.fs"), source, 500);

        assert!(result.symbols.len() >= 2); // module + function
        let func = result.symbols.iter().find(|s| s.name == "add");
        assert!(func.is_some());
        assert_eq!(func.unwrap().kind, SymbolKind::Function);
    }

    #[test]
    fn extracts_type_definition() {
        let source = r#"
type Person = { Name: string; Age: int }
"#;
        let result = extract_symbols(Path::new("test.fs"), source, 500);

        let type_sym = result.symbols.iter().find(|s| s.name == "Person");
        assert!(type_sym.is_some());
        assert_eq!(type_sym.unwrap().kind, SymbolKind::Record);
    }

    #[test]
    fn extracts_module_path() {
        let source = r#"
module MyApp.Services.Payment

let process () = ()
"#;
        let result = extract_symbols(Path::new("Payment.fs"), source, 500);

        let module = result.symbols.iter().find(|s| s.name == "Payment");
        assert!(module.is_some());
        assert_eq!(module.unwrap().qualified, "MyApp.Services.Payment");
    }

    #[test]
    fn extracts_opens() {
        let source = r#"
module Test

open System
open System.Collections.Generic

let x = 1
"#;
        let result = extract_symbols(Path::new("test.fs"), source, 500);

        assert!(result.opens.contains(&"System".to_string()));
        assert!(result
            .opens
            .contains(&"System.Collections.Generic".to_string()));
    }

    #[test]
    fn extracts_visibility() {
        let source = r#"
module Test

let private helper x = x + 1
let internal process x = x * 2
let public main () = ()
let defaultFn () = ()
"#;
        let result = extract_symbols(Path::new("test.fs"), source, 500);

        let helper = result.symbols.iter().find(|s| s.name == "helper");
        assert!(helper.is_some());
        assert_eq!(helper.unwrap().visibility, Visibility::Private);

        let process_fn = result.symbols.iter().find(|s| s.name == "process");
        assert!(process_fn.is_some());
        assert_eq!(process_fn.unwrap().visibility, Visibility::Internal);

        let main = result.symbols.iter().find(|s| s.name == "main");
        assert!(main.is_some());
        assert_eq!(main.unwrap().visibility, Visibility::Public);

        // Default visibility should be Public in F#
        let default_fn = result.symbols.iter().find(|s| s.name == "defaultFn");
        assert!(default_fn.is_some());
        assert_eq!(default_fn.unwrap().visibility, Visibility::Public);
    }

    #[test]
    fn extracts_fsharp_attributes() {
        let source = r#"
[<Obsolete("Use new API")>]
[<HttpGet("/users")>]
let myFunction x = x + 1

[<Struct>]
type Point = { X: int; Y: int }
"#;
        let result = extract_symbols(Path::new("test.fs"), source, 500);

        let func = result
            .symbols
            .iter()
            .find(|s| s.name == "myFunction")
            .expect("myFunction should be found");
        let attrs = func
            .attributes
            .as_ref()
            .expect("myFunction should have attributes");
        assert!(
            attrs.contains(&"Obsolete".to_string()),
            "Should have Obsolete attribute"
        );
        assert!(
            attrs.contains(&"HttpGet".to_string()),
            "Should have HttpGet attribute"
        );

        let point = result
            .symbols
            .iter()
            .find(|s| s.name == "Point")
            .expect("Point should be found");
        let point_attrs = point
            .attributes
            .as_ref()
            .expect("Point should have attributes");
        assert!(
            point_attrs.contains(&"Struct".to_string()),
            "Should have Struct attribute"
        );
    }

    #[test]
    fn extracts_interface_implementations() {
        let source = r#"
type MyType() =
    interface IComparable with
        member x.CompareTo(obj) = 0
    interface IDisposable with
        member x.Dispose() = ()
"#;
        let result = extract_symbols(Path::new("test.fs"), source, 500);

        let my_type = result
            .symbols
            .iter()
            .find(|s| s.name == "MyType")
            .expect("MyType should be found");
        let interfaces = my_type
            .implements
            .as_ref()
            .expect("MyType should implement interfaces");
        assert!(
            interfaces.contains(&"IComparable".to_string()),
            "Should implement IComparable"
        );
        assert!(
            interfaces.contains(&"IDisposable".to_string()),
            "Should implement IDisposable"
        );
    }

    #[test]
    fn extracts_doc_comments() {
        let source = r#"
/// This is a doc comment for the function.
/// It can span multiple lines.
let myFunction x = x + 1

/// Summary about the type
type MyRecord = { X: int; Y: int }
"#;
        let result = extract_symbols(Path::new("test.fs"), source, 500);

        let func = result
            .symbols
            .iter()
            .find(|s| s.name == "myFunction")
            .expect("myFunction should be found");
        let doc = func
            .doc
            .as_ref()
            .expect("myFunction should have doc comment");
        assert!(
            doc.contains("doc comment for the function"),
            "Should contain doc comment text: {}",
            doc
        );

        let record = result
            .symbols
            .iter()
            .find(|s| s.name == "MyRecord")
            .expect("MyRecord should be found");
        let record_doc = record
            .doc
            .as_ref()
            .expect("MyRecord should have doc comment");
        assert!(
            record_doc.contains("Summary about the type"),
            "Should contain type doc: {}",
            record_doc
        );
    }

    #[test]
    fn extracts_type_signatures() {
        let source = r#"
let add (x: int) (y: int): int = x + y
let value: int = 42
let noType x y = x + y
"#;
        let result = extract_symbols(Path::new("test.fs"), source, 500);

        let add_fn = result
            .symbols
            .iter()
            .find(|s| s.name == "add")
            .expect("add should be found");
        let sig = add_fn
            .signature
            .as_ref()
            .expect("add should have a signature");
        assert_eq!(
            sig, "int -> int -> int",
            "add signature should be int -> int -> int"
        );

        let value = result
            .symbols
            .iter()
            .find(|s| s.name == "value")
            .expect("value should be found");
        let value_sig = value
            .signature
            .as_ref()
            .expect("value should have a signature");
        assert_eq!(value_sig, "int", "value signature should be int");

        let no_type = result
            .symbols
            .iter()
            .find(|s| s.name == "noType")
            .expect("noType should be found");
        assert!(
            no_type.signature.is_none(),
            "noType should not have a signature"
        );
    }

    #[test]
    #[ignore] // Debug test - run with: cargo test debug_type_signature_ast -- --ignored --nocapture
    fn debug_type_signature_ast() {
        let source = r#"
let add (x: int) (y: int): int = x + y
let value: int = 42
"#;
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_fsharp::LANGUAGE_FSHARP.into())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();

        fn print_tree(node: &tree_sitter::Node, source: &str, indent: usize) {
            let indent_str = "  ".repeat(indent);
            let text = node.utf8_text(source.as_bytes()).unwrap_or("");
            let short_text = if text.len() > 40 { &text[..40] } else { text };
            println!(
                "{}[{}] {:?} = {:?}",
                indent_str,
                node.kind(),
                node.byte_range(),
                short_text.replace("\n", "\\n")
            );
            for i in 0..node.child_count() {
                if let Some(child) = node.child(i) {
                    print_tree(&child, source, indent + 1);
                }
            }
        }

        print_tree(&tree.root_node(), source, 0);
    }

    #[test]
    #[ignore] // Debug test - run with: cargo test debug_inherit_ast -- --ignored --nocapture
    fn debug_inherit_ast() {
        let source = r#"
type Base() =
    member _.Foo() = ()

type Derived() =
    inherit Base()
"#;

        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_fsharp::LANGUAGE_FSHARP.into())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();

        fn print_tree(node: &tree_sitter::Node, source: &str, indent: usize) {
            let indent_str = "  ".repeat(indent);
            let text = node.utf8_text(source.as_bytes()).unwrap_or("");
            let short_text = if text.len() > 60 { &text[..60] } else { text };
            println!(
                "{}[{}] {:?} = {:?}",
                indent_str,
                node.kind(),
                node.byte_range(),
                short_text.replace("\n", "\\n")
            );
            for i in 0..node.child_count() {
                if let Some(child) = node.child(i) {
                    print_tree(&child, source, indent + 1);
                }
            }
        }

        print_tree(&tree.root_node(), source, 0);
    }
}
