//! White paper IDs (`G-1`, `NG-1`, `B-1`, `C-1`, `D-1`, `Q-1`) and the spec index table.

/// An ID defined in the white paper, with its text and line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WhitePaperId {
    pub id: String,
    pub text: String,
    pub line: u32,
}

const PREFIXES: [&str; 6] = ["G", "NG", "B", "C", "D", "Q"];

/// `G-1`, `NG-12`: one of the white-paper prefixes, a dash and a number.
pub fn is_whitepaper_id(s: &str) -> bool {
    match s.split_once('-') {
        Some((p, n)) => {
            PREFIXES.contains(&p) && !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())
        }
        None => false,
    }
}

fn clean_cell(s: &str) -> String {
    s.trim()
        .trim_matches(|c| c == '*' || c == '_' || c == '`')
        .trim()
        .to_string()
}

/// Split `**G-1** text`, `G-1: text` into the ID and the rest.
fn bold_opener(line: &str) -> Option<(String, String)> {
    let rest = line
        .strip_prefix("**")
        .or_else(|| line.strip_prefix("__"))?;
    let end = rest.find("**").or_else(|| rest.find("__"))?;
    let id = rest[..end].trim().trim_end_matches([':', '.']);
    if !is_whitepaper_id(id) {
        return None;
    }
    let text = rest[end + 2..].trim_start_matches([':', '.', ' ', '-', '\u{2014}']);
    Some((id.to_string(), text.trim().to_string()))
}

fn strip_list_marker(line: &str) -> &str {
    let t = line.trim_start();
    for m in ["- ", "* ", "+ "] {
        if let Some(r) = t.strip_prefix(m) {
            return r.trim_start();
        }
    }
    let digits = t.chars().take_while(char::is_ascii_digit).count();
    if digits > 0 {
        if let Some(r) = t[digits..].strip_prefix(". ") {
            return r.trim_start();
        }
    }
    t
}

/// IDs defined by bold openers (paragraphs and list items) and first table cells. Code
/// fences and HTML comments are skipped.
pub fn parse_whitepaper(text: &str) -> Vec<WhitePaperId> {
    let mut out: Vec<WhitePaperId> = Vec::new();
    let mut fence = false;
    let mut comment = false;
    // Index into `out` of the entry a continuation line extends.
    let mut open: Option<usize> = None;
    for (i, raw) in text.lines().enumerate() {
        let line = i as u32 + 1;
        let t = raw.trim();
        if t.starts_with("```") || t.starts_with("~~~") {
            fence = !fence;
            open = None;
            continue;
        }
        if fence {
            continue;
        }
        if comment {
            if t.contains("-->") {
                comment = false;
            }
            continue;
        }
        if t.starts_with("<!--") {
            if !t.contains("-->") {
                comment = true;
            }
            open = None;
            continue;
        }
        if t.is_empty() || t.starts_with('#') {
            open = None;
            continue;
        }
        let found = if t.starts_with('|') {
            let cells: Vec<&str> = t.trim_matches('|').split('|').collect();
            let first = cells.first().map(|c| clean_cell(c)).unwrap_or_default();
            is_whitepaper_id(&first).then(|| {
                let rest: Vec<String> = cells[1..].iter().map(|c| clean_cell(c)).collect();
                (first, rest.join(" | "))
            })
        } else {
            let body = strip_list_marker(t);
            bold_opener(body)
        };
        match found {
            Some((id, text)) => {
                out.push(WhitePaperId { id, text, line });
                open = if t.starts_with('|') {
                    None
                } else {
                    Some(out.len() - 1)
                };
            }
            None => {
                if let Some(k) = open {
                    if !t.starts_with('|') {
                        let e = &mut out[k];
                        if !e.text.is_empty() {
                            e.text.push(' ');
                        }
                        e.text.push_str(t);
                    }
                }
            }
        }
    }
    out
}

/// One row of the spec index table: the file it names and the ID prefixes it owns.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndexRow {
    /// The file as linked or written (`cli.md`).
    pub file: String,
    pub prefixes: Vec<String>,
    pub line: u32,
}

fn link_target(cell: &str) -> String {
    if let (Some(a), Some(b)) = (cell.find("]("), cell.rfind(')')) {
        if a + 2 <= b {
            return cell[a + 2..b].trim().to_string();
        }
    }
    clean_cell(cell)
}

/// The `File | Prefix | ...` table of `docs/spec/index.md`. `None` when there is no such
/// table.
pub fn parse_index_table(text: &str) -> Option<Vec<IndexRow>> {
    let mut rows = Vec::new();
    let mut cols: Option<(usize, usize)> = None;
    let mut fence = false;
    for (i, raw) in text.lines().enumerate() {
        let t = raw.trim();
        if t.starts_with("```") {
            fence = !fence;
        }
        if fence || !t.starts_with('|') {
            if cols.is_some() && t.is_empty() {
                cols = None;
            }
            continue;
        }
        let cells: Vec<&str> = t.trim_matches('|').split('|').collect();
        if cells
            .iter()
            .all(|c| c.trim().chars().all(|ch| matches!(ch, '-' | ':' | ' ')))
        {
            continue;
        }
        match cols {
            None => {
                let lower: Vec<String> =
                    cells.iter().map(|c| clean_cell(c).to_lowercase()).collect();
                let file = lower.iter().position(|c| c == "file");
                let prefix = lower.iter().position(|c| c == "prefix");
                if let (Some(f), Some(p)) = (file, prefix) {
                    cols = Some((f, p));
                }
            }
            Some((f, p)) => {
                let (Some(fc), Some(pc)) = (cells.get(f), cells.get(p)) else {
                    continue;
                };
                let prefixes = clean_cell(pc)
                    .split([',', ' ', '/'])
                    .map(clean_cell)
                    .filter(|s| !s.is_empty())
                    .collect();
                rows.push(IndexRow {
                    file: link_target(fc),
                    prefixes,
                    line: i as u32 + 1,
                });
            }
        }
    }
    (!rows.is_empty()).then_some(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_openers_and_table_cells() {
        let wp = "## Goals\n\n- **G-1** First goal\n  continues.\n- **G-2**: Second\n\n| ID | B |\n|---|---|\n| B-1 | `tool build` | CLI |\n\n```\n- **G-9** code\n```\n<!-- **G-8** -->\n";
        let ids = parse_whitepaper(wp);
        let got: Vec<(&str, u32)> = ids.iter().map(|i| (i.id.as_str(), i.line)).collect();
        assert_eq!(got, vec![("G-1", 3), ("G-2", 5), ("B-1", 9)]);
        assert_eq!(ids[0].text, "First goal continues.");
        assert_eq!(ids[2].text, "tool build | CLI");
    }

    #[test]
    fn reads_the_index_table() {
        let t = "# Spec\n\n| File | Prefix | Covers |\n|---|---|---|\n| [cli.md](cli.md) | CLI | x |\n| [glossary.md](glossary.md) |   | Terms |\n";
        let rows = parse_index_table(t).unwrap();
        assert_eq!(rows[0].file, "cli.md");
        assert_eq!(rows[0].prefixes, vec!["CLI"]);
        assert!(rows[1].prefixes.is_empty());
    }
}
