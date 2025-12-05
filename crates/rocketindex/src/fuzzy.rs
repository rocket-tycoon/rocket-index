//! Fuzzy string matching utilities for symbol lookup error recovery.
//!
//! Provides Levenshtein distance calculation and similar string suggestions
//! to help agents recover from typos in symbol names.

/// Calculate the Levenshtein (edit) distance between two strings.
///
/// The edit distance is the minimum number of single-character edits
/// (insertions, deletions, or substitutions) required to transform
/// one string into another.
///
/// # Examples
///
/// ```
/// use rocketindex::fuzzy::levenshtein_distance;
///
/// assert_eq!(levenshtein_distance("kitten", "sitting"), 3);
/// assert_eq!(levenshtein_distance("", "abc"), 3);
/// assert_eq!(levenshtein_distance("abc", "abc"), 0);
/// ```
#[must_use]
pub fn levenshtein_distance(a: &str, b: &str) -> usize {
    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();

    let m = a_chars.len();
    let n = b_chars.len();

    // Handle empty strings
    if m == 0 {
        return n;
    }
    if n == 0 {
        return m;
    }

    // Use two rows instead of full matrix for O(min(m,n)) space
    let mut prev_row: Vec<usize> = (0..=n).collect();
    let mut curr_row: Vec<usize> = vec![0; n + 1];

    for i in 1..=m {
        curr_row[0] = i;

        for j in 1..=n {
            let cost = if a_chars[i - 1] == b_chars[j - 1] {
                0
            } else {
                1
            };

            curr_row[j] = (prev_row[j] + 1) // deletion
                .min(curr_row[j - 1] + 1) // insertion
                .min(prev_row[j - 1] + cost); // substitution
        }

        std::mem::swap(&mut prev_row, &mut curr_row);
    }

    prev_row[n]
}

/// A suggestion with its edit distance from the query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestion {
    /// The suggested string
    pub value: String,
    /// Edit distance from the original query
    pub distance: usize,
}

impl Suggestion {
    pub fn new(value: String, distance: usize) -> Self {
        Self { value, distance }
    }
}

/// Find similar strings from a collection, sorted by edit distance.
///
/// Returns up to `max_suggestions` strings within `max_distance` edits,
/// sorted by distance (closest first), then alphabetically.
///
/// # Arguments
///
/// * `query` - The string to find similar matches for
/// * `candidates` - Iterator of candidate strings to search
/// * `max_distance` - Maximum edit distance to consider (typically 2-3)
/// * `max_suggestions` - Maximum number of suggestions to return
///
/// # Examples
///
/// ```
/// use rocketindex::fuzzy::find_similar;
///
/// let candidates = vec!["processPayment", "processOrder", "handlePayment"];
/// let suggestions = find_similar("procesPayment", candidates.iter().map(|s| *s), 2, 3);
///
/// assert_eq!(suggestions[0].value, "processPayment");
/// assert_eq!(suggestions[0].distance, 1);
/// ```
#[must_use]
pub fn find_similar<'a, I>(
    query: &str,
    candidates: I,
    max_distance: usize,
    max_suggestions: usize,
) -> Vec<Suggestion>
where
    I: Iterator<Item = &'a str>,
{
    let mut suggestions: Vec<Suggestion> = candidates
        .filter_map(|candidate| {
            let distance = levenshtein_distance(query, candidate);
            if distance <= max_distance && distance > 0 {
                Some(Suggestion::new(candidate.to_string(), distance))
            } else {
                None
            }
        })
        .collect();

    // Sort by distance first, then alphabetically for stability
    suggestions.sort_by(|a, b| a.distance.cmp(&b.distance).then(a.value.cmp(&b.value)));

    suggestions.truncate(max_suggestions);
    suggestions
}

/// Default maximum edit distance for suggestions.
pub const DEFAULT_MAX_DISTANCE: usize = 3;

/// Default maximum number of suggestions to return.
pub const DEFAULT_MAX_SUGGESTIONS: usize = 5;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_levenshtein_identical() {
        assert_eq!(levenshtein_distance("hello", "hello"), 0);
        assert_eq!(levenshtein_distance("", ""), 0);
    }

    #[test]
    fn test_levenshtein_empty() {
        assert_eq!(levenshtein_distance("", "abc"), 3);
        assert_eq!(levenshtein_distance("abc", ""), 3);
    }

    #[test]
    fn test_levenshtein_single_edit() {
        // Substitution
        assert_eq!(levenshtein_distance("cat", "bat"), 1);
        // Insertion
        assert_eq!(levenshtein_distance("cat", "cats"), 1);
        // Deletion
        assert_eq!(levenshtein_distance("cats", "cat"), 1);
    }

    #[test]
    fn test_levenshtein_classic_example() {
        // Classic example: kitten -> sitting
        // k->s, e->i, +g = 3 edits
        assert_eq!(levenshtein_distance("kitten", "sitting"), 3);
    }

    #[test]
    fn test_levenshtein_case_sensitive() {
        assert_eq!(levenshtein_distance("Hello", "hello"), 1);
        assert_eq!(levenshtein_distance("ABC", "abc"), 3);
    }

    #[test]
    fn test_levenshtein_typos() {
        // Common programming typos
        assert_eq!(levenshtein_distance("procesPayment", "processPayment"), 1);
        assert_eq!(levenshtein_distance("getUserNmae", "getUserName"), 2);
        assert_eq!(levenshtein_distance("valeu", "value"), 2);
    }

    #[test]
    fn test_find_similar_basic() {
        let candidates = ["apple", "apply", "banana", "application"];
        let suggestions = find_similar("aple", candidates.iter().copied(), 2, 5);

        assert!(!suggestions.is_empty());
        assert_eq!(suggestions[0].value, "apple");
        assert_eq!(suggestions[0].distance, 1);
    }

    #[test]
    fn test_find_similar_respects_max_distance() {
        let candidates = ["abc", "xyz", "abcd"];
        let suggestions = find_similar("ab", candidates.iter().copied(), 1, 5);

        // Only "abc" is within distance 1
        assert_eq!(suggestions.len(), 1);
        assert_eq!(suggestions[0].value, "abc");
    }

    #[test]
    fn test_find_similar_respects_max_suggestions() {
        let candidates = ["aa", "ab", "ac", "ad", "ae"];
        let suggestions = find_similar("a", candidates.iter().copied(), 2, 3);

        assert_eq!(suggestions.len(), 3);
    }

    #[test]
    fn test_find_similar_excludes_exact_match() {
        let candidates = ["hello", "hallo", "hullo"];
        let suggestions = find_similar("hello", candidates.iter().copied(), 2, 5);

        // Should not include exact match
        assert!(suggestions.iter().all(|s| s.value != "hello"));
    }

    #[test]
    fn test_find_similar_sorted_by_distance() {
        let candidates = ["abcdef", "abcd", "abcde"];
        let suggestions = find_similar("abc", candidates.iter().copied(), 3, 5);

        // Should be sorted by distance
        for i in 1..suggestions.len() {
            assert!(suggestions[i - 1].distance <= suggestions[i].distance);
        }
    }

    #[test]
    fn test_find_similar_programming_symbols() {
        let candidates = [
            "processPayment",
            "processOrder",
            "handlePayment",
            "PaymentProcessor",
            "processRefund",
        ];
        let suggestions = find_similar("procesPayment", candidates.iter().copied(), 2, 3);

        assert!(!suggestions.is_empty());
        assert_eq!(suggestions[0].value, "processPayment");
        assert_eq!(suggestions[0].distance, 1);
    }
}
