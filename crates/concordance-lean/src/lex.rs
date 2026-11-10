//! Lexical scanner for Lean 4 source: tokens and comments, nothing more.
//!
//! Handles nested `/- -/` block comments, `--` line comments, `/-- -/` doc comments,
//! `/-! -/` module docs, string (including raw) and character literals, and
//! `«guillemet»` names. Whatever it cannot classify becomes a one-character symbol, so
//! scanning never fails.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokKind {
    Ident,
    Str,
    Char,
    Num,
    Sym,
}

#[derive(Clone, Debug)]
pub struct Tok {
    pub kind: TokKind,
    pub start: usize,
    pub end: usize,
    /// 1-based line / char column of the first char.
    pub line: u32,
    pub col: u32,
    pub first_on_line: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommentKind {
    Line,
    Block,
    /// `/-- ... -/`
    Doc,
    /// `/-! ... -/`
    ModuleDoc,
}

#[derive(Clone, Debug)]
pub struct Comment {
    pub kind: CommentKind,
    pub start: usize,
    pub end: usize,
    pub start_line: u32,
    pub end_line: u32,
    /// Nothing but whitespace before it on its first line.
    pub own_line: bool,
    /// The comment text without its markers.
    pub text: String,
}

pub struct Lexed {
    pub toks: Vec<Tok>,
    pub comments: Vec<Comment>,
    starts: Vec<usize>,
}

impl Lexed {
    /// 1-based (line, char column) of a byte offset.
    pub fn pos(&self, text: &str, off: usize) -> (u32, u32) {
        let off = off.min(text.len());
        let line = self.starts.partition_point(|s| *s <= off).max(1) - 1;
        let start = self.starts[line];
        let col = text.get(start..off).map(|s| s.chars().count()).unwrap_or(0);
        (line as u32 + 1, col as u32 + 1)
    }
}

fn ident_start(c: char) -> bool {
    (c.is_alphabetic() && !matches!(c, 'λ' | 'Π' | 'Σ')) || c == '_' || c == '«'
}

fn ident_rest(c: char) -> bool {
    c.is_alphanumeric()
        || matches!(c, '_' | '\'' | '!' | '?')
        || ('\u{2080}'..='\u{209c}').contains(&c)
        || ('\u{1d62}'..='\u{1d6a}').contains(&c)
}

pub fn lex(text: &str) -> Lexed {
    let cs: Vec<(usize, char)> = text.char_indices().collect();
    let off = |i: usize| cs.get(i).map(|c| c.0).unwrap_or(text.len());
    let ch = |i: usize| cs.get(i).map(|c| c.1);
    let mut starts = vec![0];
    starts.extend(text.match_indices('\n').map(|(i, _)| i + 1));
    let mut lx = Lexed {
        toks: vec![],
        comments: vec![],
        starts,
    };
    let mut i = 0;
    let mut raw: Vec<(TokKind, usize, usize)> = vec![];
    while i < cs.len() {
        let c = cs[i].1;
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        let begin = i;
        if c == '-' && ch(i + 1) == Some('-') {
            while i < cs.len() && cs[i].1 != '\n' {
                i += 1;
            }
            let (s, e) = (off(begin), off(i));
            push_comment(&mut lx, text, CommentKind::Line, s, e, &text[s + 2..e]);
            continue;
        }
        if c == '/' && ch(i + 1) == Some('-') {
            let kind = match ch(i + 2) {
                Some('-') if ch(i + 3) != Some('/') => CommentKind::Doc,
                Some('!') => CommentKind::ModuleDoc,
                _ => CommentKind::Block,
            };
            let mut depth = 0usize;
            while i < cs.len() {
                if cs[i].1 == '/' && ch(i + 1) == Some('-') {
                    depth += 1;
                    i += 2;
                } else if cs[i].1 == '-' && ch(i + 1) == Some('/') {
                    depth = depth.saturating_sub(1);
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    i += 1;
                }
            }
            let i = i.min(cs.len());
            let (s, e) = (off(begin), off(i));
            let inner_start = s + if kind == CommentKind::Block { 2 } else { 3 };
            let inner_end = if text[..e].ends_with("-/") && e >= inner_start + 2 {
                e - 2
            } else {
                e
            };
            let inner = text
                .get(inner_start..inner_end.max(inner_start))
                .unwrap_or("");
            push_comment(&mut lx, text, kind, s, e, inner);
            continue;
        }
        if c == '"' {
            i = skip_string(&cs, i + 1);
            raw.push((TokKind::Str, off(begin), off(i)));
            continue;
        }
        if c == 'r' && matches!(ch(i + 1), Some('"') | Some('#')) {
            let mut j = i + 1;
            while ch(j) == Some('#') {
                j += 1;
            }
            if ch(j) == Some('"') {
                let hashes = j - (i + 1);
                j += 1;
                while j < cs.len() {
                    if cs[j].1 == '"' {
                        let mut k = 0;
                        while k < hashes && ch(j + 1 + k) == Some('#') {
                            k += 1;
                        }
                        if k == hashes {
                            j += 1 + hashes;
                            break;
                        }
                    }
                    j += 1;
                }
                i = j.min(cs.len());
                raw.push((TokKind::Str, off(begin), off(i)));
                continue;
            }
        }
        if c == '\'' {
            if ch(i + 1) == Some('\\') {
                let mut j = i + 2;
                while j < cs.len() && cs[j].1 != '\'' && cs[j].1 != '\n' {
                    j += 1;
                }
                i = (j + 1).min(cs.len());
                raw.push((TokKind::Char, off(begin), off(i)));
                continue;
            }
            if ch(i + 2) == Some('\'') && ch(i + 1).is_some_and(|x| x != '\'' && x != '\n') {
                i += 3;
                raw.push((TokKind::Char, off(begin), off(i)));
                continue;
            }
        }
        if ident_start(c) {
            i = skip_ident(&cs, i);
            raw.push((TokKind::Ident, off(begin), off(i)));
            continue;
        }
        if c.is_ascii_digit() {
            while i < cs.len()
                && (cs[i].1.is_alphanumeric()
                    || cs[i].1 == '_'
                    || (cs[i].1 == '.' && ch(i + 1).is_some_and(|d| d.is_ascii_digit())))
            {
                i += 1;
            }
            raw.push((TokKind::Num, off(begin), off(i)));
            continue;
        }
        if c == ':' && ch(i + 1) == Some('=') {
            i += 2;
        } else {
            i += 1;
        }
        raw.push((TokKind::Sym, off(begin), off(i)));
    }
    let mut prev_end_line = 0u32;
    for (kind, s, e) in raw {
        let (line, col) = lx.pos(text, s);
        let (end_line, _) = lx.pos(text, e.saturating_sub(1).max(s));
        lx.toks.push(Tok {
            kind,
            start: s,
            end: e,
            line,
            col,
            first_on_line: prev_end_line < line,
        });
        prev_end_line = end_line;
    }
    lx
}

fn push_comment(lx: &mut Lexed, text: &str, kind: CommentKind, s: usize, e: usize, inner: &str) {
    let (start_line, _) = lx.pos(text, s);
    let (end_line, _) = lx.pos(text, e.saturating_sub(1).max(s));
    let line_start = lx.starts[start_line as usize - 1];
    let own_line = text[line_start..s].trim().is_empty();
    lx.comments.push(Comment {
        kind,
        start: s,
        end: e,
        start_line,
        end_line,
        own_line,
        text: inner.to_string(),
    });
}

fn skip_string(cs: &[(usize, char)], mut i: usize) -> usize {
    while i < cs.len() {
        match cs[i].1 {
            '\\' => i += 2,
            '"' => return i + 1,
            _ => i += 1,
        }
    }
    cs.len()
}

fn skip_ident(cs: &[(usize, char)], mut i: usize) -> usize {
    loop {
        if cs.get(i).map(|c| c.1) == Some('«') {
            while i < cs.len() && cs[i].1 != '»' {
                i += 1;
            }
            i = (i + 1).min(cs.len());
        } else {
            while i < cs.len() && ident_rest(cs[i].1) {
                i += 1;
            }
        }
        if cs.get(i).map(|c| c.1) == Some('.') && cs.get(i + 1).is_some_and(|c| ident_start(c.1)) {
            i += 1;
        } else {
            return i;
        }
    }
}

/// Split a possibly dotted name into segments, dropping `«»`.
pub fn name_segments(name: &str) -> Vec<String> {
    let mut out = vec![];
    let mut cur = String::new();
    let mut quoted = false;
    for c in name.chars() {
        match c {
            '«' => quoted = true,
            '»' => quoted = false,
            '.' if !quoted => out.push(std::mem::take(&mut cur)),
            _ => cur.push(c),
        }
    }
    out.push(cur);
    out.retain(|s| !s.is_empty());
    out
}
