//! rocketindex: Rocket-fast CLI for F# codebase indexing and navigation.
//!
//! This CLI provides access to rocketindex functionality for:
//! - Building and updating the symbol index
//! - Finding symbol definitions
//! - Searching for symbols by name
//! - Traversing dependency graphs (spider)
//! - Watching for file changes

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use rayon::prelude::*;
use rocketindex::{
    db::DEFAULT_DB_NAME,
    find_fsproj_files, parse_fsproj,
    spider::{format_spider_result, spider},
    watch::{find_fsharp_files, is_fsharp_file},
    CodeIndex, SqliteIndex,
};

/// Exit codes for the CLI
///
/// These follow the documented contract in the AI Agent Integration Strategy:
/// - 0: Success
/// - 1: Not found (valid query, no results)
/// - 2: Error (invalid input, missing file, etc.)
mod exit_codes {
    pub const SUCCESS: u8 = 0;
    pub const NOT_FOUND: u8 = 1;
    pub const ERROR: u8 = 2;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
enum OutputFormat {
    Json,
    Pretty,
    Text,
}

/// Rocket-fast F# codebase indexing and navigation tool
#[derive(Parser)]
#[command(name = "rocketindex")]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Output format
    #[arg(long, global = true, value_enum, default_value_t = OutputFormat::Json)]
    format: OutputFormat,

    /// Output results as JSON (deprecated, use --format json)
    #[arg(long, global = true, hide = true)]
    json: bool,

    /// Suppress progress output
    #[arg(short, long, global = true)]
    quiet: bool,
}

#[derive(Subcommand)]
enum Commands {
    /// Build or rebuild the index for the current directory
    Build {
        /// Root directory to index (defaults to current directory)
        #[arg(short, long, default_value = ".")]
        root: PathBuf,

        /// Also extract type information (requires dotnet fsi)
        #[arg(long)]
        extract_types: bool,
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

    /// Extract type information from a project (requires dotnet fsi)
    ExtractTypes {
        /// Path to .fsproj file
        project: PathBuf,

        /// Output directory for type cache (default: .fsharp-types/ in project dir)
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// Enable verbose output
        #[arg(short, long)]
        verbose: bool,
    },

    /// Show type cache information
    TypeInfo {
        /// Symbol qualified name to look up
        symbol: Option<String>,

        /// Type name to show members of
        #[arg(long)]
        members_of: Option<String>,
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

    // Handle deprecated --json flag
    let format = if cli.json {
        OutputFormat::Json
    } else {
        cli.format
    };

    match run(cli.command, format, cli.quiet) {
        Ok(code) => ExitCode::from(code),
        Err(e) => {
            if format == OutputFormat::Json {
                let error_json = serde_json::json!({
                    "error": "CommandFailed",
                    "message": e.to_string(),
                });
                eprintln!("{}", error_json);
            } else {
                eprintln!("Error: {:#}", e);
            }
            ExitCode::from(exit_codes::ERROR)
        }
    }
}

fn run(command: Commands, format: OutputFormat, quiet: bool) -> Result<u8> {
    match command {
        Commands::Build {
            root,
            extract_types,
        } => cmd_build(&root, extract_types, format, quiet),
        Commands::Update { root } => cmd_update(&root, format, quiet),
        Commands::Def { symbol, context } => cmd_def(&symbol, context, format, quiet),
        Commands::Refs { file } => cmd_refs(&file, format, quiet),
        Commands::Spider { symbol, depth } => cmd_spider(&symbol, depth, format, quiet),
        Commands::Symbols { pattern } => cmd_symbols(&pattern, format, quiet),
        Commands::Watch { root } => cmd_watch(&root, format, quiet),
        Commands::ExtractTypes {
            project,
            output,
            verbose,
        } => cmd_extract_types(&project, output.as_deref(), verbose, format, quiet),
        Commands::TypeInfo { symbol, members_of } => {
            cmd_type_info(symbol.as_deref(), members_of.as_deref(), format, quiet)
        }
    }
}

/// Build or rebuild the index using SQLite
fn cmd_build(root: &Path, extract_types: bool, format: OutputFormat, quiet: bool) -> Result<u8> {
    let root = root
        .canonicalize()
        .context("Failed to resolve root directory")?;

    let files = find_fsharp_files(&root).context("Failed to find F# files")?;

    // Try to find and parse .fsproj files for compilation order
    let fsproj_files = find_fsproj_files(&root);
    let mut file_order: Vec<PathBuf> = Vec::new();
    let mut fsproj_count = 0;

    for fsproj_path in &fsproj_files {
        if let Ok(info) = parse_fsproj(fsproj_path) {
            // Merge file orders from all .fsproj files
            // Files from later projects are appended (they can reference earlier ones)
            for file in info.compile_files {
                if !file_order.contains(&file) {
                    file_order.push(file);
                }
            }
            fsproj_count += 1;
        }
    }

    // Parse files in parallel using rayon
    let parse_results: Vec<_> = files
        .par_iter()
        .map(|file| match std::fs::read_to_string(file) {
            Ok(source) => {
                let result = rocketindex::extract_symbols(file, &source);
                Ok((file.clone(), result))
            }
            Err(e) => Err(format!("{}: {}", file.display(), e)),
        })
        .collect();

    // Create SQLite index
    let index_dir = root.join(".rocketindex");
    std::fs::create_dir_all(&index_dir).context("Failed to create index directory")?;

    let db_path = index_dir.join(DEFAULT_DB_NAME);

    // Remove existing database to rebuild from scratch
    if db_path.exists() {
        std::fs::remove_file(&db_path).context("Failed to remove existing index")?;
    }

    let index = SqliteIndex::create(&db_path).context("Failed to create SQLite index")?;

    // Store workspace root in metadata
    index
        .set_metadata("workspace_root", &root.to_string_lossy())
        .context("Failed to set workspace root")?;

    // Store file order if we found .fsproj files
    if !file_order.is_empty() {
        let file_order_json = serde_json::to_string(&file_order)?;
        index
            .set_metadata("file_order", &file_order_json)
            .context("Failed to set file order")?;
    }

    let mut errors = Vec::new();
    let mut symbol_count = 0;

    // TODO: Use batch insert methods for better performance
    // Individual inserts are auto-committed by SQLite
    for result in parse_results {
        match result {
            Ok((file, parse_result)) => {
                // Insert symbols
                for symbol in &parse_result.symbols {
                    if let Err(e) = index.insert_symbol(symbol) {
                        errors.push(format!("Failed to insert symbol {}: {}", symbol.name, e));
                    } else {
                        symbol_count += 1;
                    }
                }

                // Insert references
                for reference in &parse_result.references {
                    if let Err(e) = index.insert_reference(&file, reference) {
                        errors.push(format!("Failed to insert reference: {}", e));
                    }
                }

                // Insert opens
                for (line, open) in parse_result.opens.iter().enumerate() {
                    if let Err(e) = index.insert_open(&file, open, line as u32 + 1) {
                        errors.push(format!("Failed to insert open: {}", e));
                    }
                }
            }
            Err(e) => {
                errors.push(e);
            }
        }
    }

    if format == OutputFormat::Json {
        let output = serde_json::json!({
            "files": files.len(),
            "symbols": symbol_count,
            "fsproj_files": fsproj_count,
            "file_order_count": file_order.len(),
            "errors": errors,
            "database": db_path.display().to_string(),
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else if !quiet {
        println!("Indexed {} files, {} symbols", files.len(), symbol_count);
        println!("Database: {}", db_path.display());
        if fsproj_count > 0 {
            println!(
                "Found {} .fsproj file(s), {} files in compilation order",
                fsproj_count,
                file_order.len()
            );
        }
        if !errors.is_empty() {
            eprintln!("Warnings:");
            for error in &errors[..errors.len().min(10)] {
                eprintln!("  {}", error);
            }
            if errors.len() > 10 {
                eprintln!("  ... and {} more", errors.len() - 10);
            }
        }
    }

    // Optionally run type extraction
    if extract_types {
        for fsproj_path in &fsproj_files {
            if !quiet && format != OutputFormat::Json {
                println!("Extracting types from: {}", fsproj_path.display());
            }
            if let Err(e) = run_type_extraction(fsproj_path, None, false) {
                if !quiet && format != OutputFormat::Json {
                    eprintln!(
                        "Warning: Type extraction failed for {}: {}",
                        fsproj_path.display(),
                        e
                    );
                }
            }
        }
    }

    Ok(exit_codes::SUCCESS)
}

/// Run the F# type extraction script
fn run_type_extraction(
    project: &PathBuf,
    output: Option<&std::path::Path>,
    verbose: bool,
) -> Result<()> {
    use std::process::Command;

    // Find the extract-types.fsx script
    // Look in several locations:
    // 1. Same directory as the executable
    // 2. scripts/ relative to executable
    // 3. Hardcoded development path
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()));

    let script_paths = [
        exe_dir.as_ref().map(|d| d.join("extract-types.fsx")),
        exe_dir
            .as_ref()
            .map(|d| d.join("scripts/extract-types.fsx")),
        exe_dir
            .as_ref()
            .map(|d| d.join("../scripts/extract-types.fsx")),
        Some(PathBuf::from("scripts/extract-types.fsx")),
    ];

    let script_path = script_paths
        .iter()
        .filter_map(|p| p.as_ref())
        .find(|p| p.exists())
        .cloned();

    let script = match script_path {
        Some(p) => p,
        None => {
            // Fall back to inline execution hint
            anyhow::bail!(
                "extract-types.fsx not found. Please run manually:\n\
                 dotnet fsi scripts/extract-types.fsx {}",
                project.display()
            );
        }
    };

    let mut cmd = Command::new("dotnet");
    cmd.arg("fsi").arg(&script).arg(project);

    if let Some(out) = output {
        cmd.arg("--output").arg(out);
    }

    if verbose {
        cmd.arg("--verbose");
    }

    let status = cmd
        .status()
        .context("Failed to run dotnet fsi - is .NET SDK installed?")?;

    if !status.success() {
        anyhow::bail!("Type extraction failed with exit code: {:?}", status.code());
    }

    Ok(())
}

/// Extract types from a project
fn cmd_extract_types(
    project: &PathBuf,
    output: Option<&std::path::Path>,
    verbose: bool,
    format: OutputFormat,
    _quiet: bool,
) -> Result<u8> {
    if !project.exists() {
        anyhow::bail!("Project file not found: {}", project.display());
    }

    run_type_extraction(project, output, verbose)?;

    if format == OutputFormat::Json {
        let output_dir = output.map(PathBuf::from).unwrap_or_else(|| {
            project
                .parent()
                .unwrap_or(std::path::Path::new("."))
                .join(".fsharp-types")
        });
        let cache_path = output_dir.join("cache.json");

        println!(
            "{}",
            serde_json::json!({
                "success": true,
                "cache_path": cache_path.display().to_string(),
            })
        );
    }

    Ok(exit_codes::SUCCESS)
}

/// Show type cache information
fn cmd_type_info(
    symbol: Option<&str>,
    members_of: Option<&str>,
    format: OutputFormat,
    quiet: bool,
) -> Result<u8> {
    let index = load_sqlite_index()?;

    if let Some(sym) = symbol {
        match index.get_symbol_type(sym) {
            Ok(Some(type_sig)) => {
                if format == OutputFormat::Json {
                    println!(
                        "{}",
                        serde_json::json!({
                            "symbol": sym,
                            "type": type_sig,
                        })
                    );
                } else if !quiet {
                    println!("{} : {}", sym, type_sig);
                }
            }
            Ok(None) => {
                if format == OutputFormat::Json {
                    println!(
                        "{}",
                        serde_json::json!({
                            "error": "Symbol not found or has no type info",
                            "symbol": sym,
                        })
                    );
                } else {
                    eprintln!("Symbol not found or has no type info: {}", sym);
                }
                return Ok(exit_codes::NOT_FOUND);
            }
            Err(e) => {
                anyhow::bail!("Failed to query symbol type: {}", e);
            }
        }
    }

    if let Some(type_name) = members_of {
        match index.get_members(type_name) {
            Ok(members) if !members.is_empty() => {
                if format == OutputFormat::Json {
                    let member_list: Vec<_> = members
                        .iter()
                        .map(|m| {
                            serde_json::json!({
                                "member": m.member,
                                "type": m.member_type,
                                "kind": format!("{}", m.kind),
                            })
                        })
                        .collect();
                    println!(
                        "{}",
                        serde_json::json!({
                            "type": type_name,
                            "members": member_list,
                        })
                    );
                } else if !quiet {
                    println!("Members of {}:", type_name);
                    for m in members {
                        println!("  {} : {} ({})", m.member, m.member_type, m.kind);
                    }
                }
            }
            Ok(_) => {
                if format == OutputFormat::Json {
                    println!(
                        "{}",
                        serde_json::json!({
                            "error": "Type not found in type cache",
                            "type": type_name,
                        })
                    );
                } else {
                    eprintln!("Type not found in type cache: {}", type_name);
                }
                return Ok(exit_codes::NOT_FOUND);
            }
            Err(e) => {
                anyhow::bail!("Failed to query type members: {}", e);
            }
        }
    }

    if symbol.is_none() && members_of.is_none() {
        // Show summary
        let symbol_count = index.count_symbols().unwrap_or(0);
        let file_count = index.list_files().map(|f| f.len()).unwrap_or(0);

        if format == OutputFormat::Json {
            println!(
                "{}",
                serde_json::json!({
                    "symbol_count": symbol_count,
                    "file_count": file_count,
                })
            );
        } else if !quiet {
            println!("Index Info:");
            println!("  Symbols: {}", symbol_count);
            println!("  Files: {}", file_count);
        }
    }

    Ok(exit_codes::SUCCESS)
}

/// Update the index incrementally
fn cmd_update(root: &Path, format: OutputFormat, quiet: bool) -> Result<u8> {
    let root = root
        .canonicalize()
        .context("Failed to resolve root directory")?;

    let db_path = root.join(".rocketindex").join(DEFAULT_DB_NAME);
    if !db_path.exists() {
        if format == OutputFormat::Json {
            println!(
                "{}",
                serde_json::json!({"error": "Index not found. Run 'build' first."})
            );
        } else {
            eprintln!("Index not found. Run 'rocketindex build' first.");
        }
        return Ok(exit_codes::NOT_FOUND);
    }

    let index = SqliteIndex::open(&db_path).context("Failed to open SQLite index")?;

    // Find files that have changed (simplified: just re-index all files for now)
    // TODO: Use file modification times or a proper incremental strategy
    let files = find_fsharp_files(&root)?;
    let mut updated_count = 0;

    for file in &files {
        if let Ok(source) = std::fs::read_to_string(file) {
            // Clear existing data for this file
            index.clear_file(file)?;

            let result = rocketindex::extract_symbols(file, &source);

            for symbol in &result.symbols {
                index.insert_symbol(symbol)?;
            }
            for reference in &result.references {
                index.insert_reference(file, reference)?;
            }
            for (line, open) in result.opens.iter().enumerate() {
                index.insert_open(file, open, line as u32 + 1)?;
            }
            updated_count += 1;
        }
    }

    if format == OutputFormat::Json {
        println!(
            "{}",
            serde_json::json!({
                "updated": updated_count,
                "symbols": index.count_symbols().unwrap_or(0),
            })
        );
    } else if !quiet {
        println!("Updated {} files", updated_count);
    }

    Ok(exit_codes::SUCCESS)
}

/// Find the definition of a symbol
fn cmd_def(symbol: &str, context: bool, format: OutputFormat, quiet: bool) -> Result<u8> {
    let index = load_sqlite_index()?;

    // Try exact match first
    if let Ok(Some(sym)) = index.find_by_qualified(symbol) {
        output_location(&sym, context, format, quiet)?;
        return Ok(exit_codes::SUCCESS);
    }

    // Try searching for partial matches
    if let Ok(matches) = index.search(symbol, 10) {
        if let Some(sym) = matches.first() {
            output_location(sym, context, format, quiet)?;
            return Ok(exit_codes::SUCCESS);
        }
    }

    if format == OutputFormat::Json {
        println!("{}", serde_json::json!({"error": "Symbol not found"}));
    } else {
        eprintln!("Symbol not found: {}", symbol);
    }

    Ok(exit_codes::NOT_FOUND)
}

fn output_location(
    sym: &rocketindex::Symbol,
    context: bool,
    format: OutputFormat,
    quiet: bool,
) -> Result<()> {
    let loc = &sym.location;

    if format == OutputFormat::Json {
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
    } else if !quiet {
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
fn cmd_refs(file: &Path, format: OutputFormat, quiet: bool) -> Result<u8> {
    let index = load_sqlite_index()?;
    let file = file.canonicalize().context("Failed to resolve file path")?;

    let references = index
        .references_in_file(&file)
        .context("Failed to get references")?;

    if format == OutputFormat::Json {
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
    } else if !quiet {
        for reference in references {
            // Try to resolve the reference
            if let Ok(Some(resolved)) = index.find_by_qualified(&reference.name) {
                println!(
                    "{:<40} {}:{}:{}",
                    reference.name,
                    resolved.location.file.display(),
                    resolved.location.line,
                    resolved.location.column
                );
            } else {
                println!("{:<40} <external>", reference.name);
            }
        }
    }

    Ok(exit_codes::SUCCESS)
}

/// Spider from an entry point
fn cmd_spider(symbol: &str, depth: usize, format: OutputFormat, quiet: bool) -> Result<u8> {
    // Spider still uses CodeIndex for now since it has complex resolution logic
    // TODO: Update spider to use SqliteIndex
    let index = load_code_index()?;

    // First try to find the entry point
    let entry_qualified = if index.get(symbol).is_some() {
        symbol.to_string()
    } else {
        // Try to find it via search
        let matches = index.search(symbol);
        if let Some(first) = matches.first() {
            first.qualified.clone()
        } else {
            if format == OutputFormat::Json {
                println!("{}", serde_json::json!({"error": "Entry point not found"}));
            } else {
                eprintln!("Entry point not found: {}", symbol);
            }
            return Ok(exit_codes::NOT_FOUND);
        }
    };

    let result = spider(&index, &entry_qualified, depth);

    if format == OutputFormat::Json {
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
    } else if !quiet {
        print!("{}", format_spider_result(&result));
    }

    Ok(exit_codes::SUCCESS)
}

/// Search for symbols matching a pattern
fn cmd_symbols(pattern: &str, format: OutputFormat, quiet: bool) -> Result<u8> {
    let index = load_sqlite_index()?;
    let matches = index.search(pattern, 100)?;

    if format == OutputFormat::Json {
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
    } else if !quiet {
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
fn cmd_watch(root: &Path, format: OutputFormat, quiet: bool) -> Result<u8> {
    use rocketindex::watch::FileWatcher;

    let root = root
        .canonicalize()
        .context("Failed to resolve root directory")?;

    // First, ensure index exists
    if !quiet {
        println!("Building initial index...");
    }
    cmd_build(&root, false, format, quiet)?;

    let mut watcher = FileWatcher::new(&root).context("Failed to create file watcher")?;
    watcher.start().context("Failed to start watching")?;

    println!("Watching for changes... (Ctrl+C to stop)");

    loop {
        if let Some(event) = watcher.wait() {
            match event {
                rocketindex::watch::WatchEvent::Created(path)
                | rocketindex::watch::WatchEvent::Modified(path) => {
                    if is_fsharp_file(&path) {
                        println!("Updated: {}", path.display());
                        update_single_file(&root, &path)?;
                    }
                }
                rocketindex::watch::WatchEvent::Deleted(path) => {
                    if is_fsharp_file(&path) {
                        println!("Deleted: {}", path.display());
                        remove_file_from_index(&root, &path)?;
                    }
                }
                rocketindex::watch::WatchEvent::Renamed(old, new) => {
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

/// Load the SQLite index from disk
fn load_sqlite_index() -> Result<SqliteIndex> {
    let cwd = std::env::current_dir()?;
    let db_path = cwd.join(".rocketindex").join(DEFAULT_DB_NAME);

    if !db_path.exists() {
        anyhow::bail!("Index not found. Run 'rocketindex build' first.");
    }

    SqliteIndex::open(&db_path).context("Failed to open SQLite index")
}

/// Load the CodeIndex from SQLite (for spider compatibility)
/// This creates a CodeIndex by reading from the SQLite database
fn load_code_index() -> Result<CodeIndex> {
    let cwd = std::env::current_dir()?;
    let db_path = cwd.join(".rocketindex").join(DEFAULT_DB_NAME);

    if !db_path.exists() {
        anyhow::bail!("Index not found. Run 'rocketindex build' first.");
    }

    let sqlite_index = SqliteIndex::open(&db_path).context("Failed to open SQLite index")?;

    // Get workspace root from metadata
    let workspace_root = sqlite_index
        .get_metadata("workspace_root")?
        .map(PathBuf::from)
        .unwrap_or_else(|| cwd.clone());

    let mut code_index = CodeIndex::with_root(workspace_root.clone());

    // Load file order if available
    if let Ok(Some(file_order_json)) = sqlite_index.get_metadata("file_order") {
        if let Ok(file_order) = serde_json::from_str::<Vec<PathBuf>>(&file_order_json) {
            code_index.set_file_order(file_order);
        }
    }

    // Load symbols
    for file in sqlite_index.list_files()? {
        let symbols = sqlite_index.symbols_in_file(&file)?;
        for symbol in symbols {
            code_index.add_symbol(symbol);
        }

        let references = sqlite_index.references_in_file(&file)?;
        for reference in references {
            code_index.add_reference(file.clone(), reference);
        }

        let opens = sqlite_index.opens_for_file(&file)?;
        for open in opens {
            code_index.add_open(file.clone(), open);
        }
    }

    Ok(code_index)
}

/// Get a specific line from a file
fn get_line_content(file: &PathBuf, line: usize) -> Option<String> {
    let content = std::fs::read_to_string(file).ok()?;
    content.lines().nth(line - 1).map(|s| s.to_string())
}

/// Update a single file in the index
fn update_single_file(root: &Path, file: &Path) -> Result<()> {
    let db_path = root.join(".rocketindex").join(DEFAULT_DB_NAME);
    let index = SqliteIndex::open(&db_path)?;

    index.clear_file(file)?;

    if let Ok(source) = std::fs::read_to_string(file) {
        let result = rocketindex::extract_symbols(file, &source);
        for symbol in &result.symbols {
            index.insert_symbol(symbol)?;
        }
        for reference in &result.references {
            index.insert_reference(file, reference)?;
        }
        for (line, open) in result.opens.iter().enumerate() {
            index.insert_open(file, open, line as u32 + 1)?;
        }
    }

    Ok(())
}

/// Remove a file from the index
fn remove_file_from_index(root: &Path, file: &Path) -> Result<()> {
    let db_path = root.join(".rocketindex").join(DEFAULT_DB_NAME);
    let index = SqliteIndex::open(&db_path)?;

    index.clear_file(file)?;

    Ok(())
}
