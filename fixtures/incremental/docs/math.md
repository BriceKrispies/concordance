# Math helpers

## Overview

The math module collects small integer helpers used by the reporting code. It
has no state and no dependencies.

## Addition

`math::add()` must return the sum of its two arguments.

```rust
pub fn add(a: i32, b: i32) -> i32
```

## Percentages

`incremental::math::clamp_percent()` must never return a value greater than 100.

Inputs above 100 are reduced to 100; smaller inputs are returned unchanged.

## Example

```rust
use incremental::math;

assert_eq!(math::add(2, 3), 5);
assert_eq!(math::clamp_percent(140), 100);
```
