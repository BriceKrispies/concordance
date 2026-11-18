//! Set similarity shared by the textual matchers (claim identity carry-over in the engine,
//! the lint's near-duplicate rules).

use std::collections::BTreeSet;

/// Jaccard similarity `|a ∩ b| / |a ∪ b|`, in `0.0..=1.0`.
///
/// Two empty sets have no shared content, so their similarity is `0.0`: a matcher must not
/// take two texts without words as the same text.
pub fn jaccard<T: Ord>(a: &BTreeSet<T>, b: &BTreeSet<T>) -> f64 {
    let union = a.union(b).count();
    if union == 0 {
        return 0.0;
    }
    a.intersection(b).count() as f64 / union as f64
}

#[cfg(test)]
mod tests {
    use super::jaccard;
    use std::collections::BTreeSet;

    fn set(words: &[&str]) -> BTreeSet<String> {
        words.iter().map(|w| w.to_string()).collect()
    }

    #[test]
    fn overlap_is_intersection_over_union() {
        let a = set(&["a", "b", "c"]);
        let b = set(&["b", "c", "d"]);
        assert!((jaccard(&a, &b) - 0.5).abs() < 1e-9);
        assert!((jaccard(&a, &a) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn disjoint_and_empty_sets_are_not_similar() {
        assert_eq!(jaccard(&set(&["a"]), &set(&["b"])), 0.0);
        assert_eq!(jaccard(&set(&[]), &set(&[])), 0.0);
        assert_eq!(jaccard(&set(&[]), &set(&["a"])), 0.0);
    }
}
