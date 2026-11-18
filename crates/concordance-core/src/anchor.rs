//! Heading anchors. Section anchors are persisted in the document index, so this is the one
//! slug function the whole workspace uses.

/// GitHub-style heading anchor: lowercase, each whitespace character and each `-` becomes `-`,
/// `_` is kept, other punctuation is dropped. Tabs therefore become `-` (as the anchors that
/// were persisted before this function was shared were computed).
pub fn slugify(heading: &str) -> String {
    let mut out = String::new();
    for c in heading.trim().chars() {
        if c.is_alphanumeric() {
            out.extend(c.to_lowercase());
        } else if c.is_whitespace() || c == '-' {
            out.push('-');
        } else if c == '_' {
            out.push('_');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::slugify;

    #[test]
    fn spaces_and_tabs_become_dashes_and_punctuation_is_dropped() {
        assert_eq!(slugify("Data and Migrations"), "data-and-migrations");
        assert_eq!(slugify("a\tb"), "a-b");
        assert_eq!(slugify("Foo: bar_baz!"), "foo-bar_baz");
    }
}
