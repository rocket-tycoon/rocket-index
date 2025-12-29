//! Symbol extraction from Ruby source files using tree-sitter.

use std::cell::RefCell;
use std::path::Path;

use crate::parse::{find_child_by_kind, node_to_location, LanguageParser, ParseResult};
use crate::{Reference, Symbol, SymbolKind, Visibility};

// Thread-local parser reuse - avoids creating a new parser per file
thread_local! {
    static RUBY_PARSER: RefCell<tree_sitter::Parser> = RefCell::new({
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_ruby::LANGUAGE.into())
            .expect("tree-sitter-ruby grammar incompatible with tree-sitter version");
        parser
    });
}

pub struct RubyParser;

impl LanguageParser for RubyParser {
    fn extract_symbols(&self, file: &Path, source: &str, max_depth: usize) -> ParseResult {
        RUBY_PARSER.with(|parser| {
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

/// Extract doc comments (# style) preceding a node
/// Handles both RDoc and YARD style comments
fn extract_doc_comments(node: &tree_sitter::Node, source: &[u8]) -> Option<String> {
    let mut docs = Vec::new();

    let mut prev = node.prev_sibling();
    while let Some(sib) = prev {
        match sib.kind() {
            "comment" => {
                if let Ok(text) = sib.utf8_text(source) {
                    // Ruby comments start with #
                    let doc = text.trim_start_matches('#').trim();
                    if !doc.is_empty() {
                        docs.insert(0, doc.to_string());
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

/// Extract mixin modules (include/extend/prepend) from a class/module body
fn extract_mixins(node: &tree_sitter::Node, source: &[u8]) -> Vec<String> {
    let mut mixins = Vec::new();

    // Recursively search for include/extend/prepend calls in the class body
    fn collect_mixins(node: &tree_sitter::Node, source: &[u8], mixins: &mut Vec<String>) {
        if node.kind() == "call" {
            if let Some(method) = node.child_by_field_name("method") {
                if let Ok(name) = method.utf8_text(source) {
                    if name == "include" || name == "extend" || name == "prepend" {
                        // Get the module name from arguments
                        if let Some(args) = node.child_by_field_name("arguments") {
                            for i in 0..args.child_count() {
                                if let Some(arg) = args.child(i) {
                                    // Skip parentheses and commas
                                    if arg.kind() == "(" || arg.kind() == ")" || arg.kind() == "," {
                                        continue;
                                    }
                                    // Handle constant (module name) or scope_resolution
                                    if arg.kind() == "constant" || arg.kind() == "scope_resolution"
                                    {
                                        if let Ok(module_name) = arg.utf8_text(source) {
                                            mixins.push(module_name.to_string());
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // Recurse into children (but not too deep - just immediate class body)
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                // Don't recurse into nested classes/modules
                if child.kind() != "class" && child.kind() != "module" {
                    collect_mixins(&child, source, mixins);
                }
            }
        }
    }

    collect_mixins(node, source, &mut mixins);
    mixins
}

/// Extract method signature from a method node's parameters
fn extract_method_signature(node: &tree_sitter::Node, source: &[u8]) -> Option<String> {
    // Look for method_parameters or parameters child
    let params_node = node.child_by_field_name("parameters").or_else(|| {
        for i in 0..node.child_count() {
            if let Some(child) = node.child(i) {
                if child.kind() == "method_parameters" {
                    return Some(child);
                }
            }
        }
        None
    })?;

    // Get the full text of the parameters including parentheses
    if let Ok(params_text) = params_node.utf8_text(source) {
        let trimmed = params_text.trim();
        if trimmed.is_empty() {
            return None;
        }
        return Some(trimmed.to_string());
    }

    None
}

/// Current visibility state within a class/module body
#[derive(Clone, Copy, Default)]
enum VisibilityState {
    #[default]
    Public,
    Private,
    Protected,
}

impl From<VisibilityState> for Visibility {
    fn from(state: VisibilityState) -> Self {
        match state {
            VisibilityState::Public => Visibility::Public,
            VisibilityState::Private => Visibility::Private,
            VisibilityState::Protected => Visibility::Internal, // Ruby protected maps to Internal
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
    extract_recursive_inner(
        node,
        source,
        file,
        result,
        current_module,
        max_depth,
        false,
        VisibilityState::Public,
    );
}

#[allow(clippy::too_many_arguments)]
fn extract_recursive_inner(
    node: &tree_sitter::Node,
    source: &[u8],
    file: &Path,
    result: &mut ParseResult,
    current_module: Option<&str>,
    max_depth: usize,
    in_singleton_class: bool,
    visibility: VisibilityState,
) {
    if max_depth == 0 {
        return;
    }

    // Track visibility changes for methods
    let mut current_visibility = visibility;

    match node.kind() {
        "class" | "module" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Ok(name) = name_node.utf8_text(source) {
                    let qualified = qualified_name(name, current_module);
                    let kind = if node.kind() == "class" {
                        SymbolKind::Class
                    } else {
                        SymbolKind::Module
                    };

                    // Extract superclass if present (class Foo < Bar)
                    // The superclass field includes "< ClassName", so we trim the "< " prefix
                    let parent = if node.kind() == "class" {
                        node.child_by_field_name("superclass")
                            .and_then(|sc| sc.utf8_text(source).ok())
                            .map(|s| s.trim_start_matches('<').trim().to_string())
                    } else {
                        None
                    };

                    // Extract mixins (include/extend/prepend) from the class/module body
                    let mixins = extract_mixins(node, source);

                    // Extract doc comments preceding the class/module
                    let doc = extract_doc_comments(node, source);

                    result.symbols.push(Symbol {
                        name: name.to_string(),
                        qualified: qualified.clone(),
                        kind,
                        location: node_to_location(file, &name_node),
                        visibility: Visibility::Public,
                        language: "ruby".to_string(),
                        parent,
                        mixins: if mixins.is_empty() {
                            None
                        } else {
                            Some(mixins)
                        },
                        attributes: None,
                        implements: None,
                        doc,
                        signature: None,
                    });

                    // Process children with this module context
                    // In tree-sitter-ruby, class/module has a body_statement containing methods
                    // We need to find it and process visibility sequentially within it
                    if let Some(body) = find_child_by_kind(node, "body_statement") {
                        let mut class_visibility = VisibilityState::Public;
                        for i in 0..body.child_count() {
                            if let Some(child) = body.child(i) {
                                // Check for visibility modifiers
                                if child.kind() == "identifier" {
                                    let text = child.utf8_text(source).unwrap_or_default();
                                    match text {
                                        "private" => class_visibility = VisibilityState::Private,
                                        "protected" => {
                                            class_visibility = VisibilityState::Protected
                                        }
                                        "public" => class_visibility = VisibilityState::Public,
                                        _ => {}
                                    }
                                }
                                extract_recursive_inner(
                                    &child,
                                    source,
                                    file,
                                    result,
                                    Some(&qualified),
                                    max_depth - 1,
                                    false, // Reset singleton context for new class/module
                                    class_visibility,
                                );
                            }
                        }
                    } else {
                        // No body_statement, process children directly (shouldn't happen normally)
                        for i in 0..node.child_count() {
                            if let Some(child) = node.child(i) {
                                if child.id() != name_node.id() {
                                    extract_recursive_inner(
                                        &child,
                                        source,
                                        file,
                                        result,
                                        Some(&qualified),
                                        max_depth - 1,
                                        false,
                                        VisibilityState::Public,
                                    );
                                }
                            }
                        }
                    }
                    return;
                }
            }
        }

        "method" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Ok(name) = name_node.utf8_text(source) {
                    // Method name in Ruby (def foo)
                    // Qualified name: Module#foo (instance) or Module.foo (class)
                    // Use . for methods inside `class << self` blocks (singleton_class)
                    let separator = if in_singleton_class { "." } else { "#" };
                    let qualified = match current_module {
                        Some(m) => format!("{}{}{}", m, separator, name),
                        None => name.to_string(),
                    };

                    // Extract doc comments preceding the method
                    let doc = extract_doc_comments(node, source);

                    // Extract method signature (parameters)
                    let signature = extract_method_signature(node, source);

                    result.symbols.push(Symbol {
                        name: name.to_string(),
                        qualified,
                        kind: SymbolKind::Function,
                        location: node_to_location(file, &name_node),
                        visibility: current_visibility.into(),
                        language: "ruby".to_string(),
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

        // Visibility modifiers: private, protected, public (when called without args)
        // Also extract bare function call references (identifiers used as statements)
        "identifier" => {
            let text = node.utf8_text(source).unwrap_or_default();
            match text {
                "private" => current_visibility = VisibilityState::Private,
                "protected" => current_visibility = VisibilityState::Protected,
                "public" => current_visibility = VisibilityState::Public,
                _ => {
                    // Check if this identifier is a bare function call reference
                    // (standalone identifier in a statement context)
                    if is_bare_function_call(node, text) {
                        result.references.push(Reference {
                            name: text.to_string(),
                            location: node_to_location(file, node),
                        });
                    }
                }
            }
        }

        "singleton_class" => {
            // `class << self` block - methods inside are class methods
            // Don't emit a symbol for the singleton_class itself, just recurse with in_singleton_class=true
            let mut singleton_visibility = VisibilityState::Public;
            for i in 0..node.child_count() {
                if let Some(child) = node.child(i) {
                    // Check for visibility modifiers in this child
                    if child.kind() == "identifier" {
                        let text = child.utf8_text(source).unwrap_or_default();
                        match text {
                            "private" => singleton_visibility = VisibilityState::Private,
                            "protected" => singleton_visibility = VisibilityState::Protected,
                            "public" => singleton_visibility = VisibilityState::Public,
                            _ => {}
                        }
                    }
                    extract_recursive_inner(
                        &child,
                        source,
                        file,
                        result,
                        current_module,
                        max_depth - 1,
                        true, // We're now inside a singleton class
                        singleton_visibility,
                    );
                }
            }
            return;
        }

        "singleton_method" => {
            // def self.foo
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Ok(name) = name_node.utf8_text(source) {
                    let qualified = match current_module {
                        Some(m) => format!("{}.{}", m, name),
                        None => name.to_string(),
                    };

                    // Extract doc comments preceding the method
                    let doc = extract_doc_comments(node, source);

                    result.symbols.push(Symbol {
                        name: name.to_string(),
                        qualified,
                        kind: SymbolKind::Function,
                        location: node_to_location(file, &name_node),
                        visibility: current_visibility.into(),
                        language: "ruby".to_string(),
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

        "assignment" => {
            // Constant assignment: MAX_RETRIES = 5 or Point = Struct.new(:x, :y) { ... }
            if let Some(left) = node.child_by_field_name("left") {
                if left.kind() == "constant" {
                    if let Ok(name) = left.utf8_text(source) {
                        let qualified = qualified_name(name, current_module);
                        result.symbols.push(Symbol {
                            name: name.to_string(),
                            qualified: qualified.clone(),
                            kind: SymbolKind::Value,
                            location: node_to_location(file, &left),
                            visibility: Visibility::Public,
                            language: "ruby".to_string(),
                            parent: None,
                            mixins: None,
                            attributes: None,
                            implements: None,
                            doc: None,
                            signature: None,
                        });

                        // Check if this is a Struct.new or similar pattern with a block
                        // e.g., Point = Struct.new(:x, :y) do ... end
                        if let Some(right) = node.child_by_field_name("right") {
                            if right.kind() == "call" && is_struct_new_call(&right, source) {
                                // Find the do_block and process its contents with the new qualified name
                                for i in 0..right.child_count() {
                                    if let Some(child) = right.child(i) {
                                        if child.kind() == "do_block" || child.kind() == "block" {
                                            // Process the block with the struct name as context
                                            extract_recursive_inner(
                                                &child,
                                                source,
                                                file,
                                                result,
                                                Some(&qualified),
                                                max_depth - 1,
                                                false,
                                                VisibilityState::Public, // Struct blocks start with public
                                            );
                                        }
                                    }
                                }
                                return; // We've handled recursion ourselves
                            }
                        }
                    }
                }
            }
        }

        "alias" => {
            // `alias new_name old_name` - creates an alias for a method
            // First identifier child is the new name, second is the original
            let mut identifiers = Vec::new();
            for i in 0..node.child_count() {
                if let Some(child) = node.child(i) {
                    if child.kind() == "identifier" {
                        if let Ok(text) = child.utf8_text(source) {
                            identifiers.push((text.to_string(), child));
                        }
                    }
                }
            }

            if identifiers.len() >= 2 {
                let (alias_name, alias_node) = &identifiers[0];
                let separator = if in_singleton_class { "." } else { "#" };
                let qualified = match current_module {
                    Some(m) => format!("{}{}{}", m, separator, alias_name),
                    None => alias_name.to_string(),
                };

                result.symbols.push(Symbol {
                    name: alias_name.clone(),
                    qualified,
                    kind: SymbolKind::Function,
                    location: node_to_location(file, alias_node),
                    visibility: current_visibility.into(),
                    language: "ruby".to_string(),
                    parent: None,
                    mixins: None,
                    attributes: None,
                    implements: None,
                    doc: None,
                    signature: None,
                });
            }
        }

        "call" => {
            if let Some(method) = node.child_by_field_name("method") {
                if let Ok(name) = method.utf8_text(source) {
                    // Handle require
                    if name == "require" || name == "require_relative" {
                        // Get argument
                        if let Some(args) = node.child_by_field_name("arguments") {
                            // args is argument_list -> string/simple_symbol
                            if let Some(first_arg) = args.child(0) {
                                // argument_list children: ( arg )
                                // Skip opening parenthesis if present
                                let arg_node = if first_arg.kind() == "(" {
                                    args.child(1)
                                } else {
                                    Some(first_arg)
                                };

                                if let Some(arg) = arg_node {
                                    // Handle string literal
                                    if arg.kind() == "string" {
                                        if let Some(content) =
                                            find_child_by_kind(&arg, "string_content")
                                        {
                                            if let Ok(path) = content.utf8_text(source) {
                                                result.opens.push(path.to_string());
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    // Handle alias_method :new_name, :old_name
                    else if name == "alias_method" {
                        if let Some(args) = node.child_by_field_name("arguments") {
                            // First argument is the new alias name
                            for i in 0..args.child_count() {
                                if let Some(arg) = args.child(i) {
                                    let kind = arg.kind();
                                    if kind == "simple_symbol" || kind == "symbol" {
                                        if let Ok(sym_text) = arg.utf8_text(source) {
                                            let alias_name =
                                                sym_text.trim_start_matches(':').to_string();
                                            let separator =
                                                if in_singleton_class { "." } else { "#" };
                                            let qualified = match current_module {
                                                Some(m) => {
                                                    format!("{}{}{}", m, separator, alias_name)
                                                }
                                                None => alias_name.to_string(),
                                            };

                                            result.symbols.push(Symbol {
                                                name: alias_name,
                                                qualified,
                                                kind: SymbolKind::Function,
                                                location: node_to_location(file, &arg),
                                                visibility: Visibility::Public,
                                                language: "ruby".to_string(),
                                                parent: None,
                                                mixins: None,
                                                attributes: None,
                                                implements: None,
                                                doc: None,
                                                signature: None,
                                            });
                                            // Only take the first argument (the new alias name)
                                            break;
                                        }
                                    }
                                }
                            }
                        }
                    }
                    // Handle attributes
                    else if name == "attr_reader"
                        || name == "attr_writer"
                        || name == "attr_accessor"
                    {
                        if let Some(args) = node.child_by_field_name("arguments") {
                            // Iterate over all arguments
                            for i in 0..args.child_count() {
                                if let Some(arg) = args.child(i) {
                                    let kind = arg.kind();
                                    // Handle :symbol and "string"
                                    let attr_name = if kind == "simple_symbol" || kind == "symbol" {
                                        arg.utf8_text(source)
                                            .ok()
                                            .map(|s| s.trim_start_matches(':').to_string())
                                    } else if kind == "string" {
                                        find_child_by_kind(&arg, "string_content").and_then(|c| {
                                            c.utf8_text(source).ok().map(|s| s.to_string())
                                        })
                                    } else {
                                        None
                                    };

                                    if let Some(name) = attr_name {
                                        let separator = "#";
                                        let qualified = match current_module {
                                            Some(m) => format!("{}{}{}", m, separator, name),
                                            None => name.to_string(),
                                        };

                                        result.symbols.push(Symbol {
                                            name,
                                            qualified,
                                            kind: SymbolKind::Member,
                                            location: node_to_location(file, &arg),
                                            visibility: Visibility::Public,
                                            language: "ruby".to_string(),
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
                    // Handle Rails scope - creates class methods
                    // scope :active, -> { where(active: true) }
                    else if name == "scope" {
                        if let Some(args) = node.child_by_field_name("arguments") {
                            // First argument is the scope name
                            for i in 0..args.child_count() {
                                if let Some(arg) = args.child(i) {
                                    let kind = arg.kind();
                                    if kind == "simple_symbol" || kind == "symbol" {
                                        if let Ok(sym_text) = arg.utf8_text(source) {
                                            let scope_name =
                                                sym_text.trim_start_matches(':').to_string();
                                            // Scopes are class methods (User.active)
                                            let qualified = match current_module {
                                                Some(m) => format!("{}.{}", m, scope_name),
                                                None => scope_name.clone(),
                                            };

                                            result.symbols.push(Symbol {
                                                name: scope_name,
                                                qualified,
                                                kind: SymbolKind::Function,
                                                location: node_to_location(file, &arg),
                                                visibility: Visibility::Public,
                                                language: "ruby".to_string(),
                                                parent: None,
                                                mixins: None,
                                                attributes: None,
                                                implements: None,
                                                doc: None,
                                                signature: None,
                                            });
                                            // Only take the first symbol argument
                                            break;
                                        }
                                    }
                                }
                            }
                        }
                    }
                    // Handle delegate - creates instance methods
                    // delegate :name, :email, to: :profile
                    // delegate :company_name, to: :company, prefix: true
                    else if name == "delegate" {
                        if let Some(args) = node.child_by_field_name("arguments") {
                            let mut prefix: Option<String> = None;
                            let mut method_names: Vec<(String, tree_sitter::Node)> = Vec::new();

                            // First pass: collect method names and find prefix
                            for i in 0..args.child_count() {
                                if let Some(arg) = args.child(i) {
                                    let kind = arg.kind();
                                    if kind == "simple_symbol" || kind == "symbol" {
                                        if let Ok(sym_text) = arg.utf8_text(source) {
                                            let method_name =
                                                sym_text.trim_start_matches(':').to_string();
                                            method_names.push((method_name, arg));
                                        }
                                    } else if kind == "pair" {
                                        // Check for prefix: true or prefix: :custom
                                        if let Some(key) =
                                            find_child_by_kind(&arg, "hash_key_symbol")
                                        {
                                            if let Ok(key_text) = key.utf8_text(source) {
                                                if key_text == "prefix" {
                                                    // Get the value
                                                    if let Some(val) = arg.child(2) {
                                                        if val.kind() == "true" {
                                                            // prefix: true - use the target name
                                                            // Find "to:" pair to get target
                                                            for j in 0..args.child_count() {
                                                                if let Some(to_pair) = args.child(j)
                                                                {
                                                                    if to_pair.kind() == "pair" {
                                                                        if let Some(to_key) =
                                                                            find_child_by_kind(
                                                                                &to_pair,
                                                                                "hash_key_symbol",
                                                                            )
                                                                        {
                                                                            if let Ok(to_key_text) =
                                                                                to_key.utf8_text(
                                                                                    source,
                                                                                )
                                                                            {
                                                                                if to_key_text
                                                                                    == "to"
                                                                                {
                                                                                    if let Some(
                                                                                        to_val,
                                                                                    ) = to_pair
                                                                                        .child(2)
                                                                                    {
                                                                                        if let Ok(
                                                                                            to_text,
                                                                                        ) = to_val
                                                                                            .utf8_text(
                                                                                            source,
                                                                                        )
                                                                                        {
                                                                                            prefix = Some(to_text.trim_start_matches(':').to_string());
                                                                                        }
                                                                                    }
                                                                                }
                                                                            }
                                                                        }
                                                                    }
                                                                }
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }

                            // Second pass: create symbols for each method
                            for (method_name, arg_node) in method_names {
                                let final_name = if let Some(ref p) = prefix {
                                    format!("{}_{}", p, method_name)
                                } else {
                                    method_name
                                };

                                let qualified = match current_module {
                                    Some(m) => format!("{}#{}", m, final_name),
                                    None => final_name.clone(),
                                };

                                result.symbols.push(Symbol {
                                    name: final_name,
                                    qualified,
                                    kind: SymbolKind::Function,
                                    location: node_to_location(file, &arg_node),
                                    visibility: Visibility::Public,
                                    language: "ruby".to_string(),
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
                    // Handle define_method - creates instance methods dynamically
                    // define_method :custom_method do ... end
                    else if name == "define_method" {
                        if let Some(args) = node.child_by_field_name("arguments") {
                            // First argument is the method name
                            for i in 0..args.child_count() {
                                if let Some(arg) = args.child(i) {
                                    let kind = arg.kind();
                                    if kind == "simple_symbol" || kind == "symbol" {
                                        if let Ok(sym_text) = arg.utf8_text(source) {
                                            let method_name =
                                                sym_text.trim_start_matches(':').to_string();
                                            let separator =
                                                if in_singleton_class { "." } else { "#" };
                                            let qualified = match current_module {
                                                Some(m) => {
                                                    format!("{}{}{}", m, separator, method_name)
                                                }
                                                None => method_name.clone(),
                                            };

                                            result.symbols.push(Symbol {
                                                name: method_name,
                                                qualified,
                                                kind: SymbolKind::Function,
                                                location: node_to_location(file, &arg),
                                                visibility: current_visibility.into(),
                                                language: "ruby".to_string(),
                                                parent: None,
                                                mixins: None,
                                                attributes: None,
                                                implements: None,
                                                doc: None,
                                                signature: None,
                                            });
                                            // Only take the first symbol argument
                                            break;
                                        }
                                    }
                                }
                            }
                        }
                    }
                    // Handle Rails associations - creates instance methods
                    // has_many :posts, has_one :profile, belongs_to :user
                    else if name == "has_many"
                        || name == "has_one"
                        || name == "belongs_to"
                        || name == "has_and_belongs_to_many"
                    {
                        if let Some(args) = node.child_by_field_name("arguments") {
                            // First argument is the association name
                            for i in 0..args.child_count() {
                                if let Some(arg) = args.child(i) {
                                    let kind = arg.kind();
                                    if kind == "simple_symbol" || kind == "symbol" {
                                        if let Ok(sym_text) = arg.utf8_text(source) {
                                            let assoc_name =
                                                sym_text.trim_start_matches(':').to_string();
                                            // Associations are instance methods
                                            let qualified = match current_module {
                                                Some(m) => format!("{}#{}", m, assoc_name),
                                                None => assoc_name.clone(),
                                            };

                                            result.symbols.push(Symbol {
                                                name: assoc_name,
                                                qualified,
                                                kind: SymbolKind::Member,
                                                location: node_to_location(file, &arg),
                                                visibility: Visibility::Public,
                                                language: "ruby".to_string(),
                                                parent: None,
                                                mixins: None,
                                                attributes: None,
                                                implements: None,
                                                doc: None,
                                                signature: None,
                                            });
                                            // Only take the first symbol argument
                                            break;
                                        }
                                    }
                                }
                            }
                        }
                    }
                    // Handle Rails callbacks - creates references to methods
                    // before_action :authenticate_user!, after_action :log_access
                    else if name == "before_action"
                        || name == "after_action"
                        || name == "around_action"
                        || name == "before_filter"
                        || name == "after_filter"
                        || name == "around_filter"
                    {
                        if let Some(args) = node.child_by_field_name("arguments") {
                            for i in 0..args.child_count() {
                                if let Some(arg) = args.child(i) {
                                    let kind = arg.kind();
                                    if kind == "simple_symbol" || kind == "symbol" {
                                        if let Ok(sym_text) = arg.utf8_text(source) {
                                            let method_name = sym_text
                                                .trim_start_matches(':')
                                                .trim_end_matches('!')
                                                .to_string();
                                            // Callbacks reference existing methods
                                            result.references.push(Reference {
                                                name: method_name,
                                                location: node_to_location(file, &arg),
                                            });
                                            // Only take the first symbol (method name)
                                            break;
                                        }
                                    }
                                }
                            }
                        }
                    }
                    // Handle validate (custom validation method reference)
                    // validate :custom_validation
                    else if name == "validate" {
                        if let Some(args) = node.child_by_field_name("arguments") {
                            for i in 0..args.child_count() {
                                if let Some(arg) = args.child(i) {
                                    let kind = arg.kind();
                                    if kind == "simple_symbol" || kind == "symbol" {
                                        if let Ok(sym_text) = arg.utf8_text(source) {
                                            let method_name =
                                                sym_text.trim_start_matches(':').to_string();
                                            // validate references an existing method
                                            result.references.push(Reference {
                                                name: method_name,
                                                location: node_to_location(file, &arg),
                                            });
                                            // Only take the first symbol
                                            break;
                                        }
                                    }
                                }
                            }
                        }
                    }
                    // Generic method call reference
                    else {
                        let mut method_name = name.to_string();
                        // Try to qualify with receiver if it's a constant/module (e.g. User.find)
                        if let Some(receiver) = node.child_by_field_name("receiver") {
                            let kind = receiver.kind();
                            if kind == "constant" || kind == "scope_resolution" {
                                if let Ok(receiver_name) = receiver.utf8_text(source) {
                                    method_name = format!("{}.{}", receiver_name, name);
                                }
                            }
                        }

                        result.references.push(Reference {
                            name: method_name,
                            location: node_to_location(file, &method),
                        });
                    }
                }
            }
        }

        // Extract references from constants (class/module names used in code)
        "constant" => {
            if is_reference_context(node) {
                if let Ok(name) = node.utf8_text(source) {
                    result.references.push(Reference {
                        name: name.to_string(),
                        location: node_to_location(file, node),
                    });
                }
            }
        }

        // Extract references from scope resolutions (like Foo::Bar)
        "scope_resolution" => {
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

    // Recurse
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            extract_recursive_inner(
                &child,
                source,
                file,
                result,
                current_module,
                max_depth - 1,
                in_singleton_class,
                current_visibility,
            );
        }
    }
}

fn qualified_name(name: &str, current_module: Option<&str>) -> String {
    match current_module {
        Some(m) => format!("{}::{}", m, name),
        None => name.to_string(),
    }
}

/// Determine if a constant node is in a reference context (not a definition)
fn is_reference_context(node: &tree_sitter::Node) -> bool {
    let parent = match node.parent() {
        Some(p) => p,
        None => return false,
    };

    let parent_kind = parent.kind();

    // Definition contexts (NOT references)

    // Class definition: class Foo
    if parent_kind == "class" {
        if let Some(name_node) = parent.child_by_field_name("name") {
            if name_node.id() == node.id() {
                return false;
            }
        }
    }

    // Module definition: module Foo
    if parent_kind == "module" {
        if let Some(name_node) = parent.child_by_field_name("name") {
            if name_node.id() == node.id() {
                return false;
            }
        }
    }

    // Constant assignment: CONST = value
    if parent_kind == "assignment" {
        if let Some(left) = parent.child_by_field_name("left") {
            if left.id() == node.id() {
                return false;
            }
        }
    }

    // Superclass in class definition: class Foo < Bar
    // The superclass is actually a reference, so keep it as is

    true
}

/// Check if an identifier node is a bare function call (no parentheses)
/// In Ruby, `foo` can be either a local variable reference or a method call.
/// We treat standalone identifiers in statement context as potential function calls.
fn is_bare_function_call(node: &tree_sitter::Node, text: &str) -> bool {
    let parent = match node.parent() {
        Some(p) => p,
        None => return false,
    };

    let parent_kind = parent.kind();

    // Statement contexts where an identifier is likely a function call:
    // - body_statement (method body, class body)
    // - then, else (if/unless branches)
    // - do_block, block (block bodies)
    // - program (top-level)
    // - begin (begin/rescue blocks)
    // - ensure
    let is_statement_context = matches!(
        parent_kind,
        "body_statement" | "then" | "else" | "do_block" | "block" | "program" | "begin" | "ensure"
    );

    if !is_statement_context {
        return false;
    }

    // Exclude known keywords/builtins that aren't really function calls
    if matches!(
        text,
        "nil" | "true" | "false" | "self" | "super" | "__FILE__" | "__LINE__" | "__ENCODING__"
    ) {
        return false;
    }

    // Exclude if the identifier is the method name in a "call" parent
    // (though this shouldn't happen since call uses "method" field)
    if parent_kind == "call" {
        if let Some(method) = parent.child_by_field_name("method") {
            if method.id() == node.id() {
                return false;
            }
        }
    }

    // Exclude if this is the name of a method definition
    if parent_kind == "method" || parent_kind == "singleton_method" {
        if let Some(name) = parent.child_by_field_name("name") {
            if name.id() == node.id() {
                return false;
            }
        }
    }

    true
}

/// Check if a call node is Struct.new (or similar struct-creating calls)
fn is_struct_new_call(node: &tree_sitter::Node, source: &[u8]) -> bool {
    // Look for pattern: Struct.new or OpenStruct.new
    let mut has_struct_receiver = false;
    let mut has_new_method = false;

    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            if child.kind() == "constant" {
                if let Ok(text) = child.utf8_text(source) {
                    if text == "Struct" || text == "OpenStruct" {
                        has_struct_receiver = true;
                    }
                }
            }
            if child.kind() == "identifier" {
                if let Ok(text) = child.utf8_text(source) {
                    if text == "new" {
                        has_new_method = true;
                    }
                }
            }
        }
    }

    has_struct_receiver && has_new_method
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::extract_symbols;

    #[test]
    fn extracts_bare_function_call_references() {
        // Test that bare function calls (without parentheses) are extracted as references
        // Matches the actual fixture at tests/fixtures/minimal/ruby/main.rb
        let source = r#"
def helper
  42
end

def main_function
  x = helper
  puts x
end

def caller_a
  main_function
end

def caller_b
  main_function
  helper
end

class ChildClass < MyClass
  def method
    main_function
    super
  end
end
"#;
        let result = extract_symbols(Path::new("test.rb"), source, 500);

        let ref_names: Vec<_> = result.references.iter().map(|r| r.name.as_str()).collect();

        // Bare function calls should be extracted as references
        assert!(
            ref_names.contains(&"main_function"),
            "Should have reference to 'main_function', found: {:?}",
            ref_names
        );

        // Count occurrences - main_function is called 3 times
        // (in caller_a, caller_b, and ChildClass#method)
        let main_function_count = ref_names.iter().filter(|&&n| n == "main_function").count();
        assert!(
            main_function_count >= 3,
            "Should have at least 3 references to 'main_function', found: {}",
            main_function_count
        );
    }

    #[test]
    fn extracts_ruby_class() {
        let source = r#"
class User
  def initialize(name)
    @name = name
  end

  def name
    @name
  end
end
"#;
        let result = extract_symbols(Path::new("user.rb"), source, 500);

        // User, initialize, name
        let user = result.symbols.iter().find(|s| s.name == "User").unwrap();
        assert_eq!(user.kind, SymbolKind::Class);
        assert_eq!(user.language, "ruby");

        let init = result
            .symbols
            .iter()
            .find(|s| s.name == "initialize")
            .unwrap();
        assert_eq!(init.kind, SymbolKind::Function);
        assert_eq!(init.qualified, "User#initialize");

        let name = result.symbols.iter().find(|s| s.name == "name").unwrap();
        assert_eq!(name.kind, SymbolKind::Function);
        assert_eq!(name.qualified, "User#name");
    }

    #[test]
    fn extracts_ruby_module() {
        let source = r#"
module MyApp
  module Utils
    def self.helper
    end
  end
end
"#;
        let result = extract_symbols(Path::new("utils.rb"), source, 500);

        let myapp = result.symbols.iter().find(|s| s.name == "MyApp").unwrap();
        assert_eq!(myapp.kind, SymbolKind::Module);

        let utils = result.symbols.iter().find(|s| s.name == "Utils").unwrap();
        assert_eq!(utils.qualified, "MyApp::Utils");

        let helper = result.symbols.iter().find(|s| s.name == "helper").unwrap();
        assert_eq!(helper.qualified, "MyApp::Utils.helper");
    }

    #[test]
    fn extracts_ruby_require() {
        let source = r#"
require 'json'
require_relative 'user'
"#;
        let result = extract_symbols(Path::new("main.rb"), source, 500);

        assert!(result.opens.contains(&"json".to_string()));
        assert!(result.opens.contains(&"user".to_string()));
    }

    #[test]
    fn extracts_ruby_constants() {
        let source = r#"
MAX_RETRIES = 5
DEFAULT_CONFIG = {
  timeout: 10
}

class User
  STATUS_ACTIVE = 'active'
end
"#;
        let result = extract_symbols(Path::new("constants.rb"), source, 500);

        let max_retries = result
            .symbols
            .iter()
            .find(|s| s.name == "MAX_RETRIES")
            .unwrap();
        assert_eq!(max_retries.kind, SymbolKind::Value);
        assert_eq!(max_retries.qualified, "MAX_RETRIES");

        let status = result
            .symbols
            .iter()
            .find(|s| s.name == "STATUS_ACTIVE")
            .unwrap();
        assert_eq!(status.kind, SymbolKind::Value);
        assert_eq!(status.qualified, "User::STATUS_ACTIVE");
    }

    #[test]
    fn extracts_ruby_attributes() {
        let source = r#"
class User
  attr_reader :name
  attr_accessor :email, :age
  attr_writer :password
end
"#;
        let result = extract_symbols(Path::new("user.rb"), source, 500);

        let name = result.symbols.iter().find(|s| s.name == "name").unwrap();
        assert_eq!(name.kind, SymbolKind::Member);
        assert_eq!(name.qualified, "User#name");

        let email = result.symbols.iter().find(|s| s.name == "email").unwrap();
        assert_eq!(email.kind, SymbolKind::Member);
        assert_eq!(email.qualified, "User#email");

        let age = result.symbols.iter().find(|s| s.name == "age").unwrap();
        assert_eq!(age.kind, SymbolKind::Member);

        let password = result
            .symbols
            .iter()
            .find(|s| s.name == "password")
            .unwrap();
        assert_eq!(password.kind, SymbolKind::Member);
    }

    #[test]
    fn extracts_ruby_mixins() {
        let source = r#"
class User
  include Comparable
  include ActiveModel::Validations
  extend ClassMethods
  prepend Logging
end

module Service
  include Enumerable
end
"#;
        let result = extract_symbols(Path::new("user.rb"), source, 500);

        let user = result.symbols.iter().find(|s| s.name == "User").unwrap();
        assert_eq!(user.kind, SymbolKind::Class);

        // Check mixins are captured
        let mixins = user.mixins.as_ref().expect("User should have mixins");
        assert!(mixins.contains(&"Comparable".to_string()));
        assert!(mixins.contains(&"ActiveModel::Validations".to_string()));
        assert!(mixins.contains(&"ClassMethods".to_string()));
        assert!(mixins.contains(&"Logging".to_string()));

        let service = result.symbols.iter().find(|s| s.name == "Service").unwrap();
        let svc_mixins = service.mixins.as_ref().expect("Service should have mixins");
        assert!(svc_mixins.contains(&"Enumerable".to_string()));
    }

    #[test]
    fn extracts_ruby_superclass() {
        let source = r#"
class Admin < User
  def admin?
    true
  end
end

class Guest < User
end

module MyApp
  class ApiClient < Common::Client::Base
  end
end
"#;
        let result = extract_symbols(Path::new("admin.rb"), source, 500);

        let admin = result.symbols.iter().find(|s| s.name == "Admin").unwrap();
        assert_eq!(admin.kind, SymbolKind::Class);
        assert_eq!(admin.parent.as_deref(), Some("User"));

        let guest = result.symbols.iter().find(|s| s.name == "Guest").unwrap();
        assert_eq!(guest.parent.as_deref(), Some("User"));

        let api_client = result
            .symbols
            .iter()
            .find(|s| s.name == "ApiClient")
            .unwrap();
        assert_eq!(api_client.parent.as_deref(), Some("Common::Client::Base"));
    }

    // ============================================================
    // QUIRK TESTS: These test known indexing quirks/gaps
    // ============================================================

    #[test]
    fn extracts_class_self_block_methods_as_class_methods() {
        // QUIRK: Methods defined in `class << self` blocks should be indexed
        // as class methods (using `.` separator) not instance methods (`#`)
        let source = r#"
class User
  class << self
    def find_by_email(email)
      # class method
    end

    def create_default
      # another class method
    end
  end

  def instance_method
    # regular instance method
  end
end
"#;
        let result = extract_symbols(Path::new("user.rb"), source, 500);

        // Instance method should use # separator
        let instance_method = result
            .symbols
            .iter()
            .find(|s| s.name == "instance_method")
            .unwrap();
        assert_eq!(instance_method.qualified, "User#instance_method");

        // Class methods from `class << self` should use . separator
        let find_by_email = result
            .symbols
            .iter()
            .find(|s| s.name == "find_by_email")
            .unwrap();
        assert_eq!(
            find_by_email.qualified, "User.find_by_email",
            "class << self methods should be indexed as class methods with . separator"
        );

        let create_default = result
            .symbols
            .iter()
            .find(|s| s.name == "create_default")
            .unwrap();
        assert_eq!(
            create_default.qualified, "User.create_default",
            "class << self methods should be indexed as class methods with . separator"
        );
    }

    #[test]
    fn extracts_method_aliases() {
        // QUIRK: Method aliases created via `alias` or `alias_method` are not indexed
        let source = r#"
class User
  def full_name
    first_name + " " + last_name
  end

  alias name full_name
  alias_method :display_name, :full_name

  def to_s
    full_name
  end

  alias inspect to_s
end
"#;
        let result = extract_symbols(Path::new("user.rb"), source, 500);

        // Original methods should be indexed
        let full_name = result
            .symbols
            .iter()
            .find(|s| s.name == "full_name")
            .unwrap();
        assert_eq!(full_name.qualified, "User#full_name");

        let to_s = result.symbols.iter().find(|s| s.name == "to_s").unwrap();
        assert_eq!(to_s.qualified, "User#to_s");

        // Aliases should also be indexed
        let name_alias = result.symbols.iter().find(|s| s.name == "name");
        assert!(
            name_alias.is_some(),
            "alias name should be indexed as User#name"
        );
        assert_eq!(name_alias.unwrap().qualified, "User#name");

        let display_name_alias = result.symbols.iter().find(|s| s.name == "display_name");
        assert!(
            display_name_alias.is_some(),
            "alias_method :display_name should be indexed as User#display_name"
        );
        assert_eq!(display_name_alias.unwrap().qualified, "User#display_name");

        let inspect_alias = result.symbols.iter().find(|s| s.name == "inspect");
        assert!(
            inspect_alias.is_some(),
            "alias inspect should be indexed as User#inspect"
        );
        assert_eq!(inspect_alias.unwrap().qualified, "User#inspect");
    }

    #[test]
    fn extracts_struct_block_methods_with_correct_parent() {
        // QUIRK: Methods defined inside `Struct.new { ... }` blocks are indexed
        // with the wrong parent (outer module instead of the struct)
        let source = r#"
module MyApp
  Point = Struct.new(:x, :y) do
    def distance_from_origin
      Math.sqrt(x**2 + y**2)
    end

    def to_s
      "(#{x}, #{y})"
    end
  end

  class Calculator
    def calculate
    end
  end
end
"#;
        let result = extract_symbols(Path::new("point.rb"), source, 500);

        // The Point constant should be indexed
        let point = result.symbols.iter().find(|s| s.name == "Point");
        assert!(point.is_some(), "Point struct should be indexed");

        // Methods inside the struct block should have Point as their parent
        let distance = result
            .symbols
            .iter()
            .find(|s| s.name == "distance_from_origin");
        assert!(
            distance.is_some(),
            "distance_from_origin method should be indexed"
        );
        assert_eq!(
            distance.unwrap().qualified,
            "MyApp::Point#distance_from_origin",
            "Struct.new block methods should have the struct as parent"
        );

        let to_s = result
            .symbols
            .iter()
            .find(|s| s.name == "to_s" && s.qualified.contains("Point"));
        assert!(
            to_s.is_some(),
            "to_s method should be indexed with Point as parent"
        );
        assert_eq!(
            to_s.unwrap().qualified,
            "MyApp::Point#to_s",
            "Struct.new block methods should have the struct as parent"
        );

        // Regular class methods should still work
        let calculate = result
            .symbols
            .iter()
            .find(|s| s.name == "calculate")
            .unwrap();
        assert_eq!(calculate.qualified, "MyApp::Calculator#calculate");
    }

    // ============================================================
    // PRODUCTION READINESS TESTS: Rails/SRE patterns
    // ============================================================

    #[test]
    fn extracts_rails_scope_definitions() {
        // Rails scopes are common in ActiveRecord models
        let source = r#"
class User < ApplicationRecord
  scope :active, -> { where(active: true) }
  scope :recent, ->(days) { where("created_at > ?", days.ago) }
  scope :admins, -> { where(role: 'admin') }
end
"#;
        let result = extract_symbols(Path::new("user.rb"), source, 500);

        let active_scope = result.symbols.iter().find(|s| s.name == "active");
        assert!(
            active_scope.is_some(),
            "Rails scope :active should be indexed"
        );
        assert_eq!(active_scope.unwrap().qualified, "User.active");
        assert_eq!(active_scope.unwrap().kind, SymbolKind::Function);

        let recent_scope = result.symbols.iter().find(|s| s.name == "recent");
        assert!(
            recent_scope.is_some(),
            "Rails scope :recent should be indexed"
        );
        assert_eq!(recent_scope.unwrap().qualified, "User.recent");

        let admins_scope = result.symbols.iter().find(|s| s.name == "admins");
        assert!(
            admins_scope.is_some(),
            "Rails scope :admins should be indexed"
        );
        assert_eq!(admins_scope.unwrap().qualified, "User.admins");
    }

    #[test]
    fn extracts_delegate_methods() {
        // delegate is common in Rails for composition
        let source = r#"
class User
  delegate :name, :email, to: :profile
  delegate :company_name, to: :company, prefix: true
  delegate :admin?, to: :role, allow_nil: true
end
"#;
        let result = extract_symbols(Path::new("user.rb"), source, 500);

        // delegate creates methods that forward to another object
        let name_delegate = result.symbols.iter().find(|s| s.name == "name");
        assert!(
            name_delegate.is_some(),
            "delegate :name should be indexed as User#name"
        );
        assert_eq!(name_delegate.unwrap().qualified, "User#name");

        let email_delegate = result.symbols.iter().find(|s| s.name == "email");
        assert!(
            email_delegate.is_some(),
            "delegate :email should be indexed as User#email"
        );

        // prefix: true creates company_company_name
        let company_name = result
            .symbols
            .iter()
            .find(|s| s.name == "company_company_name");
        assert!(
            company_name.is_some(),
            "delegate with prefix should create prefixed method"
        );

        let admin = result.symbols.iter().find(|s| s.name == "admin?");
        assert!(admin.is_some(), "delegate :admin? should be indexed");
    }

    #[test]
    fn extracts_module_function() {
        // module_function makes methods callable both ways
        let source = r#"
module Utils
  module_function

  def format_date(date)
    date.strftime("%Y-%m-%d")
  end

  def format_time(time)
    time.strftime("%H:%M:%S")
  end
end
"#;
        let result = extract_symbols(Path::new("utils.rb"), source, 500);

        // module_function methods should be indexed as both instance and class methods
        // At minimum, they should be found
        let format_date = result.symbols.iter().find(|s| s.name == "format_date");
        assert!(
            format_date.is_some(),
            "module_function methods should be indexed"
        );

        let format_time = result.symbols.iter().find(|s| s.name == "format_time");
        assert!(
            format_time.is_some(),
            "module_function methods should be indexed"
        );
    }

    #[test]
    fn tracks_private_visibility() {
        // Visibility modifiers are important for understanding API surface
        let source = r#"
class User
  def public_method
  end

  private

  def private_helper
  end

  def another_private
  end

  protected

  def protected_method
  end

  public

  def back_to_public
  end
end
"#;
        let result = extract_symbols(Path::new("user.rb"), source, 500);

        let public_method = result
            .symbols
            .iter()
            .find(|s| s.name == "public_method")
            .unwrap();
        assert_eq!(
            public_method.visibility,
            Visibility::Public,
            "Methods before 'private' should be public"
        );

        let private_helper = result
            .symbols
            .iter()
            .find(|s| s.name == "private_helper")
            .unwrap();
        assert_eq!(
            private_helper.visibility,
            Visibility::Private,
            "Methods after 'private' should be private"
        );

        let another_private = result
            .symbols
            .iter()
            .find(|s| s.name == "another_private")
            .unwrap();
        assert_eq!(
            another_private.visibility,
            Visibility::Private,
            "Methods after 'private' should remain private"
        );

        let protected_method = result
            .symbols
            .iter()
            .find(|s| s.name == "protected_method")
            .unwrap();
        assert_eq!(
            protected_method.visibility,
            Visibility::Internal, // Using Internal for protected
            "Methods after 'protected' should be protected"
        );

        let back_to_public = result
            .symbols
            .iter()
            .find(|s| s.name == "back_to_public")
            .unwrap();
        assert_eq!(
            back_to_public.visibility,
            Visibility::Public,
            "Methods after 'public' should be public again"
        );
    }

    #[test]
    fn extracts_class_methods_via_define_method() {
        // define_method is used for dynamic method definition
        // Note: Dynamic interpolation (#{role}) can't be indexed statically
        // but static symbol args can be
        let source = r#"
class User
  define_method :custom_method do
    "custom"
  end

  define_method :another_method do |arg|
    arg.to_s
  end
end
"#;
        let result = extract_symbols(Path::new("user.rb"), source, 500);

        // Static define_method calls should be indexed
        let custom_method = result.symbols.iter().find(|s| s.name == "custom_method");
        assert!(
            custom_method.is_some(),
            "define_method :custom_method should be indexed"
        );
        assert_eq!(custom_method.unwrap().qualified, "User#custom_method");

        let another_method = result.symbols.iter().find(|s| s.name == "another_method");
        assert!(
            another_method.is_some(),
            "define_method :another_method should be indexed"
        );
    }

    #[test]
    fn extracts_doc_comments() {
        // Test top-level class/module doc comments
        let source = r#"
# A user representation
# @author Team
class User
end

# A helper module
module Helper
end
"#;
        let result = extract_symbols(Path::new("test.rb"), source, 500);

        // Class should have doc
        let user = result.symbols.iter().find(|s| s.name == "User").unwrap();
        assert!(user.doc.is_some(), "User class should have doc");
        assert!(
            user.doc.as_ref().unwrap().contains("user representation"),
            "User doc should contain 'user representation'"
        );

        // Module should have doc
        let helper = result.symbols.iter().find(|s| s.name == "Helper").unwrap();
        assert!(helper.doc.is_some(), "Helper module should have doc");
    }

    #[test]
    fn extracts_doc_comments_for_methods() {
        // Test top-level method doc comments (methods in class bodies are handled separately)
        let source = r#"
# Format the greeting
# @param name [String] name to greet
def greet(name)
  "Hello, #{name}!"
end
"#;
        let result = extract_symbols(Path::new("test.rb"), source, 500);

        // Top-level method should have doc
        let greet = result.symbols.iter().find(|s| s.name == "greet").unwrap();
        assert!(greet.doc.is_some(), "greet method should have doc");
        assert!(
            greet.doc.as_ref().unwrap().contains("Format the greeting"),
            "greet doc should contain 'Format the greeting'"
        );
    }

    #[test]
    fn extracts_ruby_references() {
        let source = r#"
class User
  def initialize(name)
    @name = name
  end

  def greet
    Helper.format_greeting(@name)
  end
end

class Helper
  def self.format_greeting(name)
    "Hello, #{name}!"
  end
end

def main
  user = User.new("Alice")
  puts user.greet
end
"#;
        let result = extract_symbols(Path::new("test.rb"), source, 500);

        assert!(
            !result.references.is_empty(),
            "Should extract references from Ruby code"
        );

        let ref_names: Vec<_> = result.references.iter().map(|r| r.name.as_str()).collect();

        // Should have references to User (in main)
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
    }

    // ============================================================
    // METHOD SIGNATURE TESTS (RocketIndex-0s6)
    // ============================================================

    #[test]
    fn extracts_method_signatures_basic() {
        let source = r#"
class User
  def greet(name, age)
    "Hello #{name}, age #{age}"
  end
end
"#;
        let result = extract_symbols(Path::new("test.rb"), source, 500);

        let greet = result.symbols.iter().find(|s| s.name == "greet").unwrap();
        assert!(
            greet.signature.is_some(),
            "Method should have signature extracted"
        );
        let sig = greet.signature.as_ref().unwrap();
        assert!(
            sig.contains("name") && sig.contains("age"),
            "Signature should contain parameter names: {}",
            sig
        );
    }

    #[test]
    fn extracts_method_signatures_with_defaults_and_splat() {
        let source = r#"
class Api
  def call(url, method = :get, *args, **kwargs, &block)
    # implementation
  end
end
"#;
        let result = extract_symbols(Path::new("test.rb"), source, 500);

        let call = result.symbols.iter().find(|s| s.name == "call").unwrap();
        assert!(
            call.signature.is_some(),
            "Method with complex params should have signature"
        );
        let sig = call.signature.as_ref().unwrap();
        // Should capture various parameter types
        assert!(sig.contains("url"), "Should have url param: {}", sig);
        assert!(
            sig.contains("method") || sig.contains(":get"),
            "Should have method param with default: {}",
            sig
        );
    }

    // ============================================================
    // RAILS DSL TESTS (RocketIndex-w3s)
    // ============================================================

    #[test]
    fn extracts_rails_has_many_association() {
        let source = r#"
class User < ApplicationRecord
  has_many :posts
  has_many :comments, dependent: :destroy
  has_one :profile
end
"#;
        let result = extract_symbols(Path::new("user.rb"), source, 500);

        // has_many :posts creates User#posts, User#posts=, User#post_ids, etc.
        // At minimum we should index the association name as a method
        let posts = result.symbols.iter().find(|s| s.name == "posts");
        assert!(
            posts.is_some(),
            "has_many :posts should create a 'posts' symbol. Found: {:?}",
            result.symbols.iter().map(|s| &s.name).collect::<Vec<_>>()
        );

        let comments = result.symbols.iter().find(|s| s.name == "comments");
        assert!(
            comments.is_some(),
            "has_many :comments should create a 'comments' symbol"
        );

        let profile = result.symbols.iter().find(|s| s.name == "profile");
        assert!(
            profile.is_some(),
            "has_one :profile should create a 'profile' symbol"
        );
    }

    #[test]
    fn extracts_rails_belongs_to_association() {
        let source = r#"
class Post < ApplicationRecord
  belongs_to :user
  belongs_to :category, optional: true
end
"#;
        let result = extract_symbols(Path::new("post.rb"), source, 500);

        let user = result.symbols.iter().find(|s| s.name == "user");
        assert!(
            user.is_some(),
            "belongs_to :user should create a 'user' symbol. Found: {:?}",
            result.symbols.iter().map(|s| &s.name).collect::<Vec<_>>()
        );

        let category = result.symbols.iter().find(|s| s.name == "category");
        assert!(
            category.is_some(),
            "belongs_to :category should create a 'category' symbol"
        );
    }

    #[test]
    fn extracts_rails_callbacks() {
        let source = r#"
class PostsController < ApplicationController
  before_action :authenticate_user!
  before_action :set_post, only: [:show, :edit, :update, :destroy]
  after_action :log_access
end
"#;
        let result = extract_symbols(Path::new("posts_controller.rb"), source, 500);

        // Callbacks should create references to the methods, not new symbols
        // But we could also index them as function references
        let ref_names: Vec<_> = result.references.iter().map(|r| &r.name).collect();

        // At minimum, we should have references to the callback methods
        assert!(
            ref_names.iter().any(|n| n.contains("authenticate_user"))
                || result
                    .symbols
                    .iter()
                    .any(|s| s.name.contains("authenticate_user")),
            "before_action :authenticate_user! should create a reference. Refs: {:?}, Symbols: {:?}",
            ref_names,
            result.symbols.iter().map(|s| &s.name).collect::<Vec<_>>()
        );
    }

    #[test]
    fn extracts_rails_validates() {
        let source = r#"
class User < ApplicationRecord
  validates :email, presence: true, uniqueness: true
  validates :name, length: { minimum: 2 }
  validate :custom_validation
end
"#;
        let result = extract_symbols(Path::new("user.rb"), source, 500);

        // validate :custom_validation should create a reference to the method
        let ref_names: Vec<_> = result.references.iter().map(|r| &r.name).collect();

        assert!(
            ref_names.iter().any(|n| n.contains("custom_validation"))
                || result
                    .symbols
                    .iter()
                    .any(|s| s.name.contains("custom_validation")),
            "validate :custom_validation should create a reference. Refs: {:?}",
            ref_names
        );
    }

    #[test]
    fn extracts_method_call_references() {
        let source = r#"
class Service
  def perform
    UserVerification.find_by_type(service_name, user_verification_identifier)
  end
end
"#;
        let result = extract_symbols(Path::new("service.rb"), source, 500);

        let types: Vec<_> = result.references.iter().map(|r| r.name.as_str()).collect();
        // This is expected to fail currently reference extraction isn't implemented for general calls
        assert!(
            types.contains(&"UserVerification.find_by_type") || types.contains(&"find_by_type"),
            "Should contain reference to 'find_by_type' or qualified 'UserVerification.find_by_type', found: {:?}",
            types
        );
        // UserVerification should be found as a constant reference
        assert!(
            types.contains(&"UserVerification"),
            "Should contain reference to 'UserVerification', found: {:?}",
            types
        );
    }
}
