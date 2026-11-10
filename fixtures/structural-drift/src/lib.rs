//! Header parsing and encoding.

/// A single named header with its value.
pub struct Header {
    pub name: String,
    pub value: String,
}

/// Parses a `name: value` line. Returns `None` when there is no colon.
pub fn parse_header(input: &str) -> Option<Header> {
    let (name, value) = input.split_once(':')?;
    Some(Header {
        name: name.trim().to_string(),
        value: value.trim().to_string(),
    })
}

/// Encodes a header as a single `name: value` line.
pub fn encode(h: &Header) -> String {
    format!("{}: {}", h.name, h.value)
}
