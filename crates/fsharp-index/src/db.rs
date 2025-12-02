//! SQLite-based index storage for fsharp-tools (RFC-001).
//!
//! This module provides persistent storage for the F# symbol index using SQLite.
//! Benefits over the previous JSON approach:
//! - O(log n) indexed lookups vs O(n) linear scan
//! - Low memory: query on-demand, don't load entire index
//! - Incremental updates: UPDATE single rows, no full rewrite
//! - Rich queries: LIKE patterns, JOINs for references
//! - Debuggable: inspect with `sqlite3` CLI

use std::path::{Path, PathBuf};

use rusqlite::{params, Connection, OptionalExtension};

use crate::index::Reference;
use crate::type_cache::{MemberKind, TypeMember};
use crate::{IndexError, Location, Result, Symbol, SymbolKind, Visibility};

/// Current schema version. Increment when making breaking changes.
pub const SCHEMA_VERSION: u32 = 1;

/// Default database filename within .fsharp-index/
pub const DEFAULT_DB_NAME: &str = "index.db";

/// SQLite-based index for F# symbols.
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
        let index = Self { conn };

        // Verify schema version
        let version = index.get_schema_version()?;
        if version != SCHEMA_VERSION {
            return Err(IndexError::IoError(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "Schema version mismatch: expected {}, found {}",
                    SCHEMA_VERSION, version
                ),
            )));
        }

        Ok(index)
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
            "INSERT INTO symbols (name, qualified, kind, file, line, column, end_line, end_column, visibility, source)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'syntactic')",
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
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Insert a symbol with type signature.
    pub fn insert_symbol_with_type(&self, symbol: &Symbol, type_signature: &str) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO symbols (name, qualified, kind, type_signature, file, line, column, end_line, end_column, visibility, source)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 'semantic')",
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
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Insert multiple symbols in a transaction for efficiency.
    pub fn insert_symbols(&self, symbols: &[Symbol]) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO symbols (name, qualified, kind, file, line, column, end_line, end_column, visibility, source)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'syntactic')",
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
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Find a symbol by its qualified name. Returns the first match.
    #[must_use = "query results should not be ignored"]
    pub fn find_by_qualified(&self, qualified: &str) -> Result<Option<Symbol>> {
        let symbol = self
            .conn
            .query_row(
                "SELECT name, qualified, kind, file, line, column, end_line, end_column, visibility
                 FROM symbols WHERE qualified = ?1 LIMIT 1",
                params![qualified],
                row_to_symbol,
            )
            .optional()?;
        Ok(symbol)
    }

    /// Find all symbols with the given qualified name (for overloads).
    pub fn find_all_by_qualified(&self, qualified: &str) -> Result<Vec<Symbol>> {
        let mut stmt = self.conn.prepare(
            "SELECT name, qualified, kind, file, line, column, end_line, end_column, visibility
             FROM symbols WHERE qualified = ?1",
        )?;

        let symbols = stmt
            .query_map(params![qualified], row_to_symbol)?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(symbols)
    }

    /// Search for symbols matching a pattern. Supports SQL LIKE wildcards (% and _).
    #[must_use = "search results should not be ignored"]
    pub fn search(&self, pattern: &str, limit: usize) -> Result<Vec<Symbol>> {
        // Convert glob-style wildcards to SQL LIKE
        let sql_pattern = pattern.replace('*', "%").replace('?', "_");

        let mut stmt = self.conn.prepare(
            "SELECT name, qualified, kind, file, line, column, end_line, end_column, visibility
             FROM symbols
             WHERE name LIKE ?1 OR qualified LIKE ?1
             LIMIT ?2",
        )?;

        let symbols = stmt
            .query_map(params![sql_pattern, limit as i64], row_to_symbol)?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(symbols)
    }

    /// Get all symbols defined in a file.
    pub fn symbols_in_file(&self, file: &Path) -> Result<Vec<Symbol>> {
        let file_str = file.to_string_lossy();
        let mut stmt = self.conn.prepare(
            "SELECT name, qualified, kind, file, line, column, end_line, end_column, visibility
             FROM symbols WHERE file = ?1",
        )?;

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

    /// Find all references to a name (short or qualified).
    pub fn find_references(&self, name: &str) -> Result<Vec<Reference>> {
        let mut stmt = self.conn.prepare(
            "SELECT name, file, line, column FROM refs
             WHERE name = ?1 OR name LIKE '%.' || ?1",
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
    source TEXT DEFAULT 'syntactic'
);

CREATE INDEX IF NOT EXISTS idx_symbols_qualified ON symbols(qualified);
CREATE INDEX IF NOT EXISTS idx_symbols_name ON symbols(name);
CREATE INDEX IF NOT EXISTS idx_symbols_file ON symbols(file);
CREATE INDEX IF NOT EXISTS idx_symbols_kind ON symbols(kind);

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

    Ok(Symbol {
        name,
        qualified,
        kind: str_to_symbol_kind(&kind_str),
        location: Location::with_end(PathBuf::from(file), line, column, end_line, end_column),
        visibility: str_to_visibility(&visibility_str),
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

fn symbol_kind_to_str(kind: SymbolKind) -> &'static str {
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

fn visibility_to_str(vis: Visibility) -> &'static str {
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

        let results = index.search("Payment%", 100).unwrap();
        assert_eq!(results.len(), 2);

        let results = index.search("Order%", 100).unwrap();
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
}
