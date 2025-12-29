//! Spider: Dependency graph traversal from an entry point.
//!
//! The spider crawls from a starting symbol, following references to build
//! a dependency graph. This is useful for understanding code flow and
//! identifying which symbols are reachable from a given entry point.

use std::collections::{HashSet, VecDeque};
use std::path::Path;

use crate::index::Reference;
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
#[must_use]
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
                    if let Some(resolved) =
                        try_resolve_reference(index, &reference.name, opens, &symbol.location.file)
                    {
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
/// 1. Direct match (already qualified) - respecting compilation order
/// 2. Via open statements - respecting compilation order
/// 3. Partial match on the name - respecting compilation order
///
/// The `from_file` parameter is used to respect F# compilation order:
/// a symbol is only visible if its defining file comes before `from_file`.
fn try_resolve_reference(
    index: &CodeIndex,
    name: &str,
    opens: &[String],
    from_file: &Path,
) -> Option<String> {
    // Try direct match first (respecting compilation order)
    if let Some(symbol) = index.get(name) {
        if index.can_reference(from_file, &symbol.location.file) {
            return Some(name.to_string());
        }
    }

    // Try with each open statement (respecting compilation order)
    for open in opens {
        let qualified = format!("{}.{}", open, name);
        if let Some(symbol) = index.get(&qualified) {
            if index.can_reference(from_file, &symbol.location.file) {
                return Some(qualified);
            }
        }
    }

    // Try partial match - look for symbols ending with this name
    // This handles cases like "List.map" where we need to find "Microsoft.FSharp.Collections.List.map"
    // Filter by compilation order
    let search_results = index.search(name);
    for matched in search_results {
        if index.can_reference(from_file, &matched.location.file) {
            return Some(matched.qualified.clone());
        }
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

/// Spider backwards from an entry point, finding callers up to a maximum depth.
///
/// This is the reverse of `spider()` - instead of following what a symbol calls,
/// it follows what calls the symbol (impact analysis).
///
/// # Arguments
/// * `index` - The code index to search
/// * `entry_point` - The qualified name of the starting symbol
/// * `max_depth` - Maximum depth to traverse (0 = only entry point)
///
/// # Returns
/// A `SpiderResult` containing all callers in breadth-first order.
#[must_use]
pub fn reverse_spider(index: &CodeIndex, entry_point: &str, max_depth: usize) -> SpiderResult {
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

                // Don't follow callers beyond max depth
                if depth >= max_depth {
                    continue;
                }

                // Find all references TO this symbol
                let references = index.find_references(&qualified_name);

                // For each reference, find the containing symbol (the caller)
                for reference in references {
                    if let Some(caller) = find_containing_symbol(index, reference) {
                        if !visited.contains(&caller.qualified) {
                            queue.push_back((caller.qualified.clone(), depth + 1));
                        }
                    }
                }
            }
            None => {
                // Entry point not found
                if depth == 0 {
                    result.unresolved.push(qualified_name);
                }
            }
        }
    }

    result
}

/// Find the symbol that contains a given reference (for determining callers).
///
/// Uses a heuristic: the callable symbol (Function or Member) whose definition
/// starts closest to (but before) the reference line in the same file is likely
/// the containing symbol.
///
/// Only considers callable symbols (Function, Member) as potential callers,
/// filtering out variables, types, modules, etc. which cannot be callers.
fn find_containing_symbol<'a>(index: &'a CodeIndex, reference: &Reference) -> Option<&'a Symbol> {
    let symbols = index.symbols_in_file(&reference.location.file);

    // Find the callable symbol that most likely contains this reference
    // Heuristic: the callable symbol with the largest line number that's still <= reference line
    symbols
        .into_iter()
        .filter(|s| s.location.line <= reference.location.line)
        .filter(|s| s.kind.is_callable())
        .max_by_key(|s| s.location.line)
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
        make_symbol_with_kind(name, qualified, file, line, SymbolKind::Function)
    }

    fn make_symbol_with_kind(
        name: &str,
        qualified: &str,
        file: &str,
        line: u32,
        kind: SymbolKind,
    ) -> Symbol {
        Symbol {
            name: name.to_string(),
            qualified: qualified.to_string(),
            kind,
            location: Location::new(PathBuf::from(file), line, 1),
            visibility: Visibility::Public,
            language: "fsharp".to_string(),
            parent: None,
            mixins: None,
            attributes: None,
            implements: None,
            doc: None,
            signature: None,
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

    // =========================================================================
    // Reverse Spider Tests
    // =========================================================================

    #[test]
    fn test_reverse_spider_single_node() {
        let mut index = CodeIndex::new();
        index.add_symbol(make_symbol("main", "Program.main", "src/Program.fs", 10));

        let result = reverse_spider(&index, "Program.main", 5);

        assert_eq!(result.nodes.len(), 1);
        assert_eq!(result.nodes[0].symbol.qualified, "Program.main");
        assert_eq!(result.nodes[0].depth, 0);
    }

    #[test]
    fn test_reverse_spider_finds_callers() {
        let mut index = CodeIndex::new();
        // helper is defined in Utils.fs
        index.add_symbol(make_symbol("helper", "Utils.helper", "src/Utils.fs", 5));
        // main is defined in Program.fs and calls helper
        index.add_symbol(make_symbol("main", "Program.main", "src/Program.fs", 10));

        // Add a reference from Program.fs to Utils.helper (main calls helper)
        index.add_reference(
            PathBuf::from("src/Program.fs"),
            make_reference("Utils.helper", "src/Program.fs", 15),
        );

        // Reverse spider from helper should find main as a caller
        let result = reverse_spider(&index, "Utils.helper", 5);

        assert_eq!(result.nodes.len(), 2);
        assert!(result
            .nodes
            .iter()
            .any(|n| n.symbol.qualified == "Utils.helper" && n.depth == 0));
        assert!(result
            .nodes
            .iter()
            .any(|n| n.symbol.qualified == "Program.main" && n.depth == 1));
    }

    #[test]
    fn test_reverse_spider_respects_max_depth() {
        let mut index = CodeIndex::new();
        index.add_symbol(make_symbol("a", "M.a", "a.fs", 1));
        index.add_symbol(make_symbol("b", "M.b", "b.fs", 1));
        index.add_symbol(make_symbol("c", "M.c", "c.fs", 1));

        // c -> b -> a (c calls b, b calls a)
        // So reverse from a: a <- b <- c
        index.add_reference(PathBuf::from("b.fs"), make_reference("M.a", "b.fs", 5));
        index.add_reference(PathBuf::from("c.fs"), make_reference("M.b", "c.fs", 5));

        // With max_depth = 1, should only get a and b
        let result = reverse_spider(&index, "M.a", 1);

        assert_eq!(result.nodes.len(), 2);
        assert!(result.nodes.iter().all(|n| n.symbol.qualified != "M.c"));
    }

    #[test]
    fn test_reverse_spider_multiple_callers() {
        let mut index = CodeIndex::new();
        // helper is called by both main and test
        index.add_symbol(make_symbol("helper", "Utils.helper", "src/Utils.fs", 5));
        index.add_symbol(make_symbol("main", "Program.main", "src/Program.fs", 10));
        index.add_symbol(make_symbol("test", "Tests.test", "tests/Test.fs", 10));

        // Both files reference helper
        index.add_reference(
            PathBuf::from("src/Program.fs"),
            make_reference("helper", "src/Program.fs", 15),
        );
        index.add_reference(
            PathBuf::from("tests/Test.fs"),
            make_reference("helper", "tests/Test.fs", 15),
        );

        let result = reverse_spider(&index, "Utils.helper", 5);

        assert_eq!(result.nodes.len(), 3);
        assert!(result
            .nodes
            .iter()
            .any(|n| n.symbol.qualified == "Utils.helper" && n.depth == 0));
        assert!(result
            .nodes
            .iter()
            .any(|n| n.symbol.qualified == "Program.main" && n.depth == 1));
        assert!(result
            .nodes
            .iter()
            .any(|n| n.symbol.qualified == "Tests.test" && n.depth == 1));
    }

    #[test]
    fn test_reverse_spider_not_found() {
        let index = CodeIndex::new();
        let result = reverse_spider(&index, "NonExistent.symbol", 5);

        assert!(result.nodes.is_empty());
        assert!(result
            .unresolved
            .contains(&"NonExistent.symbol".to_string()));
    }

    #[test]
    fn test_reverse_spider_no_cycles() {
        let mut index = CodeIndex::new();
        index.add_symbol(make_symbol("a", "M.a", "a.fs", 1));
        index.add_symbol(make_symbol("b", "M.b", "b.fs", 1));

        // a and b call each other (cycle)
        index.add_reference(PathBuf::from("a.fs"), make_reference("M.b", "a.fs", 5));
        index.add_reference(PathBuf::from("b.fs"), make_reference("M.a", "b.fs", 5));

        // Should not infinite loop - each symbol visited only once
        let result = reverse_spider(&index, "M.a", 10);

        assert_eq!(result.nodes.len(), 2);
    }

    #[test]
    fn test_find_containing_symbol_prefers_functions_over_variables() {
        // This test reproduces a bug where local variables were incorrectly
        // selected as callers instead of the enclosing function.
        //
        // Scenario (like redis networking.c):
        //   Line 2860: fn processCommandAndResetClient  <- enclosing function
        //   Line 2861: let deadclient = ...             <- local variable
        //   Line 2862: let old_client = ...             <- local variable
        //   Line 2864: processCommand(c)                <- call site (reference)
        //
        // The bug: old_client (line 2862) was selected as caller because
        // it has the largest line number <= 2864.
        // The fix: only consider callable symbols (Function, Member).

        let mut index = CodeIndex::new();

        // Target function being called
        index.add_symbol(make_symbol("helper", "helper", "utils.c", 10));

        // Enclosing function (the actual caller) - line 100
        index.add_symbol(make_symbol(
            "processCommand",
            "processCommand",
            "main.c",
            100,
        ));

        // Local variables inside the function - lines 101, 102 (after function start)
        index.add_symbol(make_symbol_with_kind(
            "deadclient",
            "deadclient",
            "main.c",
            101,
            SymbolKind::Value,
        ));
        index.add_symbol(make_symbol_with_kind(
            "old_client",
            "old_client",
            "main.c",
            102,
            SymbolKind::Value,
        ));

        // Reference to helper() at line 105 (inside the function)
        index.add_reference(
            PathBuf::from("main.c"),
            make_reference("helper", "main.c", 105),
        );

        // Reverse spider should find processCommand as the caller, NOT old_client
        let result = reverse_spider(&index, "helper", 1);

        assert_eq!(result.nodes.len(), 2, "Should have helper and one caller");

        let caller = result
            .nodes
            .iter()
            .find(|n| n.depth == 1)
            .expect("Should have a caller at depth 1");

        assert_eq!(
            caller.symbol.qualified, "processCommand",
            "Caller should be the function, not a local variable"
        );
        assert_eq!(
            caller.symbol.kind,
            SymbolKind::Function,
            "Caller should be a Function, not a Value"
        );
    }
}
