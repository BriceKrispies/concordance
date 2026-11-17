//! A JSONC reader (JSON with `//` and `/* */` comments and trailing commas) that keeps
//! the byte span of every value, so entities can be located in the file.

#[derive(Debug)]
pub enum Value {
    Null,
    Bool(bool),
    /// The number as written.
    Num(String),
    Str(String),
    Arr(Vec<Node>),
    /// Members in file order; duplicate keys are kept (lookups take the last).
    Obj(Vec<(String, Node)>),
}

#[derive(Debug)]
pub struct Node {
    pub value: Value,
    /// Byte offset of the first byte of the value (the opening quote of a string).
    pub start: usize,
    /// Byte offset just past the value.
    pub end: usize,
}

impl Node {
    pub fn get(&self, key: &str) -> Option<&Node> {
        match &self.value {
            Value::Obj(m) => m.iter().rev().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }
    pub fn str(&self) -> Option<&str> {
        match &self.value {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }
    pub fn get_str(&self, key: &str) -> Option<&str> {
        self.get(key).and_then(Node::str)
    }
    pub fn items(&self) -> &[Node] {
        match &self.value {
            Value::Arr(a) => a,
            _ => &[],
        }
    }
    pub fn members(&self) -> &[(String, Node)] {
        match &self.value {
            Value::Obj(m) => m,
            _ => &[],
        }
    }
    pub fn num(&self) -> Option<f64> {
        match &self.value {
            Value::Num(n) => n.parse().ok(),
            _ => None,
        }
    }
}

#[derive(Debug, PartialEq)]
pub struct ParseError {
    pub offset: usize,
    pub message: String,
}

pub fn parse(text: &str) -> Result<Node, ParseError> {
    let mut p = Parser {
        b: text.as_bytes(),
        s: text,
        i: 0,
        depth: 0,
    };
    if text.starts_with('\u{feff}') {
        p.i = 3;
    }
    p.ws()?;
    let v = p.value()?;
    p.ws()?;
    if p.i < p.b.len() {
        return Err(p.err("unexpected content after the top-level value"));
    }
    Ok(v)
}

/// Whether `text` is an Exegra specification: a JSONC object with a top-level `"exegra"`
/// key. Reads only as far as needed, without building values.
pub fn has_top_level_key(text: &str, key: &str) -> bool {
    let mut p = Parser {
        b: text.as_bytes(),
        s: text,
        i: if text.starts_with('\u{feff}') { 3 } else { 0 },
        depth: 0,
    };
    let scan = |p: &mut Parser| -> Result<bool, ParseError> {
        p.ws()?;
        if p.peek() != Some(b'{') {
            return Ok(false);
        }
        p.i += 1;
        loop {
            p.ws()?;
            match p.peek() {
                Some(b'}') | None => return Ok(false),
                Some(b',') => {
                    p.i += 1;
                    continue;
                }
                _ => {}
            }
            let k = p.string()?;
            if k == key {
                return Ok(true);
            }
            p.ws()?;
            p.expect(b':')?;
            p.ws()?;
            p.skip_value()?;
        }
    };
    scan(&mut p).unwrap_or(false)
}

const MAX_DEPTH: usize = 256;

struct Parser<'a> {
    b: &'a [u8],
    s: &'a str,
    i: usize,
    depth: usize,
}

impl<'a> Parser<'a> {
    fn err(&self, m: &str) -> ParseError {
        ParseError {
            offset: self.i,
            message: m.to_string(),
        }
    }
    fn peek(&self) -> Option<u8> {
        self.b.get(self.i).copied()
    }
    fn expect(&mut self, c: u8) -> Result<(), ParseError> {
        if self.peek() == Some(c) {
            self.i += 1;
            Ok(())
        } else {
            Err(self.err(&format!("expected `{}`", c as char)))
        }
    }
    /// Skip whitespace and comments.
    fn ws(&mut self) -> Result<(), ParseError> {
        loop {
            match self.peek() {
                Some(b' ' | b'\t' | b'\n' | b'\r') => self.i += 1,
                Some(b'/') if self.b.get(self.i + 1) == Some(&b'/') => {
                    while let Some(c) = self.peek() {
                        if c == b'\n' {
                            break;
                        }
                        self.i += 1;
                    }
                }
                Some(b'/') if self.b.get(self.i + 1) == Some(&b'*') => {
                    let rest = &self.s[self.i + 2..];
                    match rest.find("*/") {
                        Some(n) => self.i += 2 + n + 2,
                        None => return Err(self.err("unterminated block comment")),
                    }
                }
                _ => return Ok(()),
            }
        }
    }

    fn value(&mut self) -> Result<Node, ParseError> {
        let start = self.i;
        let value = match self.peek() {
            Some(b'{') => {
                self.enter()?;
                self.i += 1;
                let mut members = Vec::new();
                loop {
                    self.ws()?;
                    if self.peek() == Some(b'}') {
                        self.i += 1;
                        break;
                    }
                    let k = self.string()?;
                    self.ws()?;
                    self.expect(b':')?;
                    self.ws()?;
                    let v = self.value()?;
                    members.push((k, v));
                    self.ws()?;
                    match self.peek() {
                        Some(b',') => self.i += 1,
                        Some(b'}') => {}
                        _ => return Err(self.err("expected `,` or `}`")),
                    }
                }
                self.depth -= 1;
                Value::Obj(members)
            }
            Some(b'[') => {
                self.enter()?;
                self.i += 1;
                let mut items = Vec::new();
                loop {
                    self.ws()?;
                    if self.peek() == Some(b']') {
                        self.i += 1;
                        break;
                    }
                    items.push(self.value()?);
                    self.ws()?;
                    match self.peek() {
                        Some(b',') => self.i += 1,
                        Some(b']') => {}
                        _ => return Err(self.err("expected `,` or `]`")),
                    }
                }
                self.depth -= 1;
                Value::Arr(items)
            }
            Some(b'"') => Value::Str(self.string()?),
            Some(b't') => self.word("true", Value::Bool(true))?,
            Some(b'f') => self.word("false", Value::Bool(false))?,
            Some(b'n') => self.word("null", Value::Null)?,
            Some(b'-' | b'0'..=b'9') => {
                while let Some(b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9') = self.peek() {
                    self.i += 1;
                }
                Value::Num(self.s[start..self.i].to_string())
            }
            _ => return Err(self.err("expected a value")),
        };
        Ok(Node {
            value,
            start,
            end: self.i,
        })
    }

    fn enter(&mut self) -> Result<(), ParseError> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(self.err("nesting too deep"));
        }
        Ok(())
    }

    fn word(&mut self, w: &str, v: Value) -> Result<Value, ParseError> {
        if self.s[self.i..].starts_with(w) {
            self.i += w.len();
            Ok(v)
        } else {
            Err(self.err("expected a value"))
        }
    }

    fn skip_value(&mut self) -> Result<(), ParseError> {
        match self.peek() {
            Some(b'"') => {
                self.string()?;
            }
            Some(b'{' | b'[') => {
                let mut depth = 0usize;
                loop {
                    self.ws()?;
                    match self.peek() {
                        None => return Err(self.err("unterminated value")),
                        Some(b'"') => {
                            self.string()?;
                        }
                        Some(b'{' | b'[') => {
                            depth += 1;
                            self.i += 1;
                        }
                        Some(b'}' | b']') => {
                            depth -= 1;
                            self.i += 1;
                            if depth == 0 {
                                return Ok(());
                            }
                        }
                        Some(_) => self.i += 1,
                    }
                }
            }
            Some(_) => {
                self.value()?;
            }
            None => return Err(self.err("expected a value")),
        }
        Ok(())
    }

    fn string(&mut self) -> Result<String, ParseError> {
        self.expect(b'"')?;
        let mut out = String::new();
        let mut run = self.i;
        loop {
            match self.peek() {
                None => return Err(self.err("unterminated string")),
                Some(b'"') => {
                    out.push_str(&self.s[run..self.i]);
                    self.i += 1;
                    return Ok(out);
                }
                Some(b'\\') => {
                    out.push_str(&self.s[run..self.i]);
                    let (c, n) =
                        decode_escape(self.s, self.i).ok_or_else(|| self.err("bad escape"))?;
                    out.push(c);
                    self.i += n;
                    run = self.i;
                }
                Some(b'\n') => return Err(self.err("newline in string")),
                Some(_) => self.i += 1,
            }
        }
    }
}

/// The character an escape at byte `at` (a backslash) stands for, and its length in bytes.
fn decode_escape(s: &str, at: usize) -> Option<(char, usize)> {
    let b = s.as_bytes();
    let c = match *b.get(at + 1)? {
        b'"' => '"',
        b'\\' => '\\',
        b'/' => '/',
        b'b' => '\u{8}',
        b'f' => '\u{c}',
        b'n' => '\n',
        b'r' => '\r',
        b't' => '\t',
        b'u' => {
            let hi = u32::from_str_radix(s.get(at + 2..at + 6)?, 16).ok()?;
            if (0xD800..0xDC00).contains(&hi) && s.get(at + 6..at + 8) == Some("\\u") {
                let lo = u32::from_str_radix(s.get(at + 8..at + 12)?, 16).ok()?;
                if (0xDC00..0xE000).contains(&lo) {
                    let cp = 0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00);
                    return Some((char::from_u32(cp)?, 12));
                }
            }
            return Some((char::from_u32(hi).unwrap_or('\u{fffd}'), 6));
        }
        _ => return None,
    };
    Some((c, 2))
}

/// Walk the string literal whose opening quote is at `quote`, calling `f(decoded char,
/// byte offset of its source text)` until `f` returns false or the literal ends.
/// Returns the offset where the walk stopped.
pub fn walk_literal(src: &str, from: usize, mut f: impl FnMut(char, usize) -> bool) -> usize {
    let b = src.as_bytes();
    let mut i = from;
    while i < b.len() {
        let (c, n) = match b[i] {
            b'"' => return i,
            b'\\' => match decode_escape(src, i) {
                Some(x) => x,
                None => return i,
            },
            _ => {
                let c = src[i..].chars().next().unwrap_or('\u{fffd}');
                (c, c.len_utf8())
            }
        };
        if !f(c, i) {
            return i;
        }
        i += n;
    }
    i
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_jsonc_with_spans() {
        let t = "// head\n{\n  \"a\": [1, \"x\\n\\u00e9\\ud83d\\ude00\",], /* c */\n  \"b\": {\"c\": null,},\n}\n";
        let v = parse(t).unwrap();
        let a = v.get("a").unwrap();
        assert_eq!(a.items()[1].str(), Some("x\né😀"));
        assert_eq!(&t[a.items()[1].start..a.items()[1].start + 1], "\"");
        assert!(v.get("b").unwrap().get("c").is_some());
        assert!(parse("{\"a\": }").is_err());
        assert!(parse("{\"a\": \"x").is_err());
    }

    #[test]
    fn sniffs_the_top_level_key_only() {
        assert!(has_top_level_key(
            "{ // c\n \"x\": {\"exegra\": 1}, \"exegra\": \"0.1\" }",
            "exegra"
        ));
        assert!(!has_top_level_key("{ \"x\": {\"exegra\": 1} }", "exegra"));
        assert!(!has_top_level_key("[\"exegra\"]", "exegra"));
        assert!(!has_top_level_key("not json", "exegra"));
    }

    #[test]
    fn walks_literal_source_offsets() {
        let t = "\"a\\\"b\"";
        let mut seen = Vec::new();
        walk_literal(t, 1, |c, at| {
            seen.push((c, at));
            true
        });
        assert_eq!(seen, vec![('a', 1), ('"', 2), ('b', 4)]);
    }
}
