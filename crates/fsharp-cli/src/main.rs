//! fsharp-index: Command-line tool for F# codebase indexing and navigation.
//!
//! This CLI provides access to fsharp-index functionality for:
//! - Building and updating the symbol index
//! - Finding symbol definitions
//! - Searching for symbols by name
//! - Traversing dependency graphs (spider)
//! - Watching for file changes

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use fsharp_index::{
    spider::{format_spider_result, spider},
    watch::{find_fsharp_files, is_fsharp_file},
    CodeIndex,
};

/// Exit codes for the CLI
mod exit_codes {
    pub const SUCCESS: u8 = 0;
    pub const SYMBOL_NOT_FOUND: u8 = 1;
    pub const INDEX_NOT_FOUND: u8 = 2;
    pub const PARSE_ERROR: u8 = 3;
    pub const INVALID_ARGS: u8 = 4;
}

/// F# codebase indexing and navigation tool
#[derive(Parser)]
#[command(name = "fsharp-index")]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Output results as JSON
    #[arg(long, global = true)]
    json: bool,
}

#[derive(Subcommand)]
enum Commands {
    /// Build or rebuild the index for the current directory
    Build {
        /// Root directory to index (defaults to current directory)
        #[arg(short, long, default_value = ".")]
        root: PathBuf,
    },

    /// Incrementally update the index for changed files
    Update {
        /// Root directory (defaults to current directory)
        #[arg(short, long, default_value = ".")]
        root: PathBuf,
    },

    /// Find the definition of a symbol
    Def {
        /// Symbol name (can be qualified like "MyModule.myFunction")
        symbol: String,

        /// Show the source line containing the definition
        #[arg(long)]
        context: bool,
    },

    /// List references to symbols in a file
    Refs {
        /// File to analyze
        file: PathBuf,
    },

    /// Spider from an entry point symbol
    Spider {
        /// Entry point symbol (qualified name)
        symbol: String,

        /// Maximum depth to traverse
        #[arg(short, long, default_value = "5")]
        depth: usize,
    },

    /// Search for symbols matching a pattern
    Symbols {
        /// Pattern to match (supports * wildcards)
        pattern: String,
    },

    /// Watch for file changes and update the index
    Watch {
        /// Root directory to watch (defaults to current directory)
        #[arg(short, long, default_value = ".")]
        root: PathBuf,
    },
}

fn main() -> ExitCode {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::WARN.into()),
        )
        .init();

    let cli = Cli::parse();

    match run(cli) {
        Ok(code) => ExitCode::from(code),
        Err(e) => {
            eprintln!("Error: {:#}", e);
            ExitCode::from(exit_codes::INVALID_ARGS)
        }
    }
}

fn run(cli: Cli) -> Result<u8> {
    match cli.command {
        Commands::Build { root } => cmd_build(&root, cli.json),
        Commands::Update { root } => cmd_update(&root, cli.json),
        Commands::Def { symbol, context } => cmd_def(&symbol, context, cli.json),
        Commands::Refs { file } => cmd_refs(&file, cli.json),
        Commands::Spider { symbol, depth } => cmd_spider(&symbol, depth, cli.json),
        Commands::Symbols { pattern } => cmd_symbols(&pattern, cli.json),
        Commands::Watch { root } => cmd_watch(&root),
    }
}

/// Build or rebuild the index
fn cmd_build(root: &PathBuf, json: bool) -> Result<u8> {
    let root = root
        .canonicalize()
        .context("Failed to resolve root directory")?;

    let files = find_fsharp_files(&root).context("Failed to find F# files")?;

    // Create index with workspace root for relative path storage
    let mut index = CodeIndex::with_root(root.clone());
    let mut errors = Vec::new();

    for file in &files {
        match std::fs::read_to_string(file) {
            Ok(source) => {
                let result = fsharp_index::extract_symbols(file, &source);
                for symbol in result.symbols {
                    index.add_symbol(symbol);
                }
                for reference in result.references {
                    index.add_reference(file.clone(), reference);
                }
                for open in result.opens {
                    index.add_open(file.clone(), open);
                }
            }
            Err(e) => {
                errors.push(format!("{}: {}", file.display(), e));
            }
        }
    }

    // Save index to disk
    let index_dir = root.join(".fsharp-index");
    std::fs::create_dir_all(&index_dir).context("Failed to create index directory")?;

    let index_file = index_dir.join("index.json");
    let index_json = serde_json::to_string_pretty(&index)?;
    std::fs::write(&index_file, index_json).context("Failed to write index file")?;

    if json {
        let output = serde_json::json!({
            "files": files.len(),
            "symbols": index.symbol_count(),
            "errors": errors,
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        println!(
            "Indexed {} files, {} symbols",
            files.len(),
            index.symbol_count()
        );
        if !errors.is_empty() {
            eprintln!("Warnings:");
            for error in errors {
                eprintln!("  {}", error);
            }
        }
    }

    Ok(exit_codes::SUCCESS)
}

/// Update the index incrementally
fn cmd_update(root: &PathBuf, json: bool) -> Result<u8> {
    let root = root
        .canonicalize()
        .context("Failed to resolve root directory")?;

    let index_file = root.join(".fsharp-index/index.json");
    if !index_file.exists() {
        if json {
            println!(
                "{}",
                serde_json::json!({"error": "Index not found. Run 'build' first."})
            );
        } else {
            eprintln!("Index not found. Run 'fsharp-index build' first.");
        }
        return Ok(exit_codes::INDEX_NOT_FOUND);
    }

    let index_content = std::fs::read_to_string(&index_file)?;
    let mut index: CodeIndex = serde_json::from_str(&index_content)?;

    // Set workspace root for path resolution
    index.set_workspace_root(root.clone());

    // Find files that have changed (simplified: just re-index all files for now)
    // TODO: Use file modification times or a proper incremental strategy
    let files = find_fsharp_files(&root)?;
    let mut updated_count = 0;

    for file in &files {
        if let Ok(source) = std::fs::read_to_string(file) {
            index.clear_file(file);
            let result = fsharp_index::extract_symbols(file, &source);
            for symbol in result.symbols {
                index.add_symbol(symbol);
            }
            for reference in result.references {
                index.add_reference(file.clone(), reference);
            }
            for open in result.opens {
                index.add_open(file.clone(), open);
            }
            updated_count += 1;
        }
    }

    // Save updated index
    let index_json = serde_json::to_string_pretty(&index)?;
    std::fs::write(&index_file, index_json)?;

    if json {
        println!(
            "{}",
            serde_json::json!({
                "updated": updated_count,
                "symbols": index.symbol_count(),
            })
        );
    } else {
        println!("Updated {} files", updated_count);
    }

    Ok(exit_codes::SUCCESS)
}

/// Find the definition of a symbol
fn cmd_def(symbol: &str, context: bool, json: bool) -> Result<u8> {
    let index = load_index()?;

    // Try exact match first
    if let Some(sym) = index.get(symbol) {
        output_location(sym, context, json)?;
        return Ok(exit_codes::SUCCESS);
    }

    // Try searching for partial matches
    let matches = index.search(symbol);
    if let Some(sym) = matches.first() {
        output_location(sym, context, json)?;
        return Ok(exit_codes::SUCCESS);
    }

    if json {
        println!("{}", serde_json::json!({"error": "Symbol not found"}));
    } else {
        eprintln!("Symbol not found: {}", symbol);
    }

    Ok(exit_codes::SYMBOL_NOT_FOUND)
}

fn output_location(sym: &fsharp_index::Symbol, context: bool, json: bool) -> Result<()> {
    let loc = &sym.location;

    if json {
        let mut output = serde_json::json!({
            "file": loc.file.display().to_string(),
            "line": loc.line,
            "column": loc.column,
            "name": sym.name,
            "qualified": sym.qualified,
            "kind": format!("{}", sym.kind),
        });

        if context {
            if let Some(line_content) = get_line_content(&loc.file, loc.line as usize) {
                output["context"] = serde_json::Value::String(line_content);
            }
        }

        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        println!("{}:{}:{}", loc.file.display(), loc.line, loc.column);
        if context {
            if let Some(line_content) = get_line_content(&loc.file, loc.line as usize) {
                println!("    {}", line_content.trim());
            }
        }
    }

    Ok(())
}

/// List references in a file
fn cmd_refs(file: &PathBuf, json: bool) -> Result<u8> {
    let index = load_index()?;
    let file = file.canonicalize().context("Failed to resolve file path")?;

    let references = index.references_in_file(&file);

    if json {
        let refs: Vec<_> = references
            .iter()
            .map(|r| {
                serde_json::json!({
                    "name": r.name,
                    "line": r.location.line,
                    "column": r.location.column,
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&refs)?);
    } else {
        for reference in references {
            // Try to resolve the reference
            if let Some(resolved) = index.resolve(&reference.name, &file) {
                println!(
                    "{:<40} {}:{}:{}",
                    reference.name,
                    resolved.symbol.location.file.display(),
                    resolved.symbol.location.line,
                    resolved.symbol.location.column
                );
            } else {
                println!("{:<40} <external>", reference.name);
            }
        }
    }

    Ok(exit_codes::SUCCESS)
}

/// Spider from an entry point
fn cmd_spider(symbol: &str, depth: usize, json: bool) -> Result<u8> {
    let index = load_index()?;

    // First try to find the entry point
    let entry_qualified = if index.get(symbol).is_some() {
        symbol.to_string()
    } else {
        // Try to find it via search
        let matches = index.search(symbol);
        if let Some(first) = matches.first() {
            first.qualified.clone()
        } else {
            if json {
                println!("{}", serde_json::json!({"error": "Entry point not found"}));
            } else {
                eprintln!("Entry point not found: {}", symbol);
            }
            return Ok(exit_codes::SYMBOL_NOT_FOUND);
        }
    };

    let result = spider(&index, &entry_qualified, depth);

    if json {
        let nodes: Vec<_> = result
            .nodes
            .iter()
            .map(|n| {
                serde_json::json!({
                    "name": n.symbol.name,
                    "qualified": n.symbol.qualified,
                    "file": n.symbol.location.file.display().to_string(),
                    "line": n.symbol.location.line,
                    "column": n.symbol.location.column,
                    "depth": n.depth,
                })
            })
            .collect();

        let output = serde_json::json!({
            "nodes": nodes,
            "unresolved": result.unresolved,
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        print!("{}", format_spider_result(&result));
    }

    Ok(exit_codes::SUCCESS)
}

/// Search for symbols matching a pattern
fn cmd_symbols(pattern: &str, json: bool) -> Result<u8> {
    let index = load_index()?;
    let matches = index.search(pattern);

    if json {
        let symbols: Vec<_> = matches
            .iter()
            .map(|s| {
                serde_json::json!({
                    "name": s.name,
                    "qualified": s.qualified,
                    "kind": format!("{}", s.kind),
                    "file": s.location.file.display().to_string(),
                    "line": s.location.line,
                    "column": s.location.column,
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&symbols)?);
    } else {
        for sym in matches {
            println!(
                "{:<40} {}:{}:{:<8} {}",
                sym.qualified,
                sym.location.file.display(),
                sym.location.line,
                sym.location.column,
                sym.kind
            );
        }
    }

    Ok(exit_codes::SUCCESS)
}

/// Watch for file changes
fn cmd_watch(root: &PathBuf) -> Result<u8> {
    use fsharp_index::watch::FileWatcher;

    let root = root
        .canonicalize()
        .context("Failed to resolve root directory")?;

    // First, ensure index exists
    println!("Building initial index...");
    cmd_build(&root, false)?;

    let mut watcher = FileWatcher::new(&root).context("Failed to create file watcher")?;
    watcher.start().context("Failed to start watching")?;

    println!("Watching for changes... (Ctrl+C to stop)");

    loop {
        if let Some(event) = watcher.wait() {
            match event {
                fsharp_index::watch::WatchEvent::Created(path)
                | fsharp_index::watch::WatchEvent::Modified(path) => {
                    if is_fsharp_file(&path) {
                        println!("Updated: {}", path.display());
                        update_single_file(&root, &path)?;
                    }
                }
                fsharp_index::watch::WatchEvent::Deleted(path) => {
                    if is_fsharp_file(&path) {
                        println!("Deleted: {}", path.display());
                        remove_file_from_index(&root, &path)?;
                    }
                }
                fsharp_index::watch::WatchEvent::Renamed(old, new) => {
                    if is_fsharp_file(&old) || is_fsharp_file(&new) {
                        println!("Renamed: {} -> {}", old.display(), new.display());
                        remove_file_from_index(&root, &old)?;
                        if is_fsharp_file(&new) {
                            update_single_file(&root, &new)?;
                        }
                    }
                }
            }
        }
    }
}

/// Load the index from disk
fn load_index() -> Result<CodeIndex> {
    let cwd = std::env::current_dir()?;
    let index_file = cwd.join(".fsharp-index/index.json");

    if !index_file.exists() {
        anyhow::bail!("Index not found. Run 'fsharp-index build' first.");
    }

    let content = std::fs::read_to_string(&index_file)?;
    let mut index: CodeIndex = serde_json::from_str(&content)?;

    // Set workspace root for absolute path resolution
    index.set_workspace_root(cwd);

    Ok(index)
}

/// Get a specific line from a file
fn get_line_content(file: &PathBuf, line: usize) -> Option<String> {
    let content = std::fs::read_to_string(file).ok()?;
    content.lines().nth(line - 1).map(|s| s.to_string())
}

/// Update a single file in the index
fn update_single_file(root: &PathBuf, file: &PathBuf) -> Result<()> {
    let index_file = root.join(".fsharp-index/index.json");
    let content = std::fs::read_to_string(&index_file)?;
    let mut index: CodeIndex = serde_json::from_str(&content)?;

    // Set workspace root for path resolution
    index.set_workspace_root(root.clone());

    index.clear_file(file);

    if let Ok(source) = std::fs::read_to_string(file) {
        let result = fsharp_index::extract_symbols(file, &source);
        for symbol in result.symbols {
            index.add_symbol(symbol);
        }
        for reference in result.references {
            index.add_reference(file.clone(), reference);
        }
        for open in result.opens {
            index.add_open(file.clone(), open);
        }
    }

    let index_json = serde_json::to_string_pretty(&index)?;
    std::fs::write(&index_file, index_json)?;

    Ok(())
}

/// Remove a file from the index
fn remove_file_from_index(root: &PathBuf, file: &PathBuf) -> Result<()> {
    let index_file = root.join(".fsharp-index/index.json");
    let content = std::fs::read_to_string(&index_file)?;
    let mut index: CodeIndex = serde_json::from_str(&content)?;

    // Set workspace root for path resolution
    index.set_workspace_root(root.clone());

    index.clear_file(file);

    let index_json = serde_json::to_string_pretty(&index)?;
    std::fs::write(&index_file, index_json)?;

    Ok(())
}
