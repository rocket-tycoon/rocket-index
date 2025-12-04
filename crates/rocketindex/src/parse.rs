//! Symbol extraction from F# source files using tree-sitter.
//!
//! This module walks the tree-sitter CST and extracts:
//! - Symbol definitions (functions, types, modules, etc.)
//! - Symbol references (identifiers used but not defined)

use std::path::Path;

use crate::{Location, Reference, Symbol, SymbolKind, Visibility};

/// A syntax error detected during parsing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyntaxError {
    /// Error message describing the issue
    pub message: String,
    /// Location in the source file
    pub location: Location,
}

/// A warning generated during parsing (non-fatal issues).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseWarning {
    /// Warning message describing the issue
    pub message: String,
    /// Optional location in the source file
    pub location: Option<Location>,
}

/// Result of extracting symbols from a single file.
#[derive(Debug, Clone, Default)]
pub struct ParseResult {
    /// Symbols defined in this file
    pub symbols: Vec<Symbol>,
    /// References to symbols (identifiers used)
    pub references: Vec<Reference>,
    /// Module opens/imports in this file
    pub opens: Vec<String>,
    /// The module/namespace path for this file
    pub module_path: Option<String>,
    /// Syntax errors detected during parsing
    pub errors: Vec<SyntaxError>,
    /// Warnings generated during parsing (non-fatal issues like depth limits)
    pub warnings: Vec<ParseWarning>,
}

/// Extract symbols and references from F# source code.
///
/// # Arguments
/// * `file` - Path to the source file (for location tracking)
/// * `source` - The F# source code content
/// * `max_depth` - Maximum recursion depth for parsing
///
/// # Returns
/// A `ParseResult` containing all extracted symbols, references, and syntax errors.
pub fn extract_symbols(file: &Path, source: &str, max_depth: usize) -> ParseResult {
    let mut parser = tree_sitter::Parser::new();

    // Set the F# language - LANGUAGE_FSHARP is a compile-time constant from tree-sitter-fsharp,
    // so this can only fail if there's a version mismatch (a build-time issue, not runtime).
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

/// Extract syntax errors from the tree-sitter parse tree.
///
/// Tree-sitter marks parse errors in two ways:
/// 1. ERROR nodes - explicit error recovery nodes
/// 2. MISSING nodes - expected tokens that weren't found
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
///
/// Uses `Option<&str>` for current_module to avoid cloning on every iteration.
/// Module names are only created at module boundaries, not per-node.
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
            for i in 0..node.child_count() {
                if let Some(child) = node.child(i) {
                    extract_type_defn(&child, source, file, result, current_module);
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

fn extract_type_defn(
    node: &tree_sitter::Node,
    source: &[u8],
    file: &Path,
    result: &mut ParseResult,
    current_module: Option<&str>,
) {
    let kind = match node.kind() {
        "record_type_defn" => SymbolKind::Record,
        "union_type_defn" => SymbolKind::Union,
        "class_type_defn" => SymbolKind::Class,
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
            let symbol = Symbol {
                name: trimmed.to_string(),
                qualified: qualified_name(trimmed, current_module),
                kind,
                location: node_to_location(file, &name_node),
                visibility: Visibility::Public,
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
            let symbol = Symbol {
                name: trimmed.to_string(),
                qualified: qualified_name(trimmed, current_module),
                kind,
                location: node_to_location(file, &name_node),
                visibility: Visibility::Public,
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
                        };
                        result.symbols.push(symbol);
                    }
                }
            }
            extract_members(&child, source, file, result, current_module, depth + 1);
        }
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

                let symbol = Symbol {
                    name: trimmed.to_string(),
                    qualified,
                    kind,
                    location: node_to_location(file, &name_node),
                    visibility: extract_visibility(node, source),
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
                    let symbol = Symbol {
                        name: trimmed.to_string(),
                        qualified: qualified_name(trimmed, current_module),
                        kind: SymbolKind::Value,
                        location: node_to_location(file, &name_node),
                        visibility: extract_visibility(node, source),
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
///
/// Looks for `access_modifier` child nodes first (the proper tree-sitter approach),
/// then falls back to checking for `function_declaration_left` children which may
/// contain the access modifier.
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

    #[test]
    fn test_parse_result_default() {
        let result = ParseResult::default();
        assert!(result.symbols.is_empty());
        assert!(result.references.is_empty());
        assert!(result.opens.is_empty());
        assert!(result.module_path.is_none());
        assert!(result.errors.is_empty());
        assert!(result.warnings.is_empty());
    }

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
    #[ignore]
    fn dump_tree_sitter_cst() {
        // Test namespace with module inside
        let source = r#"
namespace RocketSpec.Core

module TestMetadata =
    let empty = { Tags = [] }
"#;

        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_fsharp::LANGUAGE_FSHARP.into())
            .expect("failed to load F# grammar");
        let tree = parser.parse(source, None).expect("failed to parse source");

        println!("{}", tree.root_node().to_sexp());
    }

    #[test]
    fn test_types_fs_parsing() {
        // Test a namespace file with types (simulating Types.fs structure)
        let source = r#"
namespace RocketSpec.Core

open System

type TestResult =
    | Pass
    | Fail of exn option
    | Skipped of reason: string

type TestMetadata =
    { Tags: string list
      Traits: Map<string, string> }

module TestMetadata =
    let empty = { Tags = []; Traits = Map.empty }

    let addTag tag metadata =
        { metadata with Tags = tag :: metadata.Tags }
"#;
        let result = extract_symbols(Path::new("Types.fs"), source, 500);

        // Debug: print all symbols
        for s in &result.symbols {
            eprintln!(
                "Found: {} ({:?}) at {}:{}",
                s.name, s.kind, s.qualified, s.location.line
            );
        }

        // Should find TestResult (union)
        let test_result = result.symbols.iter().find(|s| s.name == "TestResult");
        assert!(test_result.is_some(), "Should find TestResult union");
        assert_eq!(test_result.unwrap().kind, SymbolKind::Union);

        // Should find TestMetadata (record)
        let test_meta = result
            .symbols
            .iter()
            .find(|s| s.name == "TestMetadata" && s.kind == SymbolKind::Record);
        assert!(test_meta.is_some(), "Should find TestMetadata record");

        // Should find TestMetadata module - may use module_defn node type
        let test_meta_mod = result
            .symbols
            .iter()
            .find(|s| s.name == "TestMetadata" && s.kind == SymbolKind::Module);
        assert!(test_meta_mod.is_some(), "Should find TestMetadata module");

        // Should find functions in module
        let empty_fn = result.symbols.iter().find(|s| s.name == "empty");
        assert!(empty_fn.is_some(), "Should find empty value");

        let add_tag = result.symbols.iter().find(|s| s.name == "addTag");
        assert!(add_tag.is_some(), "Should find addTag function");
    }

    #[test]
    #[ignore]
    fn dump_visibility_cst() {
        // Test visibility modifiers
        let source = r#"
module Test

let private helper x = x + 1
let internal process x = x * 2
let public main () = ()
"#;

        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_fsharp::LANGUAGE_FSHARP.into())
            .expect("failed to load F# grammar");
        let tree = parser.parse(source, None).expect("failed to parse source");

        println!("{}", tree.root_node().to_sexp());
    }

    #[test]
    #[ignore]
    fn test_real_types_fs() {
        let source = std::fs::read_to_string(
            "/Users/alastair/work/rocket-tycoon/RocketSpec/src/RocketSpec.Core/Types.fs",
        )
        .unwrap();

        // Check for parse errors first
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_fsharp::LANGUAGE_FSHARP.into())
            .expect("failed to load F# grammar");
        let tree = parser.parse(&source, None).expect("failed to parse source");
        let root = tree.root_node();

        println!("Root kind: {}", root.kind());
        println!("Has error: {}", root.has_error());
        println!("Child count: {}", root.child_count());

        // Print first few children
        for i in 0..std::cmp::min(5, root.child_count()) {
            if let Some(child) = root.child(i) {
                println!(
                    "Child {}: {} (error: {})",
                    i,
                    child.kind(),
                    child.is_error()
                );
            }
        }

        let result = extract_symbols(std::path::Path::new("Types.fs"), &source, 500);

        println!("Found {} symbols:", result.symbols.len());
        for s in &result.symbols {
            println!(
                "  {} ({:?}) at {}:{}",
                s.name, s.kind, s.qualified, s.location.line
            );
        }

        assert!(
            !result.symbols.is_empty(),
            "Should extract symbols from Types.fs"
        );
    }

    // ============================================================
    // Syntax Error Extraction Tests (TDD - Phase 1)
    // ============================================================

    #[test]
    fn detects_syntax_error_missing_equals() {
        let source = r#"
module Test

let x 42
"#;
        let result = extract_symbols(Path::new("test.fs"), source, 500);

        assert!(!result.errors.is_empty(), "Should detect syntax error");
        // The error should be near line 4 where "let x 42" is invalid
        let error = &result.errors[0];
        assert!(
            error.location.line >= 3,
            "Error should be on or after line 3"
        );
    }

    #[test]
    fn detects_syntax_error_unclosed_bracket() {
        let source = r#"
module Test

let items = [1; 2; 3
"#;
        let result = extract_symbols(Path::new("test.fs"), source, 500);

        assert!(!result.errors.is_empty(), "Should detect unclosed bracket");
    }

    #[test]
    fn detects_syntax_error_incomplete_match() {
        let source = r#"
module Test

let result = match x with
    | Some
"#;
        let result = extract_symbols(Path::new("test.fs"), source, 500);

        // Incomplete match expression should be an error
        assert!(
            !result.errors.is_empty(),
            "Should detect incomplete match expression"
        );
    }

    #[test]
    fn no_errors_for_valid_code() {
        let source = r#"
module Test

let add x y = x + y

type Person = { Name: string; Age: int }

let greet person =
    printfn "Hello %s" person.Name
"#;
        let result = extract_symbols(Path::new("test.fs"), source, 500);

        assert!(
            result.errors.is_empty(),
            "Valid code should have no errors, but got: {:?}",
            result.errors
        );
    }

    #[test]
    fn detects_multiple_errors() {
        let source = r#"
module Test

let x 42
let y [1; 2
"#;
        let result = extract_symbols(Path::new("test.fs"), source, 500);

        assert!(
            !result.errors.is_empty(),
            "Should detect at least one error"
        );
    }

    #[test]
    fn error_has_correct_location() {
        let source = r#"module Test

let x 42"#;
        let result = extract_symbols(Path::new("test.fs"), source, 500);

        assert!(!result.errors.is_empty(), "Should detect syntax error");
        let error = &result.errors[0];
        // Error should be on line 3 (the invalid let binding)
        assert_eq!(error.location.line, 3, "Error should be on line 3");
    }
}
