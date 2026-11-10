//! A small in-memory string cache.

use std::collections::HashMap;

/// A key-value cache of owned strings.
#[derive(Default)]
pub struct Cache {
    entries: HashMap<String, String>,
}

impl Cache {
    /// Returns a copy of the value stored under `key`, if any.
    pub fn get(&self, key: &str) -> Option<String> {
        self.entries.get(key).cloned()
    }

    /// Stores `value` under `key`, replacing any previous value.
    pub fn insert(&mut self, key: String, value: String) {
        self.entries.insert(key, value);
    }

    /// Removes all entries from the cache.
    pub fn evict(&mut self) {
        self.entries.clear();
    }
}
