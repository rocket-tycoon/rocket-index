//! Symbol extraction from Ruby source files using tree-sitter.

use std::cell::RefCell;
use std::path::Path;

use crate::parse::{find_child_by_kind, node_to_location, LanguageParser, ParseResult};
use crate::{Symbol, SymbolKind, Visibility};

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
                        doc: None,
                        signature: None,
                    });

                    // Process children with this module context
                    // We need to find the body (usually 'body' field or children)
                    // In tree-sitter-ruby, class/module body is just children
                    for i in 0..node.child_count() {
                        if let Some(child) = node.child(i) {
                            if child.id() != name_node.id() {
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

        "method" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Ok(name) = name_node.utf8_text(source) {
                    // Method name in Ruby (def foo)
                    // Qualified name: Module::foo (instance) or Module.foo (class)
                    // For now, we use # for instance methods to distinguish
                    let separator = "#";
                    let qualified = match current_module {
                        Some(m) => format!("{}{}{}", m, separator, name),
                        None => name.to_string(),
                    };

                    result.symbols.push(Symbol {
                        name: name.to_string(),
                        qualified,
                        kind: SymbolKind::Function,
                        location: node_to_location(file, &name_node),
                        visibility: Visibility::Public, // TODO: Track visibility (public/private/protected)
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

        "singleton_method" => {
            // def self.foo
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Ok(name) = name_node.utf8_text(source) {
                    let qualified = match current_module {
                        Some(m) => format!("{}.{}", m, name),
                        None => name.to_string(),
                    };

                    result.symbols.push(Symbol {
                        name: name.to_string(),
                        qualified,
                        kind: SymbolKind::Function,
                        location: node_to_location(file, &name_node),
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

        "assignment" => {
            // Constant assignment: MAX_RETRIES = 5
            if let Some(left) = node.child_by_field_name("left") {
                if left.kind() == "constant" {
                    if let Ok(name) = left.utf8_text(source) {
                        let qualified = qualified_name(name, current_module);
                        result.symbols.push(Symbol {
                            name: name.to_string(),
                            qualified,
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
                    }
                }
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
                }
            }
        }

        _ => {}
    }

    // Recurse
    for i in 0..node.child_count() {
        if let Some(child) = node.child(i) {
            extract_recursive(&child, source, file, result, current_module, max_depth - 1);
        }
    }
}

fn qualified_name(name: &str, current_module: Option<&str>) -> String {
    match current_module {
        Some(m) => format!("{}::{}", m, name),
        None => name.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::extract_symbols;

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
}
