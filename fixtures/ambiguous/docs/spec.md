# String cache

## Overview

The cache keeps recently used strings so that repeated lookups are cheap. It is
an in-memory map and is intended for a single thread of control.

## Lookup

`Cache::get()` must return `None` for missing keys.

## Storage

`Cache::insert()` must be fast.

The cache must hold at most 100 entries.

## Eviction

The cache should evict entries in a timely manner when appropriate.

Entries must expire after some time, if possible.

## Capacity

Later revisions raised the limit. The cache must hold at most 500 entries.

## Example

```rust
use ambiguous::Cache;

let mut cache = Cache::default();
cache.insert("greeting".to_string(), "hello".to_string());
assert_eq!(cache.get("greeting"), Some("hello".to_string()));
cache.evict();
```
