//! Type cache for storing and querying type information extracted at build time.
//!
//! This module implements the type cache system described in RFC-001.
//! Type information is extracted from F# projects using FSharp.Compiler.Service
//! at build time and stored as JSON for fast querying by the Rust runtime.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

use crate::Result;

/// The current schema version for type cache files.
/// Increment this when making breaking changes to the format.
pub const TYPE_CACHE_VERSION: u32 = 1;

/// Kind of type member (property, method, field, etc.)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MemberKind {
    Property,
    Method,
    Field,
    Event,
}

impl std::fmt::Display for MemberKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MemberKind::Property => write!(f, "property"),
            MemberKind::Method => write!(f, "method"),
            MemberKind::Field => write!(f, "field"),
            MemberKind::Event => write!(f, "event"),
        }
    }
}

/// Parameter information for functions and methods
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParameterInfo {
    /// Parameter name
    pub name: String,
    /// Parameter type signature
    #[serde(rename = "type")]
    pub type_signature: String,
}

/// A symbol with its type information, as stored in the JSON cache
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypedSymbol {
    /// Short name of the symbol
    pub name: String,
    /// Fully qualified name
    pub qualified: String,
    /// Type signature (e.g., "string", "int -> string", "User -> Async<Result<Response, Error>>")
    #[serde(rename = "type")]
    pub type_signature: String,
    /// Source file path
    pub file: String,
    /// Line number (1-indexed)
    pub line: u32,
    /// Optional parameter information for functions
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parameters: Vec<ParameterInfo>,
}

/// A type member (property, method, field) as stored in the JSON cache
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypeMember {
    /// The type this member belongs to (e.g., "User", "string")
    #[serde(rename = "type")]
    pub type_name: String,
    /// The member name (e.g., "Name", "Length", "ToString")
    pub member: String,
    /// The member's type signature
    pub member_type: String,
    /// Kind of member
    pub kind: MemberKind,
}

/// The JSON schema for the type cache file
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TypeCacheSchema {
    /// Schema version for compatibility checking
    pub version: u32,
    /// ISO 8601 timestamp of when the cache was generated
    pub extracted_at: String,
    /// Project name
    pub project: String,
    /// All symbols with their types
    pub symbols: Vec<TypedSymbol>,
    /// All type members
    pub members: Vec<TypeMember>,
}

/// Runtime-optimized type cache with fast lookups
#[derive(Debug, Clone)]
pub struct TypeCache {
    /// Schema version
    version: u32,
    /// Project name
    project: String,
    /// When the cache was extracted
    extracted_at: String,
    /// qualified_name -> type signature
    symbol_types: HashMap<String, TypedSymbol>,
    /// type_name -> Vec<TypeMember>
    type_members: HashMap<String, Vec<TypeMember>>,
}

impl TypeCache {
    /// Create a new empty TypeCache
    pub fn new() -> Self {
        Self {
            version: TYPE_CACHE_VERSION,
            project: String::new(),
            extracted_at: String::new(),
            symbol_types: HashMap::new(),
            type_members: HashMap::new(),
        }
    }

    /// Load a type cache from a JSON file
    pub fn load(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)?;
        Self::from_json(&content)
    }

    /// Parse a type cache from JSON string
    pub fn from_json(json: &str) -> Result<Self> {
        let schema: TypeCacheSchema = serde_json::from_str(json)?;
        Ok(Self::from_schema(schema))
    }

    /// Build a TypeCache from the JSON schema
    pub fn from_schema(schema: TypeCacheSchema) -> Self {
        let mut symbol_types = HashMap::new();
        for sym in schema.symbols {
            symbol_types.insert(sym.qualified.clone(), sym);
        }

        let mut type_members: HashMap<String, Vec<TypeMember>> = HashMap::new();
        for member in schema.members {
            type_members
                .entry(member.type_name.clone())
                .or_default()
                .push(member);
        }

        Self {
            version: schema.version,
            project: schema.project,
            extracted_at: schema.extracted_at,
            symbol_types,
            type_members,
        }
    }

    /// Get the type signature of a symbol by its qualified name
    pub fn get_type(&self, qualified_name: &str) -> Option<&str> {
        self.symbol_types
            .get(qualified_name)
            .map(|s| s.type_signature.as_str())
    }

    /// Get full symbol information by qualified name
    pub fn get_symbol(&self, qualified_name: &str) -> Option<&TypedSymbol> {
        self.symbol_types.get(qualified_name)
    }

    /// Get all members of a type
    pub fn get_members(&self, type_name: &str) -> Option<&[TypeMember]> {
        self.type_members.get(type_name).map(|v| v.as_slice())
    }

    /// Get a specific member of a type
    pub fn get_member(&self, type_name: &str, member_name: &str) -> Option<&TypeMember> {
        self.type_members
            .get(type_name)?
            .iter()
            .find(|m| m.member == member_name)
    }

    /// Get the schema version
    pub fn version(&self) -> u32 {
        self.version
    }

    /// Get the project name
    pub fn project(&self) -> &str {
        &self.project
    }

    /// Get the extraction timestamp
    pub fn extracted_at(&self) -> &str {
        &self.extracted_at
    }

    /// Check if this cache is compatible with the current version
    pub fn is_compatible(&self) -> bool {
        self.version == TYPE_CACHE_VERSION
    }

    /// Get the number of symbols in the cache
    pub fn symbol_count(&self) -> usize {
        self.symbol_types.len()
    }

    /// Get the number of types with members in the cache
    pub fn type_count(&self) -> usize {
        self.type_members.len()
    }

    /// Check if the cache is empty
    pub fn is_empty(&self) -> bool {
        self.symbol_types.is_empty() && self.type_members.is_empty()
    }
}

impl Default for TypeCache {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Sample JSON matching the RFC schema for testing
    fn sample_cache_json() -> &'static str {
        r#"{
            "version": 1,
            "extracted_at": "2024-12-02T10:30:00Z",
            "project": "RocketSpec.Core",
            "symbols": [
                {
                    "name": "myString",
                    "qualified": "MyModule.myString",
                    "type": "string",
                    "file": "src/MyModule.fs",
                    "line": 42
                },
                {
                    "name": "processUser",
                    "qualified": "UserService.processUser",
                    "type": "User -> Async<Result<Response, Error>>",
                    "file": "src/UserService.fs",
                    "line": 15,
                    "parameters": [
                        { "name": "user", "type": "User" }
                    ]
                }
            ],
            "members": [
                {
                    "type": "User",
                    "member": "Name",
                    "member_type": "string",
                    "kind": "property"
                },
                {
                    "type": "User",
                    "member": "Save",
                    "member_type": "unit -> Async<unit>",
                    "kind": "method"
                },
                {
                    "type": "string",
                    "member": "Length",
                    "member_type": "int",
                    "kind": "property"
                }
            ]
        }"#
    }

    // =========================================================================
    // Task 1.1: Schema Types Tests
    // =========================================================================

    #[test]
    fn test_deserialize_typed_symbol() {
        let json = r#"{
            "name": "helper",
            "qualified": "MyApp.Utils.helper",
            "type": "int -> string",
            "file": "src/Utils.fs",
            "line": 10
        }"#;

        let symbol: TypedSymbol = serde_json::from_str(json).unwrap();
        assert_eq!(symbol.name, "helper");
        assert_eq!(symbol.qualified, "MyApp.Utils.helper");
        assert_eq!(symbol.type_signature, "int -> string");
        assert_eq!(symbol.file, "src/Utils.fs");
        assert_eq!(symbol.line, 10);
        assert!(symbol.parameters.is_empty());
    }

    #[test]
    fn test_deserialize_typed_symbol_with_parameters() {
        let json = r#"{
            "name": "processUser",
            "qualified": "UserService.processUser",
            "type": "User -> Async<Result<Response, Error>>",
            "file": "src/UserService.fs",
            "line": 15,
            "parameters": [
                { "name": "user", "type": "User" }
            ]
        }"#;

        let symbol: TypedSymbol = serde_json::from_str(json).unwrap();
        assert_eq!(symbol.parameters.len(), 1);
        assert_eq!(symbol.parameters[0].name, "user");
        assert_eq!(symbol.parameters[0].type_signature, "User");
    }

    #[test]
    fn test_deserialize_type_member() {
        let json = r#"{
            "type": "User",
            "member": "Name",
            "member_type": "string",
            "kind": "property"
        }"#;

        let member: TypeMember = serde_json::from_str(json).unwrap();
        assert_eq!(member.type_name, "User");
        assert_eq!(member.member, "Name");
        assert_eq!(member.member_type, "string");
        assert_eq!(member.kind, MemberKind::Property);
    }

    #[test]
    fn test_deserialize_member_kinds() {
        let property: TypeMember = serde_json::from_str(
            r#"{"type": "T", "member": "X", "member_type": "int", "kind": "property"}"#,
        )
        .unwrap();
        let method: TypeMember = serde_json::from_str(
            r#"{"type": "T", "member": "X", "member_type": "int", "kind": "method"}"#,
        )
        .unwrap();
        let field: TypeMember = serde_json::from_str(
            r#"{"type": "T", "member": "X", "member_type": "int", "kind": "field"}"#,
        )
        .unwrap();

        assert_eq!(property.kind, MemberKind::Property);
        assert_eq!(method.kind, MemberKind::Method);
        assert_eq!(field.kind, MemberKind::Field);
    }

    #[test]
    fn test_deserialize_full_cache_schema() {
        let schema: TypeCacheSchema = serde_json::from_str(sample_cache_json()).unwrap();

        assert_eq!(schema.version, 1);
        assert_eq!(schema.extracted_at, "2024-12-02T10:30:00Z");
        assert_eq!(schema.project, "RocketSpec.Core");
        assert_eq!(schema.symbols.len(), 2);
        assert_eq!(schema.members.len(), 3);
    }

    #[test]
    fn test_serialize_roundtrip() {
        let original: TypeCacheSchema = serde_json::from_str(sample_cache_json()).unwrap();
        let json = serde_json::to_string(&original).unwrap();
        let parsed: TypeCacheSchema = serde_json::from_str(&json).unwrap();

        assert_eq!(original.version, parsed.version);
        assert_eq!(original.project, parsed.project);
        assert_eq!(original.symbols.len(), parsed.symbols.len());
        assert_eq!(original.members.len(), parsed.members.len());
    }

    // =========================================================================
    // Task 1.2: TypeCache Runtime Structure Tests
    // =========================================================================

    #[test]
    fn test_type_cache_from_json() {
        let cache = TypeCache::from_json(sample_cache_json()).unwrap();

        assert_eq!(cache.version(), 1);
        assert_eq!(cache.project(), "RocketSpec.Core");
        assert_eq!(cache.extracted_at(), "2024-12-02T10:30:00Z");
        assert!(cache.is_compatible());
    }

    #[test]
    fn test_get_type_by_qualified_name() {
        let cache = TypeCache::from_json(sample_cache_json()).unwrap();

        assert_eq!(cache.get_type("MyModule.myString"), Some("string"));
        assert_eq!(
            cache.get_type("UserService.processUser"),
            Some("User -> Async<Result<Response, Error>>")
        );
        assert_eq!(cache.get_type("NonExistent.symbol"), None);
    }

    #[test]
    fn test_get_symbol_by_qualified_name() {
        let cache = TypeCache::from_json(sample_cache_json()).unwrap();

        let symbol = cache.get_symbol("MyModule.myString").unwrap();
        assert_eq!(symbol.name, "myString");
        assert_eq!(symbol.file, "src/MyModule.fs");
        assert_eq!(symbol.line, 42);
    }

    #[test]
    fn test_get_members_of_type() {
        let cache = TypeCache::from_json(sample_cache_json()).unwrap();

        let user_members = cache.get_members("User").unwrap();
        assert_eq!(user_members.len(), 2);

        let member_names: Vec<&str> = user_members.iter().map(|m| m.member.as_str()).collect();
        assert!(member_names.contains(&"Name"));
        assert!(member_names.contains(&"Save"));
    }

    #[test]
    fn test_get_members_of_nonexistent_type() {
        let cache = TypeCache::from_json(sample_cache_json()).unwrap();

        assert!(cache.get_members("NonExistentType").is_none());
    }

    #[test]
    fn test_get_specific_member() {
        let cache = TypeCache::from_json(sample_cache_json()).unwrap();

        let name_prop = cache.get_member("User", "Name").unwrap();
        assert_eq!(name_prop.member_type, "string");
        assert_eq!(name_prop.kind, MemberKind::Property);

        let save_method = cache.get_member("User", "Save").unwrap();
        assert_eq!(save_method.member_type, "unit -> Async<unit>");
        assert_eq!(save_method.kind, MemberKind::Method);

        // Member on different type
        let length = cache.get_member("string", "Length").unwrap();
        assert_eq!(length.member_type, "int");
    }

    #[test]
    fn test_get_nonexistent_member() {
        let cache = TypeCache::from_json(sample_cache_json()).unwrap();

        assert!(cache.get_member("User", "NonExistent").is_none());
        assert!(cache.get_member("NonExistentType", "Name").is_none());
    }

    #[test]
    fn test_symbol_count() {
        let cache = TypeCache::from_json(sample_cache_json()).unwrap();

        assert_eq!(cache.symbol_count(), 2);
    }

    #[test]
    fn test_type_count() {
        let cache = TypeCache::from_json(sample_cache_json()).unwrap();

        // User and string have members
        assert_eq!(cache.type_count(), 2);
    }

    #[test]
    fn test_empty_cache() {
        let cache = TypeCache::new();

        assert!(cache.is_empty());
        assert_eq!(cache.symbol_count(), 0);
        assert_eq!(cache.type_count(), 0);
        assert!(cache.is_compatible());
    }

    #[test]
    fn test_incompatible_version() {
        let json = r#"{
            "version": 999,
            "extracted_at": "2024-12-02T10:30:00Z",
            "project": "Test",
            "symbols": [],
            "members": []
        }"#;

        let cache = TypeCache::from_json(json).unwrap();
        assert!(!cache.is_compatible());
    }

    #[test]
    fn test_invalid_json_returns_error() {
        let result = TypeCache::from_json("not valid json");
        assert!(result.is_err());
    }

    #[test]
    fn test_missing_fields_returns_error() {
        let json = r#"{ "version": 1 }"#;
        let result = TypeCache::from_json(json);
        assert!(result.is_err());
    }

    // =========================================================================
    // Task 1.2: File Loading Tests
    // =========================================================================

    #[test]
    fn test_load_from_file() {
        use std::io::Write;
        let temp_dir = tempfile::tempdir().unwrap();
        let cache_path = temp_dir.path().join("cache.json");

        let mut file = std::fs::File::create(&cache_path).unwrap();
        file.write_all(sample_cache_json().as_bytes()).unwrap();

        let cache = TypeCache::load(&cache_path).unwrap();
        assert_eq!(cache.symbol_count(), 2);
        assert_eq!(cache.project(), "RocketSpec.Core");
    }

    #[test]
    fn test_load_missing_file_returns_error() {
        let result = TypeCache::load(Path::new("/nonexistent/path/cache.json"));
        assert!(result.is_err());
    }

    // =========================================================================
    // Display trait tests
    // =========================================================================

    #[test]
    fn test_member_kind_display() {
        assert_eq!(format!("{}", MemberKind::Property), "property");
        assert_eq!(format!("{}", MemberKind::Method), "method");
        assert_eq!(format!("{}", MemberKind::Field), "field");
        assert_eq!(format!("{}", MemberKind::Event), "event");
    }
}
