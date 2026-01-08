//! Symbol importance ranking for repo maps.
//!
//! This module provides ranking functionality to identify the most important
//! symbols in a codebase based on how widely they are referenced.
//!
//! # Algorithm
//!
//! Unlike PageRank (used by Aider), we use a simpler **weighted file diversity**
//! approach that can be computed with a single SQL query:
//!
//! 1. **Primary signal**: Number of distinct files that reference a symbol
//!    - A symbol referenced by 10 different files is more important than one
//!      referenced 50 times from a single file
//!
//! 2. **Secondary signal**: Symbol kind weight
//!    - Modules/Classes > Functions > Variables
//!
//! 3. **Tertiary signal**: Visibility
//!    - Public > Internal > Private
//!
//! # Example
//!
//! ```ignore
//! use rocketindex::{SqliteIndex, ranking};
//!
//! let index = SqliteIndex::open(path)?;
//! let ranked = index.rank_symbols(50)?;
//!
//! for symbol in ranked {
//!     println!("{} - referenced by {} files",
//!         symbol.symbol.qualified,
//!         symbol.file_diversity);
//! }
//! ```

use std::path::PathBuf;

use crate::{Symbol, SymbolKind, Visibility};

/// A symbol with its computed importance ranking.
#[derive(Debug, Clone)]
pub struct RankedSymbol {
    /// The symbol being ranked
    pub symbol: Symbol,
    /// Number of distinct files that reference this symbol
    pub file_diversity: usize,
    /// Total number of references to this symbol
    pub total_refs: usize,
    /// Computed importance score (higher = more important)
    pub score: f64,
}

/// Configuration for ranking behavior.
#[derive(Debug, Clone)]
pub struct RankingConfig {
    /// Weight for file diversity signal (default: 1.0)
    pub diversity_weight: f64,
    /// Weight for symbol kind (default: 0.3)
    pub kind_weight: f64,
    /// Weight for visibility (default: 0.1)
    pub visibility_weight: f64,
}

impl Default for RankingConfig {
    fn default() -> Self {
        Self {
            diversity_weight: 1.0,
            kind_weight: 0.3,
            visibility_weight: 0.1,
        }
    }
}

/// Detail level for repo map output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DetailLevel {
    /// Top N most important symbols across the project
    #[default]
    Summary,
    /// All files with ranked symbols (limited per file)
    Normal,
    /// Full output with all symbols (no ranking)
    Full,
}

impl DetailLevel {
    /// Parse detail level from string.
    pub fn parse(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "summary" => DetailLevel::Summary,
            "normal" => DetailLevel::Normal,
            "full" => DetailLevel::Full,
            _ => DetailLevel::Summary,
        }
    }
}

/// Get the weight for a symbol kind (higher = more important).
pub fn kind_weight(kind: SymbolKind) -> u32 {
    match kind {
        SymbolKind::Module => 5,
        SymbolKind::Class => 4,
        SymbolKind::Interface => 4,
        SymbolKind::Record => 3,
        SymbolKind::Union => 3,
        SymbolKind::Type => 2,
        SymbolKind::Function => 2,
        SymbolKind::Value => 1,
        SymbolKind::Member => 1,
    }
}

/// Get the weight for visibility (higher = more important).
pub fn visibility_weight(visibility: Visibility) -> u32 {
    match visibility {
        Visibility::Public => 3,
        Visibility::Internal => 2,
        Visibility::Private => 1,
    }
}

/// Compute the importance score for a symbol.
pub fn compute_score(
    file_diversity: usize,
    total_refs: usize,
    kind: SymbolKind,
    visibility: Visibility,
    config: &RankingConfig,
) -> f64 {
    let diversity_score = file_diversity as f64 * config.diversity_weight;
    let kind_score = kind_weight(kind) as f64 * config.kind_weight;
    let visibility_score = visibility_weight(visibility) as f64 * config.visibility_weight;

    // Log scale for total refs to avoid dominating the score
    let ref_bonus = if total_refs > 0 {
        (total_refs as f64).ln() * 0.1
    } else {
        0.0
    };

    diversity_score + kind_score + visibility_score + ref_bonus
}

/// Group ranked symbols by file, preserving rank order within each file.
pub fn group_by_file(symbols: Vec<RankedSymbol>) -> Vec<(PathBuf, Vec<RankedSymbol>)> {
    use std::collections::BTreeMap;

    let mut by_file: BTreeMap<PathBuf, Vec<RankedSymbol>> = BTreeMap::new();

    for symbol in symbols {
        let file = symbol.symbol.location.file.clone();
        by_file.entry(file).or_default().push(symbol);
    }

    // Convert to vec, sorted by file path
    by_file.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_kind_weight_ordering() {
        assert!(kind_weight(SymbolKind::Module) > kind_weight(SymbolKind::Class));
        assert!(kind_weight(SymbolKind::Class) > kind_weight(SymbolKind::Function));
        assert!(kind_weight(SymbolKind::Function) > kind_weight(SymbolKind::Member));
    }

    #[test]
    fn test_visibility_weight_ordering() {
        assert!(visibility_weight(Visibility::Public) > visibility_weight(Visibility::Internal));
        assert!(visibility_weight(Visibility::Internal) > visibility_weight(Visibility::Private));
    }

    #[test]
    fn test_compute_score_diversity_dominant() {
        let config = RankingConfig::default();

        // Symbol referenced by 10 files should score higher than
        // symbol referenced 100 times from 1 file
        let score_diverse =
            compute_score(10, 10, SymbolKind::Function, Visibility::Public, &config);
        let score_concentrated =
            compute_score(1, 100, SymbolKind::Function, Visibility::Public, &config);

        assert!(score_diverse > score_concentrated);
    }

    #[test]
    fn test_detail_level_parsing() {
        assert_eq!(DetailLevel::parse("summary"), DetailLevel::Summary);
        assert_eq!(DetailLevel::parse("SUMMARY"), DetailLevel::Summary);
        assert_eq!(DetailLevel::parse("normal"), DetailLevel::Normal);
        assert_eq!(DetailLevel::parse("full"), DetailLevel::Full);
        assert_eq!(DetailLevel::parse("invalid"), DetailLevel::Summary);
    }
}
