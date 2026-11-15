//! Sample library for the Exegra parity fixture.

/// Names of the targets a manifest lists, in order.
pub fn build_project(manifest: &str) -> Vec<String> {
    manifest.split_whitespace().map(str::to_string).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verifies: LIB-002
    #[test]
    fn empty_manifest_has_no_targets() {
        assert!(build_project("").is_empty());
    }
}
