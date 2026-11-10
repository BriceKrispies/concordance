//! The `## Cases` table of a conformance case file.
//!
//! A case file is Markdown with prose and a `## Cases` heading followed by a pipe table whose
//! first column is `Case`. The lint checks the shape; the conformance driver reads the rows.

/// One data row, with the source line it is on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CaseRow {
    pub line: u32,
    /// Cell texts, trimmed, with `\|` unescaped.
    pub cells: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CaseTable {
    /// Header cell texts.
    pub columns: Vec<String>,
    pub rows: Vec<CaseRow>,
}

impl CaseTable {
    /// Index of the column named `name` (case-insensitive).
    pub fn column(&self, name: &str) -> Option<usize> {
        self.columns
            .iter()
            .position(|c| c.eq_ignore_ascii_case(name))
    }
}

fn split_row(line: &str) -> Vec<String> {
    let t = line.trim();
    let t = t.strip_prefix('|').unwrap_or(t);
    let t = t.strip_suffix('|').unwrap_or(t);
    let mut cells = Vec::new();
    let mut cur = String::new();
    let mut chars = t.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' && chars.peek() == Some(&'|') {
            cur.push('|');
            chars.next();
        } else if c == '|' {
            cells.push(cur.trim().to_string());
            cur.clear();
        } else {
            cur.push(c);
        }
    }
    cells.push(cur.trim().to_string());
    cells
}

fn is_separator(line: &str) -> bool {
    let t = line.trim();
    t.contains('-') && t.chars().all(|c| matches!(c, '|' | '-' | ':' | ' '))
}

/// Parse the table under the `## Cases` heading. Errors say what is wrong.
pub fn parse_cases(text: &str) -> Result<CaseTable, String> {
    let lines: Vec<&str> = text.lines().collect();
    let Some(h) = lines.iter().position(|l| {
        l.trim_end()
            .strip_prefix("## ")
            .is_some_and(|t| t.trim().eq_ignore_ascii_case("cases"))
    }) else {
        return Err("no `## Cases` heading".into());
    };
    let mut i = h + 1;
    while i < lines.len() && !lines[i].trim_start().starts_with('|') {
        if lines[i].starts_with('#') {
            break;
        }
        i += 1;
    }
    if i >= lines.len() || !lines[i].trim_start().starts_with('|') {
        return Err("`## Cases` holds no table".into());
    }
    let columns = split_row(lines[i]);
    if !columns
        .first()
        .is_some_and(|c| c.eq_ignore_ascii_case("case"))
    {
        return Err("the first column of the `## Cases` table must be `Case`".into());
    }
    if !lines.get(i + 1).is_some_and(|l| is_separator(l)) {
        return Err("the `## Cases` table has no separator row".into());
    }
    let mut rows = Vec::new();
    let mut names = std::collections::HashSet::new();
    for (k, l) in lines.iter().enumerate().skip(i + 2) {
        if !l.trim_start().starts_with('|') {
            break;
        }
        let cells = split_row(l);
        let line = k as u32 + 1;
        if cells.len() != columns.len() {
            return Err(format!(
                "line {line}: {} cells, the header has {}",
                cells.len(),
                columns.len()
            ));
        }
        if cells[0].is_empty() || !names.insert(cells[0].clone()) {
            return Err(format!(
                "line {line}: case name `{}` is empty or repeated",
                cells[0]
            ));
        }
        rows.push(CaseRow { line, cells });
    }
    if rows.is_empty() {
        return Err("the `## Cases` table has no rows".into());
    }
    Ok(CaseTable { columns, rows })
}
