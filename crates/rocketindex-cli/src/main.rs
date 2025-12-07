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
use std::time::Instant;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use indicatif::{ProgressBar, ProgressStyle};
use rayon::prelude::*;
use rocketindex::git;
use rocketindex::{
    config::Config,
    db::DEFAULT_DB_NAME,
    find_fsproj_files, parse_fsproj,
    spider::{format_spider_result, reverse_spider, spider},
    watch::{find_source_files_with_exclusions, is_supported_file},
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

mod guidelines;
mod skills;

#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
enum OutputFormat {
    Json,
    Pretty,
    Text,
}

/// Rocket-fast F# codebase indexing and navigation tool
#[derive(Parser)]
#[command(name = "rkt")]
#[command(author, version = env!("RKT_VERSION"), about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Output format
    #[arg(long, global = true, value_enum, default_value_t = OutputFormat::Json)]
    format: OutputFormat,

    /// Suppress progress output
    #[arg(short, long, global = true)]
    quiet: bool,

    /// Use compact output (no pretty-printing, minimal fields)
    #[arg(long, global = true)]
    concise: bool,
}

#[derive(Subcommand)]
enum Commands {
    /// Index the codebase (build or rebuild the symbol database)
    Index {
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

        /// Show git provenance information (author, date, commit)
        #[arg(long)]
        git: bool,
    },

    /// Find references to a symbol or list references in a file
    Refs {
        /// File to analyze (lists all references in the file)
        #[arg(short, long, conflicts_with = "symbol")]
        file: Option<PathBuf>,

        /// Symbol to find all uses of (across entire codebase)
        #[arg(short, long, conflicts_with = "file")]
        symbol: Option<String>,

        /// Number of context lines to show around each reference
        #[arg(short, long, default_value = "0")]
        context: usize,
    },

    /// Spider from an entry point symbol
    Spider {
        /// Entry point symbol (qualified name)
        symbol: String,

        /// Maximum depth to traverse
        #[arg(short, long, default_value = "5")]
        depth: usize,

        /// Reverse spider: find callers instead of callees (impact analysis)
        #[arg(short, long)]
        reverse: bool,
    },

    /// Search for symbols matching a pattern
    Symbols {
        /// Pattern to match (supports * wildcards)
        pattern: String,

        /// Filter by language (e.g., "ruby", "fsharp")
        #[arg(short, long)]
        language: Option<String>,

        /// Use fuzzy matching (find symbols within edit distance of pattern)
        #[arg(long)]
        fuzzy: bool,
    },

    /// Find direct callers of a symbol (single-level reverse spider)
    Callers {
        /// Symbol to find callers for (qualified name)
        symbol: String,
    },

    /// Find classes that inherit from a parent class
    Subclasses {
        /// Parent class name to find subclasses of
        parent: String,
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

    /// Show git blame for a symbol or file location
    Blame {
        /// Symbol name or file:line (e.g. "src/App.fs:10")
        target: String,
    },

    /// Show git history for a symbol
    History {
        /// Symbol name
        symbol: String,
    },

    /// Check RocketIndex health and configuration
    Doctor,

    /// Set up editor integrations (slash commands, rules, etc.)
    Setup {
        /// Editor to set up: claude, cursor, vscode
        editor: String,
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

    match run(cli.command, cli.format, cli.quiet, cli.concise) {
        Ok(code) => ExitCode::from(code),
        Err(e) => {
            if cli.format == OutputFormat::Json {
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

fn run(command: Commands, format: OutputFormat, quiet: bool, concise: bool) -> Result<u8> {
    match command {
        Commands::Index {
            root,
            extract_types,
        } => cmd_index(&root, extract_types, format, quiet),
        Commands::Update { root } => cmd_update(&root, format, quiet),
        Commands::Def {
            symbol,
            context,
            git,
        } => cmd_def(&symbol, context, git, format, quiet, concise),
        Commands::Refs {
            file,
            symbol,
            context,
        } => cmd_refs(
            file.as_deref(),
            symbol.as_deref(),
            context,
            format,
            quiet,
            concise,
        ),
        Commands::Spider {
            symbol,
            depth,
            reverse,
        } => cmd_spider(&symbol, depth, reverse, format, quiet, concise),
        Commands::Symbols {
            pattern,
            language,
            fuzzy,
        } => cmd_symbols(&pattern, language.as_deref(), fuzzy, format, quiet, concise),
        Commands::Callers { symbol } => cmd_callers(&symbol, format, quiet, concise),
        Commands::Subclasses { parent } => cmd_subclasses(&parent, format, quiet, concise),
        Commands::Watch { root } => cmd_watch(&root, format, quiet),
        Commands::ExtractTypes {
            project,
            output,
            verbose,
        } => cmd_extract_types(&project, output.as_deref(), verbose, format, quiet),
        Commands::TypeInfo { symbol, members_of } => {
            cmd_type_info(symbol.as_deref(), members_of.as_deref(), format, quiet)
        }
        Commands::Blame { target } => cmd_blame(&target, format, quiet, concise),
        Commands::History { symbol } => cmd_history(&symbol, format, quiet, concise),
        Commands::Doctor => cmd_doctor(format, quiet),
        Commands::Setup { editor } => cmd_setup(&editor, format, quiet),
    }
}

/// Index the codebase using SQLite (build or rebuild)
fn cmd_index(root: &Path, extract_types: bool, format: OutputFormat, quiet: bool) -> Result<u8> {
    let root = root
        .canonicalize()
        .context("Failed to resolve root directory")?;

    // Load configuration
    let config = Config::load(&root);
    let exclude_dirs = config.excluded_dirs();

    if !quiet && !config.exclude_dirs.is_empty() {
        eprintln!("Custom exclusions: {}", config.exclude_dirs.join(", "));
    }

    let files = find_source_files_with_exclusions(&root, &exclude_dirs)
        .context("Failed to find source files")?;

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
    let max_depth = config.max_recursion_depth;

    // Create progress bar for parsing (only in non-quiet, non-JSON mode)
    let parse_progress = if !quiet && format != OutputFormat::Json {
        let pb = ProgressBar::new(files.len() as u64);
        pb.set_style(
            ProgressStyle::default_bar()
                .template("{spinner:.green} [{bar:40.cyan/blue}] {pos}/{len} files ({eta})")
                .unwrap()
                .progress_chars("#>-"),
        );
        pb.set_message("Parsing files...");
        Some(pb)
    } else {
        None
    };

    let parse_results: Vec<_> = files
        .par_iter()
        .map(|file| {
            let result = match std::fs::read_to_string(file) {
                Ok(source) => {
                    let result = rocketindex::extract_symbols(file, &source, max_depth);
                    Ok((file.clone(), result))
                }
                Err(e) => Err(format!("{}: {}", file.display(), e)),
            };
            if let Some(ref pb) = parse_progress {
                pb.inc(1);
            }
            result
        })
        .collect();

    if let Some(pb) = parse_progress {
        pb.finish_with_message("Parsing complete");
    }

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
    let mut warnings = Vec::new();

    // Collect all data for batch insertion
    let mut all_symbols = Vec::new();
    let mut all_references: Vec<(PathBuf, rocketindex::index::Reference)> = Vec::new();
    let mut all_opens: Vec<(PathBuf, String, u32)> = Vec::new();

    for result in parse_results {
        match result {
            Ok((file, parse_result)) => {
                all_symbols.extend(parse_result.symbols);

                for reference in parse_result.references {
                    all_references.push((file.clone(), reference));
                }

                for (line, open) in parse_result.opens.into_iter().enumerate() {
                    all_opens.push((file.clone(), open, line as u32 + 1));
                }

                // Collect warnings
                for warning in parse_result.warnings {
                    warnings.push(format!(
                        "{}: {} ({})",
                        file.display(),
                        warning.message,
                        warning
                            .location
                            .map(|l| format!("{}:{}", l.line, l.column))
                            .unwrap_or_else(|| "unknown location".to_string())
                    ));
                }
            }
            Err(e) => {
                errors.push(e);
            }
        }
    }

    let symbol_count = all_symbols.len();
    let ref_count = all_references.len();
    let open_count = all_opens.len();

    // Create progress bar for insertion (only in non-quiet, non-JSON mode)
    let insert_progress = if !quiet && format != OutputFormat::Json {
        let total = 3; // symbols, references, opens
        let pb = ProgressBar::new(total);
        pb.set_style(
            ProgressStyle::default_bar()
                .template("{spinner:.green} [{bar:40.cyan/blue}] {pos}/{len} ({msg})")
                .unwrap()
                .progress_chars("#>-"),
        );
        Some(pb)
    } else {
        None
    };

    // Batch insert symbols
    if let Some(ref pb) = insert_progress {
        pb.set_message(format!("Inserting {} symbols...", symbol_count));
    }
    if let Err(e) = index.insert_symbols(&all_symbols) {
        errors.push(format!("Failed to batch insert symbols: {}", e));
    }
    if let Some(ref pb) = insert_progress {
        pb.inc(1);
    }

    // Batch insert references
    if let Some(ref pb) = insert_progress {
        pb.set_message(format!("Inserting {} references...", ref_count));
    }
    let ref_tuples: Vec<_> = all_references
        .iter()
        .map(|(f, r)| (f.as_path(), r))
        .collect();
    if let Err(e) = index.insert_references(&ref_tuples) {
        errors.push(format!("Failed to batch insert references: {}", e));
    }
    if let Some(ref pb) = insert_progress {
        pb.inc(1);
    }

    // Batch insert opens
    if let Some(ref pb) = insert_progress {
        pb.set_message(format!("Inserting {} opens...", open_count));
    }
    let open_tuples: Vec<_> = all_opens
        .iter()
        .map(|(f, m, l)| (f.as_path(), m.as_str(), *l))
        .collect();
    if let Err(e) = index.insert_opens(&open_tuples) {
        errors.push(format!("Failed to batch insert opens: {}", e));
    }
    if let Some(ref pb) = insert_progress {
        pb.finish_with_message("Indexing complete");
    }

    if format == OutputFormat::Json {
        let output = serde_json::json!({
            "files": files.len(),
            "symbols": symbol_count,
            "fsproj_files": fsproj_count,
            "file_order_count": file_order.len(),
            "errors": errors,
            "warnings": warnings,
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
        if !warnings.is_empty() {
            eprintln!("Warnings:");
            for warning in &warnings[..warnings.len().min(10)] {
                eprintln!("  {}", warning);
            }
            if warnings.len() > 10 {
                eprintln!("  ... and {} more", warnings.len() - 10);
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
            eprintln!("Index not found. Run 'rkt index' first.");
        }
        return Ok(exit_codes::NOT_FOUND);
    }

    let index = SqliteIndex::open(&db_path).context("Failed to open SQLite index")?;

    // Load configuration for exclusions
    let config = Config::load(&root);
    let exclude_dirs = config.excluded_dirs();

    // Find files that have changed (simplified: just re-index all files for now)
    // TODO: Use file modification times or a proper incremental strategy
    let files = find_source_files_with_exclusions(&root, &exclude_dirs)?;
    let mut updated_count = 0;

    for file in &files {
        if let Ok(source) = std::fs::read_to_string(file) {
            // Clear existing data for this file
            index.clear_file(file)?;

            let result = rocketindex::extract_symbols(file, &source, config.max_recursion_depth);

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
fn cmd_def(
    symbol: &str,
    context: bool,
    git: bool,
    format: OutputFormat,
    quiet: bool,
    concise: bool,
) -> Result<u8> {
    let index = load_sqlite_index()?;

    // Try exact match first
    if let Ok(Some(sym)) = index.find_by_qualified(symbol) {
        output_location(&sym, context, git, format, quiet, concise)?;
        return Ok(exit_codes::SUCCESS);
    }

    // Try searching for partial matches
    if let Ok(matches) = index.search(symbol, 10, None) {
        if let Some(sym) = matches.first() {
            output_location(sym, context, git, format, quiet, concise)?;
            return Ok(exit_codes::SUCCESS);
        }
    }

    // Symbol not found - try to provide helpful suggestions
    let suggestions = index
        .suggest_similar(
            symbol,
            rocketindex::fuzzy::DEFAULT_MAX_DISTANCE,
            rocketindex::fuzzy::DEFAULT_MAX_SUGGESTIONS,
        )
        .unwrap_or_default();

    if format == OutputFormat::Json {
        let suggestion_strs: Vec<&str> = suggestions.iter().map(|s| s.value.as_str()).collect();
        let output = serde_json::json!({
            "error": "Symbol not found",
            "symbol": symbol,
            "suggestions": suggestion_strs
        });
        println!(
            "{}",
            if concise {
                serde_json::to_string(&output)?
            } else {
                serde_json::to_string_pretty(&output)?
            }
        );
    } else if !quiet {
        eprintln!("Symbol not found: {}", symbol);
        if !suggestions.is_empty() {
            eprintln!("Did you mean:");
            for suggestion in &suggestions {
                eprintln!("  {} (distance: {})", suggestion.value, suggestion.distance);
            }
        }
    }

    Ok(exit_codes::NOT_FOUND)
}

fn output_location(
    sym: &rocketindex::Symbol,
    context: bool,
    git: bool,
    format: OutputFormat,
    quiet: bool,
    concise: bool,
) -> Result<()> {
    let loc = &sym.location;

    // Get git info if requested
    let git_info = if git {
        // Assume running from workspace root, so relative path works
        git::get_blame(&loc.file, loc.line).ok()
    } else {
        None
    };

    if format == OutputFormat::Json {
        let output = if concise {
            // Concise mode: minimal fields only
            let mut output = serde_json::json!({
                "file": loc.file.display().to_string(),
                "line": loc.line,
                "column": loc.column,
            });
            if let Some(info) = git_info {
                output["git"] = serde_json::json!(info);
            }
            output
        } else {
            // Full mode: all fields
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
            if let Some(info) = git_info {
                output["git"] = serde_json::json!(info);
            }
            output
        };

        println!(
            "{}",
            if concise {
                serde_json::to_string(&output)?
            } else {
                serde_json::to_string_pretty(&output)?
            }
        );
    } else if !quiet {
        println!("{}:{}:{}", loc.file.display(), loc.line, loc.column);
        if context {
            if let Some(line_content) = get_line_content(&loc.file, loc.line as usize) {
                println!("    {}", line_content.trim());
            }
        }
        if let Some(info) = git_info {
            // Prioritize "why" over "who" - show message first
            let type_prefix = info
                .commit_type
                .as_ref()
                .map(|t| format!("[{}] ", t))
                .unwrap_or_default();
            println!(
                "    Git: {}{} ({}) by {}",
                type_prefix, info.message, info.date_relative, info.author
            );
        }
    }

    Ok(())
}

/// Find references to a symbol or list references in a file
fn cmd_refs(
    file: Option<&Path>,
    symbol: Option<&str>,
    context_lines: usize,
    format: OutputFormat,
    quiet: bool,
    concise: bool,
) -> Result<u8> {
    let index = load_sqlite_index()?;

    match (file, symbol) {
        // Symbol mode: find all uses of a symbol across the codebase
        (None, Some(sym)) => cmd_refs_symbol(&index, sym, context_lines, format, quiet, concise),
        // File mode: list all references in a file
        (Some(f), None) => cmd_refs_file(&index, f, format, quiet, concise),
        // Neither specified
        (None, None) => {
            anyhow::bail!("Either --file or --symbol must be specified");
        }
        // Both specified (shouldn't happen due to clap conflicts_with)
        (Some(_), Some(_)) => {
            anyhow::bail!("Cannot specify both --file and --symbol");
        }
    }
}

/// Find all uses of a symbol across the codebase
fn cmd_refs_symbol(
    index: &rocketindex::db::SqliteIndex,
    symbol: &str,
    context_lines: usize,
    format: OutputFormat,
    quiet: bool,
    concise: bool,
) -> Result<u8> {
    let references = index
        .find_references(symbol)
        .context("Failed to find references")?;

    if references.is_empty() {
        if format == OutputFormat::Json {
            println!("[]");
        } else if !quiet {
            eprintln!("No references found for '{}'", symbol);
        }
        return Ok(exit_codes::NOT_FOUND);
    }

    if format == OutputFormat::Json {
        let refs: Vec<_> = references
            .iter()
            .map(|r| {
                let mut obj = serde_json::json!({
                    "name": r.name,
                    "file": r.location.file.display().to_string(),
                    "line": r.location.line,
                    "column": r.location.column,
                });

                // Add context if requested
                if context_lines > 0 {
                    if let Ok(context) =
                        get_context_lines(&r.location.file, r.location.line, context_lines)
                    {
                        obj["context"] = serde_json::Value::String(context);
                    }
                }

                obj
            })
            .collect();

        println!(
            "{}",
            if concise {
                serde_json::to_string(&refs)?
            } else {
                serde_json::to_string_pretty(&refs)?
            }
        );
    } else if !quiet {
        println!("References to '{}' ({} found):", symbol, references.len());
        println!();

        for reference in &references {
            println!(
                "  {}:{}:{}",
                reference.location.file.display(),
                reference.location.line,
                reference.location.column
            );

            if context_lines > 0 {
                if let Ok(context) = get_context_lines(
                    &reference.location.file,
                    reference.location.line,
                    context_lines,
                ) {
                    for (i, line) in context.lines().enumerate() {
                        let line_num =
                            reference.location.line as i64 - context_lines as i64 + i as i64;
                        if line_num > 0 {
                            let marker = if line_num == reference.location.line as i64 {
                                ">"
                            } else {
                                " "
                            };
                            println!("    {} {:4} | {}", marker, line_num, line);
                        }
                    }
                    println!();
                }
            }
        }
    }

    Ok(exit_codes::SUCCESS)
}

/// List all references in a file
fn cmd_refs_file(
    index: &rocketindex::db::SqliteIndex,
    file: &Path,
    format: OutputFormat,
    quiet: bool,
    concise: bool,
) -> Result<u8> {
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
        println!(
            "{}",
            if concise {
                serde_json::to_string(&refs)?
            } else {
                serde_json::to_string_pretty(&refs)?
            }
        );
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

/// Get context lines around a specific line in a file
fn get_context_lines(file: &Path, line: u32, context: usize) -> Result<String> {
    let content = std::fs::read_to_string(file)?;
    let lines: Vec<&str> = content.lines().collect();

    let line_idx = line.saturating_sub(1) as usize;
    let start = line_idx.saturating_sub(context);
    let end = (line_idx + context + 1).min(lines.len());

    Ok(lines[start..end].join("\n"))
}

/// Spider from an entry point
fn cmd_spider(
    symbol: &str,
    depth: usize,
    reverse: bool,
    format: OutputFormat,
    quiet: bool,
    concise: bool,
) -> Result<u8> {
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
            // Get fuzzy suggestions from all symbol names (short and qualified)
            let all_names = index.all_names_for_fuzzy();
            let suggestions = rocketindex::fuzzy::find_similar(
                symbol,
                all_names.iter().map(|s| s.as_str()),
                rocketindex::fuzzy::DEFAULT_MAX_DISTANCE,
                rocketindex::fuzzy::DEFAULT_MAX_SUGGESTIONS,
            );

            if format == OutputFormat::Json {
                let suggestion_strs: Vec<&str> =
                    suggestions.iter().map(|s| s.value.as_str()).collect();
                println!(
                    "{}",
                    serde_json::json!({
                        "error": "Entry point not found",
                        "symbol": symbol,
                        "suggestions": suggestion_strs
                    })
                );
            } else {
                eprintln!("Entry point not found: {}", symbol);
                if !suggestions.is_empty() {
                    eprintln!("Did you mean:");
                    for s in &suggestions {
                        eprintln!("  {} (distance: {})", s.value, s.distance);
                    }
                }
            }
            return Ok(exit_codes::NOT_FOUND);
        }
    };

    let result = if reverse {
        reverse_spider(&index, &entry_qualified, depth)
    } else {
        spider(&index, &entry_qualified, depth)
    };

    if format == OutputFormat::Json {
        let nodes: Vec<_> = result
            .nodes
            .iter()
            .map(|n| {
                if concise {
                    // Concise mode: minimal fields
                    serde_json::json!({
                        "qualified": n.symbol.qualified,
                        "depth": n.depth,
                    })
                } else {
                    serde_json::json!({
                        "name": n.symbol.name,
                        "qualified": n.symbol.qualified,
                        "file": n.symbol.location.file.display().to_string(),
                        "line": n.symbol.location.line,
                        "column": n.symbol.location.column,
                        "depth": n.depth,
                    })
                }
            })
            .collect();

        let output = serde_json::json!({
            "nodes": nodes,
            "unresolved": result.unresolved,
        });
        println!(
            "{}",
            if concise {
                serde_json::to_string(&output)?
            } else {
                serde_json::to_string_pretty(&output)?
            }
        );
    } else if !quiet {
        print!("{}", format_spider_result(&result));
    }

    Ok(exit_codes::SUCCESS)
}

/// Find direct callers of a symbol (single-level reverse spider)
fn cmd_callers(symbol: &str, format: OutputFormat, quiet: bool, concise: bool) -> Result<u8> {
    let index = load_code_index()?;

    // First try to find the symbol
    let qualified = if index.get(symbol).is_some() {
        symbol.to_string()
    } else {
        let matches = index.search(symbol);
        if let Some(first) = matches.first() {
            first.qualified.clone()
        } else {
            // Get fuzzy suggestions (short and qualified names)
            let all_names = index.all_names_for_fuzzy();
            let suggestions = rocketindex::fuzzy::find_similar(
                symbol,
                all_names.iter().map(|s| s.as_str()),
                rocketindex::fuzzy::DEFAULT_MAX_DISTANCE,
                rocketindex::fuzzy::DEFAULT_MAX_SUGGESTIONS,
            );

            if format == OutputFormat::Json {
                let suggestion_strs: Vec<&str> =
                    suggestions.iter().map(|s| s.value.as_str()).collect();
                println!(
                    "{}",
                    serde_json::json!({
                        "error": "Symbol not found",
                        "symbol": symbol,
                        "suggestions": suggestion_strs
                    })
                );
            } else {
                eprintln!("Symbol not found: {}", symbol);
                if !suggestions.is_empty() {
                    eprintln!("Did you mean:");
                    for s in &suggestions {
                        eprintln!("  {} (distance: {})", s.value, s.distance);
                    }
                }
            }
            return Ok(exit_codes::NOT_FOUND);
        }
    };

    // Use reverse_spider with depth=1 for single-level callers
    let result = reverse_spider(&index, &qualified, 1);

    // Filter to only show callers (depth=1), not the symbol itself (depth=0)
    let callers: Vec<_> = result.nodes.iter().filter(|n| n.depth == 1).collect();

    if format == OutputFormat::Json {
        let caller_list: Vec<_> = callers
            .iter()
            .map(|n| {
                if concise {
                    serde_json::json!({
                        "qualified": n.symbol.qualified,
                        "file": n.symbol.location.file.display().to_string(),
                        "line": n.symbol.location.line,
                    })
                } else {
                    serde_json::json!({
                        "name": n.symbol.name,
                        "qualified": n.symbol.qualified,
                        "kind": format!("{}", n.symbol.kind),
                        "file": n.symbol.location.file.display().to_string(),
                        "line": n.symbol.location.line,
                        "column": n.symbol.location.column,
                    })
                }
            })
            .collect();

        let output = serde_json::json!({
            "symbol": qualified,
            "callers": caller_list,
        });
        println!(
            "{}",
            if concise {
                serde_json::to_string(&output)?
            } else {
                serde_json::to_string_pretty(&output)?
            }
        );
    } else if !quiet {
        if callers.is_empty() {
            println!("No callers found for: {}", qualified);
        } else {
            println!("Callers of {}:", qualified);
            for caller in callers {
                println!(
                    "  {} ({}:{})",
                    caller.symbol.qualified,
                    caller.symbol.location.file.display(),
                    caller.symbol.location.line
                );
            }
        }
    }

    Ok(exit_codes::SUCCESS)
}

/// Find classes that inherit from a parent class
fn cmd_subclasses(parent: &str, format: OutputFormat, quiet: bool, concise: bool) -> Result<u8> {
    let cwd = std::env::current_dir()?;
    let db_path = cwd.join(".rocketindex").join(DEFAULT_DB_NAME);
    if !db_path.exists() {
        if format == OutputFormat::Json {
            println!(
                "{}",
                serde_json::json!({
                    "error": "IndexNotFound",
                    "message": "No index found. Run 'rkt index' first."
                })
            );
        } else {
            eprintln!("No index found. Run 'rkt index' first.");
        }
        return Ok(exit_codes::ERROR);
    }

    let db = SqliteIndex::open(&db_path)?;
    let subclasses = db.find_subclasses(parent)?;

    if format == OutputFormat::Json {
        let subclass_list: Vec<_> = subclasses
            .iter()
            .map(|s| {
                if concise {
                    serde_json::json!({
                        "qualified": s.qualified,
                        "file": s.location.file.display().to_string(),
                        "line": s.location.line,
                    })
                } else {
                    serde_json::json!({
                        "name": s.name,
                        "qualified": s.qualified,
                        "kind": format!("{}", s.kind),
                        "file": s.location.file.display().to_string(),
                        "line": s.location.line,
                        "column": s.location.column,
                        "parent": s.parent,
                    })
                }
            })
            .collect();

        let output = serde_json::json!({
            "parent": parent,
            "subclasses": subclass_list,
            "count": subclasses.len(),
        });
        println!(
            "{}",
            if concise {
                serde_json::to_string(&output)?
            } else {
                serde_json::to_string_pretty(&output)?
            }
        );
    } else if !quiet {
        if subclasses.is_empty() {
            println!("No subclasses found for: {}", parent);
        } else {
            println!("Subclasses of {} ({} found):", parent, subclasses.len());
            for s in &subclasses {
                println!(
                    "  {} ({}:{})",
                    s.qualified,
                    s.location.file.display(),
                    s.location.line
                );
            }
        }
    }

    if subclasses.is_empty() {
        Ok(exit_codes::NOT_FOUND)
    } else {
        Ok(exit_codes::SUCCESS)
    }
}

/// Search for symbols matching a pattern
fn cmd_symbols(
    pattern: &str,
    language: Option<&str>,
    fuzzy: bool,
    format: OutputFormat,
    quiet: bool,
    concise: bool,
) -> Result<u8> {
    let index = load_sqlite_index()?;

    if fuzzy {
        // Fuzzy search mode - find symbols within edit distance
        let matches = index.fuzzy_search(
            pattern,
            rocketindex::fuzzy::DEFAULT_MAX_DISTANCE,
            100,
            language,
        )?;

        if format == OutputFormat::Json {
            let symbols: Vec<_> = matches
                .iter()
                .map(|(s, distance)| {
                    if concise {
                        serde_json::json!({
                            "qualified": s.qualified,
                            "file": s.location.file.display().to_string(),
                            "line": s.location.line,
                        })
                    } else {
                        serde_json::json!({
                            "name": s.name,
                            "qualified": s.qualified,
                            "kind": format!("{}", s.kind),
                            "file": s.location.file.display().to_string(),
                            "line": s.location.line,
                            "column": s.location.column,
                            "distance": distance,
                        })
                    }
                })
                .collect();
            println!(
                "{}",
                if concise {
                    serde_json::to_string(&symbols)?
                } else {
                    serde_json::to_string_pretty(&symbols)?
                }
            );
        } else if !quiet {
            for (sym, distance) in matches {
                println!(
                    "{:<40} {}:{}:{:<8} {} (distance: {})",
                    sym.qualified,
                    sym.location.file.display(),
                    sym.location.line,
                    sym.location.column,
                    sym.kind,
                    distance
                );
            }
        }
    } else {
        // Standard pattern search
        let matches = index.search(pattern, 100, language)?;

        if format == OutputFormat::Json {
            let symbols: Vec<_> = matches
                .iter()
                .map(|s| {
                    if concise {
                        serde_json::json!({
                            "qualified": s.qualified,
                            "file": s.location.file.display().to_string(),
                            "line": s.location.line,
                        })
                    } else {
                        serde_json::json!({
                            "name": s.name,
                            "qualified": s.qualified,
                            "kind": format!("{}", s.kind),
                            "file": s.location.file.display().to_string(),
                            "line": s.location.line,
                            "column": s.location.column,
                        })
                    }
                })
                .collect();
            println!(
                "{}",
                if concise {
                    serde_json::to_string(&symbols)?
                } else {
                    serde_json::to_string_pretty(&symbols)?
                }
            );
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
    cmd_index(&root, false, format, quiet)?;

    // Load config for recursion depth
    let config = Config::load(&root);
    let max_depth = config.max_recursion_depth;

    let mut watcher = FileWatcher::new(&root).context("Failed to create file watcher")?;
    watcher.start().context("Failed to start watching")?;

    println!("Watching for changes... (Ctrl+C to stop)");

    loop {
        if let Some(event) = watcher.wait() {
            match event {
                rocketindex::watch::WatchEvent::Created(path)
                | rocketindex::watch::WatchEvent::Modified(path) => {
                    if is_supported_file(&path) {
                        println!("Updated: {}", path.display());
                        update_single_file(&root, &path, max_depth)?;
                    }
                }
                rocketindex::watch::WatchEvent::Deleted(path) => {
                    if is_supported_file(&path) {
                        println!("Deleted: {}", path.display());
                        remove_file_from_index(&root, &path)?;
                    }
                }
                rocketindex::watch::WatchEvent::Renamed(old, new) => {
                    if is_supported_file(&old) || is_supported_file(&new) {
                        println!("Renamed: {} -> {}", old.display(), new.display());
                        remove_file_from_index(&root, &old)?;
                        if is_supported_file(&new) {
                            update_single_file(&root, &new, max_depth)?;
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
        anyhow::bail!("Index not found. Run 'rkt index' first.");
    }

    SqliteIndex::open(&db_path).context("Failed to open SQLite index")
}

/// Load the CodeIndex from SQLite (for spider compatibility)
/// This creates a CodeIndex by reading from the SQLite database
fn load_code_index() -> Result<CodeIndex> {
    let cwd = std::env::current_dir()?;
    let db_path = cwd.join(".rocketindex").join(DEFAULT_DB_NAME);

    if !db_path.exists() {
        anyhow::bail!("Index not found. Run 'rkt index' first.");
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
fn update_single_file(root: &Path, file: &Path, max_depth: usize) -> Result<()> {
    let db_path = root.join(".rocketindex").join(DEFAULT_DB_NAME);
    let index = SqliteIndex::open(&db_path)?;

    index.clear_file(file)?;

    if let Ok(source) = std::fs::read_to_string(file) {
        let result = rocketindex::extract_symbols(file, &source, max_depth);
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

/// Show git blame for a symbol or file location
fn cmd_blame(target: &str, format: OutputFormat, quiet: bool, _concise: bool) -> Result<u8> {
    // Check if target is file:line
    let (file, line) = if let Some((f, l)) = target.rsplit_once(':') {
        if let Ok(line_num) = l.parse::<u32>() {
            (PathBuf::from(f), line_num)
        } else {
            // Not a line number, treat as symbol
            resolve_symbol_location(target)?
        }
    } else {
        // Treat as symbol
        resolve_symbol_location(target)?
    };

    let info = git::get_blame(&file, line)?;

    if format == OutputFormat::Json {
        println!("{}", serde_json::to_string_pretty(&info)?);
    } else if !quiet {
        println!("Blame for {}:{}", file.display(), line);
        // Prioritize "why" and "when" over "who"
        let type_str = info
            .commit_type
            .as_ref()
            .map(|t| format!(" [{}]", t))
            .unwrap_or_default();
        println!("  Message: {}{}", info.message, type_str);
        println!("  When:    {} ({})", info.date_relative, info.date);
        println!("  Commit:  {}", info.commit);
        println!("  Author:  {}", info.author);
    }

    Ok(exit_codes::SUCCESS)
}

fn resolve_symbol_location(symbol: &str) -> Result<(PathBuf, u32)> {
    let index = load_sqlite_index()?;

    // Try exact match
    if let Ok(Some(sym)) = index.find_by_qualified(symbol) {
        return Ok((sym.location.file.clone(), sym.location.line));
    }

    // Try partial match
    if let Ok(matches) = index.search(symbol, 1, None) {
        if let Some(sym) = matches.first() {
            return Ok((sym.location.file.clone(), sym.location.line));
        }
    }

    anyhow::bail!("Symbol not found: {}", symbol)
}

/// Show git history for a symbol
fn cmd_history(symbol: &str, format: OutputFormat, quiet: bool, _concise: bool) -> Result<u8> {
    let index = load_sqlite_index()?;

    let sym = if let Ok(Some(s)) = index.find_by_qualified(symbol) {
        s
    } else if let Ok(matches) = index.search(symbol, 1, None) {
        if let Some(s) = matches.first() {
            s.clone()
        } else {
            anyhow::bail!("Symbol not found: {}", symbol);
        }
    } else {
        anyhow::bail!("Symbol not found: {}", symbol);
    };

    let history = git::get_history(&sym.location.file, sym.location.line, sym.location.end_line)?;

    if format == OutputFormat::Json {
        println!("{}", serde_json::to_string_pretty(&history)?);
    } else if !quiet {
        println!(
            "History for {} ({}:{}):",
            sym.qualified,
            sym.location.file.display(),
            sym.location.line
        );
        for info in history {
            // Truncate commit hash to 7 chars
            let short_hash = if info.commit.len() > 7 {
                &info.commit[..7]
            } else {
                &info.commit
            };
            // Format: why | when | reference (author omitted - often "Claude" now)
            let type_prefix = info
                .commit_type
                .as_ref()
                .map(|t| format!("[{}] ", t))
                .unwrap_or_default();
            println!(
                "  {} | {} | {}{}",
                short_hash, info.date_relative, type_prefix, info.message
            );
        }
    }

    Ok(exit_codes::SUCCESS)
}

/// Check RocketIndex health and configuration
fn cmd_doctor(format: OutputFormat, quiet: bool) -> Result<u8> {
    let cwd = std::env::current_dir()?;
    let mut checks: Vec<(&str, bool, String)> = Vec::new();
    let mut suggestions: Vec<String> = Vec::new();

    // Check 1: Index exists
    let index_dir = cwd.join(".rocketindex");
    let db_path = index_dir.join(DEFAULT_DB_NAME);
    let index_exists = db_path.exists();

    if index_exists {
        checks.push(("Index", true, format!("{}", db_path.display())));
    } else {
        checks.push(("Index", false, "Not found".to_string()));
        suggestions.push("Run 'rkt index' to create the index".to_string());
    }

    // Check 2: Symbol and file counts (if index exists)
    let (symbol_count, file_count) = if index_exists {
        if let Ok(index) = SqliteIndex::open(&db_path) {
            let symbols = index.count_symbols().unwrap_or(0);
            let files = index.list_files().map(|f| f.len()).unwrap_or(0);
            (symbols, files)
        } else {
            (0, 0)
        }
    } else {
        (0, 0)
    };

    if index_exists {
        checks.push((
            "Symbols",
            symbol_count > 0,
            format!("{} symbols indexed", symbol_count),
        ));
        checks.push((
            "Files",
            file_count > 0,
            format!("{} files indexed", file_count),
        ));

        if symbol_count == 0 {
            suggestions.push(
                "No symbols found. Check that source files exist and are supported.".to_string(),
            );
        }
    }

    // Check 3: Git repository (informational - not required)
    let is_git_repo = git::is_git_repo();
    checks.push((
        "Git",
        true,
        if is_git_repo {
            "Repository detected".to_string()
        } else {
            "Not a git repository (blame/history unavailable)".to_string()
        },
    ));

    // Check 4: .fsproj files (informational - not a failure if 0)
    let fsproj_files = find_fsproj_files(&cwd);
    let fsproj_count = fsproj_files.len();
    checks.push((
        "F# Projects",
        true,
        format!("{} .fsproj file(s)", fsproj_count),
    ));

    // Check 5: Configuration file (informational - defaults are fine)
    let config_path = cwd.join(".rocketindex.toml");
    let config_exists = config_path.exists();
    checks.push((
        "Config",
        true,
        if config_exists {
            ".rocketindex.toml found".to_string()
        } else {
            "Using defaults".to_string()
        },
    ));

    // Check 6: Supported languages (based on file extensions in index)
    if index_exists {
        if let Ok(index) = SqliteIndex::open(&db_path) {
            if let Ok(files) = index.list_files() {
                let mut languages: std::collections::HashSet<&str> =
                    std::collections::HashSet::new();
                for file in &files {
                    if let Some(ext) = file.extension().and_then(|e| e.to_str()) {
                        match ext {
                            "fs" | "fsi" | "fsx" => {
                                languages.insert("F#");
                            }
                            "rb" => {
                                languages.insert("Ruby");
                            }
                            "cs" => {
                                languages.insert("C#");
                            }
                            _ => {}
                        }
                    }
                }
                if !languages.is_empty() {
                    let lang_list: Vec<_> = languages.into_iter().collect();
                    checks.push(("Languages", true, lang_list.join(", ")));
                }
            }
        }
    }

    // Output results
    if format == OutputFormat::Json {
        let check_list: Vec<_> = checks
            .iter()
            .map(|(name, ok, msg)| {
                serde_json::json!({
                    "check": name,
                    "ok": ok,
                    "message": msg
                })
            })
            .collect();

        let output = serde_json::json!({
            "checks": check_list,
            "suggestions": suggestions,
            "healthy": checks.iter().all(|(_, ok, _)| *ok)
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else if !quiet {
        println!("RocketIndex Health Check\n");

        for (name, ok, msg) in &checks {
            let status = if *ok { "✓" } else { "✗" };
            println!("  {} {}: {}", status, name, msg);
        }

        if !suggestions.is_empty() {
            println!("\nSuggestions:");
            for suggestion in &suggestions {
                println!("  • {}", suggestion);
            }
        }

        println!();
        if checks.iter().all(|(_, ok, _)| *ok) {
            println!("All checks passed!");
        } else {
            println!("Some checks failed. See suggestions above.");
        }
    }

    Ok(exit_codes::SUCCESS)
}

/// Set up editor integrations
fn cmd_setup(editor: &str, format: OutputFormat, quiet: bool) -> Result<u8> {
    let cwd = std::env::current_dir()?;

    match editor.to_lowercase().as_str() {
        "claude" | "claude-code" => setup_claude_code(&cwd, format, quiet),
        "cursor" => setup_cursor(&cwd, format, quiet),
        "copilot" | "github-copilot" => setup_copilot(&cwd, format, quiet),
        "vscode" => {
            if format == OutputFormat::Json {
                println!(
                    "{}",
                    serde_json::json!({"error": "VSCode setup not yet implemented"})
                );
            } else {
                eprintln!("VSCode setup not yet implemented. Coming soon!");
            }
            Ok(exit_codes::NOT_FOUND)
        }
        _ => {
            if format == OutputFormat::Json {
                println!(
                    "{}",
                    serde_json::json!({
                        "error": "Unknown editor",
                        "supported": ["claude", "cursor", "copilot", "vscode"]
                    })
                );
            } else {
                eprintln!("Unknown editor: {}", editor);
                eprintln!("Supported editors: claude, cursor, copilot, vscode");
            }
            Ok(exit_codes::ERROR)
        }
    }
}

/// Detect the primary programming language of a project by counting file extensions
#[allow(dead_code)]
fn detect_primary_language(cwd: &Path) -> Option<String> {
    use std::collections::HashMap;

    let mut counts: HashMap<&str, usize> = HashMap::new();

    // Walk the directory (shallow, skip hidden dirs and common non-source dirs)
    if let Ok(entries) = std::fs::read_dir(cwd) {
        for entry in entries.flatten() {
            let path = entry.path();

            // Skip hidden directories and common non-source directories
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if name.starts_with('.')
                    || name == "node_modules"
                    || name == "target"
                    || name == "vendor"
                    || name == "dist"
                    || name == "build"
                {
                    continue;
                }
            }

            // Count files by extension (recursive but limited)
            count_extensions(&path, &mut counts, 3);
        }
    }

    // Map extensions to language names
    let language_map: HashMap<&str, &str> = [
        ("rs", "Rust"),
        ("fs", "F#"),
        ("fsx", "F#"),
        ("rb", "Ruby"),
        ("ts", "TypeScript"),
        ("tsx", "TypeScript"),
        ("py", "Python"),
        ("go", "Go"),
    ]
    .into_iter()
    .collect();

    // Find the dominant language
    let mut language_counts: HashMap<&str, usize> = HashMap::new();
    for (ext, count) in &counts {
        if let Some(lang) = language_map.get(ext) {
            *language_counts.entry(lang).or_default() += count;
        }
    }

    language_counts
        .into_iter()
        .max_by_key(|(_, count)| *count)
        .filter(|(_, count)| *count >= 3) // Require at least 3 files
        .map(|(lang, _)| lang.to_string())
}

/// Recursively count file extensions up to a certain depth
#[allow(dead_code)]
fn count_extensions(
    path: &Path,
    counts: &mut std::collections::HashMap<&'static str, usize>,
    depth: usize,
) {
    if depth == 0 {
        return;
    }

    if path.is_file() {
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            // Map to static str for the extensions we care about
            let static_ext: Option<&'static str> = match ext {
                "rs" => Some("rs"),
                "fs" => Some("fs"),
                "fsx" => Some("fsx"),
                "rb" => Some("rb"),
                "ts" => Some("ts"),
                "tsx" => Some("tsx"),
                "py" => Some("py"),
                "go" => Some("go"),
                _ => None,
            };
            if let Some(e) = static_ext {
                *counts.entry(e).or_default() += 1;
            }
        }
    } else if path.is_dir() {
        if let Ok(entries) = std::fs::read_dir(path) {
            for entry in entries.flatten() {
                let entry_path = entry.path();
                // Skip hidden dirs
                if let Some(name) = entry_path.file_name().and_then(|n| n.to_str()) {
                    if name.starts_with('.') {
                        continue;
                    }
                }
                count_extensions(&entry_path, counts, depth - 1);
            }
        }
    }
}

fn should_show_index_feedback(format: OutputFormat, quiet: bool) -> bool {
    if quiet {
        return false;
    }

    if format == OutputFormat::Json {
        return dialoguer::console::Term::stderr().is_term();
    }

    true
}

// =============================================================================
// Setup Wizard Screens
// =============================================================================

/// Screen 1: Welcome
fn setup_screen_welcome() -> Result<()> {
    println!(
        r#"
RocketIndex Setup for Claude Code
══════════════════════════════════

RocketIndex gives AI agents fast, indexed code navigation - the same
"go to definition" and "find callers" you have in your IDE, but via CLI.

This setup will:
  1. Index your codebase (build symbol database)
  2. Configure agents (optional)
  3. Create .rocketindex/AGENTS.md (command reference)
  4. Update CLAUDE.md (project instructions)

Press Enter to continue..."#
    );

    let _ = dialoguer::Input::<String>::new()
        .allow_empty(true)
        .interact_text()?;

    Ok(())
}

/// Screen 2: Code Indexing - returns true if indexing was performed
fn setup_screen_indexing(cwd: &Path, format: OutputFormat, quiet: bool) -> Result<bool> {
    use dialoguer::Confirm;

    let index_path = cwd.join(".rocketindex").join(DEFAULT_DB_NAME);
    if index_path.exists() {
        // Index already exists, skip this screen
        return Ok(false);
    }

    println!(
        r#"
Code Indexing
─────────────

RocketIndex will scan your codebase to build a symbol database for
fast code navigation. This enables `rkt def`, `rkt callers`, `rkt spider`,
and other commands.

What it does:
  • Parses source files to extract symbols (functions, classes, types)
  • Creates .rocketindex/index.db (add to .gitignore)
  • Respects .gitignore - ignored files are not indexed

Supported languages: F#, Ruby, Python, Rust, Go, TypeScript, JavaScript

Estimated time: ~1-3 seconds per 1,000 files
"#
    );

    let proceed = Confirm::new()
        .with_prompt("Proceed with indexing?")
        .default(true)
        .interact_opt()?;

    if proceed != Some(true) {
        println!("\nSkipping indexing. Run `rkt index` later to build the index.\n");
        return Ok(false);
    }

    let started = Instant::now();
    println!("\nIndexing codebase...");

    match cmd_index(cwd, false, format, quiet) {
        Ok(code) if code == exit_codes::SUCCESS => {
            if !quiet {
                println!("Indexed in {:.1?}\n", started.elapsed());
            }
            println!("Press Enter to continue...");
            let _ = dialoguer::Input::<String>::new()
                .allow_empty(true)
                .interact_text()?;
            Ok(true)
        }
        Ok(code) => anyhow::bail!("Indexing failed (exit code {})", code),
        Err(err) => Err(err),
    }
}

/// Agent setup choice
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AgentSetupChoice {
    InstallAgents,
    IntegrationNotes,
    Skip,
}

/// Screen 3: Agent Setup
fn setup_screen_agents(cwd: &Path, quiet: bool) -> Result<Vec<String>> {
    use dialoguer::{MultiSelect, Select};

    let mut created_files = Vec::new();
    let skills_dir = cwd.join(".claude").join("skills");

    println!(
        r#"
Agent Setup
───────────

RocketIndex includes role-based agents that help Claude Code work more
effectively. Each agent has domain expertise and knows how to use `rkt`
for code navigation.

How would you like to configure agents?
"#
    );

    let choices = &[
        "Install RocketIndex agents (Lead Engineer, QA, Security, SRE, Product Manager)",
        "Add RocketIndex to an existing/alternate agent library",
        "Skip agent setup",
    ];

    let selection = Select::new().items(choices).default(0).interact_opt()?;

    let choice = match selection {
        Some(0) => AgentSetupChoice::InstallAgents,
        Some(1) => AgentSetupChoice::IntegrationNotes,
        _ => AgentSetupChoice::Skip,
    };

    match choice {
        AgentSetupChoice::InstallAgents => {
            // Always install rocketindex agent first
            if let Some(rocketindex_agent) = skills::AGENTS.iter().find(|a| a.name == "rocketindex")
            {
                let agent_dir = skills_dir.join(rocketindex_agent.name);
                std::fs::create_dir_all(&agent_dir)?;
                let agent_path = agent_dir.join("SKILL.md");
                std::fs::write(&agent_path, rocketindex_agent.content)?;
                created_files.push(agent_path.display().to_string());
            }

            println!(
                r#"
Select Agents to Install
────────────────────────

Use Space to toggle, Enter to confirm.
"#
            );

            let optional_agents: Vec<_> = skills::AGENTS
                .iter()
                .filter(|a| a.name != "rocketindex")
                .collect();

            let items: Vec<String> = optional_agents
                .iter()
                .map(|a| format!("{:<18} {}", a.display_name, a.description))
                .collect();

            let defaults: Vec<bool> = vec![true; items.len()];
            let selections = MultiSelect::new()
                .with_prompt("Select agents")
                .items(&items)
                .defaults(&defaults)
                .interact_opt()?;

            if let Some(selected) = selections {
                println!("\nInstalling agents...");
                for idx in selected {
                    let agent = optional_agents[idx];
                    let agent_dir = skills_dir.join(agent.name);
                    std::fs::create_dir_all(&agent_dir)?;

                    let agent_path = agent_dir.join("SKILL.md");
                    std::fs::write(&agent_path, agent.content)?;
                    created_files.push(agent_path.display().to_string());

                    if !quiet {
                        println!("  * .claude/skills/{}/SKILL.md", agent.name);
                    }
                }
            }
        }

        AgentSetupChoice::IntegrationNotes => {
            println!(
                r#"
Add RocketIndex to Your Agents
──────────────────────────────

Add the following to the TOP of each agent file, right after the title.
Choose the snippet that matches each agent's role:

+-- For Engineering/Coding Agents ------------------------------------+
|                                                                     |
| > **Code Navigation**: Use `rkt` for code lookups.                  |
| > - Before writing: `rkt symbols "pattern*"` to find existing code  |
| > - Before changing: `rkt callers "Symbol"` for impact analysis     |
| > - Run `rkt watch` in a background terminal.                       |
| > See `.rocketindex/AGENTS.md` for full command reference.          |
|                                                                     |
+---------------------------------------------------------------------+

+-- For QA/Testing Agents --------------------------------------------+
|                                                                     |
| > **Code Navigation**: Use `rkt` for finding tests and usages.      |
| > - Find tests: `rkt symbols "*Test*"`                              |
| > - Find usages: `rkt refs "Symbol"`                                |
| > - Run `rkt watch` in a background terminal.                       |
| > See `.rocketindex/AGENTS.md` for full command reference.          |
|                                                                     |
+---------------------------------------------------------------------+

+-- For Security/Review Agents ---------------------------------------+
|                                                                     |
| > **Code Navigation**: Use `rkt` for tracing data flow.             |
| > - Trace paths: `rkt spider "handler" -d 3`                        |
| > - Find sensitive code: `rkt symbols "*password*"`                 |
| > - Run `rkt watch` in a background terminal.                       |
| > See `.rocketindex/AGENTS.md` for full command reference.          |
|                                                                     |
+---------------------------------------------------------------------+

+-- For SRE/Debugging Agents -----------------------------------------+
|                                                                     |
| > **Code Navigation**: Use `rkt` for stacktrace analysis.           |
| > - Trace errors: `rkt spider "failingFn" --reverse -d 3`           |
| > - Find error types: `rkt symbols "*Error*"`                       |
| > - Run `rkt watch` in a background terminal.                       |
| > See `.rocketindex/AGENTS.md` for full command reference.          |
|                                                                     |
+---------------------------------------------------------------------+

Press Enter to continue..."#
            );

            let _ = dialoguer::Input::<String>::new()
                .allow_empty(true)
                .interact_text()?;
        }

        AgentSetupChoice::Skip => {
            // Do nothing, proceed to configuration
        }
    }

    Ok(created_files)
}

/// Screen 4: Configuration Files
fn setup_screen_configuration(
    cwd: &Path,
    _format: OutputFormat,
    _quiet: bool,
    created_files: &mut Vec<String>,
) -> Result<()> {
    println!(
        r#"
Configuration Files
───────────────────

Creating project configuration...
"#
    );

    // Create/update .rocketindex/AGENTS.md
    let agents_md_path = cwd.join(".rocketindex").join("AGENTS.md");
    if let Some(parent) = agents_md_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let agents_section = skills::get_agents_summary();
    let agents_content = std::fs::read_to_string(&agents_md_path).unwrap_or_default();

    if !agents_content.contains("RocketIndex") {
        if agents_content.is_empty() {
            let new_content = format!("# Agent Instructions\n\n{}", agents_section);
            std::fs::write(&agents_md_path, new_content)?;
            created_files.push(agents_md_path.display().to_string());
        } else {
            let updated = format!("{}\n\n{}", agents_content.trim_end(), agents_section);
            std::fs::write(&agents_md_path, updated)?;
        }
    }
    println!("  * .rocketindex/AGENTS.md    Command reference for AI agents");

    // Update CLAUDE.md if it exists
    let claude_md_path = cwd.join("CLAUDE.md");
    if claude_md_path.exists() {
        let claude_content = std::fs::read_to_string(&claude_md_path).unwrap_or_default();
        let rocketindex_note = "**Note**: This project uses [RocketIndex](https://github.com/rocket-tycoon/rocket-index) for code navigation.\n   For definitions, callers, and dependencies use `rkt`. See `.rocketindex/AGENTS.md` for commands.\n";

        if !claude_content.contains("RocketIndex") {
            let updated = if let Some(pos) = claude_content.find("\n\n") {
                format!(
                    "{}\n\n{}\n{}",
                    &claude_content[..pos],
                    rocketindex_note,
                    &claude_content[pos + 2..]
                )
            } else {
                format!("{}\n\n{}", claude_content, rocketindex_note)
            };
            std::fs::write(&claude_md_path, updated)?;
        }
        println!("  * CLAUDE.md                 Updated with RocketIndex note");
    }

    // Update .github/copilot-instructions.md if it exists
    let copilot_path = cwd.join(".github").join("copilot-instructions.md");
    if copilot_path.exists() {
        let copilot_content = std::fs::read_to_string(&copilot_path).unwrap_or_default();
        let rocketindex_note = "**Note**: This project uses [RocketIndex](https://github.com/rocket-tycoon/rocket-index) for code navigation.\n   For definitions, callers, and dependencies use `rkt`. See `.rocketindex/AGENTS.md` for commands.\n";

        if !copilot_content.contains("RocketIndex") {
            let updated = if let Some(pos) = copilot_content.find("\n\n") {
                format!(
                    "{}\n\n{}\n{}",
                    &copilot_content[..pos],
                    rocketindex_note,
                    &copilot_content[pos + 2..]
                )
            } else {
                format!("{}\n\n{}", copilot_content, rocketindex_note)
            };
            std::fs::write(&copilot_path, updated)?;
            println!("  * .github/copilot-instructions.md");
        }
    }

    // Add to .gitignore
    let gitignore_path = cwd.join(".gitignore");
    let gitignore_entry = ".rocketindex/index.db";
    if gitignore_path.exists() {
        let content = std::fs::read_to_string(&gitignore_path).unwrap_or_default();
        if !content.contains(gitignore_entry) {
            let updated = format!("{}\n{}\n", content.trim_end(), gitignore_entry);
            std::fs::write(&gitignore_path, updated)?;
            println!("  * .gitignore                Added .rocketindex/index.db");
        }
    }

    println!("\nPress Enter to continue...");
    let _ = dialoguer::Input::<String>::new()
        .allow_empty(true)
        .interact_text()?;

    Ok(())
}

/// Screen 5: Complete
fn setup_screen_complete() {
    println!(
        r#"
Setup Complete!
───────────────

RocketIndex is ready. Here's how to get started:

  Start watch mode (run in background terminal):
  $ rkt watch

  Find a definition:
  $ rkt def "MyFunction"

  Find callers (before refactoring):
  $ rkt callers "MyFunction"

  Check health:
  $ rkt doctor

For full documentation, see .rocketindex/AGENTS.md

Happy coding! 🚀
"#
    );
}

// =============================================================================
// Legacy Setup Helpers
// =============================================================================

/// Ensure an index exists before installing editor tooling
fn ensure_initial_index(cwd: &Path, format: OutputFormat, quiet: bool) -> Result<()> {
    let index_path = cwd.join(".rocketindex").join(DEFAULT_DB_NAME);
    if index_path.exists() {
        return Ok(());
    }

    let show_feedback = should_show_index_feedback(format, quiet);
    let started = Instant::now();

    if show_feedback {
        println!("Building initial RocketIndex index (rkt index)...");
    }

    match cmd_index(cwd, false, format, quiet) {
        Ok(code) if code == exit_codes::SUCCESS => {
            if show_feedback {
                println!(
                    "Initial RocketIndex index ready in {:.1?}",
                    started.elapsed()
                );
            }
            Ok(())
        }
        Ok(code) => anyhow::bail!("rkt index failed during setup (exit code {})", code),
        Err(err) => Err(err),
    }
}

/// Set up Claude Code with 5-screen wizard flow
fn setup_claude_code(cwd: &Path, format: OutputFormat, quiet: bool) -> Result<u8> {
    let is_interactive = !quiet && dialoguer::console::Term::stderr().is_term();

    // For non-interactive mode, use legacy behavior
    if !is_interactive {
        return setup_claude_code_non_interactive(cwd, format, quiet);
    }

    // Screen 1: Welcome
    setup_screen_welcome()?;

    // Screen 2: Code Indexing
    setup_screen_indexing(cwd, format, quiet)?;

    // Screen 3: Agent Setup
    let mut created_files = setup_screen_agents(cwd, quiet)?;

    // Screen 4: Configuration Files
    setup_screen_configuration(cwd, format, quiet, &mut created_files)?;

    // Screen 5: Complete
    setup_screen_complete();

    Ok(exit_codes::SUCCESS)
}

/// Non-interactive setup for CI/scripts (legacy behavior)
fn setup_claude_code_non_interactive(cwd: &Path, format: OutputFormat, quiet: bool) -> Result<u8> {
    let mut created_files = Vec::new();

    // Ensure index exists
    ensure_initial_index(cwd, format, quiet)?;

    // Install rocketindex agent
    let skills_dir = cwd.join(".claude").join("skills");
    if let Some(rocketindex_agent) = skills::AGENTS.iter().find(|a| a.name == "rocketindex") {
        let agent_dir = skills_dir.join(rocketindex_agent.name);
        std::fs::create_dir_all(&agent_dir)?;
        let agent_path = agent_dir.join("SKILL.md");
        std::fs::write(&agent_path, rocketindex_agent.content)?;
        created_files.push(agent_path.display().to_string());
    }

    // Create AGENTS.md
    let agents_md_path = cwd.join(".rocketindex").join("AGENTS.md");
    if let Some(parent) = agents_md_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let agents_section = skills::get_agents_summary();
    let agents_content = std::fs::read_to_string(&agents_md_path).unwrap_or_default();

    if !agents_content.contains("RocketIndex") {
        let new_content = if agents_content.is_empty() {
            format!("# Agent Instructions\n\n{}", agents_section)
        } else {
            format!("{}\n\n{}", agents_content.trim_end(), agents_section)
        };
        std::fs::write(&agents_md_path, new_content)?;
        created_files.push(agents_md_path.display().to_string());
    }

    // Update CLAUDE.md if it exists
    let claude_md_path = cwd.join("CLAUDE.md");
    if claude_md_path.exists() {
        let claude_content = std::fs::read_to_string(&claude_md_path).unwrap_or_default();
        let rocketindex_note = "**Note**: This project uses [RocketIndex](https://github.com/rocket-tycoon/rocket-index) for code navigation.\n   For definitions, callers, and dependencies use `rkt`. See `.rocketindex/AGENTS.md` for commands.\n";

        if !claude_content.contains("RocketIndex") {
            let updated = if let Some(pos) = claude_content.find("\n\n") {
                format!(
                    "{}\n\n{}\n{}",
                    &claude_content[..pos],
                    rocketindex_note,
                    &claude_content[pos + 2..]
                )
            } else {
                format!("{}\n\n{}", claude_content, rocketindex_note)
            };
            std::fs::write(&claude_md_path, updated)?;
        }
    }

    // Update .github/copilot-instructions.md if it exists
    let copilot_path = cwd.join(".github").join("copilot-instructions.md");
    if copilot_path.exists() {
        let copilot_content = std::fs::read_to_string(&copilot_path).unwrap_or_default();
        let rocketindex_note = "**Note**: This project uses [RocketIndex](https://github.com/rocket-tycoon/rocket-index) for code navigation.\n   For definitions, callers, and dependencies use `rkt`. See `.rocketindex/AGENTS.md` for commands.\n";

        if !copilot_content.contains("RocketIndex") {
            let updated = if let Some(pos) = copilot_content.find("\n\n") {
                format!(
                    "{}\n\n{}\n{}",
                    &copilot_content[..pos],
                    rocketindex_note,
                    &copilot_content[pos + 2..]
                )
            } else {
                format!("{}\n\n{}", copilot_content, rocketindex_note)
            };
            std::fs::write(&copilot_path, updated)?;
        }
    }

    // JSON output for non-interactive
    if format == OutputFormat::Json {
        println!(
            "{}",
            serde_json::json!({
                "editor": "claude-code",
                "created": created_files,
                "usage": "See .rocketindex/AGENTS.md for detailed instructions"
            })
        );
    }

    Ok(exit_codes::SUCCESS)
}

/// Set up Cursor rules
fn setup_cursor(cwd: &Path, format: OutputFormat, quiet: bool) -> Result<u8> {
    ensure_initial_index(cwd, format, quiet)?;

    let rules_path = cwd.join(".cursor").join("rules");
    std::fs::create_dir_all(rules_path.parent().unwrap())?;

    // Create Cursor rules file
    let rules_content = r#"# RocketIndex Code Navigation

This project uses RocketIndex for fast code navigation. Before exploring the codebase:

1. Run `rkt index` to build/update the symbol index
2. Use `rkt def "Symbol"` to find definitions
3. Use `rkt callers "Symbol"` before refactoring to understand impact
4. Use `rkt spider "Entry.point" -d 3` to explore dependencies

Key commands:
- `rkt def "MyModule.myFunction"` - Jump to definition
- `rkt symbols "pattern*"` - Search symbols (supports wildcards)
- `rkt callers "Symbol"` - Find all callers (impact analysis)
- `rkt blame "src/file.fs:42"` - Git blame for a line
- `rkt doctor` - Check index health

Tips:
- Use `--concise` flag for minimal JSON output
- The index is stored in `.rocketindex/` (add to .gitignore)
- Run `rkt index` after significant changes
"#;

    std::fs::write(&rules_path, rules_content)?;

    if format == OutputFormat::Json {
        println!(
            "{}",
            serde_json::json!({
                "editor": "cursor",
                "created": [rules_path.display().to_string()],
                "usage": "Cursor will now see RocketIndex guidance in .cursor/rules"
            })
        );
    } else if !quiet {
        println!("Cursor setup complete!");
        println!("  Created: {}", rules_path.display());
        println!();
        println!("Cursor will now see RocketIndex guidance in .cursor/rules");
    }

    Ok(exit_codes::SUCCESS)
}

/// Set up GitHub Copilot instructions
fn setup_copilot(cwd: &Path, format: OutputFormat, quiet: bool) -> Result<u8> {
    let mut created_files = Vec::new();

    ensure_initial_index(cwd, format, quiet)?;

    let copilot_path = cwd.join(".github").join("copilot-instructions.md");
    if let Some(parent) = copilot_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let copilot_content = std::fs::read_to_string(&copilot_path).unwrap_or_default();
    let rocketindex_section = r#"## RocketIndex Code Navigation

This project uses [RocketIndex](https://github.com/rocket-tycoon/rocket-index) for fast code navigation.

**Before exploring the codebase:**
1. Run `rkt index` to build/update the symbol index
2. Use `rkt def "Symbol"` to find definitions
3. Use `rkt callers "Symbol"` before refactoring to understand impact
4. Use `rkt spider "Entry.point" -d 3` to explore dependencies

**Key commands:**
- `rkt def "MyModule.myFunction"` - Jump to definition
- `rkt symbols "pattern*"` - Search symbols (supports wildcards)
- `rkt callers "Symbol"` - Find all callers (impact analysis)
- `rkt spider "Entry.point" -d 3` - Dependency graph from entry point
- `rkt blame "src/file.fs:42"` - Git blame for a line
- `rkt doctor` - Check index health

**Tips:**
- Use `--concise` flag for minimal JSON output
- The index is stored in `.rocketindex/` (add to .gitignore)
- Run `rkt index` after significant changes
"#;

    // Only add if not already present (idempotent)
    if !copilot_content.contains("RocketIndex") {
        if copilot_content.is_empty() {
            // Create new file with header
            let new_content = format!("# Copilot Instructions\n\n{}\n", rocketindex_section);
            std::fs::write(&copilot_path, new_content)?;
            created_files.push(copilot_path.display().to_string());
        } else {
            // Append to existing file
            let updated = format!("{}\n\n{}", copilot_content.trim_end(), rocketindex_section);
            std::fs::write(&copilot_path, updated)?;
        }
    }

    if format == OutputFormat::Json {
        println!(
            "{}",
            serde_json::json!({
                "editor": "copilot",
                "file": copilot_path.display().to_string(),
                "created": created_files,
                "updated": copilot_content.is_empty() || !copilot_content.contains("RocketIndex"),
                "usage": "GitHub Copilot will now see RocketIndex guidance"
            })
        );
    } else if !quiet {
        if !created_files.is_empty() {
            println!("GitHub Copilot setup complete!");
            println!("  Created: {}", copilot_path.display());
        } else if copilot_content.contains("RocketIndex") {
            println!("GitHub Copilot already configured with RocketIndex guidance.");
        } else {
            println!("GitHub Copilot setup complete!");
            println!("  Updated: {}", copilot_path.display());
        }
        println!();
        println!("GitHub Copilot will now see RocketIndex guidance.");
    }

    Ok(exit_codes::SUCCESS)
}
