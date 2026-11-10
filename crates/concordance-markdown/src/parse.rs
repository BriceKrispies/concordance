//! pulldown-cmark event walk: flattens a document into headings, text blocks and code blocks.

use crate::inline::Inline;
use concordance_core::doc::BlockKind;
use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};

pub enum Raw {
    Heading {
        level: u8,
        plain: String,
        inline: Inline,
        start: usize,
    },
    /// `whole_row` is set for table rows: the block is not split into sentences.
    Text {
        kind: BlockKind,
        inline: Inline,
        whole_row: Option<(usize, usize)>,
    },
    Code {
        lang: String,
        content: String,
        first_byte: usize,
    },
}

#[derive(Clone, Copy, PartialEq)]
enum Container {
    Item,
    Quote,
}

struct CodeAcc {
    lang: String,
    fenced: bool,
    content: String,
    first_byte: Option<usize>,
}

#[derive(Default)]
struct Walker {
    out: Vec<Raw>,
    cur: Option<Inline>,
    containers: Vec<Container>,
    heading: Option<(u8, String, usize)>,
    code: Option<CodeAcc>,
    links: Vec<(usize, String)>,
    in_cell: bool,
    headers: Vec<String>,
    cells: Vec<Inline>,
    image_depth: usize,
}

impl Walker {
    fn kind(&self) -> BlockKind {
        match self.containers.last() {
            Some(Container::Item) => BlockKind::ListItem,
            Some(Container::Quote) => BlockKind::BlockQuote,
            None => BlockKind::Paragraph,
        }
    }

    fn flush(&mut self) {
        if self.in_cell || self.heading.is_some() {
            return;
        }
        if let Some(mut inl) = self.cur.take() {
            inl.finish();
            if !inl.is_blank() {
                let kind = self.kind();
                self.out.push(Raw::Text {
                    kind,
                    inline: inl,
                    whole_row: None,
                });
            }
        }
    }

    fn inline(&mut self) -> &mut Inline {
        self.cur.get_or_insert_with(Inline::new)
    }

    fn text(&mut self, s: &str, bytes: (usize, usize)) {
        if let Some(c) = self.code.as_mut() {
            if c.first_byte.is_none() {
                c.first_byte = Some(bytes.0);
            }
            c.content.push_str(s);
            return;
        }
        if let Some(h) = self.heading.as_mut() {
            h.1.push_str(s);
        }
        self.inline().push_text(s, bytes);
    }

    fn handle(&mut self, ev: Event<'_>, range: std::ops::Range<usize>) {
        let bytes = (range.start, range.end);
        match ev {
            Event::Start(tag) => self.start(tag, bytes),
            Event::End(end) => self.end(end, bytes),
            Event::Text(s) => self.text(&s, bytes),
            Event::Code(s) => {
                if self.code.is_some() {
                    return;
                }
                if let Some(h) = self.heading.as_mut() {
                    h.1.push_str(&s);
                }
                self.inline().push_code(&s, bytes);
            }
            Event::SoftBreak | Event::HardBreak => {
                if let Some(c) = self.code.as_mut() {
                    c.content.push('\n');
                } else {
                    if let Some(h) = self.heading.as_mut() {
                        h.1.push(' ');
                    }
                    self.inline().push_space(bytes);
                }
            }
            Event::Rule => self.flush(),
            _ => {}
        }
    }

    fn start(&mut self, tag: Tag<'_>, bytes: (usize, usize)) {
        match tag {
            Tag::Paragraph | Tag::List(_) | Tag::Table(_) | Tag::HtmlBlock => self.flush(),
            Tag::Item => {
                self.flush();
                self.containers.push(Container::Item);
            }
            Tag::BlockQuote(_) => {
                self.flush();
                self.containers.push(Container::Quote);
            }
            Tag::Heading { level, .. } => {
                self.flush();
                self.heading = Some((level as u8, String::new(), bytes.0));
            }
            Tag::CodeBlock(kind) => {
                self.flush();
                let (fenced, lang) = match kind {
                    CodeBlockKind::Fenced(info) => (true, info.to_string()),
                    CodeBlockKind::Indented => (false, String::new()),
                };
                self.code = Some(CodeAcc {
                    lang,
                    fenced,
                    content: String::new(),
                    first_byte: None,
                });
            }
            Tag::TableHead | Tag::TableRow => self.cells.clear(),
            Tag::TableCell => {
                self.in_cell = true;
                self.cur = Some(Inline::new());
            }
            Tag::Link { dest_url, .. } => {
                if self.image_depth == 0 {
                    let ts = self.inline().text.len();
                    self.links.push((ts, dest_url.to_string()));
                }
            }
            Tag::Image { .. } => self.image_depth += 1,
            _ => {}
        }
    }

    fn end(&mut self, end: TagEnd, bytes: (usize, usize)) {
        match end {
            TagEnd::Paragraph | TagEnd::List(_) | TagEnd::Table | TagEnd::HtmlBlock => self.flush(),
            TagEnd::Item | TagEnd::BlockQuote(_) => {
                self.flush();
                self.containers.pop();
            }
            TagEnd::Heading(_) => {
                if let Some((level, plain, start)) = self.heading.take() {
                    let mut inline = self.cur.take().unwrap_or_default();
                    inline.finish();
                    let plain = plain.split_whitespace().collect::<Vec<_>>().join(" ");
                    self.out.push(Raw::Heading {
                        level,
                        plain,
                        inline,
                        start,
                    });
                }
            }
            TagEnd::CodeBlock => {
                if let Some(c) = self.code.take() {
                    if c.fenced {
                        self.out.push(Raw::Code {
                            lang: c.lang,
                            content: c.content,
                            first_byte: c.first_byte.unwrap_or(bytes.0),
                        });
                    }
                }
            }
            TagEnd::TableCell => {
                self.in_cell = false;
                let mut inl = self.cur.take().unwrap_or_default();
                inl.finish();
                self.cells.push(inl);
            }
            TagEnd::TableHead => {
                self.headers = self.cells.drain(..).map(|c| c.text).collect();
            }
            TagEnd::TableRow => {
                let cells = std::mem::take(&mut self.cells);
                let mut row = Inline::new();
                for (i, cell) in cells.iter().enumerate() {
                    if cell.is_blank() {
                        continue;
                    }
                    if !row.text.is_empty() {
                        row.push_raw("; ");
                    }
                    match self.headers.get(i) {
                        Some(h) if !h.is_empty() => row.push_raw(&format!("{h}: ")),
                        _ => {}
                    }
                    row.append(cell);
                }
                if !row.is_blank() {
                    self.out.push(Raw::Text {
                        kind: BlockKind::TableRow,
                        inline: row,
                        whole_row: Some(bytes),
                    });
                }
            }
            TagEnd::Link => {
                if let Some((ts, dest)) = self.links.pop() {
                    self.inline().add_link(ts, dest, bytes);
                }
            }
            TagEnd::Image => self.image_depth = self.image_depth.saturating_sub(1),
            _ => {}
        }
    }
}

/// Parse `text` (front matter already blanked) into flat raw blocks.
pub fn walk(text: &str) -> Vec<Raw> {
    let mut w = Walker::default();
    let parser = Parser::new_ext(text, Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH);
    for (ev, range) in parser.into_offset_iter() {
        w.handle(ev, range);
    }
    w.flush();
    w.out
}
