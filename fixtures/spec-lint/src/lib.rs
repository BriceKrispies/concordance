//! Sample library for the specification lint fixture.

/// Names of the targets a manifest lists, in order.
pub fn build_project(manifest: &str) -> Vec<String> {
    let _ = hash_manifest(manifest);
    manifest.split_whitespace().map(str::to_string).collect()
}

fn hash_manifest(manifest: &str) -> u64 {
    manifest.bytes().map(u64::from).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verifies: CLI-001
    #[test]
    fn missing_file_is_refused() {
        let targets = build_project("");
        assert!(targets.is_empty());
    }

    /// Verifies: CLI-001
    #[test]
    fn missing_file_leaves_no_output() {
        let targets = build_project(" ");
        assert!(targets.is_empty());
    }

    /// Verifies: CLI-002
    #[test]
    fn lists_built_targets() {
        let targets = build_project("a b");
        assert_eq!(targets.len(), 2);
    }

    /// Verifies: CLI-003
    #[test]
    fn hashing_is_stable() {
        let sum = hash_manifest("ab");
        assert_eq!(sum, 195);
    }

    /// Verifies: CLI-404
    #[test]
    fn names_a_requirement_nobody_defined() {
        let targets = build_project("x");
        assert_eq!(targets.len(), 1);
    }
}
