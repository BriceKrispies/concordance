//! Item signatures inside fenced Rust code blocks of specifications.

use concordance_core::lang::DocSignature as Signature;

fn strip_qualifiers(line: &str) -> &str {
    let mut s = line.trim_start();
    loop {
        let before = s;
        if s.starts_with("pub(") {
            if let Some(i) = s.find(')') {
                s = s[i + 1..].trim_start();
            }
        }
        for kw in ["pub ", "async ", "const ", "unsafe "] {
            if let Some(r) = s.strip_prefix(kw) {
                s = r.trim_start();
            }
        }
        if s == before {
            return s;
        }
    }
}

fn item_name(stripped: &str) -> Option<String> {
    for kw in ["fn ", "struct ", "enum ", "trait "] {
        if let Some(rest) = stripped.strip_prefix(kw) {
            let name: String = rest
                .trim_start()
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if !name.is_empty() {
                return Some(name);
            }
        }
    }
    None
}

/// Find the end of a header: index of `{` or `;` at bracket depth 0.
fn header_end(s: &str) -> Option<(usize, bool)> {
    let mut depth = 0i32;
    for (i, c) in s.char_indices() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => depth -= 1,
            '{' if depth <= 0 => return Some((i, false)),
            ';' if depth <= 0 => return Some((i, true)),
            _ => {}
        }
    }
    None
}

pub fn extract(content: &str) -> Vec<Signature> {
    let lines: Vec<&str> = content.lines().collect();
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut container1 = false;
    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start();
        let stripped = strip_qualifiers(line);
        if !trimmed.starts_with("//") && (depth == 0 || (depth == 1 && container1)) {
            if let Some(name) = item_name(stripped) {
                let mut header = String::new();
                let mut end = i;
                let mut found = false;
                for (k, l) in lines.iter().enumerate().skip(i).take(20) {
                    let piece: &str = if k == i { trimmed } else { l };
                    if let Some((idx, semi)) = header_end(piece) {
                        let upto = if semi { idx + 1 } else { idx };
                        push_piece(&mut header, &piece[..upto], k == i);
                        end = k;
                        found = true;
                        break;
                    }
                    push_piece(&mut header, piece, k == i);
                    end = k;
                }
                if !found {
                    // Unterminated header: keep only the first line.
                    header.clear();
                    push_piece(&mut header, trimmed, true);
                    end = i;
                }
                let name_col = line
                    .find(name.as_str())
                    .map(|b| line[..b].chars().count())
                    .unwrap_or(0);
                out.push(Signature {
                    name,
                    text: header.trim_end().to_string(),
                    start: i,
                    end,
                    name_col,
                });
            }
        }
        let starts_container = ["impl", "trait ", "mod "]
            .iter()
            .any(|k| stripped.starts_with(k));
        if trimmed.starts_with("//") {
            continue;
        }
        for ch in line.chars() {
            match ch {
                '{' => {
                    depth += 1;
                    if depth == 1 {
                        container1 = starts_container;
                    }
                }
                '}' => depth = depth.saturating_sub(1),
                _ => {}
            }
        }
    }
    out
}

fn push_piece(header: &mut String, piece: &str, first: bool) {
    if !first {
        header.push('\n');
    }
    header.push_str(piece.trim_end());
}
