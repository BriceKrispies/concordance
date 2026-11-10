# Header codec

## Overview

Headers travel between peers as short `name: value` lines. This document
describes the parsing and encoding helpers. The background on the bytes on the
wire is in the [wire format](wire-format.md#framing) notes. Older design
discussion is in the [old notes](notes.md).

The byte-level encoding is implemented in `src/codec.rs`.

## Parsing

`parse_headers()` must return `None` for any line that has no colon.

`Header::from_bytes()` must never panic on malformed input. It returns `None`
instead.

`parse_header()` returns the first header found in the line. It must return
`None` when the input contains no `:`.

## Encoding

`encode()` must produce a single `name: value` line with no trailing newline.

The encoder takes the header by reference and returns a new `String`:

```rust
pub fn encode(h: &Header, buf: &mut String)
```

## Example

```rust
use structural_drift::{encode, parse_header};

let h = parse_header("Host: example.com").unwrap();
assert_eq!(encode(&h), "Host: example.com");
```
