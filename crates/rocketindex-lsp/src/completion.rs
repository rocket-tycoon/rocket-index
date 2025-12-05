//! Completion support for F# language server.
//!
//! Provides keyword and symbol completion.

use std::path::Path;

use rocketindex::{CodeIndex, Symbol, SymbolKind};
use tower_lsp::lsp_types::{CompletionItem, CompletionItemKind};

/// F# keywords for completion.
/// Reference: https://docs.microsoft.com/en-us/dotnet/fsharp/language-reference/keyword-reference
pub const FSHARP_KEYWORDS: &[&str] = &[
    // Core keywords
    "let",
    "let!",
    "do",
    "do!",
    "return",
    "return!",
    "yield",
    "yield!",
    "if",
    "then",
    "else",
    "elif",
    "match",
    "with",
    "for",
    "to",
    "downto",
    "in",
    "while",
    "try",
    "finally",
    "raise",
    "failwith",
    "fun",
    "function",
    // Type keywords
    "type",
    "and",
    "or",
    "not",
    "rec",
    "mutable",
    "inline",
    "private",
    "internal",
    "public",
    "static",
    "member",
    "override",
    "abstract",
    "default",
    "val",
    // Module/namespace
    "module",
    "namespace",
    "open",
    // Object-oriented
    "new",
    "inherit",
    "interface",
    "class",
    "struct",
    "as",
    "base",
    "this",
    // Pattern matching
    "when",
    "null",
    "true",
    "false",
    // Async/computation
    "async",
    "use",
    "use!",
    "lazy",
    "seq",
    "assert",
    // Other
    "begin",
    "end",
    "done",
    "of",
    "exception",
    "extern",
    "global",
    "upcast",
    "downcast",
    "fixed",
];

/// Generate completion items for F# keywords.
///
/// Optionally filters by a prefix.
#[must_use]
pub fn keyword_completions(prefix: Option<&str>) -> Vec<CompletionItem> {
    FSHARP_KEYWORDS
        .iter()
        .filter(|kw| match prefix {
            Some(p) if !p.is_empty() => kw.starts_with(p),
            _ => true,
        })
        .map(|kw| CompletionItem {
            label: (*kw).to_string(),
            kind: Some(CompletionItemKind::KEYWORD),
            detail: Some("F# keyword".to_string()),
            ..Default::default()
        })
        .collect()
}

/// Convert our SymbolKind to LSP CompletionItemKind.
fn to_completion_kind(kind: SymbolKind) -> CompletionItemKind {
    match kind {
        SymbolKind::Module => CompletionItemKind::MODULE,
        SymbolKind::Function => CompletionItemKind::FUNCTION,
        SymbolKind::Value => CompletionItemKind::VARIABLE,
        SymbolKind::Type => CompletionItemKind::TYPE_PARAMETER,
        SymbolKind::Record => CompletionItemKind::STRUCT,
        SymbolKind::Union => CompletionItemKind::ENUM,
        SymbolKind::Interface => CompletionItemKind::INTERFACE,
        SymbolKind::Class => CompletionItemKind::CLASS,
        SymbolKind::Member => CompletionItemKind::METHOD,
    }
}

/// Generate completion items for symbols visible from the current file.
///
/// This includes:
/// - Symbols defined in the current file
/// - Symbols from opened modules
/// - Qualified symbols from the index
///
/// # Arguments
/// * `index` - The code index to search
/// * `current_file` - The file where completion is requested
/// * `prefix` - Optional prefix to filter symbols
/// * `limit` - Maximum number of results to return
#[must_use]
pub fn symbol_completions(
    index: &CodeIndex,
    current_file: &Path,
    prefix: Option<&str>,
    limit: usize,
) -> Vec<CompletionItem> {
    let mut items = Vec::new();

    // Get opened modules for this file
    let opens: Vec<String> = index.opens_for_file(current_file).to_vec();

    // Search for matching symbols
    let search_pattern = prefix.unwrap_or("");
    let candidates: Vec<&Symbol> = if search_pattern.is_empty() {
        // Return symbols from opened modules + current file
        let mut syms: Vec<&Symbol> = Vec::new();

        // Add symbols from current file
        for sym in index.symbols_in_file(current_file) {
            syms.push(sym);
        }

        // Add symbols from opened modules
        for open in &opens {
            for sym in index.symbols_in_module(open) {
                syms.push(sym);
            }
        }

        syms
    } else {
        // Search globally and filter
        index.search(search_pattern)
    };

    for sym in candidates.into_iter().take(limit) {
        // Skip if doesn't match prefix (case-insensitive)
        if let Some(p) = prefix {
            if !p.is_empty()
                && !sym.name.to_lowercase().starts_with(&p.to_lowercase())
                && !sym.qualified.to_lowercase().contains(&p.to_lowercase())
            {
                continue;
            }
        }

        // Determine if this symbol is directly accessible or needs qualification
        let is_in_scope = is_symbol_in_scope(&sym.qualified, current_file, &opens);

        let (label, insert_text) = if is_in_scope {
            // Symbol is directly accessible by short name
            (sym.name.clone(), None)
        } else {
            // Symbol needs qualification - show qualified name
            (sym.name.clone(), Some(sym.qualified.clone()))
        };

        let detail = format!("{} ({})", sym.kind, sym.qualified);

        items.push(CompletionItem {
            label,
            insert_text,
            kind: Some(to_completion_kind(sym.kind)),
            detail: Some(detail),
            ..Default::default()
        });
    }

    items
}

/// Check if a symbol is directly accessible (without qualification) from the current context.
fn is_symbol_in_scope(qualified: &str, current_file: &Path, opens: &[String]) -> bool {
    // Extract the module part of the qualified name
    let module_part = qualified.rsplit_once('.').map(|(m, _)| m).unwrap_or("");

    if module_part.is_empty() {
        // Top-level symbol, always in scope
        return true;
    }

    // Check if the module is opened
    for open in opens {
        if module_part == open || module_part.starts_with(&format!("{}.", open)) {
            return true;
        }
    }

    // Check if it's in the same file's module
    // (simplified: just check if the file name matches the last module component)
    if let Some(file_stem) = current_file.file_stem().and_then(|s| s.to_str()) {
        if module_part.ends_with(file_stem) || module_part.split('.').any(|p| p == file_stem) {
            return true;
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use rocketindex::{Location, Visibility};
    use std::path::PathBuf;

    #[test]
    fn keyword_list_is_not_empty() {
        // The len check implicitly verifies non-empty
        assert!(FSHARP_KEYWORDS.len() >= 50, "Should have many keywords");
    }

    #[test]
    fn keyword_list_contains_common_keywords() {
        assert!(FSHARP_KEYWORDS.contains(&"let"));
        assert!(FSHARP_KEYWORDS.contains(&"match"));
        assert!(FSHARP_KEYWORDS.contains(&"type"));
        assert!(FSHARP_KEYWORDS.contains(&"module"));
        assert!(FSHARP_KEYWORDS.contains(&"open"));
        assert!(FSHARP_KEYWORDS.contains(&"if"));
        assert!(FSHARP_KEYWORDS.contains(&"then"));
        assert!(FSHARP_KEYWORDS.contains(&"else"));
    }

    #[test]
    fn keyword_completions_returns_all_when_no_prefix() {
        let completions = keyword_completions(None);
        assert_eq!(completions.len(), FSHARP_KEYWORDS.len());
    }

    #[test]
    fn keyword_completions_filters_by_prefix() {
        let completions = keyword_completions(Some("let"));
        assert!(completions.iter().any(|c| c.label == "let"));
        assert!(completions.iter().any(|c| c.label == "let!"));
        assert!(!completions.iter().any(|c| c.label == "match"));
    }

    #[test]
    fn keyword_completions_returns_correct_kind() {
        let completions = keyword_completions(Some("let"));
        for item in &completions {
            assert_eq!(item.kind, Some(CompletionItemKind::KEYWORD));
        }
    }

    #[test]
    fn keyword_completions_empty_prefix_returns_all() {
        let completions = keyword_completions(Some(""));
        assert_eq!(completions.len(), FSHARP_KEYWORDS.len());
    }

    #[test]
    fn keyword_completions_no_match_returns_empty() {
        let completions = keyword_completions(Some("xyz"));
        assert!(completions.is_empty());
    }

    // ============================================================
    // Symbol Completion Tests
    // ============================================================

    fn create_test_index() -> CodeIndex {
        let mut index = CodeIndex::new();

        // Add some test symbols
        index.add_symbol(Symbol::new(
            "helper".to_string(),
            "MyApp.Utils.helper".to_string(),
            SymbolKind::Function,
            Location::new(PathBuf::from("Utils.fs"), 10, 1),
            Visibility::Public,
            "fsharp".to_string(),
        ));

        index.add_symbol(Symbol::new(
            "User".to_string(),
            "MyApp.Domain.User".to_string(),
            SymbolKind::Record,
            Location::new(PathBuf::from("Domain.fs"), 5, 1),
            Visibility::Public,
            "fsharp".to_string(),
        ));

        index.add_symbol(Symbol::new(
            "processUser".to_string(),
            "MyApp.Services.processUser".to_string(),
            SymbolKind::Function,
            Location::new(PathBuf::from("Services.fs"), 15, 1),
            Visibility::Public,
            "fsharp".to_string(),
        ));

        // Add an open statement
        index.add_open(PathBuf::from("Services.fs"), "MyApp.Domain".to_string());

        index
    }

    #[test]
    fn symbol_completions_returns_matching_symbols() {
        let index = create_test_index();
        let file = PathBuf::from("test.fs");

        let completions = symbol_completions(&index, &file, Some("proc"), 50);

        assert!(
            completions.iter().any(|c| c.label == "processUser"),
            "Should find processUser"
        );
    }

    #[test]
    fn symbol_completions_filters_by_prefix() {
        let index = create_test_index();
        let file = PathBuf::from("test.fs");

        let completions = symbol_completions(&index, &file, Some("help"), 50);

        assert!(
            completions.iter().any(|c| c.label == "helper"),
            "Should find helper"
        );
        assert!(
            !completions.iter().any(|c| c.label == "processUser"),
            "Should not find processUser"
        );
    }

    #[test]
    fn symbol_completions_includes_types() {
        let index = create_test_index();
        let file = PathBuf::from("test.fs");

        let completions = symbol_completions(&index, &file, Some("User"), 50);

        let user_completion = completions.iter().find(|c| c.label == "User");
        assert!(user_completion.is_some(), "Should find User type");
        assert_eq!(
            user_completion.unwrap().kind,
            Some(CompletionItemKind::STRUCT)
        );
    }

    #[test]
    fn symbol_completions_respects_limit() {
        let index = create_test_index();
        let file = PathBuf::from("test.fs");

        let completions = symbol_completions(&index, &file, None, 1);

        assert!(completions.len() <= 1, "Should respect limit");
    }

    #[test]
    fn symbol_completions_shows_kind_in_detail() {
        let index = create_test_index();
        let file = PathBuf::from("test.fs");

        let completions = symbol_completions(&index, &file, Some("helper"), 50);

        let helper = completions.iter().find(|c| c.label == "helper").unwrap();
        assert!(
            helper.detail.as_ref().unwrap().contains("Function"),
            "Detail should include kind"
        );
        assert!(
            helper
                .detail
                .as_ref()
                .unwrap()
                .contains("MyApp.Utils.helper"),
            "Detail should include qualified name"
        );
    }
}
