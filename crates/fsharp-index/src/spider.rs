//! Spider: Dependency graph traversal from an entry point.
//!
//! The spider crawls from a starting symbol, following references to build
//! a dependency graph. This is useful for understanding code flow and
//! identifying which symbols are reachable from a given entry point.

use std::collections::{HashSet, VecDeque};
use std::path::Path;

use crate::{CodeIndex, Symbol};

/// A node in the spider's dependency graph.
#[derive(Debug, Clone)]
pub struct SpiderNode {
    /// The symbol at this node
    pub symbol: Symbol,
    /// Depth from the entry point (0 = entry point itself)
    pub depth: usize,
}

/// Result of spidering from an entry point.
#[derive(Debug, Default)]
pub struct SpiderResult {
    /// Nodes visited in breadth-first order
    pub nodes: Vec<SpiderNode>,
    /// Symbols that couldn't be resolved (external or undefined)
    pub unresolved: Vec<String>,
}

impl SpiderResult {
    /// Create a new empty spider result.
    pub fn new() -> Self {
        Self::default()
    }

    /// Get all unique files touched by the spider.
    pub fn files(&self) -> HashSet<&Path> {
        self.nodes
            .iter()
            .map(|n| n.symbol.location.file.as_path())
            .collect()
    }

    /// Get nodes at a specific depth.
    pub fn at_depth(&self, depth: usize) -> Vec<&SpiderNode> {
        self.nodes.iter().filter(|n| n.depth == depth).collect()
    }
}

/// Spider from an entry point symbol, following references up to a maximum depth.
///
/// # Arguments
/// * `index` - The code index to search
/// * `entry_point` - The qualified name of the starting symbol
/// * `max_depth` - Maximum depth to traverse (0 = only entry point)
///
/// # Returns
/// A `SpiderResult` containing all reachable symbols in breadth-first order.
pub fn spider(index: &CodeIndex, entry_point: &str, max_depth: usize) -> SpiderResult {
    let mut result = SpiderResult::new();
    let mut visited: HashSet<String> = HashSet::new();
    let mut queue: VecDeque<(String, usize)> = VecDeque::new();

    // Start with the entry point
    queue.push_back((entry_point.to_string(), 0));

    while let Some((qualified_name, depth)) = queue.pop_front() {
        // Skip if already visited
        if visited.contains(&qualified_name) {
            continue;
        }
        visited.insert(qualified_name.clone());

        // Try to find the symbol
        match index.get(&qualified_name) {
            Some(symbol) => {
                result.nodes.push(SpiderNode {
                    symbol: symbol.clone(),
                    depth,
                });

                // Don't follow references beyond max depth
                if depth >= max_depth {
                    continue;
                }

                // Get references from the file where this symbol is defined
                let references = index.references_in_file(&symbol.location.file);
                let opens = index.opens_for_file(&symbol.location.file);

                // Try to resolve each reference and add to queue
                for reference in references {
                    if let Some(resolved) = try_resolve_reference(index, &reference.name, opens) {
                        if !visited.contains(&resolved) {
                            queue.push_back((resolved, depth + 1));
                        }
                    } else {
                        // Track unresolved references
                        if !result.unresolved.contains(&reference.name) {
                            result.unresolved.push(reference.name.clone());
                        }
                    }
                }
            }
            None => {
                // Entry point or reference not found
                if depth == 0 {
                    // Entry point not found - this is significant
                    result.unresolved.push(qualified_name);
                }
            }
        }
    }

    result
}

/// Try to resolve a reference name to a qualified symbol name.
///
/// This attempts resolution in order:
/// 1. Direct match (already qualified)
/// 2. Via open statements
/// 3. Partial match on the name
fn try_resolve_reference(index: &CodeIndex, name: &str, opens: &[String]) -> Option<String> {
    // Try direct match first
    if index.get(name).is_some() {
        return Some(name.to_string());
    }

    // Try with each open statement
    for open in opens {
        let qualified = format!("{}.{}", open, name);
        if index.get(&qualified).is_some() {
            return Some(qualified);
        }
    }

    // Try partial match - look for symbols ending with this name
    // This handles cases like "List.map" where we need to find "Microsoft.FSharp.Collections.List.map"
    let search_results = index.search(name);
    if let Some(first_match) = search_results.first() {
        return Some(first_match.qualified.clone());
    }

    None
}

/// Spider from a file's entry points (top-level definitions).
///
/// This finds all top-level symbols in the file and spiders from each.
pub fn spider_from_file(index: &CodeIndex, file: &Path, max_depth: usize) -> SpiderResult {
    let mut combined_result = SpiderResult::new();
    let mut visited: HashSet<String> = HashSet::new();

    for symbol in index.symbols_in_file(file) {
        if visited.contains(&symbol.qualified) {
            continue;
        }

        let sub_result = spider(index, &symbol.qualified, max_depth);

        for node in sub_result.nodes {
            if !visited.contains(&node.symbol.qualified) {
                visited.insert(node.symbol.qualified.clone());
                combined_result.nodes.push(node);
            }
        }

        for unresolved in sub_result.unresolved {
            if !combined_result.unresolved.contains(&unresolved) {
                combined_result.unresolved.push(unresolved);
            }
        }
    }

    combined_result
}

/// Format spider result for display.
pub fn format_spider_result(result: &SpiderResult) -> String {
    let mut output = String::new();

    for node in &result.nodes {
        let loc = &node.symbol.location;
        let indent = "  ".repeat(node.depth);
        output.push_str(&format!(
            "{}{}:{}:{} {}\n",
            indent,
            loc.file.display(),
            loc.line,
            loc.column,
            node.symbol.qualified
        ));
    }

    if !result.unresolved.is_empty() {
        output.push_str("\nUnresolved references:\n");
        for name in &result.unresolved {
            output.push_str(&format!("  {} <external>\n", name));
        }
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Location, Reference, SymbolKind, Visibility};
    use std::path::PathBuf;

    fn make_symbol(name: &str, qualified: &str, file: &str, line: u32) -> Symbol {
        Symbol {
            name: name.to_string(),
            qualified: qualified.to_string(),
            kind: SymbolKind::Function,
            location: Location::new(PathBuf::from(file), line, 1),
            visibility: Visibility::Public,
        }
    }

    fn make_reference(name: &str, file: &str, line: u32) -> Reference {
        Reference {
            name: name.to_string(),
            location: Location::new(PathBuf::from(file), line, 1),
        }
    }

    #[test]
    fn test_spider_single_node() {
        let mut index = CodeIndex::new();
        index.add_symbol(make_symbol("main", "Program.main", "src/Program.fs", 10));

        let result = spider(&index, "Program.main", 5);

        assert_eq!(result.nodes.len(), 1);
        assert_eq!(result.nodes[0].symbol.qualified, "Program.main");
        assert_eq!(result.nodes[0].depth, 0);
    }

    #[test]
    fn test_spider_follows_references() {
        let mut index = CodeIndex::new();
        index.add_symbol(make_symbol("main", "Program.main", "src/Program.fs", 10));
        index.add_symbol(make_symbol("helper", "Utils.helper", "src/Utils.fs", 5));

        // Add a reference from Program.fs to Utils.helper
        index.add_reference(
            PathBuf::from("src/Program.fs"),
            make_reference("Utils.helper", "src/Program.fs", 15),
        );

        let result = spider(&index, "Program.main", 5);

        assert_eq!(result.nodes.len(), 2);
        assert!(result
            .nodes
            .iter()
            .any(|n| n.symbol.qualified == "Program.main" && n.depth == 0));
        assert!(result
            .nodes
            .iter()
            .any(|n| n.symbol.qualified == "Utils.helper" && n.depth == 1));
    }

    #[test]
    fn test_spider_respects_max_depth() {
        let mut index = CodeIndex::new();
        index.add_symbol(make_symbol("a", "M.a", "a.fs", 1));
        index.add_symbol(make_symbol("b", "M.b", "b.fs", 1));
        index.add_symbol(make_symbol("c", "M.c", "c.fs", 1));

        // a -> b -> c
        index.add_reference(PathBuf::from("a.fs"), make_reference("M.b", "a.fs", 2));
        index.add_reference(PathBuf::from("b.fs"), make_reference("M.c", "b.fs", 2));

        // With max_depth = 1, should only get a and b
        let result = spider(&index, "M.a", 1);

        assert_eq!(result.nodes.len(), 2);
        assert!(result.nodes.iter().all(|n| n.symbol.qualified != "M.c"));
    }

    #[test]
    fn test_spider_tracks_unresolved() {
        let mut index = CodeIndex::new();
        index.add_symbol(make_symbol("main", "Program.main", "src/Program.fs", 10));

        // Reference to external symbol
        index.add_reference(
            PathBuf::from("src/Program.fs"),
            make_reference("Console.WriteLine", "src/Program.fs", 15),
        );

        let result = spider(&index, "Program.main", 5);

        assert!(result.unresolved.contains(&"Console.WriteLine".to_string()));
    }

    #[test]
    fn test_spider_result_files() {
        let mut index = CodeIndex::new();
        index.add_symbol(make_symbol("a", "M.a", "src/a.fs", 1));
        index.add_symbol(make_symbol("b", "M.b", "src/b.fs", 1));

        index.add_reference(
            PathBuf::from("src/a.fs"),
            make_reference("M.b", "src/a.fs", 2),
        );

        let result = spider(&index, "M.a", 5);
        let files = result.files();

        assert_eq!(files.len(), 2);
        assert!(files.contains(Path::new("src/a.fs")));
        assert!(files.contains(Path::new("src/b.fs")));
    }

    #[test]
    fn test_spider_resolves_with_opens() {
        let mut index = CodeIndex::new();
        index.add_symbol(make_symbol("main", "Program.main", "src/Program.fs", 10));
        index.add_symbol(make_symbol(
            "helper",
            "MyApp.Utils.helper",
            "src/Utils.fs",
            5,
        ));

        // Add open statement
        index.add_open(PathBuf::from("src/Program.fs"), "MyApp.Utils".to_string());

        // Reference uses short name
        index.add_reference(
            PathBuf::from("src/Program.fs"),
            make_reference("helper", "src/Program.fs", 15),
        );

        let result = spider(&index, "Program.main", 5);

        assert_eq!(result.nodes.len(), 2);
        assert!(result
            .nodes
            .iter()
            .any(|n| n.symbol.qualified == "MyApp.Utils.helper"));
    }
}
