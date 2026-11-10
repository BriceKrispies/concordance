/// Returns the input converted to uppercase.
pub fn shout(s: &str) -> String {
    s.to_uppercase()
}

/// Counts the whitespace-separated words in the input.
pub fn word_count(s: &str) -> usize {
    s.split_whitespace().count()
}
