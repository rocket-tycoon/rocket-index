//! In-memory document store for tracking open files and their content.
//!
//! This module provides a thread-safe store for document content, enabling
//! the language server to work with unsaved changes instead of only saved files.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;
use tower_lsp::lsp_types::{Position, TextDocumentContentChangeEvent, Url};

#[allow(unused_imports)]
use tower_lsp::lsp_types::Url as _;

/// A document tracked in memory.
#[derive(Debug, Clone)]
pub struct Document {
    /// The current content of the document
    pub content: String,
    /// The document version (incremented on each change)
    pub version: i32,
}

impl Document {
    /// Create a new document with the given content.
    pub fn new(content: String, version: i32) -> Self {
        Self { content, version }
    }

    /// Apply incremental changes to the document content.
    ///
    /// Handles both full document sync and incremental changes.
    pub fn apply_changes(
        &mut self,
        changes: Vec<TextDocumentContentChangeEvent>,
        new_version: i32,
    ) {
        for change in changes {
            match change.range {
                Some(range) => {
                    // Incremental change - replace the specified range
                    let start_offset = self.position_to_offset(&range.start);
                    let end_offset = self.position_to_offset(&range.end);

                    if let (Some(start), Some(end)) = (start_offset, end_offset) {
                        let mut new_content = String::with_capacity(
                            self.content.len() - (end - start) + change.text.len(),
                        );
                        new_content.push_str(&self.content[..start]);
                        new_content.push_str(&change.text);
                        new_content.push_str(&self.content[end..]);
                        self.content = new_content;
                    }
                }
                None => {
                    // Full document sync - replace entire content
                    self.content = change.text;
                }
            }
        }
        self.version = new_version;
    }

    /// Convert a Position (line, character) to a byte offset in the content.
    fn position_to_offset(&self, position: &Position) -> Option<usize> {
        let mut offset = 0;
        let mut current_line = 0;

        for line in self.content.lines() {
            if current_line == position.line {
                // Found the target line - now find the character offset
                let char_offset = line
                    .char_indices()
                    .nth(position.character as usize)
                    .map(|(i, _)| i)
                    .unwrap_or(line.len());
                return Some(offset + char_offset);
            }
            // Move past this line and its newline character
            offset += line.len() + 1; // +1 for the newline
            current_line += 1;
        }

        // If we're at the end of the file
        if current_line == position.line && position.character == 0 {
            return Some(offset);
        }

        None
    }
}

/// Thread-safe store for open documents.
#[derive(Debug, Default)]
pub struct DocumentStore {
    /// Map from file path to document
    documents: Arc<RwLock<HashMap<PathBuf, Document>>>,
}

impl DocumentStore {
    /// Create a new empty document store.
    pub fn new() -> Self {
        Self {
            documents: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Open a document and store its content.
    pub async fn open(&self, uri: &Url, content: String, version: i32) {
        if let Ok(path) = uri.to_file_path() {
            let document = Document::new(content, version);
            self.documents.write().await.insert(path, document);
        }
    }

    /// Apply changes to an open document.
    pub async fn change(
        &self,
        uri: &Url,
        changes: Vec<TextDocumentContentChangeEvent>,
        version: i32,
    ) {
        if let Ok(path) = uri.to_file_path() {
            let mut docs = self.documents.write().await;
            if let Some(doc) = docs.get_mut(&path) {
                doc.apply_changes(changes, version);
            }
        }
    }

    /// Close a document and remove it from the store.
    pub async fn close(&self, uri: &Url) {
        if let Ok(path) = uri.to_file_path() {
            self.documents.write().await.remove(&path);
        }
    }

    /// Get the content of a document.
    ///
    /// Returns the in-memory content if the document is open,
    /// otherwise attempts to read from disk.
    pub async fn get_content(&self, path: &PathBuf) -> Option<String> {
        // First, check if the document is open in memory
        {
            let docs = self.documents.read().await;
            if let Some(doc) = docs.get(path) {
                return Some(doc.content.clone());
            }
        }

        // Fall back to reading from disk
        std::fs::read_to_string(path).ok()
    }
}

impl Clone for DocumentStore {
    fn clone(&self) -> Self {
        Self {
            documents: Arc::clone(&self.documents),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_position_to_offset_simple() {
        let doc = Document::new("hello\nworld\n".to_string(), 1);

        // First character of first line
        assert_eq!(doc.position_to_offset(&Position::new(0, 0)), Some(0));
        // Third character of first line
        assert_eq!(doc.position_to_offset(&Position::new(0, 2)), Some(2));
        // First character of second line
        assert_eq!(doc.position_to_offset(&Position::new(1, 0)), Some(6));
        // Third character of second line
        assert_eq!(doc.position_to_offset(&Position::new(1, 2)), Some(8));
    }

    #[test]
    fn test_apply_full_change() {
        let mut doc = Document::new("original content".to_string(), 1);

        doc.apply_changes(
            vec![TextDocumentContentChangeEvent {
                range: None,
                range_length: None,
                text: "new content".to_string(),
            }],
            2,
        );

        assert_eq!(doc.content, "new content");
        assert_eq!(doc.version, 2);
    }

    #[test]
    fn test_apply_incremental_change() {
        let mut doc = Document::new("hello world".to_string(), 1);

        // Replace "world" with "rust"
        doc.apply_changes(
            vec![TextDocumentContentChangeEvent {
                range: Some(tower_lsp::lsp_types::Range {
                    start: Position::new(0, 6),
                    end: Position::new(0, 11),
                }),
                range_length: None,
                text: "rust".to_string(),
            }],
            2,
        );

        assert_eq!(doc.content, "hello rust");
        assert_eq!(doc.version, 2);
    }

    #[test]
    fn test_apply_insert() {
        let mut doc = Document::new("hello world".to_string(), 1);

        // Insert "beautiful " before "world"
        doc.apply_changes(
            vec![TextDocumentContentChangeEvent {
                range: Some(tower_lsp::lsp_types::Range {
                    start: Position::new(0, 6),
                    end: Position::new(0, 6),
                }),
                range_length: None,
                text: "beautiful ".to_string(),
            }],
            2,
        );

        assert_eq!(doc.content, "hello beautiful world");
    }

    #[test]
    fn test_apply_delete() {
        let mut doc = Document::new("hello beautiful world".to_string(), 1);

        // Delete "beautiful "
        doc.apply_changes(
            vec![TextDocumentContentChangeEvent {
                range: Some(tower_lsp::lsp_types::Range {
                    start: Position::new(0, 6),
                    end: Position::new(0, 16),
                }),
                range_length: None,
                text: "".to_string(),
            }],
            2,
        );

        assert_eq!(doc.content, "hello world");
    }

    #[test]
    fn test_multiline_position() {
        let doc = Document::new("line one\nline two\nline three".to_string(), 1);

        // Start of line 2
        assert_eq!(doc.position_to_offset(&Position::new(2, 0)), Some(18));
        // "three" starts at position 5 on line 2
        assert_eq!(doc.position_to_offset(&Position::new(2, 5)), Some(23));
    }

    #[tokio::test]
    async fn test_document_store_open_close() {
        let store = DocumentStore::new();
        let uri = Url::parse("file:///test.fs").unwrap();
        let path = uri.to_file_path().unwrap();

        // Initially empty
        assert!(store.get_content(&path).await.is_none());

        // Open document
        store.open(&uri, "content".to_string(), 1).await;
        assert_eq!(store.get_content(&path).await, Some("content".to_string()));

        // Close document
        store.close(&uri).await;
        assert!(store.get_content(&path).await.is_none());
    }

    #[tokio::test]
    async fn test_document_store_change() {
        let store = DocumentStore::new();
        let uri = Url::parse("file:///test.fs").unwrap();
        let path = uri.to_file_path().unwrap();

        // Open document
        store.open(&uri, "hello".to_string(), 1).await;

        // Apply change
        store
            .change(
                &uri,
                vec![TextDocumentContentChangeEvent {
                    range: None,
                    range_length: None,
                    text: "goodbye".to_string(),
                }],
                2,
            )
            .await;

        assert_eq!(store.get_content(&path).await, Some("goodbye".to_string()));
    }
}
