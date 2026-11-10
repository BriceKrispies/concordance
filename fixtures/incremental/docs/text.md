# Text helpers

## Overview

The text module holds string utilities for the command output. They are
independent of the math module.

## Shouting

`text::shout()` must return the input with every letter converted to uppercase.

## Counting words

`text::word_count()` must count the whitespace-separated words in its input.

```rust
pub fn word_count(s: &str) -> usize
```

## Example

```rust
use incremental::text;

assert_eq!(text::shout("hi"), "HI");
assert_eq!(text::word_count("one two three"), 3);
```
