//! SQLite-based index storage for RocketIndex.
//!
//! This module provides persistent storage for the symbol index using SQLite.
//! Benefits over the previous JSON approach:
//! - O(log n) indexed lookups vs O(n) linear scan
//! - Low memory: query on-demand, don't load entire index
//! - Incremental updates: UPDATE single rows, no full rewrite
//! - Rich queries: LIKE patterns, JOINs for references
//! - Debuggable: inspect with `sqlite3` CLI
//!
//! # Examples
//!
//! Create an in-memory index for testing:
//!
//! ```
//! use rocketindex::SqliteIndex;
//!
//! let index = SqliteIndex::in_memory().unwrap();
//! assert_eq!(index.count_symbols().unwrap(), 0);
//! ```
//!
//! Store and query symbols:
//!
//! ```
//! use rocketindex::{SqliteIndex, Symbol, SymbolKind, Location, Visibility};
//! use std::path::PathBuf;
//!
//! let index = SqliteIndex::in_memory().unwrap();
//!
//! // Insert a symbol
//! let symbol = Symbol::new(
//!     "process_payment".to_string(),
//!     "PaymentService.process_payment".to_string(),
//!     SymbolKind::Function,
//!     Location::new(PathBuf::from("src/payment.rs"), 42, 5),
//!     Visibility::Public,
//!     "rust".to_string(),
//! );
//! index.insert_symbol(&symbol).unwrap();
//!
//! // Query by qualified name
//! let found = index.find_by_qualified("PaymentService.process_payment").unwrap();
//! assert!(found.is_some());
//! assert_eq!(found.unwrap().name, "process_payment");
//!
//! // Search with wildcards
//! let results = index.search("Payment*", 10, None).unwrap();
//! assert_eq!(results.len(), 1);
//! ```

use std::path::{Path, PathBuf};

use rusqlite::{params, Connection, OptionalExtension};

use crate::index::Reference;
use crate::type_cache::{MemberKind, TypeMember};
use crate::{IndexError, Location, Result, Symbol, SymbolKind, Visibility};

/// Current schema version. Increment when making breaking changes.
pub const SCHEMA_VERSION: u32 = 4;

/// Standard columns selected when querying symbols.
/// Must match the order expected by `row_to_symbol`.
const SYMBOL_COLUMNS: &str = "name, qualified, kind, file, line, column, end_line, end_column, visibility, language, parent, mixins, attributes, implements, doc, signature";

/// Default database filename within .rocketindex/
pub const DEFAULT_DB_NAME: &str = "index.db";

/// SQLite-based index for symbol storage and querying.
///
/// `SqliteIndex` provides persistent storage for extracted symbols with
/// efficient lookup operations. It supports:
/// - Exact lookups by qualified name (O(log n))
/// - Wildcard searches (LIKE patterns)
/// - Full-text search (FTS5) for fast prefix matching
/// - Batch insert operations with transactions
///
/// # Examples
///
/// ```
/// use rocketindex::SqliteIndex;
///
/// // Create an in-memory index (for testing)
/// let index = SqliteIndex::in_memory().unwrap();
///
/// // Check it's empty
/// assert_eq!(index.count_symbols().unwrap(), 0);
/// ```
///
/// For persistent storage, use `create` or `open`:
///
/// ```no_run
/// use rocketindex::SqliteIndex;
/// use std::path::Path;
///
/// // Create a new database
/// let index = SqliteIndex::create(Path::new(".rocketindex/index.db")).unwrap();
///
/// // Later, open an existing database
/// let index = SqliteIndex::open(Path::new(".rocketindex/index.db")).unwrap();
/// ```
pub struct SqliteIndex {
    conn: Connection,
}

impl SqliteIndex {
    /// Create a new database at the given path, initializing the schema.
    /// Fails if the database already exists.
    pub fn create(path: &Path) -> Result<Self> {
        if path.exists() {
            return Err(IndexError::IoError(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                format!("Database already exists: {}", path.display()),
            )));
        }

        let conn = Connection::open(path)?;
        let index = Self { conn };
        index.init_schema()?;
        Ok(index)
    }

    /// Open an existing database. Fails if it doesn't exist or has incompatible schema.
    pub fn open(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Err(IndexError::IndexNotFound);
        }

        let conn = Connection::open(path)?;

        // Aggressive performance tuning for read-heavy workloads
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA cache_size = -64000;
             PRAGMA mmap_size = 268435456;
             PRAGMA temp_store = MEMORY;",
        )?;

        let index = Self { conn };

        // Check and migrate schema if needed
        let version = index.get_schema_version()?;
        if version < SCHEMA_VERSION {
            index.migrate_schema(version)?;
        } else if version > SCHEMA_VERSION {
            return Err(IndexError::IoError(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "Schema version {} is newer than supported version {}",
                    version, SCHEMA_VERSION
                ),
            )));
        }

        Ok(index)
    }

    /// Migrate database schema from an older version.
    fn migrate_schema(&self, from_version: u32) -> Result<()> {
        // Migration v3 -> v4: Add file_mtimes table
        if from_version < 4 {
            self.conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS file_mtimes (
                    path TEXT PRIMARY KEY,
                    mtime INTEGER NOT NULL
                );",
            )?;
            self.set_metadata("schema_version", "4")?;
            tracing::info!("Migrated database schema from v{} to v4", from_version);
        }

        Ok(())
    }

    /// Open an existing database or create a new one.
    pub fn open_or_create(path: &Path) -> Result<Self> {
        if path.exists() {
            Self::open(path)
        } else {
            // Ensure parent directory exists
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            Self::create(path)
        }
    }

    /// Create an in-memory database (useful for testing).
    pub fn in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        let index = Self { conn };
        index.init_schema()?;
        Ok(index)
    }

    /// Initialize the database schema.
    fn init_schema(&self) -> Result<()> {
        // Performance tuning for write-heavy indexing
        // Note: synchronous=NORMAL is safe with WAL mode and has minimal overhead
        self.conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA cache_size = -64000;
             PRAGMA mmap_size = 268435456;
             PRAGMA temp_store = MEMORY;
             PRAGMA locking_mode = EXCLUSIVE;",
        )?;
        self.conn.execute_batch(SCHEMA_SQL)?;
        self.set_metadata("schema_version", &SCHEMA_VERSION.to_string())?;
        Ok(())
    }

    /// Get the schema version from metadata.
    pub fn get_schema_version(&self) -> Result<u32> {
        let version: Option<String> = self
            .conn
            .query_row(
                "SELECT value FROM metadata WHERE key = 'schema_version'",
                [],
                |row| row.get(0),
            )
            .optional()?;

        match version {
            Some(v) => v.parse().map_err(|_| {
                IndexError::IoError(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "Invalid schema version",
                ))
            }),
            None => Ok(0), // No version = legacy or empty DB
        }
    }

    /// Set a metadata key-value pair.
    pub fn set_metadata(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO metadata (key, value) VALUES (?1, ?2)",
            params![key, value],
        )?;
        Ok(())
    }

    /// Get a metadata value by key.
    pub fn get_metadata(&self, key: &str) -> Result<Option<String>> {
        let value: Option<String> = self
            .conn
            .query_row(
                "SELECT value FROM metadata WHERE key = ?1",
                params![key],
                |row| row.get(0),
            )
            .optional()?;
        Ok(value)
    }

    // =========================================================================
    // Symbol Operations
    // =========================================================================

    /// Insert a symbol into the database. Returns the inserted row ID.
    pub fn insert_symbol(&self, symbol: &Symbol) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO symbols (name, qualified, kind, file, line, column, end_line, end_column, visibility, source, language, parent, mixins, attributes, implements, doc, signature)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'syntactic', ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
            params![
                symbol.name,
                symbol.qualified,
                symbol_kind_to_str(symbol.kind),
                symbol.location.file.to_string_lossy(),
                symbol.location.line,
                symbol.location.column,
                symbol.location.end_line,
                symbol.location.end_column,
                visibility_to_str(symbol.visibility),
                symbol.language,
                symbol.parent,
                symbol.mixins.as_ref().map(|v| serde_json::to_string(v).unwrap_or_default()),
                symbol.attributes.as_ref().map(|v| serde_json::to_string(v).unwrap_or_default()),
                symbol.implements.as_ref().map(|v| serde_json::to_string(v).unwrap_or_default()),
                symbol.doc,
                symbol.signature,
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Insert a symbol with type signature.
    pub fn insert_symbol_with_type(&self, symbol: &Symbol, type_signature: &str) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO symbols (name, qualified, kind, type_signature, file, line, column, end_line, end_column, visibility, source, language, parent, mixins, attributes, implements, doc, signature)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 'semantic', ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
            params![
                symbol.name,
                symbol.qualified,
                symbol_kind_to_str(symbol.kind),
                type_signature,
                symbol.location.file.to_string_lossy(),
                symbol.location.line,
                symbol.location.column,
                symbol.location.end_line,
                symbol.location.end_column,
                visibility_to_str(symbol.visibility),
                symbol.language,
                symbol.parent,
                symbol.mixins.as_ref().map(|v| serde_json::to_string(v).unwrap_or_default()),
                symbol.attributes.as_ref().map(|v| serde_json::to_string(v).unwrap_or_default()),
                symbol.implements.as_ref().map(|v| serde_json::to_string(v).unwrap_or_default()),
                symbol.doc,
                symbol.signature,
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Insert multiple symbols in a transaction for efficiency.
    pub fn insert_symbols(&self, symbols: &[Symbol]) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO symbols (name, qualified, kind, file, line, column, end_line, end_column, visibility, language, source, parent, mixins, attributes, implements, doc, signature)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 'syntactic', ?11, ?12, ?13, ?14, ?15, ?16)",
            )?;

            for symbol in symbols {
                stmt.execute(params![
                    symbol.name,
                    symbol.qualified,
                    symbol_kind_to_str(symbol.kind),
                    symbol.location.file.to_string_lossy(),
                    symbol.location.line,
                    symbol.location.column,
                    symbol.location.end_line,
                    symbol.location.end_column,
                    visibility_to_str(symbol.visibility),
                    symbol.language,
                    symbol.parent,
                    symbol
                        .mixins
                        .as_ref()
                        .map(|v| serde_json::to_string(v).unwrap_or_default()),
                    symbol
                        .attributes
                        .as_ref()
                        .map(|v| serde_json::to_string(v).unwrap_or_default()),
                    symbol
                        .implements
                        .as_ref()
                        .map(|v| serde_json::to_string(v).unwrap_or_default()),
                    symbol.doc,
                    symbol.signature,
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Find a symbol by its qualified name. Returns the first match.
    #[must_use = "query results should not be ignored"]
    pub fn find_by_qualified(&self, qualified: &str) -> Result<Option<Symbol>> {
        let query = format!(
            "SELECT {} FROM symbols WHERE qualified = ?1 LIMIT 1",
            SYMBOL_COLUMNS
        );
        let symbol = self
            .conn
            .query_row(&query, params![qualified], row_to_symbol)
            .optional()?;
        Ok(symbol)
    }

    /// Find all symbols with the given qualified name (for overloads).
    pub fn find_all_by_qualified(&self, qualified: &str) -> Result<Vec<Symbol>> {
        let query = format!(
            "SELECT {} FROM symbols WHERE qualified = ?1",
            SYMBOL_COLUMNS
        );
        let mut stmt = self.conn.prepare(&query)?;

        let symbols = stmt
            .query_map(params![qualified], row_to_symbol)?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(symbols)
    }

    /// Search for symbols matching a pattern. Supports SQL LIKE wildcards (% and _).
    #[must_use = "search results should not be ignored"]
    pub fn search(
        &self,
        pattern: &str,
        limit: usize,
        language: Option<&str>,
    ) -> Result<Vec<Symbol>> {
        // Convert glob-style wildcards to SQL LIKE
        let sql_pattern = pattern.replace('*', "%").replace('?', "_");

        let query = if language.is_some() {
            format!(
                "SELECT {} FROM symbols WHERE (name LIKE ?1 OR qualified LIKE ?1) AND language = ?2 LIMIT ?3",
                SYMBOL_COLUMNS
            )
        } else {
            format!(
                "SELECT {} FROM symbols WHERE (name LIKE ?1 OR qualified LIKE ?1) LIMIT ?2",
                SYMBOL_COLUMNS
            )
        };
        let mut stmt = self.conn.prepare(&query)?;

        let symbols = if let Some(lang) = language {
            stmt.query_map(params![sql_pattern, lang, limit as i64], row_to_symbol)?
        } else {
            stmt.query_map(params![sql_pattern, limit as i64], row_to_symbol)?
        };

        let symbols = symbols.collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(symbols)
    }

    /// Search for symbols using FTS5 full-text search.
    ///
    /// This is faster than LIKE for prefix and word-based searches.
    /// Supports FTS5 query syntax:
    /// - `word` - exact word match
    /// - `word*` - prefix match
    /// - `word1 word2` - both words must appear
    /// - `"word1 word2"` - exact phrase
    ///
    /// Falls back to LIKE search for patterns that FTS5 can't handle well
    /// (e.g., suffix matches like `*Service` or complex wildcards).
    #[must_use = "search results should not be ignored"]
    pub fn search_fts(
        &self,
        pattern: &str,
        limit: usize,
        language: Option<&str>,
    ) -> Result<Vec<Symbol>> {
        // Check if this is a pattern FTS5 can handle well
        let trimmed = pattern.trim();

        // FTS5 works well for:
        // - Exact words: "Service"
        // - Prefix: "Service*" or "Serv*"
        // - Multiple words: "Service Handler"
        //
        // FTS5 does NOT work well for:
        // - Suffix: "*Service"
        // - Contains: "*Serv*"
        // - Complex patterns: "*a*b*"

        let is_fts_suitable = !trimmed.starts_with('*') && !trimmed.contains("**");

        if is_fts_suitable {
            // Convert to FTS5 query
            let fts_query = if trimmed.ends_with('*') {
                // Already a prefix query, keep as-is
                trimmed.to_string()
            } else if trimmed.contains('*') {
                // Has wildcards in middle - not suitable for FTS
                return self.search(pattern, limit, language);
            } else {
                // Exact word - add prefix wildcard for partial matching
                format!("{}*", trimmed)
            };

            let result = self.search_fts_raw(&fts_query, limit, language);

            // If FTS fails (e.g., syntax error), fall back to LIKE
            match result {
                Ok(symbols) => return Ok(symbols),
                Err(_) => return self.search(pattern, limit, language),
            }
        }

        // Fall back to LIKE for patterns FTS can't handle
        self.search(pattern, limit, language)
    }

    /// Raw FTS5 search - directly executes an FTS5 query.
    fn search_fts_raw(
        &self,
        fts_query: &str,
        limit: usize,
        language: Option<&str>,
    ) -> Result<Vec<Symbol>> {
        let prefixed_cols = SYMBOL_COLUMNS
            .split(", ")
            .map(|c| format!("s.{}", c))
            .collect::<Vec<_>>()
            .join(", ");
        let query = if language.is_some() {
            format!(
                "SELECT {} FROM symbols s JOIN symbols_fts fts ON s.id = fts.rowid WHERE symbols_fts MATCH ?1 AND s.language = ?2 ORDER BY rank LIMIT ?3",
                prefixed_cols
            )
        } else {
            format!(
                "SELECT {} FROM symbols s JOIN symbols_fts fts ON s.id = fts.rowid WHERE symbols_fts MATCH ?1 ORDER BY rank LIMIT ?2",
                prefixed_cols
            )
        };
        let mut stmt = self.conn.prepare(&query)?;

        let symbols = if let Some(lang) = language {
            stmt.query_map(params![fts_query, lang, limit as i64], row_to_symbol)?
        } else {
            stmt.query_map(params![fts_query, limit as i64], row_to_symbol)?
        };

        let symbols = symbols.collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(symbols)
    }

    /// Get all symbols defined in a file.
    pub fn symbols_in_file(&self, file: &Path) -> Result<Vec<Symbol>> {
        let file_str = file.to_string_lossy();
        let query = format!("SELECT {} FROM symbols WHERE file = ?1", SYMBOL_COLUMNS);
        let mut stmt = self.conn.prepare(&query)?;

        let symbols = stmt
            .query_map(params![file_str.as_ref()], row_to_symbol)?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(symbols)
    }

    /// Delete all symbols in a file.
    pub fn delete_symbols_in_file(&self, file: &Path) -> Result<usize> {
        let file_str = file.to_string_lossy();
        let count = self.conn.execute(
            "DELETE FROM symbols WHERE file = ?1",
            params![file_str.as_ref()],
        )?;
        Ok(count)
    }

    /// Count total symbols in the index.
    pub fn count_symbols(&self) -> Result<usize> {
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM symbols", [], |row| row.get(0))?;
        Ok(count as usize)
    }

    /// Find similar symbol names for "did you mean?" suggestions.
    ///
    /// Returns symbols within `max_distance` edits of the query,
    /// sorted by edit distance (closest first).
    ///
    /// # Arguments
    ///
    /// * `query` - The symbol name to find suggestions for
    /// * `max_distance` - Maximum edit distance to consider (default: 3)
    /// * `max_suggestions` - Maximum suggestions to return (default: 5)
    pub fn suggest_similar(
        &self,
        query: &str,
        max_distance: usize,
        max_suggestions: usize,
    ) -> Result<Vec<crate::fuzzy::Suggestion>> {
        // Get all unique symbol names and qualified names
        let mut stmt = self.conn.prepare(
            "SELECT DISTINCT name FROM symbols
             UNION
             SELECT DISTINCT qualified FROM symbols",
        )?;

        let names: Vec<String> = stmt
            .query_map([], |row| row.get(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        // Run fuzzy matching
        let suggestions = crate::fuzzy::find_similar(
            query,
            names.iter().map(String::as_str),
            max_distance,
            max_suggestions,
        );

        Ok(suggestions)
    }

    /// Search for symbols using fuzzy matching (edit distance).
    ///
    /// Returns symbols whose name or qualified name is within `max_distance`
    /// edits of the query, sorted by edit distance (closest first).
    ///
    /// # Arguments
    ///
    /// * `query` - The pattern to fuzzy match against
    /// * `max_distance` - Maximum edit distance to consider
    /// * `limit` - Maximum number of results to return
    /// * `language` - Optional language filter
    pub fn fuzzy_search(
        &self,
        query: &str,
        max_distance: usize,
        limit: usize,
        language: Option<&str>,
    ) -> Result<Vec<(Symbol, usize)>> {
        // OPTIMIZATION: Use FTS to get candidates first, then filter by edit distance.
        // This avoids loading all 36k+ symbols into memory for every fuzzy search.
        // We search for prefix matches which are likely to have low edit distance.

        // Generate candidate prefixes from the query (first N chars)
        let candidate_limit = limit * 20; // Get more candidates than needed for filtering
        let symbols = if query.len() >= 2 {
            // Use FTS prefix search for candidate generation
            let prefix = &query[..query.len().min(4)];
            let fts_query = format!("{}*", prefix);

            let prefixed_cols = SYMBOL_COLUMNS
                .split(", ")
                .map(|c| format!("s.{}", c))
                .collect::<Vec<_>>()
                .join(", ");

            let sql = if language.is_some() {
                format!(
                    "SELECT {} FROM symbols s JOIN symbols_fts fts ON s.id = fts.rowid WHERE symbols_fts MATCH ?1 AND s.language = ?2 LIMIT ?3",
                    prefixed_cols
                )
            } else {
                format!(
                    "SELECT {} FROM symbols s JOIN symbols_fts fts ON s.id = fts.rowid WHERE symbols_fts MATCH ?1 LIMIT ?2",
                    prefixed_cols
                )
            };

            let mut stmt = self.conn.prepare(&sql)?;
            let result: Vec<Symbol> = if let Some(lang) = language {
                stmt.query_map(
                    params![fts_query, lang, candidate_limit as i64],
                    row_to_symbol,
                )?
            } else {
                stmt.query_map(params![fts_query, candidate_limit as i64], row_to_symbol)?
            }
            .collect::<std::result::Result<Vec<_>, _>>()?;

            // If FTS found enough candidates, use them; otherwise fall back to full scan
            if result.len() >= limit {
                result
            } else {
                // Fall back to scanning (for very short queries or no FTS matches)
                self.fuzzy_search_full_scan(query, language, candidate_limit)?
            }
        } else {
            // Query too short for FTS, do full scan
            self.fuzzy_search_full_scan(query, language, candidate_limit)?
        };

        // Filter by edit distance and sort
        let mut results = Vec::new();
        for symbol in symbols {
            let name_dist = crate::fuzzy::levenshtein_distance(query, &symbol.name);
            let qual_dist = crate::fuzzy::levenshtein_distance(query, &symbol.qualified);
            let dist = std::cmp::min(name_dist, qual_dist);

            if dist <= max_distance {
                results.push((symbol, dist));
            }
        }

        results.sort_by_key(|(_, dist)| *dist);
        results.truncate(limit);

        Ok(results)
    }

    /// Full table scan for fuzzy search (fallback when FTS can't help)
    fn fuzzy_search_full_scan(
        &self,
        _query: &str,
        language: Option<&str>,
        limit: usize,
    ) -> Result<Vec<Symbol>> {
        let sql = if language.is_some() {
            format!(
                "SELECT {} FROM symbols WHERE language = ?1 LIMIT ?2",
                SYMBOL_COLUMNS
            )
        } else {
            format!("SELECT {} FROM symbols LIMIT ?1", SYMBOL_COLUMNS)
        };
        let mut stmt = self.conn.prepare(&sql)?;

        let symbols: Vec<Symbol> = if let Some(lang) = language {
            stmt.query_map(params![lang, limit as i64], row_to_symbol)?
        } else {
            stmt.query_map(params![limit as i64], row_to_symbol)?
        }
        .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(symbols)
    }

    /// Find all classes/modules that inherit from the given parent.
    /// Uses index on parent column for exact matches, with optimized suffix matching.
    pub fn find_subclasses(&self, parent: &str) -> Result<Vec<Symbol>> {
        // First try exact match using the index (fast path)
        let query = format!("SELECT {} FROM symbols WHERE parent = ?1", SYMBOL_COLUMNS);
        let mut stmt = self.conn.prepare(&query)?;
        let mut symbols: Vec<Symbol> = stmt
            .query_map(params![parent], row_to_symbol)?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        // Also match with leading :: (e.g., "::Common::Client::Base")
        let prefixed = format!("::{}", parent);
        let query2 = format!("SELECT {} FROM symbols WHERE parent = ?1", SYMBOL_COLUMNS);
        let mut stmt2 = self.conn.prepare(&query2)?;
        let prefixed_symbols: Vec<Symbol> = stmt2
            .query_map(params![prefixed], row_to_symbol)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        symbols.extend(prefixed_symbols);

        Ok(symbols)
    }
    /// List all indexed files.
    pub fn list_files(&self) -> Result<Vec<PathBuf>> {
        let mut stmt = self
            .conn
            .prepare("SELECT DISTINCT file FROM symbols ORDER BY file")?;

        let files = stmt
            .query_map([], |row| {
                let file: String = row.get(0)?;
                Ok(PathBuf::from(file))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(files)
    }

    /// Get type signature for a symbol by qualified name.
    pub fn get_symbol_type(&self, qualified: &str) -> Result<Option<String>> {
        let type_sig: Option<String> = self
            .conn
            .query_row(
                "SELECT type_signature FROM symbols WHERE qualified = ?1 AND type_signature IS NOT NULL LIMIT 1",
                params![qualified],
                |row| row.get(0),
            )
            .optional()?;
        Ok(type_sig)
    }

    /// Update type signature for existing symbol(s).
    pub fn update_symbol_type(&self, qualified: &str, type_signature: &str) -> Result<usize> {
        let count = self.conn.execute(
            "UPDATE symbols SET type_signature = ?1, source = 'semantic' WHERE qualified = ?2",
            params![type_signature, qualified],
        )?;
        Ok(count)
    }

    // =========================================================================
    // Reference Operations
    // =========================================================================

    /// Insert a reference.
    pub fn insert_reference(&self, file: &Path, reference: &Reference) -> Result<i64> {
        let file_str = file.to_string_lossy();
        self.conn.execute(
            "INSERT INTO refs (name, file, line, column) VALUES (?1, ?2, ?3, ?4)",
            params![
                reference.name,
                file_str.as_ref(),
                reference.location.line,
                reference.location.column,
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Insert multiple references in a transaction for efficiency.
    pub fn insert_references(&self, refs: &[(&Path, &Reference)]) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        {
            let mut stmt =
                tx.prepare("INSERT INTO refs (name, file, line, column) VALUES (?1, ?2, ?3, ?4)")?;

            for (file, reference) in refs {
                let file_str = file.to_string_lossy();
                stmt.execute(params![
                    reference.name,
                    file_str.as_ref(),
                    reference.location.line,
                    reference.location.column,
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Find all references to a name (short or qualified).
    /// Matches exact name or qualified names ending with the name (e.g., "User" matches
    /// "User", "Module.User", "Module::User", etc.)
    pub fn find_references(&self, name: &str) -> Result<Vec<Reference>> {
        let mut stmt = self.conn.prepare(
            "SELECT name, file, line, column FROM refs
             WHERE name = ?1
                OR name LIKE '%.' || ?1
                OR name LIKE '%::' || ?1",
        )?;

        let refs = stmt
            .query_map(params![name], |row| {
                let name: String = row.get(0)?;
                let file: String = row.get(1)?;
                let line: u32 = row.get(2)?;
                let column: u32 = row.get(3)?;
                Ok(Reference {
                    name,
                    location: Location::new(PathBuf::from(file), line, column),
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(refs)
    }

    /// Get all references in a file.
    pub fn references_in_file(&self, file: &Path) -> Result<Vec<Reference>> {
        let file_str = file.to_string_lossy();
        let mut stmt = self
            .conn
            .prepare("SELECT name, file, line, column FROM refs WHERE file = ?1")?;

        let refs = stmt
            .query_map(params![file_str.as_ref()], |row| {
                let name: String = row.get(0)?;
                let file: String = row.get(1)?;
                let line: u32 = row.get(2)?;
                let column: u32 = row.get(3)?;
                Ok(Reference {
                    name,
                    location: Location::new(PathBuf::from(file), line, column),
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(refs)
    }

    /// Delete all references in a file.
    pub fn delete_references_in_file(&self, file: &Path) -> Result<usize> {
        let file_str = file.to_string_lossy();
        let count = self.conn.execute(
            "DELETE FROM refs WHERE file = ?1",
            params![file_str.as_ref()],
        )?;
        Ok(count)
    }

    // =========================================================================
    // Opens Operations
    // =========================================================================

    /// Insert an open statement.
    pub fn insert_open(&self, file: &Path, module_path: &str, line: u32) -> Result<i64> {
        let file_str = file.to_string_lossy();
        self.conn.execute(
            "INSERT INTO opens (file, module_path, line) VALUES (?1, ?2, ?3)",
            params![file_str.as_ref(), module_path, line],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Insert multiple open statements in a transaction for efficiency.
    pub fn insert_opens(&self, opens: &[(&Path, &str, u32)]) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        {
            let mut stmt =
                tx.prepare("INSERT INTO opens (file, module_path, line) VALUES (?1, ?2, ?3)")?;

            for (file, module_path, line) in opens {
                let file_str = file.to_string_lossy();
                stmt.execute(params![file_str.as_ref(), *module_path, *line])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Get all opens for a file.
    pub fn opens_for_file(&self, file: &Path) -> Result<Vec<String>> {
        let file_str = file.to_string_lossy();
        let mut stmt = self
            .conn
            .prepare("SELECT module_path FROM opens WHERE file = ?1 ORDER BY line")?;

        let opens = stmt
            .query_map(params![file_str.as_ref()], |row| row.get(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(opens)
    }

    /// Delete all opens in a file.
    pub fn delete_opens_in_file(&self, file: &Path) -> Result<usize> {
        let file_str = file.to_string_lossy();
        let count = self.conn.execute(
            "DELETE FROM opens WHERE file = ?1",
            params![file_str.as_ref()],
        )?;
        Ok(count)
    }

    // =========================================================================
    // Type Member Operations
    // =========================================================================

    /// Insert a type member.
    pub fn insert_member(&self, member: &TypeMember) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO members (type_name, member_name, member_type, kind) VALUES (?1, ?2, ?3, ?4)",
            params![
                member.type_name,
                member.member,
                member.member_type,
                member_kind_to_str(member.kind),
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Insert multiple type members in a transaction.
    pub fn insert_members(&self, members: &[TypeMember]) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO members (type_name, member_name, member_type, kind) VALUES (?1, ?2, ?3, ?4)",
            )?;

            for member in members {
                stmt.execute(params![
                    member.type_name,
                    member.member,
                    member.member_type,
                    member_kind_to_str(member.kind),
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Get all members of a type.
    pub fn get_members(&self, type_name: &str) -> Result<Vec<TypeMember>> {
        let mut stmt = self.conn.prepare(
            "SELECT type_name, member_name, member_type, kind FROM members WHERE type_name = ?1",
        )?;

        let members = stmt
            .query_map(params![type_name], row_to_type_member)?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(members)
    }

    /// Get a specific member of a type.
    pub fn get_member(&self, type_name: &str, member_name: &str) -> Result<Option<TypeMember>> {
        let member = self
            .conn
            .query_row(
                "SELECT type_name, member_name, member_type, kind
                 FROM members WHERE type_name = ?1 AND member_name = ?2 LIMIT 1",
                params![type_name, member_name],
                row_to_type_member,
            )
            .optional()?;
        Ok(member)
    }

    /// Delete all members of a type.
    pub fn delete_type_members(&self, type_name: &str) -> Result<usize> {
        let count = self.conn.execute(
            "DELETE FROM members WHERE type_name = ?1",
            params![type_name],
        )?;
        Ok(count)
    }

    /// Clear all members (used before re-extraction).
    pub fn clear_all_members(&self) -> Result<usize> {
        let count = self.conn.execute("DELETE FROM members", [])?;
        Ok(count)
    }

    // =========================================================================
    // File-level Operations
    // =========================================================================

    /// Update all data for a file in a single transaction (clear + insert).
    /// More efficient than separate clear + insert calls as it avoids
    /// multiple transaction commits and reduces I/O.
    pub fn update_file_data(
        &self,
        file: &Path,
        symbols: &[Symbol],
        references: &[Reference],
        opens: &[(String, u32)], // (module_path, line)
    ) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        let file_str = file.to_string_lossy();

        // Clear existing data
        tx.execute(
            "DELETE FROM symbols WHERE file = ?1",
            params![file_str.as_ref()],
        )?;
        tx.execute(
            "DELETE FROM refs WHERE file = ?1",
            params![file_str.as_ref()],
        )?;
        tx.execute(
            "DELETE FROM opens WHERE file = ?1",
            params![file_str.as_ref()],
        )?;

        // Insert symbols
        {
            let mut stmt = tx.prepare(
                "INSERT INTO symbols (name, qualified, kind, file, line, column, end_line, end_column, visibility, language, source, parent, mixins, attributes, implements, doc, signature)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 'syntactic', ?11, ?12, ?13, ?14, ?15, ?16)",
            )?;
            for symbol in symbols {
                stmt.execute(params![
                    symbol.name,
                    symbol.qualified,
                    symbol_kind_to_str(symbol.kind),
                    symbol.location.file.to_string_lossy(),
                    symbol.location.line,
                    symbol.location.column,
                    symbol.location.end_line,
                    symbol.location.end_column,
                    visibility_to_str(symbol.visibility),
                    symbol.language,
                    symbol.parent,
                    symbol
                        .mixins
                        .as_ref()
                        .map(|v| serde_json::to_string(v).unwrap_or_default()),
                    symbol
                        .attributes
                        .as_ref()
                        .map(|v| serde_json::to_string(v).unwrap_or_default()),
                    symbol
                        .implements
                        .as_ref()
                        .map(|v| serde_json::to_string(v).unwrap_or_default()),
                    symbol.doc,
                    symbol.signature,
                ])?;
            }
        }

        // Insert references
        {
            let mut stmt =
                tx.prepare("INSERT INTO refs (name, file, line, column) VALUES (?1, ?2, ?3, ?4)")?;
            for reference in references {
                stmt.execute(params![
                    reference.name,
                    file_str.as_ref(),
                    reference.location.line,
                    reference.location.column,
                ])?;
            }
        }

        // Insert opens
        {
            let mut stmt =
                tx.prepare("INSERT INTO opens (file, module_path, line) VALUES (?1, ?2, ?3)")?;
            for (module_path, line) in opens {
                stmt.execute(params![file_str.as_ref(), module_path, *line])?;
            }
        }

        tx.commit()?;
        Ok(())
    }

    /// Clear all data for a file (symbols, references, opens).
    pub fn clear_file(&self, file: &Path) -> Result<()> {
        self.delete_symbols_in_file(file)?;
        self.delete_references_in_file(file)?;
        self.delete_opens_in_file(file)?;
        Ok(())
    }

    /// Begin a transaction for batch operations.
    pub fn begin_transaction(&self) -> Result<rusqlite::Transaction<'_>> {
        Ok(self.conn.unchecked_transaction()?)
    }

    // =========================================================================
    // File Mtime Tracking (for incremental refresh)
    // =========================================================================

    /// Record the modification time of a file.
    pub fn set_file_mtime(&self, file: &Path, mtime: u64) -> Result<()> {
        let file_str = file.to_string_lossy();
        self.conn.execute(
            "INSERT OR REPLACE INTO file_mtimes (path, mtime) VALUES (?1, ?2)",
            params![file_str.as_ref(), mtime as i64],
        )?;
        Ok(())
    }

    /// Get the recorded modification time of a file.
    pub fn get_file_mtime(&self, file: &Path) -> Result<Option<u64>> {
        let file_str = file.to_string_lossy();
        let mtime: Option<i64> = self
            .conn
            .query_row(
                "SELECT mtime FROM file_mtimes WHERE path = ?1",
                params![file_str.as_ref()],
                |row| row.get(0),
            )
            .optional()?;
        Ok(mtime.map(|m| m as u64))
    }

    /// Delete the mtime record for a file.
    pub fn delete_file_mtime(&self, file: &Path) -> Result<()> {
        let file_str = file.to_string_lossy();
        self.conn.execute(
            "DELETE FROM file_mtimes WHERE path = ?1",
            params![file_str.as_ref()],
        )?;
        Ok(())
    }

    /// Clear all mtime records.
    pub fn clear_all_mtimes(&self) -> Result<usize> {
        let count = self.conn.execute("DELETE FROM file_mtimes", [])?;
        Ok(count)
    }

    /// Get all tracked file paths.
    pub fn get_tracked_files(&self) -> Result<Vec<PathBuf>> {
        let mut stmt = self.conn.prepare("SELECT path FROM file_mtimes")?;
        let files = stmt
            .query_map([], |row| {
                let path: String = row.get(0)?;
                Ok(PathBuf::from(path))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(files)
    }

    /// Find files that are stale (mtime on disk differs from recorded mtime).
    ///
    /// Returns a list of (path, reason) tuples where reason is:
    /// - "modified" - file exists but mtime changed
    /// - "deleted" - file was tracked but no longer exists
    /// - "new" - file exists on disk but wasn't tracked
    ///
    /// This is designed to be fast (<100ms for typical projects).
    pub fn find_stale_files(
        &self,
        source_files: &[PathBuf],
    ) -> Result<Vec<(PathBuf, &'static str)>> {
        let mut stale = Vec::new();

        // Check tracked files for modifications/deletions
        let mut stmt = self.conn.prepare("SELECT path, mtime FROM file_mtimes")?;
        let tracked: Vec<(String, i64)> = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        // Build set of tracked paths for new file detection
        let tracked_set: std::collections::HashSet<_> =
            tracked.iter().map(|(p, _)| PathBuf::from(p)).collect();

        for (path_str, recorded_mtime) in &tracked {
            let path = PathBuf::from(path_str);

            if !path.exists() {
                stale.push((path, "deleted"));
                continue;
            }

            // Check if mtime changed
            if let Ok(metadata) = std::fs::metadata(&path) {
                if let Ok(modified) = metadata.modified() {
                    let disk_mtime = modified
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs())
                        .unwrap_or(0);
                    if disk_mtime != *recorded_mtime as u64 {
                        stale.push((path, "modified"));
                    }
                }
            }
        }

        // Check for new files (on disk but not tracked)
        for file in source_files {
            if !tracked_set.contains(file) {
                stale.push((file.clone(), "new"));
            }
        }

        Ok(stale)
    }
}

// ============================================================================
// Schema SQL
// ============================================================================

const SCHEMA_SQL: &str = r#"
-- Metadata table for versioning and configuration
CREATE TABLE IF NOT EXISTS metadata (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

-- Main symbols table (syntactic + type info merged)
CREATE TABLE IF NOT EXISTS symbols (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    qualified TEXT NOT NULL,
    kind TEXT NOT NULL,
    type_signature TEXT,
    file TEXT NOT NULL,
    line INTEGER NOT NULL,
    column INTEGER NOT NULL,
    end_line INTEGER,
    end_column INTEGER,
    visibility TEXT DEFAULT 'public',
    source TEXT DEFAULT 'syntactic',
    language TEXT DEFAULT 'fsharp',
    parent TEXT,
    mixins TEXT,
    attributes TEXT,
    implements TEXT,
    doc TEXT,
    signature TEXT
);

CREATE INDEX IF NOT EXISTS idx_symbols_qualified ON symbols(qualified);
CREATE INDEX IF NOT EXISTS idx_symbols_name ON symbols(name);
CREATE INDEX IF NOT EXISTS idx_symbols_file ON symbols(file);
CREATE INDEX IF NOT EXISTS idx_symbols_kind ON symbols(kind);
CREATE INDEX IF NOT EXISTS idx_symbols_parent ON symbols(parent);

-- FTS5 virtual table for fast full-text search on symbol names
-- Uses content= to make it an "external content" table linked to symbols
CREATE VIRTUAL TABLE IF NOT EXISTS symbols_fts USING fts5(
    name,
    qualified,
    content='symbols',
    content_rowid='id',
    tokenize='unicode61 tokenchars _'
);

-- Triggers to keep FTS index in sync with symbols table
CREATE TRIGGER IF NOT EXISTS symbols_ai AFTER INSERT ON symbols BEGIN
    INSERT INTO symbols_fts(rowid, name, qualified) VALUES (new.id, new.name, new.qualified);
END;

CREATE TRIGGER IF NOT EXISTS symbols_ad AFTER DELETE ON symbols BEGIN
    INSERT INTO symbols_fts(symbols_fts, rowid, name, qualified) VALUES('delete', old.id, old.name, old.qualified);
END;

CREATE TRIGGER IF NOT EXISTS symbols_au AFTER UPDATE ON symbols BEGIN
    INSERT INTO symbols_fts(symbols_fts, rowid, name, qualified) VALUES('delete', old.id, old.name, old.qualified);
    INSERT INTO symbols_fts(rowid, name, qualified) VALUES (new.id, new.name, new.qualified);
END;

-- Type members (properties, methods) for member access resolution
CREATE TABLE IF NOT EXISTS members (
    id INTEGER PRIMARY KEY,
    type_name TEXT NOT NULL,
    member_name TEXT NOT NULL,
    member_type TEXT,
    kind TEXT NOT NULL,
    file TEXT,
    line INTEGER
);

CREATE INDEX IF NOT EXISTS idx_members_type ON members(type_name);
CREATE INDEX IF NOT EXISTS idx_members_name ON members(member_name);

-- References for find-usages
CREATE TABLE IF NOT EXISTS refs (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    file TEXT NOT NULL,
    line INTEGER NOT NULL,
    column INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_refs_name ON refs(name);
CREATE INDEX IF NOT EXISTS idx_refs_file ON refs(file);

-- Open statements for resolution context
CREATE TABLE IF NOT EXISTS opens (
    id INTEGER PRIMARY KEY,
    file TEXT NOT NULL,
    module_path TEXT NOT NULL,
    line INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_opens_file ON opens(file);

-- File modification times for incremental refresh
CREATE TABLE IF NOT EXISTS file_mtimes (
    path TEXT PRIMARY KEY,
    mtime INTEGER NOT NULL
);
"#;

// ============================================================================
// Helper Functions
// ============================================================================

fn row_to_symbol(row: &rusqlite::Row<'_>) -> rusqlite::Result<Symbol> {
    let name: String = row.get(0)?;
    let qualified: String = row.get(1)?;
    let kind_str: String = row.get(2)?;
    let file: String = row.get(3)?;
    let line: u32 = row.get(4)?;
    let column: u32 = row.get(5)?;
    let end_line: u32 = row.get::<_, Option<u32>>(6)?.unwrap_or(line);
    let end_column: u32 = row.get::<_, Option<u32>>(7)?.unwrap_or(column);
    let visibility_str: String = row
        .get::<_, Option<String>>(8)?
        .unwrap_or_else(|| "public".to_string());
    let language: String = row
        .get::<_, Option<String>>(9)?
        .unwrap_or_else(|| "fsharp".to_string());
    let parent: Option<String> = row.get(10)?;
    let mixins_json: Option<String> = row.get(11)?;
    let attributes_json: Option<String> = row.get(12)?;
    let implements_json: Option<String> = row.get(13)?;
    let doc: Option<String> = row.get(14)?;
    let signature: Option<String> = row.get(15)?;

    let mixins = mixins_json.and_then(|j| serde_json::from_str(&j).ok());
    let attributes = attributes_json.and_then(|j| serde_json::from_str(&j).ok());
    let implements = implements_json.and_then(|j| serde_json::from_str(&j).ok());

    Ok(Symbol {
        name,
        qualified,
        kind: str_to_symbol_kind(&kind_str),
        location: Location::with_end(PathBuf::from(file), line, column, end_line, end_column),
        visibility: str_to_visibility(&visibility_str),
        language,
        parent,
        mixins,
        attributes,
        implements,
        doc,
        signature,
    })
}

fn row_to_type_member(row: &rusqlite::Row<'_>) -> rusqlite::Result<TypeMember> {
    let type_name: String = row.get(0)?;
    let member_name: String = row.get(1)?;
    let member_type: String = row.get::<_, Option<String>>(2)?.unwrap_or_default();
    let kind_str: String = row.get(3)?;

    Ok(TypeMember {
        type_name,
        member: member_name,
        member_type,
        kind: str_to_member_kind(&kind_str),
    })
}

pub(crate) fn symbol_kind_to_str(kind: SymbolKind) -> &'static str {
    match kind {
        SymbolKind::Module => "module",
        SymbolKind::Function => "function",
        SymbolKind::Value => "value",
        SymbolKind::Type => "type",
        SymbolKind::Record => "record",
        SymbolKind::Union => "union",
        SymbolKind::Interface => "interface",
        SymbolKind::Class => "class",
        SymbolKind::Member => "member",
    }
}

fn str_to_symbol_kind(s: &str) -> SymbolKind {
    match s {
        "module" => SymbolKind::Module,
        "function" => SymbolKind::Function,
        "value" => SymbolKind::Value,
        "type" => SymbolKind::Type,
        "record" => SymbolKind::Record,
        "union" => SymbolKind::Union,
        "interface" => SymbolKind::Interface,
        "class" => SymbolKind::Class,
        "member" => SymbolKind::Member,
        _ => SymbolKind::Value,
    }
}

pub(crate) fn visibility_to_str(vis: Visibility) -> &'static str {
    match vis {
        Visibility::Public => "public",
        Visibility::Internal => "internal",
        Visibility::Private => "private",
    }
}

fn str_to_visibility(s: &str) -> Visibility {
    match s {
        "public" => Visibility::Public,
        "internal" => Visibility::Internal,
        "private" => Visibility::Private,
        _ => Visibility::Public,
    }
}

fn member_kind_to_str(kind: MemberKind) -> &'static str {
    match kind {
        MemberKind::Property => "property",
        MemberKind::Method => "method",
        MemberKind::Field => "field",
        MemberKind::Event => "event",
    }
}

fn str_to_member_kind(s: &str) -> MemberKind {
    match s {
        "property" => MemberKind::Property,
        "method" => MemberKind::Method,
        "field" => MemberKind::Field,
        "event" => MemberKind::Event,
        _ => MemberKind::Property,
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn make_symbol(name: &str, qualified: &str, file: &str, line: u32) -> Symbol {
        Symbol {
            name: name.to_string(),
            qualified: qualified.to_string(),
            kind: SymbolKind::Function,
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

    // =========================================================================
    // Schema Tests
    // =========================================================================

    #[test]
    fn test_create_in_memory_database() {
        let index = SqliteIndex::in_memory().unwrap();
        assert_eq!(index.get_schema_version().unwrap(), SCHEMA_VERSION);
    }

    #[test]
    fn test_metadata_operations() {
        let index = SqliteIndex::in_memory().unwrap();

        index.set_metadata("test_key", "test_value").unwrap();
        assert_eq!(
            index.get_metadata("test_key").unwrap(),
            Some("test_value".to_string())
        );
        assert_eq!(index.get_metadata("nonexistent").unwrap(), None);
    }

    #[test]
    fn test_create_database_file() {
        let temp_dir = tempfile::tempdir().unwrap();
        let db_path = temp_dir.path().join("test.db");

        let index = SqliteIndex::create(&db_path).unwrap();
        assert_eq!(index.count_symbols().unwrap(), 0);
        drop(index);

        // Should be able to reopen
        let index2 = SqliteIndex::open(&db_path).unwrap();
        assert_eq!(index2.get_schema_version().unwrap(), SCHEMA_VERSION);
    }

    #[test]
    fn test_create_fails_if_exists() {
        let temp_dir = tempfile::tempdir().unwrap();
        let db_path = temp_dir.path().join("test.db");

        SqliteIndex::create(&db_path).unwrap();
        let result = SqliteIndex::create(&db_path);
        assert!(result.is_err());
    }

    #[test]
    fn test_open_fails_if_not_exists() {
        let result = SqliteIndex::open(Path::new("/nonexistent/path.db"));
        assert!(result.is_err());
    }

    #[test]
    fn test_open_or_create() {
        let temp_dir = tempfile::tempdir().unwrap();
        let db_path = temp_dir.path().join("subdir/test.db");

        // Should create
        let index1 = SqliteIndex::open_or_create(&db_path).unwrap();
        index1
            .insert_symbol(&make_symbol("foo", "M.foo", "test.fs", 1))
            .unwrap();
        drop(index1);

        // Should open existing
        let index2 = SqliteIndex::open_or_create(&db_path).unwrap();
        assert_eq!(index2.count_symbols().unwrap(), 1);
    }

    // =========================================================================
    // Symbol Tests
    // =========================================================================

    #[test]
    fn test_insert_and_find_symbol() {
        let index = SqliteIndex::in_memory().unwrap();
        let symbol = make_symbol("helper", "Utils.helper", "src/Utils.fs", 10);

        let id = index.insert_symbol(&symbol).unwrap();
        assert!(id > 0);

        let found = index.find_by_qualified("Utils.helper").unwrap();
        assert!(found.is_some());
        let found = found.unwrap();
        assert_eq!(found.name, "helper");
        assert_eq!(found.qualified, "Utils.helper");
    }

    #[test]
    fn test_find_nonexistent_symbol() {
        let index = SqliteIndex::in_memory().unwrap();
        let found = index.find_by_qualified("NonExistent.symbol").unwrap();
        assert!(found.is_none());
    }

    #[test]
    fn test_find_all_by_qualified_overloads() {
        let index = SqliteIndex::in_memory().unwrap();

        // Insert two symbols with same qualified name (simulating overloads)
        index
            .insert_symbol(&make_symbol("parse", "Parser.parse", "src/Parser.fs", 10))
            .unwrap();
        index
            .insert_symbol(&make_symbol("parse", "Parser.parse", "src/Parser.fs", 20))
            .unwrap();

        let all = index.find_all_by_qualified("Parser.parse").unwrap();
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn test_search_symbols() {
        let index = SqliteIndex::in_memory().unwrap();

        index
            .insert_symbol(&make_symbol(
                "PaymentService",
                "App.PaymentService",
                "a.fs",
                1,
            ))
            .unwrap();
        index
            .insert_symbol(&make_symbol(
                "PaymentRequest",
                "App.PaymentRequest",
                "a.fs",
                2,
            ))
            .unwrap();
        index
            .insert_symbol(&make_symbol("OrderService", "App.OrderService", "b.fs", 1))
            .unwrap();

        let results = index.search("Payment%", 100, None).unwrap();
        assert_eq!(results.len(), 2);

        let results = index.search("Order%", 100, None).unwrap();
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_symbols_in_file() {
        let index = SqliteIndex::in_memory().unwrap();

        index
            .insert_symbol(&make_symbol("foo", "M.foo", "src/a.fs", 1))
            .unwrap();
        index
            .insert_symbol(&make_symbol("bar", "M.bar", "src/a.fs", 2))
            .unwrap();
        index
            .insert_symbol(&make_symbol("baz", "M.baz", "src/b.fs", 1))
            .unwrap();

        let symbols = index.symbols_in_file(Path::new("src/a.fs")).unwrap();
        assert_eq!(symbols.len(), 2);
    }

    #[test]
    fn test_delete_symbols_in_file() {
        let index = SqliteIndex::in_memory().unwrap();

        index
            .insert_symbol(&make_symbol("foo", "M.foo", "src/a.fs", 1))
            .unwrap();
        index
            .insert_symbol(&make_symbol("bar", "M.bar", "src/b.fs", 1))
            .unwrap();

        let deleted = index.delete_symbols_in_file(Path::new("src/a.fs")).unwrap();
        assert_eq!(deleted, 1);
        assert_eq!(index.count_symbols().unwrap(), 1);
    }

    #[test]
    fn test_insert_symbols_batch() {
        let index = SqliteIndex::in_memory().unwrap();

        let symbols = vec![
            make_symbol("a", "M.a", "test.fs", 1),
            make_symbol("b", "M.b", "test.fs", 2),
            make_symbol("c", "M.c", "test.fs", 3),
        ];

        index.insert_symbols(&symbols).unwrap();
        assert_eq!(index.count_symbols().unwrap(), 3);
    }

    #[test]
    fn test_list_files() {
        let index = SqliteIndex::in_memory().unwrap();

        index
            .insert_symbol(&make_symbol("a", "M.a", "src/A.fs", 1))
            .unwrap();
        index
            .insert_symbol(&make_symbol("b", "M.b", "src/B.fs", 1))
            .unwrap();
        index
            .insert_symbol(&make_symbol("c", "M.c", "src/A.fs", 2))
            .unwrap();

        let files = index.list_files().unwrap();
        assert_eq!(files.len(), 2);
        assert!(files.contains(&PathBuf::from("src/A.fs")));
        assert!(files.contains(&PathBuf::from("src/B.fs")));
    }

    // =========================================================================
    // Type Signature Tests
    // =========================================================================

    #[test]
    fn test_insert_symbol_with_type() {
        let index = SqliteIndex::in_memory().unwrap();
        let symbol = make_symbol("process", "Service.process", "src/Service.fs", 10);

        index
            .insert_symbol_with_type(&symbol, "User -> Async<Result<Response, Error>>")
            .unwrap();

        let type_sig = index.get_symbol_type("Service.process").unwrap();
        assert_eq!(
            type_sig,
            Some("User -> Async<Result<Response, Error>>".to_string())
        );
    }

    #[test]
    fn test_update_symbol_type() {
        let index = SqliteIndex::in_memory().unwrap();
        let symbol = make_symbol("foo", "M.foo", "test.fs", 1);

        index.insert_symbol(&symbol).unwrap();
        assert!(index.get_symbol_type("M.foo").unwrap().is_none());

        index.update_symbol_type("M.foo", "int -> string").unwrap();
        assert_eq!(
            index.get_symbol_type("M.foo").unwrap(),
            Some("int -> string".to_string())
        );
    }

    // =========================================================================
    // Reference Tests
    // =========================================================================

    #[test]
    fn test_insert_and_find_references() {
        let index = SqliteIndex::in_memory().unwrap();

        let reference = Reference {
            name: "helper".to_string(),
            location: Location::new(PathBuf::from("src/Main.fs"), 10, 5),
        };

        index
            .insert_reference(Path::new("src/Main.fs"), &reference)
            .unwrap();

        let refs = index.find_references("helper").unwrap();
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].name, "helper");
    }

    #[test]
    fn test_references_in_file() {
        let index = SqliteIndex::in_memory().unwrap();

        let ref1 = Reference {
            name: "foo".to_string(),
            location: Location::new(PathBuf::from("src/Main.fs"), 10, 5),
        };
        let ref2 = Reference {
            name: "bar".to_string(),
            location: Location::new(PathBuf::from("src/Main.fs"), 20, 5),
        };

        index
            .insert_reference(Path::new("src/Main.fs"), &ref1)
            .unwrap();
        index
            .insert_reference(Path::new("src/Main.fs"), &ref2)
            .unwrap();

        let refs = index.references_in_file(Path::new("src/Main.fs")).unwrap();
        assert_eq!(refs.len(), 2);
    }

    // =========================================================================
    // Opens Tests
    // =========================================================================

    #[test]
    fn test_insert_and_get_opens() {
        let index = SqliteIndex::in_memory().unwrap();

        index
            .insert_open(Path::new("src/Main.fs"), "System", 1)
            .unwrap();
        index
            .insert_open(Path::new("src/Main.fs"), "FSharp.Core", 2)
            .unwrap();

        let opens = index.opens_for_file(Path::new("src/Main.fs")).unwrap();
        assert_eq!(opens.len(), 2);
        assert_eq!(opens[0], "System");
        assert_eq!(opens[1], "FSharp.Core");
    }

    // =========================================================================
    // Type Member Tests
    // =========================================================================

    #[test]
    fn test_insert_and_get_members() {
        let index = SqliteIndex::in_memory().unwrap();

        let member = TypeMember {
            type_name: "User".to_string(),
            member: "Name".to_string(),
            member_type: "string".to_string(),
            kind: MemberKind::Property,
        };

        index.insert_member(&member).unwrap();

        let members = index.get_members("User").unwrap();
        assert_eq!(members.len(), 1);
        assert_eq!(members[0].member, "Name");
        assert_eq!(members[0].kind, MemberKind::Property);
    }

    #[test]
    fn test_get_specific_member() {
        let index = SqliteIndex::in_memory().unwrap();

        index
            .insert_member(&TypeMember {
                type_name: "User".to_string(),
                member: "Name".to_string(),
                member_type: "string".to_string(),
                kind: MemberKind::Property,
            })
            .unwrap();

        index
            .insert_member(&TypeMember {
                type_name: "User".to_string(),
                member: "Save".to_string(),
                member_type: "unit -> Async<unit>".to_string(),
                kind: MemberKind::Method,
            })
            .unwrap();

        let member = index.get_member("User", "Save").unwrap();
        assert!(member.is_some());
        assert_eq!(member.unwrap().kind, MemberKind::Method);

        let member = index.get_member("User", "NonExistent").unwrap();
        assert!(member.is_none());
    }

    #[test]
    fn test_insert_members_batch() {
        let index = SqliteIndex::in_memory().unwrap();

        let members = vec![
            TypeMember {
                type_name: "User".to_string(),
                member: "Name".to_string(),
                member_type: "string".to_string(),
                kind: MemberKind::Property,
            },
            TypeMember {
                type_name: "User".to_string(),
                member: "Age".to_string(),
                member_type: "int".to_string(),
                kind: MemberKind::Property,
            },
        ];

        index.insert_members(&members).unwrap();

        let all = index.get_members("User").unwrap();
        assert_eq!(all.len(), 2);
    }

    // =========================================================================
    // Clear File Tests
    // =========================================================================

    #[test]
    fn test_clear_file() {
        let index = SqliteIndex::in_memory().unwrap();

        // Add symbols, references, and opens for a file
        index
            .insert_symbol(&make_symbol("foo", "M.foo", "src/Test.fs", 1))
            .unwrap();
        index
            .insert_reference(
                Path::new("src/Test.fs"),
                &Reference {
                    name: "bar".to_string(),
                    location: Location::new(PathBuf::from("src/Test.fs"), 5, 1),
                },
            )
            .unwrap();
        index
            .insert_open(Path::new("src/Test.fs"), "System", 1)
            .unwrap();

        // Clear the file
        index.clear_file(Path::new("src/Test.fs")).unwrap();

        // All should be empty
        assert_eq!(
            index
                .symbols_in_file(Path::new("src/Test.fs"))
                .unwrap()
                .len(),
            0
        );
        assert_eq!(
            index
                .references_in_file(Path::new("src/Test.fs"))
                .unwrap()
                .len(),
            0
        );
        assert_eq!(
            index
                .opens_for_file(Path::new("src/Test.fs"))
                .unwrap()
                .len(),
            0
        );
    }

    // =========================================================================
    // FTS5 Search Tests
    // =========================================================================

    #[test]
    fn test_search_fts_prefix() {
        let index = SqliteIndex::in_memory().unwrap();

        index
            .insert_symbol(&make_symbol(
                "PaymentService",
                "App.PaymentService",
                "a.fs",
                1,
            ))
            .unwrap();
        index
            .insert_symbol(&make_symbol(
                "PaymentRequest",
                "App.PaymentRequest",
                "a.fs",
                2,
            ))
            .unwrap();
        index
            .insert_symbol(&make_symbol("OrderService", "App.OrderService", "b.fs", 1))
            .unwrap();

        // FTS5 prefix search
        let results = index.search_fts("Payment*", 100, None).unwrap();
        assert_eq!(results.len(), 2);

        // FTS5 exact word (becomes prefix)
        let results = index.search_fts("Order", 100, None).unwrap();
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_search_fts_falls_back_for_suffix() {
        let index = SqliteIndex::in_memory().unwrap();

        index
            .insert_symbol(&make_symbol(
                "PaymentService",
                "App.PaymentService",
                "a.fs",
                1,
            ))
            .unwrap();
        index
            .insert_symbol(&make_symbol("OrderService", "App.OrderService", "b.fs", 1))
            .unwrap();
        index
            .insert_symbol(&make_symbol("OrderHandler", "App.OrderHandler", "b.fs", 2))
            .unwrap();

        // Suffix search falls back to LIKE
        let results = index.search_fts("*Service", 100, None).unwrap();
        assert_eq!(results.len(), 2);

        // Contains search falls back to LIKE
        let results = index.search_fts("*Order*", 100, None).unwrap();
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_fts_sync_on_delete() {
        let index = SqliteIndex::in_memory().unwrap();

        index
            .insert_symbol(&make_symbol("foo", "M.foo", "src/a.fs", 1))
            .unwrap();
        index
            .insert_symbol(&make_symbol("bar", "M.bar", "src/b.fs", 1))
            .unwrap();

        // Should find foo
        let results = index.search_fts("foo", 100, None).unwrap();
        assert_eq!(results.len(), 1);

        // Delete file with foo
        index.delete_symbols_in_file(Path::new("src/a.fs")).unwrap();

        // FTS index should be updated - no more foo
        let results = index.search_fts("foo", 100, None).unwrap();
        assert_eq!(results.len(), 0);

        // bar should still be there
        let results = index.search_fts("bar", 100, None).unwrap();
        assert_eq!(results.len(), 1);
    }
}
