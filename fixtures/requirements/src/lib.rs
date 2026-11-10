//! A tiny comment stripper used by the requirement-verification fixture.

/// Length in bytes of the block comment starting at `text[0]` (which must be `/*`),
/// or `None` if it is never closed. Comments nest.
pub fn block_comment_len(text: &str) -> Option<usize> {
    let b = text.as_bytes();
    let mut depth = 0usize;
    let mut i = 0usize;
    while i + 1 < b.len() {
        match (b[i], b[i + 1]) {
            (b'/', b'*') => {
                depth += 1;
                i += 2;
            }
            (b'*', b'/') => {
                depth -= 1;
                i += 2;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => i += 1,
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verifies: LEX-001
    #[test]
    fn nested_comment_ends_at_the_balancing_close() {
        assert_eq!(block_comment_len("/* a /* b */ c */ d"), Some(17));
    }

    /// Verifies: LEX-404
    #[test]
    fn verifies_a_requirement_that_does_not_exist() {
        assert_eq!(block_comment_len("/**/"), Some(4));
    }
}
