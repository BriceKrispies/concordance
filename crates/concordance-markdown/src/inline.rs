//! Inline accumulator: normalized text plus the code spans and links found in it,
//! with enough anchors to map text offsets back to source bytes.

/// A code span or link found inside accumulated text. Offsets are into `Inline::text`.
#[derive(Clone, Debug)]
pub enum Item {
    Code {
        ts: usize,
        te: usize,
        code: String,
        bytes: (usize, usize),
    },
    Link {
        ts: usize,
        te: usize,
        dest: String,
        bytes: (usize, usize),
    },
}

impl Item {
    pub fn start(&self) -> usize {
        match self {
            Item::Code { ts, .. } | Item::Link { ts, .. } => *ts,
        }
    }
    fn shift(&mut self, by: usize) {
        match self {
            Item::Code { ts, te, .. } | Item::Link { ts, te, .. } => {
                *ts += by;
                *te += by;
            }
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Inline {
    pub text: String,
    pub items: Vec<Item>,
    /// (text offset, source byte start, source byte end)
    pub anchors: Vec<(usize, usize, usize)>,
}

impl Inline {
    pub fn new() -> Self {
        Inline::default()
    }

    fn push_collapsed(&mut self, s: &str) {
        for c in s.chars() {
            if c.is_whitespace() {
                if !self.text.is_empty() && !self.text.ends_with(' ') {
                    self.text.push(' ');
                }
            } else {
                self.text.push(c);
            }
        }
    }

    pub fn push_text(&mut self, s: &str, bytes: (usize, usize)) {
        self.anchors.push((self.text.len(), bytes.0, bytes.1));
        self.push_collapsed(s);
    }

    pub fn push_space(&mut self, bytes: (usize, usize)) {
        self.anchors.push((self.text.len(), bytes.0, bytes.1));
        self.push_collapsed(" ");
    }

    pub fn push_code(&mut self, code: &str, bytes: (usize, usize)) {
        self.anchors.push((self.text.len(), bytes.0, bytes.1));
        let ts = self.text.len();
        self.text.push('`');
        self.push_collapsed(code);
        self.text.push('`');
        let te = self.text.len();
        let code = code.split_whitespace().collect::<Vec<_>>().join(" ");
        self.items.push(Item::Code {
            ts,
            te,
            code,
            bytes,
        });
    }

    pub fn add_link(&mut self, ts: usize, dest: String, bytes: (usize, usize)) {
        let te = self.text.len();
        self.items.push(Item::Link {
            ts,
            te,
            dest,
            bytes,
        });
    }

    pub fn is_blank(&self) -> bool {
        self.text.trim().is_empty()
    }

    /// Trim trailing whitespace (leading is never produced).
    pub fn finish(&mut self) {
        while self.text.ends_with(' ') {
            self.text.pop();
        }
    }

    /// Append `other` after the current content.
    pub fn append(&mut self, other: &Inline) {
        let by = self.text.len();
        self.text.push_str(&other.text);
        for it in &other.items {
            let mut it = it.clone();
            it.shift(by);
            self.items.push(it);
        }
        for a in &other.anchors {
            self.anchors.push((a.0 + by, a.1, a.2));
        }
    }

    pub fn push_raw(&mut self, s: &str) {
        self.text.push_str(s);
    }

    /// Source byte for a text offset (best effort).
    pub fn byte_at(&self, off: usize) -> usize {
        let i = self.anchors.partition_point(|a| a.0 <= off);
        if i == 0 {
            return self.anchors.first().map(|a| a.1).unwrap_or(0);
        }
        let a = self.anchors[i - 1];
        let delta = off - a.0;
        let len = a.2.saturating_sub(a.1);
        a.1 + delta.min(len.saturating_sub(1))
    }
}
