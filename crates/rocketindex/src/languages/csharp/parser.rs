//! Symbol extraction from C# source files using tree-sitter.

use std::cell::RefCell;
use std::path::Path;

use crate::parse::{node_to_location, LanguageParser, ParseResult};
use crate::{Reference, Symbol, SymbolKind, Visibility};

// Thread-local parser reuse - avoids creating a new parser per file
thread_local! {
    static CSHARP_PARSER: RefCell<tree_sitter::Parser> = RefCell::new({
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_c_sharp::LANGUAGE.into())
            .expect("tree-sitter-c-sharp grammar incompatible with tree-sitter version");
        parser
    });
}

pub struct CSharpParser;

impl LanguageParser for CSharpParser {
    fn extract_symbols(&self, file: &Path, source: &str, max_depth: usize) -> ParseResult {
        CSHARP_PARSER.with(|parser| {
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

            // Check for file-scoped namespace first - these don't have bodies,
            // so we need to pass the namespace to all sibling declarations
            let file_namespace = extract_file_scoped_namespace(&root, source.as_bytes());

            extract_recursive(
                &root,
                source.as_bytes(),
                file,
                &mut result,
                file_namespace.as_deref(),
                max_depth,
                0,
            );

            // Extract references in a separate pass
            extract_references_recursive(&root, source.as_bytes(), file, &mut result);

            result
        })
    }
}

/// Extract file-scoped namespace from a compilation unit.
/// File-scoped namespaces (namespace X;) don't have bodies - all subsequent
/// declarations in the file are within that namespace.
fn extract_file_scoped_namespace(root: &tree_sitter::Node, source: &[u8]) -> Option<String> {
    for i in 0..root.child_count() {
        if let Some(child) = root.child(i) {
            if child.kind() == "file_scoped_namespace_declaration" {
                if let Some(name_node) = child.child_by_field_name("name") {
                    return Some(node_text(&name_node, source));
                }
            }
        }
    }
    None
}

fn node_text(node: &tree_sitter::Node, source: &[u8]) -> String {
    node.utf8_text(source).unwrap_or_default().to_string()
}

/// Extract XML doc comments (/// style) preceding a node
fn extract_doc_comments(node: &tree_sitter::Node, source: &[u8]) -> Option<String> {
    let mut docs = Vec::new();

    let mut prev = node.prev_sibling();
    while let Some(sib) = prev {
        match sib.kind() {
            "comment" => {
                if let Ok(text) = sib.utf8_text(source) {
                    // C# XML doc comments start with ///
                    if text.starts_with("///") {
                        let doc = text.trim_start_matches("///").trim();
                        if !doc.is_empty() {
                            docs.insert(0, doc.to_string());
                        }
                    }
                }
                prev = sib.prev_sibling();
            }
            _ => break, // Stop at non-comment
        }
    }

    if docs.is_empty() {
        None
    } else {
        Some(docs.join("\n"))
    }
}

fn extract_recursive(
    node: &tree_sitter::Node,
    source: &[u8],
    file: &Path,
    result: &mut ParseResult,
    namespace: Option<&str>,
    max_depth: usize,
    current_depth: usize,
) {
    if current_depth > max_depth {
        return;
    }

    // Build qualified name prefix from current namespace context
    let context_prefix = namespace.unwrap_or("");

    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            match child.kind() {
                "class_declaration" => {
                    if let Some(symbol) = extract_type_declaration(
                        &child,
                        source,
                        file,
                        context_prefix,
                        SymbolKind::Class,
                    ) {
                        let nested_prefix = symbol.qualified.clone();
                        result.symbols.push(symbol);
                        // Recurse into class body for nested types and members
                        if let Some(body) = child.child_by_field_name("body") {
                            extract_class_members(
                                &body,
                                source,
                                file,
                                result,
                                &nested_prefix,
                                max_depth,
                                current_depth,
                            );
                        }
                    }
                }
                "interface_declaration" => {
                    if let Some(symbol) = extract_type_declaration(
                        &child,
                        source,
                        file,
                        context_prefix,
                        SymbolKind::Interface,
                    ) {
                        let nested_prefix = symbol.qualified.clone();
                        result.symbols.push(symbol);
                        // Recurse into interface body for method signatures
                        if let Some(body) = child.child_by_field_name("body") {
                            extract_class_members(
                                &body,
                                source,
                                file,
                                result,
                                &nested_prefix,
                                max_depth,
                                current_depth,
                            );
                        }
                    }
                }
                "struct_declaration" => {
                    // Map C# struct to Record (similar to F# records)
                    if let Some(symbol) = extract_type_declaration(
                        &child,
                        source,
                        file,
                        context_prefix,
                        SymbolKind::Record,
                    ) {
                        let nested_prefix = symbol.qualified.clone();
                        result.symbols.push(symbol);
                        if let Some(body) = child.child_by_field_name("body") {
                            extract_class_members(
                                &body,
                                source,
                                file,
                                result,
                                &nested_prefix,
                                max_depth,
                                current_depth,
                            );
                        }
                    }
                }
                "enum_declaration" => {
                    // Map C# enum to Union (similar to F# discriminated unions)
                    if let Some(symbol) = extract_type_declaration(
                        &child,
                        source,
                        file,
                        context_prefix,
                        SymbolKind::Union,
                    ) {
                        let nested_prefix = symbol.qualified.clone();
                        result.symbols.push(symbol);
                        // Extract enum members
                        if let Some(body) = child.child_by_field_name("body") {
                            extract_enum_members(&body, source, file, result, &nested_prefix);
                        }
                    }
                }
                "record_declaration" | "record_struct_declaration" => {
                    // C# records map to Record kind
                    if let Some(symbol) = extract_type_declaration(
                        &child,
                        source,
                        file,
                        context_prefix,
                        SymbolKind::Record,
                    ) {
                        let nested_prefix = symbol.qualified.clone();
                        result.symbols.push(symbol);
                        // Extract record parameters as members
                        extract_record_parameters(&child, source, file, result, &nested_prefix);
                        // Recurse into record body if present
                        if let Some(body) = child.child_by_field_name("body") {
                            extract_class_members(
                                &body,
                                source,
                                file,
                                result,
                                &nested_prefix,
                                max_depth,
                                current_depth,
                            );
                        }
                    }
                }
                "delegate_declaration" => {
                    if let Some(symbol) =
                        extract_delegate_declaration(&child, source, file, context_prefix)
                    {
                        result.symbols.push(symbol);
                    }
                }
                "namespace_declaration" => {
                    // Extract namespace name and combine with any parent namespace
                    let ns_name = if let Some(name_node) = child.child_by_field_name("name") {
                        let name = node_text(&name_node, source);
                        if context_prefix.is_empty() {
                            name
                        } else {
                            format!("{}.{}", context_prefix, name)
                        }
                    } else {
                        context_prefix.to_string()
                    };
                    // Recurse into namespace body
                    if let Some(body) = child.child_by_field_name("body") {
                        extract_recursive(
                            &body,
                            source,
                            file,
                            result,
                            Some(&ns_name),
                            max_depth,
                            current_depth + 1,
                        );
                    }
                }
                "file_scoped_namespace_declaration" => {
                    // Already handled at the top level - just skip this node
                }
                "using_directive" => {
                    // Extract using statement for name resolution
                    // Handles: using System; using System.Collections.Generic;
                    for i in 0..child.child_count() {
                        if let Some(name_child) = child.child(i) {
                            if name_child.kind() == "qualified_name"
                                || name_child.kind() == "identifier"
                            {
                                if let Ok(text) = name_child.utf8_text(source) {
                                    result.opens.push(text.to_string());
                                }
                            }
                        }
                    }
                }
                _ => {
                    // Recurse into other nodes (e.g., file-scoped namespace content)
                    extract_recursive(
                        &child,
                        source,
                        file,
                        result,
                        namespace,
                        max_depth,
                        current_depth + 1,
                    );
                }
            }
        }
    }
}

fn extract_type_declaration(
    node: &tree_sitter::Node,
    source: &[u8],
    file: &Path,
    namespace: &str,
    kind: SymbolKind,
) -> Option<Symbol> {
    let name_node = node.child_by_field_name("name")?;
    let name = node_text(&name_node, source);

    let qualified = if namespace.is_empty() {
        name.clone()
    } else {
        format!("{}.{}", namespace, name)
    };

    let visibility = extract_visibility(node, source);
    let doc = extract_doc_comments(node, source);

    Some(Symbol {
        name,
        qualified,
        kind,
        location: node_to_location(file, node),
        visibility,
        language: "csharp".to_string(),
        parent: None,
        mixins: None,
        attributes: None,
        implements: None,
        doc,
        signature: None,
    })
}

fn extract_delegate_declaration(
    node: &tree_sitter::Node,
    source: &[u8],
    file: &Path,
    namespace: &str,
) -> Option<Symbol> {
    let name_node = node.child_by_field_name("name")?;
    let name = node_text(&name_node, source);

    let qualified = if namespace.is_empty() {
        name.clone()
    } else {
        format!("{}.{}", namespace, name)
    };

    let visibility = extract_visibility(node, source);

    Some(Symbol {
        name,
        qualified,
        kind: SymbolKind::Function, // Delegates are like function types
        location: node_to_location(file, node),
        visibility,
        language: "csharp".to_string(),
        parent: None,
        mixins: None,
        attributes: None,
        implements: None,
        doc: None,
        signature: None,
    })
}

fn extract_class_members(
    body: &tree_sitter::Node,
    source: &[u8],
    file: &Path,
    result: &mut ParseResult,
    class_prefix: &str,
    max_depth: usize,
    current_depth: usize,
) {
    if current_depth > max_depth {
        return;
    }

    for i in 0..body.child_count() {
        if let Some(child) = body.child(i) {
            match child.kind() {
                "method_declaration" => {
                    if let Some(symbol) = extract_method(&child, source, file, class_prefix) {
                        result.symbols.push(symbol);
                    }
                }
                "constructor_declaration" => {
                    if let Some(symbol) = extract_constructor(&child, source, file, class_prefix) {
                        result.symbols.push(symbol);
                    }
                }
                "field_declaration" => {
                    extract_fields(&child, source, file, result, class_prefix);
                }
                "property_declaration" => {
                    if let Some(symbol) = extract_property(&child, source, file, class_prefix) {
                        result.symbols.push(symbol);
                    }
                }
                "event_declaration" | "event_field_declaration" => {
                    if let Some(symbol) = extract_event(&child, source, file, class_prefix) {
                        result.symbols.push(symbol);
                    }
                }
                "indexer_declaration" => {
                    if let Some(symbol) = extract_indexer(&child, source, file, class_prefix) {
                        result.symbols.push(symbol);
                    }
                }
                "operator_declaration" => {
                    if let Some(symbol) = extract_operator(&child, source, file, class_prefix) {
                        result.symbols.push(symbol);
                    }
                }
                "class_declaration" => {
                    // Nested class
                    if let Some(symbol) = extract_type_declaration(
                        &child,
                        source,
                        file,
                        class_prefix,
                        SymbolKind::Class,
                    ) {
                        let nested_prefix = symbol.qualified.clone();
                        result.symbols.push(symbol);
                        if let Some(body) = child.child_by_field_name("body") {
                            extract_class_members(
                                &body,
                                source,
                                file,
                                result,
                                &nested_prefix,
                                max_depth,
                                current_depth + 1,
                            );
                        }
                    }
                }
                "struct_declaration" => {
                    // Nested struct
                    if let Some(symbol) = extract_type_declaration(
                        &child,
                        source,
                        file,
                        class_prefix,
                        SymbolKind::Record,
                    ) {
                        let nested_prefix = symbol.qualified.clone();
                        result.symbols.push(symbol);
                        if let Some(body) = child.child_by_field_name("body") {
                            extract_class_members(
                                &body,
                                source,
                                file,
                                result,
                                &nested_prefix,
                                max_depth,
                                current_depth + 1,
                            );
                        }
                    }
                }
                "interface_declaration" => {
                    // Nested interface
                    if let Some(symbol) = extract_type_declaration(
                        &child,
                        source,
                        file,
                        class_prefix,
                        SymbolKind::Interface,
                    ) {
                        let nested_prefix = symbol.qualified.clone();
                        result.symbols.push(symbol);
                        if let Some(body) = child.child_by_field_name("body") {
                            extract_class_members(
                                &body,
                                source,
                                file,
                                result,
                                &nested_prefix,
                                max_depth,
                                current_depth + 1,
                            );
                        }
                    }
                }
                "enum_declaration" => {
                    // Nested enum
                    if let Some(symbol) = extract_type_declaration(
                        &child,
                        source,
                        file,
                        class_prefix,
                        SymbolKind::Union,
                    ) {
                        let nested_prefix = symbol.qualified.clone();
                        result.symbols.push(symbol);
                        if let Some(body) = child.child_by_field_name("body") {
                            extract_enum_members(&body, source, file, result, &nested_prefix);
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

fn extract_method(
    node: &tree_sitter::Node,
    source: &[u8],
    file: &Path,
    class_prefix: &str,
) -> Option<Symbol> {
    let name_node = node.child_by_field_name("name")?;
    let name = node_text(&name_node, source);

    let qualified = format!("{}.{}", class_prefix, name);
    let visibility = extract_visibility(node, source);

    Some(Symbol {
        name,
        qualified,
        kind: SymbolKind::Function,
        location: node_to_location(file, node),
        visibility,
        language: "csharp".to_string(),
        parent: None,
        mixins: None,
        attributes: None,
        implements: None,
        doc: None,
        signature: None,
    })
}

fn extract_constructor(
    node: &tree_sitter::Node,
    source: &[u8],
    file: &Path,
    class_prefix: &str,
) -> Option<Symbol> {
    let name_node = node.child_by_field_name("name")?;
    let name = node_text(&name_node, source);

    let qualified = format!("{}.{}", class_prefix, name);
    let visibility = extract_visibility(node, source);

    Some(Symbol {
        name,
        qualified,
        kind: SymbolKind::Function, // Constructors are functions
        location: node_to_location(file, node),
        visibility,
        language: "csharp".to_string(),
        parent: None,
        mixins: None,
        attributes: None,
        implements: None,
        doc: None,
        signature: None,
    })
}

fn extract_fields(
    node: &tree_sitter::Node,
    source: &[u8],
    file: &Path,
    result: &mut ParseResult,
    class_prefix: &str,
) {
    let visibility = extract_visibility(node, source);

    // Find variable declarations within the field
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            if child.kind() == "variable_declaration" {
                // Extract each variable declarator
                for j in 0..child.child_count() {
                    if let Some(declarator) = child.child(j) {
                        if declarator.kind() == "variable_declarator" {
                            if let Some(name_node) = declarator.child_by_field_name("name") {
                                let name = node_text(&name_node, source);
                                let qualified = format!("{}.{}", class_prefix, name);

                                result.symbols.push(Symbol {
                                    name,
                                    qualified,
                                    kind: SymbolKind::Member, // Fields are members
                                    location: node_to_location(file, &declarator),
                                    visibility,
                                    language: "csharp".to_string(),
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
        }
    }
}

fn extract_property(
    node: &tree_sitter::Node,
    source: &[u8],
    file: &Path,
    class_prefix: &str,
) -> Option<Symbol> {
    let name_node = node.child_by_field_name("name")?;
    let name = node_text(&name_node, source);

    let qualified = format!("{}.{}", class_prefix, name);
    let visibility = extract_visibility(node, source);

    Some(Symbol {
        name,
        qualified,
        kind: SymbolKind::Member, // Properties are members
        location: node_to_location(file, node),
        visibility,
        language: "csharp".to_string(),
        parent: None,
        mixins: None,
        attributes: None,
        implements: None,
        doc: None,
        signature: None,
    })
}

fn extract_event(
    node: &tree_sitter::Node,
    source: &[u8],
    file: &Path,
    class_prefix: &str,
) -> Option<Symbol> {
    // Try to find the event name - can be in different places depending on event style
    let name = if let Some(name_node) = node.child_by_field_name("name") {
        node_text(&name_node, source)
    } else {
        // For event_field_declaration, look in variable_declaration
        let mut found_name = None;
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                if child.kind() == "variable_declaration" {
                    for j in 0..child.child_count() {
                        if let Some(declarator) = child.child(j) {
                            if declarator.kind() == "variable_declarator" {
                                if let Some(name_node) = declarator.child_by_field_name("name") {
                                    found_name = Some(node_text(&name_node, source));
                                    break;
                                }
                            }
                        }
                    }
                }
            }
        }
        found_name?
    };

    let qualified = format!("{}.{}", class_prefix, name);
    let visibility = extract_visibility(node, source);

    Some(Symbol {
        name,
        qualified,
        kind: SymbolKind::Member, // Events are members
        location: node_to_location(file, node),
        visibility,
        language: "csharp".to_string(),
        parent: None,
        mixins: None,
        attributes: None,
        implements: None,
        doc: None,
        signature: None,
    })
}

fn extract_indexer(
    node: &tree_sitter::Node,
    source: &[u8],
    file: &Path,
    class_prefix: &str,
) -> Option<Symbol> {
    // Indexers use "this" as their identifier
    let name = "this".to_string();
    let qualified = format!("{}.{}", class_prefix, name);
    let visibility = extract_visibility(node, source);

    Some(Symbol {
        name,
        qualified,
        kind: SymbolKind::Member, // Indexers are like special properties
        location: node_to_location(file, node),
        visibility,
        language: "csharp".to_string(),
        parent: None,
        mixins: None,
        attributes: None,
        implements: None,
        doc: None,
        signature: None,
    })
}

fn extract_operator(
    node: &tree_sitter::Node,
    source: &[u8],
    file: &Path,
    class_prefix: &str,
) -> Option<Symbol> {
    // Find the operator symbol
    let mut operator_symbol = None;
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            // Look for the operator token (e.g., +, -, ==, etc.)
            let kind = child.kind();
            if kind.starts_with("operator") || ["implicit", "explicit"].contains(&kind) {
                operator_symbol = Some(node_text(&child, source));
                break;
            }
            // Also check for conversion operators
            if kind == "implicit_type" || kind == "explicit" {
                operator_symbol = Some(kind.to_string());
                break;
            }
        }
    }

    let name = format!("operator {}", operator_symbol.unwrap_or_default());
    let qualified = format!("{}.{}", class_prefix, name);
    let visibility = extract_visibility(node, source);

    Some(Symbol {
        name,
        qualified,
        kind: SymbolKind::Function,
        location: node_to_location(file, node),
        visibility,
        language: "csharp".to_string(),
        parent: None,
        mixins: None,
        attributes: None,
        implements: None,
        doc: None,
        signature: None,
    })
}

fn extract_record_parameters(
    node: &tree_sitter::Node,
    source: &[u8],
    file: &Path,
    result: &mut ParseResult,
    record_prefix: &str,
) {
    // Look for parameter_list in the record declaration
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            if child.kind() == "parameter_list" {
                for j in 0..child.child_count() {
                    if let Some(param) = child.child(j) {
                        if param.kind() == "parameter" {
                            if let Some(name_node) = param.child_by_field_name("name") {
                                let name = node_text(&name_node, source);
                                let qualified = format!("{}.{}", record_prefix, name);

                                result.symbols.push(Symbol {
                                    name,
                                    qualified,
                                    kind: SymbolKind::Member, // Record params become properties/members
                                    location: node_to_location(file, &param),
                                    visibility: Visibility::Public,
                                    language: "csharp".to_string(),
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
        }
    }
}

fn extract_enum_members(
    body: &tree_sitter::Node,
    source: &[u8],
    file: &Path,
    result: &mut ParseResult,
    enum_prefix: &str,
) {
    for i in 0..body.child_count() {
        if let Some(child) = body.child(i) {
            if child.kind() == "enum_member_declaration" {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let name = node_text(&name_node, source);
                    let qualified = format!("{}.{}", enum_prefix, name);

                    result.symbols.push(Symbol {
                        name,
                        qualified,
                        kind: SymbolKind::Value, // Enum members are values
                        location: node_to_location(file, &child),
                        visibility: Visibility::Public,
                        language: "csharp".to_string(),
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

fn extract_visibility(node: &tree_sitter::Node, source: &[u8]) -> Visibility {
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            if child.kind() == "modifier" {
                let text = node_text(&child, source);
                match text.as_str() {
                    "public" => return Visibility::Public,
                    "private" => return Visibility::Private,
                    "protected" => return Visibility::Internal, // Map protected to Internal
                    "internal" => return Visibility::Internal,
                    _ => {}
                }
            }
        }
    }
    // Default visibility in C# is internal for top-level types, private for members
    Visibility::Private
}

/// Recursively extract references from the AST
fn extract_references_recursive(
    node: &tree_sitter::Node,
    source: &[u8],
    file: &Path,
    result: &mut ParseResult,
) {
    match node.kind() {
        // Type identifiers in C# are class/struct/interface names used as types
        "identifier" => {
            if is_type_reference_context(node, source) {
                if let Ok(name) = node.utf8_text(source) {
                    result.references.push(Reference {
                        name: name.to_string(),
                        location: node_to_location(file, node),
                    });
                }
            }
        }

        // Qualified names like System.Console
        "qualified_name" => {
            if is_type_reference_context(node, source) {
                if let Ok(name) = node.utf8_text(source) {
                    result.references.push(Reference {
                        name: name.to_string(),
                        location: node_to_location(file, node),
                    });
                }
            }
            // Don't recurse into qualified_name children - we want the full name
            return;
        }

        // Generic types like List<User>
        "generic_name" => {
            if is_type_reference_context(node, source) {
                // Extract just the base type name (not the type arguments)
                if let Some(name_node) = node.child(0) {
                    if name_node.kind() == "identifier" {
                        if let Ok(name) = name_node.utf8_text(source) {
                            result.references.push(Reference {
                                name: name.to_string(),
                                location: node_to_location(file, &name_node),
                            });
                        }
                    }
                }
            }
        }

        // Method calls: policy.Execute(), Helper.Process(), Execute()
        "invocation_expression" => {
            // Extract the method being called
            // Structure: invocation_expression -> member_access_expression | identifier
            //                                  -> argument_list
            if let Some(function) = node.child(0) {
                match function.kind() {
                    // obj.Method() or Class.Method()
                    "member_access_expression" => {
                        // Get the method name (the last identifier after the dot)
                        if let Some(name_node) = function.child_by_field_name("name") {
                            if let Ok(method_name) = name_node.utf8_text(source) {
                                // Store just the method name for simpler matching
                                result.references.push(Reference {
                                    name: method_name.to_string(),
                                    location: node_to_location(file, &name_node),
                                });
                            }
                        }
                    }
                    // Direct method call: Execute() or MethodName()
                    "identifier" => {
                        if let Ok(name) = function.utf8_text(source) {
                            result.references.push(Reference {
                                name: name.to_string(),
                                location: node_to_location(file, &function),
                            });
                        }
                    }
                    // Generic method call: Method<T>()
                    "generic_name" => {
                        if let Some(name_node) = function.child(0) {
                            if name_node.kind() == "identifier" {
                                if let Ok(name) = name_node.utf8_text(source) {
                                    result.references.push(Reference {
                                        name: name.to_string(),
                                        location: node_to_location(file, &name_node),
                                    });
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        _ => {}
    }

    // Recurse into children
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            extract_references_recursive(&child, source, file, result);
        }
    }
}

/// Determine if an identifier or qualified_name is used as a type reference
fn is_type_reference_context(node: &tree_sitter::Node, source: &[u8]) -> bool {
    let parent = match node.parent() {
        Some(p) => p,
        None => return false,
    };

    match parent.kind() {
        // Class/interface/struct definition - this is a definition, not a reference
        "class_declaration"
        | "interface_declaration"
        | "struct_declaration"
        | "enum_declaration"
        | "record_declaration"
        | "record_struct_declaration"
        | "delegate_declaration" => {
            // Check if this is the name being defined
            if let Some(name_node) = parent.child_by_field_name("name") {
                if node.id() == name_node.id() {
                    return false;
                }
            }
            // Could be in base list - that's a reference
            if is_descendant_of(node, "base_list") {
                return true;
            }
            false
        }

        // Variable declaration - the type is a reference
        "variable_declaration" => true,

        // Parameter - the type is a reference
        "parameter" => true,

        // Method return type
        "method_declaration"
        | "constructor_declaration"
        | "operator_declaration"
        | "conversion_operator_declaration" => {
            // Type before the name is a reference
            if let Some(name_node) = parent.child_by_field_name("name") {
                if node.end_byte() < name_node.start_byte() {
                    return true;
                }
            }
            false
        }

        // Property type
        "property_declaration" | "indexer_declaration" | "event_declaration" => true,

        // Object creation expression - the type is a reference
        "object_creation_expression" => true,

        // Cast expression - the type is a reference
        "cast_expression" => true,

        // Type argument (generics)
        "type_argument_list" => true,

        // Base list (inheritance)
        "base_list" => true,

        // Type constraint
        "type_parameter_constraint" | "type_parameter_constraints_clause" => true,

        // Array type
        "array_type" => true,

        // Nullable type
        "nullable_type" => true,

        // typeof expression
        "typeof_expression" => true,

        // is/as pattern
        "is_expression" | "as_expression" | "is_pattern_expression" => true,

        // Catch clause
        "catch_declaration" => true,

        // Member access - check if this is a class name in static access like Helper.Greet
        "member_access_expression" => {
            // Check if this is the first child (the object being accessed) and starts with uppercase
            if let Some(first_child) = parent.child(0) {
                if node.id() == first_child.id() {
                    // Check if name starts with uppercase (convention for class names)
                    if let Ok(name) = node.utf8_text(source) {
                        if let Some(first_char) = name.chars().next() {
                            return first_char.is_uppercase();
                        }
                    }
                }
            }
            false
        }

        // Invocation expression - check for static method calls
        "invocation_expression" => {
            // If this identifier is in a member_access_expression that's the function being called
            if let Some(first_child) = parent.child(0) {
                if first_child.kind() == "member_access_expression" {
                    // This will be handled by member_access_expression case above
                    return false;
                }
                // Direct identifier call - could be a type for constructor call
                if node.id() == first_child.id() {
                    if let Ok(name) = node.utf8_text(source) {
                        if let Some(first_char) = name.chars().next() {
                            return first_char.is_uppercase();
                        }
                    }
                }
            }
            false
        }

        _ => false,
    }
}

/// Check if a node is a descendant of a node with the given kind
fn is_descendant_of(node: &tree_sitter::Node, kind: &str) -> bool {
    let mut current = node.parent();
    while let Some(parent) = current {
        if parent.kind() == kind {
            return true;
        }
        current = parent.parent();
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::{extract_symbols, LanguageParser};

    #[test]
    fn test_basic_class() {
        let source = r#"
namespace MyApp.Models;

public class User {
    public string Name { get; set; }
    public void Save() { }
}
"#;
        let parser = CSharpParser;
        let result = parser.extract_symbols(Path::new("User.cs"), source, 100);

        let class = result.symbols.iter().find(|s| s.name == "User");
        assert!(class.is_some(), "class User should be indexed");
        let class = class.unwrap();
        assert_eq!(class.kind, SymbolKind::Class);
        assert_eq!(class.qualified, "MyApp.Models.User");
        assert_eq!(class.visibility, Visibility::Public);

        let name_prop = result.symbols.iter().find(|s| s.name == "Name");
        assert!(name_prop.is_some(), "property Name should be indexed");
        assert_eq!(name_prop.unwrap().qualified, "MyApp.Models.User.Name");

        let save_method = result.symbols.iter().find(|s| s.name == "Save");
        assert!(save_method.is_some(), "method Save should be indexed");
        assert_eq!(save_method.unwrap().qualified, "MyApp.Models.User.Save");
    }

    #[test]
    fn test_interface() {
        let source = r#"
namespace MyApp.Services;

public interface IUserService {
    User GetUser(int id);
    void SaveUser(User user);
}
"#;
        let parser = CSharpParser;
        let result = parser.extract_symbols(Path::new("IUserService.cs"), source, 100);

        let interface = result.symbols.iter().find(|s| s.name == "IUserService");
        assert!(
            interface.is_some(),
            "interface IUserService should be indexed"
        );
        assert_eq!(interface.unwrap().kind, SymbolKind::Interface);
        assert_eq!(interface.unwrap().qualified, "MyApp.Services.IUserService");

        let get_user = result.symbols.iter().find(|s| s.name == "GetUser");
        assert!(get_user.is_some(), "method GetUser should be indexed");
    }

    #[test]
    fn test_struct() {
        let source = r#"
namespace MyApp.Models;

public struct Point {
    public int X;
    public int Y;
}
"#;
        let parser = CSharpParser;
        let result = parser.extract_symbols(Path::new("Point.cs"), source, 100);

        let struct_sym = result.symbols.iter().find(|s| s.name == "Point");
        assert!(struct_sym.is_some(), "struct Point should be indexed");
        assert_eq!(struct_sym.unwrap().kind, SymbolKind::Record);

        let x_field = result.symbols.iter().find(|s| s.name == "X");
        assert!(x_field.is_some(), "field X should be indexed");
        assert_eq!(x_field.unwrap().kind, SymbolKind::Member);
    }

    #[test]
    fn test_enum() {
        let source = r#"
namespace MyApp;

public enum Status {
    Active,
    Inactive,
    Pending
}
"#;
        let parser = CSharpParser;
        let result = parser.extract_symbols(Path::new("Status.cs"), source, 100);

        let enum_sym = result.symbols.iter().find(|s| s.name == "Status");
        assert!(enum_sym.is_some(), "enum Status should be indexed");
        assert_eq!(enum_sym.unwrap().kind, SymbolKind::Union);

        let active = result.symbols.iter().find(|s| s.name == "Active");
        assert!(active.is_some(), "enum member Active should be indexed");
        assert_eq!(active.unwrap().kind, SymbolKind::Value);
        assert_eq!(active.unwrap().qualified, "MyApp.Status.Active");
    }

    #[test]
    fn test_record() {
        let source = r#"
namespace MyApp.Models;

public record Person(string FirstName, string LastName) {
    public string FullName => $"{FirstName} {LastName}";
}
"#;
        let parser = CSharpParser;
        let result = parser.extract_symbols(Path::new("Person.cs"), source, 100);

        let record = result.symbols.iter().find(|s| s.name == "Person");
        assert!(record.is_some(), "record Person should be indexed");
        assert_eq!(record.unwrap().kind, SymbolKind::Record);

        let first_name = result.symbols.iter().find(|s| s.name == "FirstName");
        assert!(
            first_name.is_some(),
            "record parameter FirstName should be indexed"
        );
        assert_eq!(
            first_name.unwrap().qualified,
            "MyApp.Models.Person.FirstName"
        );
    }

    #[test]
    fn test_nested_namespace() {
        let source = r#"
namespace MyApp {
    namespace Models {
        public class User { }
    }
}
"#;
        let parser = CSharpParser;
        let result = parser.extract_symbols(Path::new("User.cs"), source, 100);

        let class = result.symbols.iter().find(|s| s.name == "User");
        assert!(class.is_some(), "class User should be indexed");
        assert_eq!(class.unwrap().qualified, "MyApp.Models.User");
    }

    #[test]
    fn test_constructor() {
        let source = r#"
namespace MyApp;

public class Service {
    public Service() { }
    public Service(string name) { }
}
"#;
        let parser = CSharpParser;
        let result = parser.extract_symbols(Path::new("Service.cs"), source, 100);

        let constructors: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.name == "Service" && s.kind == SymbolKind::Function)
            .collect();
        assert_eq!(constructors.len(), 2, "Should find 2 constructors");
    }

    #[test]
    fn test_property_with_getter_setter() {
        let source = r#"
namespace MyApp;

public class Config {
    public string Value { get; private set; }
    public int Count { get; }
}
"#;
        let parser = CSharpParser;
        let result = parser.extract_symbols(Path::new("Config.cs"), source, 100);

        let value = result.symbols.iter().find(|s| s.name == "Value");
        assert!(value.is_some(), "property Value should be indexed");
        assert_eq!(value.unwrap().kind, SymbolKind::Member);

        let count = result.symbols.iter().find(|s| s.name == "Count");
        assert!(count.is_some(), "property Count should be indexed");
    }

    #[test]
    fn test_event() {
        let source = r#"
namespace MyApp;

public class Publisher {
    public event EventHandler<string> OnMessage;
}
"#;
        let parser = CSharpParser;
        let result = parser.extract_symbols(Path::new("Publisher.cs"), source, 100);

        let event = result.symbols.iter().find(|s| s.name == "OnMessage");
        assert!(event.is_some(), "event OnMessage should be indexed");
        assert_eq!(event.unwrap().kind, SymbolKind::Member);
    }

    #[test]
    fn test_delegate() {
        let source = r#"
namespace MyApp;

public delegate void MessageHandler(string message);
"#;
        let parser = CSharpParser;
        let result = parser.extract_symbols(Path::new("Delegates.cs"), source, 100);

        let delegate = result.symbols.iter().find(|s| s.name == "MessageHandler");
        assert!(
            delegate.is_some(),
            "delegate MessageHandler should be indexed"
        );
        assert_eq!(delegate.unwrap().qualified, "MyApp.MessageHandler");
    }

    #[test]
    fn test_nested_class() {
        let source = r#"
namespace MyApp;

public class Outer {
    public class Inner {
        public void DoWork() { }
    }
}
"#;
        let parser = CSharpParser;
        let result = parser.extract_symbols(Path::new("Outer.cs"), source, 100);

        let outer = result.symbols.iter().find(|s| s.name == "Outer");
        assert!(outer.is_some(), "class Outer should be indexed");

        let inner = result.symbols.iter().find(|s| s.name == "Inner");
        assert!(inner.is_some(), "nested class Inner should be indexed");
        assert_eq!(inner.unwrap().qualified, "MyApp.Outer.Inner");

        let do_work = result.symbols.iter().find(|s| s.name == "DoWork");
        assert!(do_work.is_some(), "method DoWork should be indexed");
        assert_eq!(do_work.unwrap().qualified, "MyApp.Outer.Inner.DoWork");
    }

    #[test]
    fn test_partial_class() {
        let source = r#"
namespace MyApp;

public partial class Service {
    public void Method1() { }
}
"#;
        let parser = CSharpParser;
        let result = parser.extract_symbols(Path::new("Service.cs"), source, 100);

        let class = result
            .symbols
            .iter()
            .find(|s| s.name == "Service" && s.kind == SymbolKind::Class);
        assert!(class.is_some(), "partial class Service should be indexed");
    }

    #[test]
    fn test_extension_method() {
        let source = r#"
namespace MyApp.Extensions;

public static class StringExtensions {
    public static string Capitalize(this string str) {
        return str.ToUpper();
    }
}
"#;
        let parser = CSharpParser;
        let result = parser.extract_symbols(Path::new("StringExtensions.cs"), source, 100);

        let class = result.symbols.iter().find(|s| s.name == "StringExtensions");
        assert!(
            class.is_some(),
            "static class StringExtensions should be indexed"
        );

        let method = result.symbols.iter().find(|s| s.name == "Capitalize");
        assert!(
            method.is_some(),
            "extension method Capitalize should be indexed"
        );
        assert_eq!(
            method.unwrap().qualified,
            "MyApp.Extensions.StringExtensions.Capitalize"
        );
    }

    #[test]
    fn test_visibility_modifiers() {
        let source = r#"
namespace MyApp;

public class Example {
    public string PublicField;
    private string _privateField;
    protected string ProtectedField;
    internal string InternalField;
}
"#;
        let parser = CSharpParser;
        let result = parser.extract_symbols(Path::new("Example.cs"), source, 100);

        let public_field = result.symbols.iter().find(|s| s.name == "PublicField");
        assert!(public_field.is_some());
        assert_eq!(public_field.unwrap().visibility, Visibility::Public);

        let private_field = result.symbols.iter().find(|s| s.name == "_privateField");
        assert!(private_field.is_some());
        assert_eq!(private_field.unwrap().visibility, Visibility::Private);

        let protected_field = result.symbols.iter().find(|s| s.name == "ProtectedField");
        assert!(protected_field.is_some());
        // Protected maps to Internal since we don't have a Protected variant
        assert_eq!(protected_field.unwrap().visibility, Visibility::Internal);

        let internal_field = result.symbols.iter().find(|s| s.name == "InternalField");
        assert!(internal_field.is_some());
        assert_eq!(internal_field.unwrap().visibility, Visibility::Internal);
    }

    #[test]
    fn test_generic_class() {
        let source = r#"
namespace MyApp.Collections;

public class Repository<T> where T : class {
    public T Get(int id) { return default; }
    public void Save(T entity) { }
}
"#;
        let parser = CSharpParser;
        let result = parser.extract_symbols(Path::new("Repository.cs"), source, 100);

        let class = result.symbols.iter().find(|s| s.name == "Repository");
        assert!(
            class.is_some(),
            "generic class Repository should be indexed"
        );
        assert_eq!(class.unwrap().qualified, "MyApp.Collections.Repository");
    }

    #[test]
    fn test_file_scoped_namespace() {
        let source = r#"
namespace MyApp.Services;

public class UserService {
    public void Create() { }
}
"#;
        let parser = CSharpParser;
        let result = parser.extract_symbols(Path::new("UserService.cs"), source, 100);

        let class = result.symbols.iter().find(|s| s.name == "UserService");
        assert!(
            class.is_some(),
            "class in file-scoped namespace should be indexed"
        );
        assert_eq!(class.unwrap().qualified, "MyApp.Services.UserService");
    }

    #[test]
    fn test_indexer() {
        let source = r#"
namespace MyApp;

public class Collection {
    public string this[int index] {
        get { return ""; }
        set { }
    }
}
"#;
        let parser = CSharpParser;
        let result = parser.extract_symbols(Path::new("Collection.cs"), source, 100);

        let indexer = result.symbols.iter().find(|s| s.name == "this");
        assert!(indexer.is_some(), "indexer should be indexed");
        assert_eq!(indexer.unwrap().qualified, "MyApp.Collection.this");
    }

    #[test]
    fn test_no_namespace() {
        let source = r#"
public class GlobalClass {
    public void Method() { }
}
"#;
        let parser = CSharpParser;
        let result = parser.extract_symbols(Path::new("GlobalClass.cs"), source, 100);

        let class = result.symbols.iter().find(|s| s.name == "GlobalClass");
        assert!(class.is_some(), "class without namespace should be indexed");
        assert_eq!(class.unwrap().qualified, "GlobalClass");
    }

    #[test]
    fn extracts_doc_comments() {
        let source = r#"
namespace MyApp;

/// <summary>
/// Represents a user in the system.
/// </summary>
/// <remarks>
/// Users can have multiple roles.
/// </remarks>
public class User {
    public string Name { get; set; }
}

/// <summary>
/// A helper class for utilities.
/// </summary>
public static class Helper {
}
"#;
        let parser = CSharpParser;
        let result = parser.extract_symbols(Path::new("User.cs"), source, 100);

        // Class should have doc
        let user = result.symbols.iter().find(|s| s.name == "User").unwrap();
        assert!(user.doc.is_some(), "User class should have doc");
        assert!(
            user.doc.as_ref().unwrap().contains("<summary>"),
            "User doc should contain <summary>: {:?}",
            user.doc
        );

        // Helper should have doc
        let helper = result.symbols.iter().find(|s| s.name == "Helper").unwrap();
        assert!(helper.doc.is_some(), "Helper class should have doc");
    }

    #[test]
    fn extracts_using_directives() {
        let source = r#"
using System;
using System.Collections.Generic;
using MyApp.Models;

namespace MyApp;

public class Service {
    public void Run() { }
}
"#;
        let parser = CSharpParser;
        let result = parser.extract_symbols(Path::new("Service.cs"), source, 100);

        assert!(!result.opens.is_empty(), "Should extract using directives");
        assert!(
            result.opens.contains(&"System".to_string()),
            "Should have 'System' in opens: {:?}",
            result.opens
        );
        assert!(
            result
                .opens
                .contains(&"System.Collections.Generic".to_string()),
            "Should have 'System.Collections.Generic' in opens: {:?}",
            result.opens
        );
        assert!(
            result.opens.contains(&"MyApp.Models".to_string()),
            "Should have 'MyApp.Models' in opens: {:?}",
            result.opens
        );
    }

    #[test]
    fn extracts_csharp_references() {
        let source = r#"
namespace MyApp;

public class Main {
    public static void Run(string[] args) {
        User user = new User("Alice");
        string greeting = Helper.Greet(user);
        Console.WriteLine(greeting);
    }
}

class User {
    private string name;
    public User(string name) { this.name = name; }
    public string GetName() { return name; }
}

class Helper {
    public static string Greet(User user) {
        return "Hello, " + user.GetName();
    }
}
"#;
        let result = extract_symbols(Path::new("Main.cs"), source, 500);

        assert!(
            !result.references.is_empty(),
            "Should extract references from C# code"
        );

        let ref_names: Vec<_> = result.references.iter().map(|r| r.name.as_str()).collect();

        // Should have references to User (in Main.Run)
        assert!(
            ref_names.contains(&"User"),
            "Should have reference to User: {:?}",
            ref_names
        );

        // Should have references to Helper
        assert!(
            ref_names.contains(&"Helper"),
            "Should have reference to Helper: {:?}",
            ref_names
        );

        // Should have references to method calls
        assert!(
            ref_names.contains(&"Greet"),
            "Should have reference to Greet method: {:?}",
            ref_names
        );
        assert!(
            ref_names.contains(&"WriteLine"),
            "Should have reference to WriteLine method: {:?}",
            ref_names
        );
        assert!(
            ref_names.contains(&"GetName"),
            "Should have reference to GetName method: {:?}",
            ref_names
        );
    }

    #[test]
    fn extracts_method_call_references() {
        let source = r#"
namespace MyApp;

public class PolicyExample {
    public void Run() {
        var policy = new RetryPolicy();
        policy.Execute();
        policy.Execute("arg");
        policy.ExecuteAndCapture(() => DoWork());
        Helper.Process();
        Console.WriteLine("test");
        DoSomething();
        GenericMethod<int>();
    }

    private void DoWork() { }
    private void DoSomething() { }
    private void GenericMethod<T>() { }
}
"#;
        let parser = CSharpParser;
        let result = parser.extract_symbols(Path::new("PolicyExample.cs"), source, 100);

        let ref_names: Vec<_> = result.references.iter().map(|r| r.name.as_str()).collect();

        // Method calls on objects should be extracted
        assert!(
            ref_names.contains(&"Execute"),
            "Should have reference to Execute method call: {:?}",
            ref_names
        );
        assert!(
            ref_names.contains(&"ExecuteAndCapture"),
            "Should have reference to ExecuteAndCapture method call: {:?}",
            ref_names
        );

        // Static method calls should be extracted
        assert!(
            ref_names.contains(&"Process"),
            "Should have reference to Helper.Process method call: {:?}",
            ref_names
        );
        assert!(
            ref_names.contains(&"WriteLine"),
            "Should have reference to Console.WriteLine method call: {:?}",
            ref_names
        );

        // Direct method calls should be extracted
        assert!(
            ref_names.contains(&"DoWork"),
            "Should have reference to DoWork method call: {:?}",
            ref_names
        );
        assert!(
            ref_names.contains(&"DoSomething"),
            "Should have reference to DoSomething method call: {:?}",
            ref_names
        );

        // Generic method calls should be extracted
        assert!(
            ref_names.contains(&"GenericMethod"),
            "Should have reference to GenericMethod<T> call: {:?}",
            ref_names
        );
    }
}
