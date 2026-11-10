//! One C# file -> `SourceIndex` (tree-sitter).

use crate::signature::normalize;
use concordance_core::code::*;
use concordance_core::{ContentHash, FileId, RepoPath, SourceRange, SymbolId};
use std::collections::HashMap;
use tree_sitter::{Node, Parser};

const MAX_DETAIL: usize = 200;

/// Attribute names (without `Attribute`) that mark a method as a test.
const TEST_ATTRIBUTES: &[&str] = &[
    "Fact",
    "Theory",
    "Test",
    "TestCase",
    "TestCaseSource",
    "TestMethod",
    "DataTestMethod",
    "SkippableFact",
    "SkippableTheory",
];

fn bounded(s: &str) -> String {
    if s.chars().count() <= MAX_DETAIL {
        s.to_string()
    } else {
        let t: String = s.chars().take(MAX_DETAIL).collect();
        format!("{t}…")
    }
}

fn is_pascal(s: &str) -> bool {
    s.chars().next().is_some_and(|c| c.is_ascii_uppercase())
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Directory segments of a file inside its project: `src/Codec/Header.cs` -> `["Codec"]`.
fn module_path(path: &RepoPath, pkg_root: Option<&RepoPath>) -> Vec<String> {
    let rel = match pkg_root {
        Some(r) if path.starts_with_dir(r) => path.as_str()[r.as_str().len()..]
            .trim_start_matches('/')
            .to_string(),
        _ => path.as_str().to_string(),
    };
    let mut segs: Vec<String> = rel.split('/').map(str::to_string).collect();
    segs.pop(); // the file itself
    if segs.first().is_some_and(|s| s == "src") {
        segs.remove(0);
    }
    segs
}

struct Lines {
    starts: Vec<usize>,
}

impl Lines {
    fn new(text: &str) -> Self {
        let mut starts = vec![0];
        starts.extend(text.match_indices('\n').map(|(i, _)| i + 1));
        Lines { starts }
    }
    /// 1-based (line, column in chars).
    fn pos(&self, text: &str, off: usize) -> (u32, u32) {
        let off = off.min(text.len());
        let line = self.starts.partition_point(|s| *s <= off).max(1) - 1;
        let start = self.starts[line];
        let col = text.get(start..off).map(|s| s.chars().count()).unwrap_or(0);
        (line as u32 + 1, col as u32 + 1)
    }
}

#[derive(Clone)]
struct Scope {
    path: Vec<String>,
    parent: Option<SymbolId>,
    /// Every enclosing type is public.
    exported: bool,
    /// Visibility of a declaration without an accessibility modifier.
    default_vis: Visibility,
}

fn is_access(m: &str) -> bool {
    matches!(m, "public" | "private" | "protected" | "internal")
}

fn visibility(mods: &[String], default: Visibility) -> Visibility {
    let has = |m: &str| mods.iter().any(|x| x == m);
    if has("public") {
        Visibility::Public
    } else if has("protected") {
        Visibility::Restricted
    } else if has("internal") {
        Visibility::Crate
    } else if has("private") {
        Visibility::Private
    } else {
        default
    }
}

struct Ctx<'t> {
    path: &'t RepoPath,
    text: &'t str,
    lines: Lines,
    line_texts: Vec<&'t str>,
    file: FileId,
    symbols: Vec<Symbol>,
    /// Byte span of each symbol (same order as `symbols`), for fact attribution.
    spans: Vec<(usize, usize)>,
    imports: Vec<Import>,
    used: HashMap<String, usize>,
}

impl<'t> Ctx<'t> {
    fn src(&self, n: Node) -> &'t str {
        self.text.get(n.start_byte()..n.end_byte()).unwrap_or("")
    }

    fn field(&self, n: Node, name: &str) -> Option<&'t str> {
        n.child_by_field_name(name).map(|c| self.src(c))
    }

    fn range(&self, start: usize, end: usize) -> SourceRange {
        let (sl, sc) = self.lines.pos(self.text, start);
        let (el, ec) = self.lines.pos(self.text, end.saturating_sub(1).max(start));
        SourceRange {
            path: self.path.clone(),
            start_line: sl,
            start_col: sc,
            end_line: el,
            end_col: ec,
        }
    }

    fn modifiers(&self, n: Node) -> Vec<String> {
        let mut cur = n.walk();
        n.children(&mut cur)
            .filter(|c| c.kind() == "modifier")
            .map(|c| self.src(c).trim().to_string())
            .collect()
    }

    /// Doc text from the comments directly above byte offset `at`: `///` XML lines,
    /// `//` lines and `/* */` blocks, with comment markers and XML tags removed.
    fn leading_doc(&self, at: usize) -> Option<String> {
        let (line, _) = self.lines.pos(self.text, at);
        let all = &self.line_texts;
        let mut k = line as usize - 1; // index of the declaration line
        let mut collected: Vec<String> = Vec::new();
        while k > 0 {
            let l = all.get(k - 1).copied().unwrap_or("").trim();
            if l.starts_with("//") {
                collected.push(l.trim_start_matches('/').trim().to_string());
                k -= 1;
            } else if l.ends_with("*/") {
                let mut block: Vec<String> = Vec::new();
                let mut j = k - 1;
                loop {
                    let b = all.get(j).copied().unwrap_or("").trim();
                    let cleaned = b
                        .trim_start_matches("/**")
                        .trim_start_matches("/*")
                        .trim_end_matches("*/")
                        .trim()
                        .trim_start_matches('*')
                        .trim();
                    block.push(cleaned.to_string());
                    if b.starts_with("/*") || j == 0 {
                        break;
                    }
                    j -= 1;
                }
                block.reverse();
                collected.extend(block.into_iter().rev());
                k = j;
            } else if l.starts_with('[') {
                k -= 1; // attribute line between doc and declaration
            } else {
                break;
            }
        }
        collected.reverse();
        let doc = clean_xml(&collected.join("\n"));
        (!doc.is_empty()).then_some(doc)
    }

    #[allow(clippy::too_many_arguments)]
    fn push(
        &mut self,
        kind: SymbolKind,
        name: &str,
        scope: &Scope,
        node: Node,
        vis: Visibility,
        signature: Option<String>,
        salt: bool,
    ) -> usize {
        let (start, end) = (node.start_byte(), node.end_byte());
        let mut path = scope.path.clone();
        path.push(name.to_string());
        let key = format!("{}\u{0}{}", kind.as_str(), path.join("::"));
        let n = self.used.entry(key).or_insert(0);
        *n += 1;
        let mut occ = if *n == 1 {
            String::new()
        } else {
            format!("#{n}")
        };
        if salt {
            // Namespaces and partial types legitimately recur across files.
            occ = format!("{}{occ}", self.path.as_str());
        }
        let id = SymbolId::from_key(&[kind.as_str(), &path.join("::"), &occ]);
        let doc = self.leading_doc(start);
        let verifies = doc.as_deref().map(verifies_from_doc).unwrap_or_default();
        let body = self.src(node);
        self.symbols.push(Symbol {
            id,
            file: self.file.clone(),
            kind,
            name: name.to_string(),
            path,
            parent: scope.parent.clone(),
            visibility: vis,
            is_public_api: vis == Visibility::Public && scope.exported,
            signature_hash: signature.as_deref().map(ContentHash::of_str),
            signature,
            doc,
            attributes: vec![],
            cfg: vec![],
            range: self.range(start, end),
            content_hash: ContentHash::of_str(&normalize(body)),
            impl_info: None,
            returns: None,
            facts: vec![],
            calls: vec![],
            is_test: false,
            verifies,
        });
        self.spans.push((start, end));
        self.symbols.len() - 1
    }

    fn child_scope(
        &self,
        scope: &Scope,
        idx: usize,
        name: &str,
        exported: bool,
        def: Visibility,
    ) -> Scope {
        let mut path = scope.path.clone();
        path.push(name.to_string());
        Scope {
            path,
            parent: Some(self.symbols[idx].id.clone()),
            exported,
            default_vis: def,
        }
    }

    /// Signature text built from the declaration's parts: attributes, accessibility
    /// modifiers and bodies are left out.
    fn header(&self, n: Node) -> String {
        let accessor_like = matches!(n.kind(), "property_declaration" | "indexer_declaration");
        let mut parts: Vec<String> = Vec::new();
        let mut cur = n.walk();
        for ch in n.children(&mut cur) {
            match ch.kind() {
                "attribute_list" | "preproc_if_in_attribute_list" | "comment" => continue,
                "modifier" => {
                    if is_access(self.src(ch).trim()) {
                        continue;
                    }
                }
                "accessor_list" => {
                    parts.push(self.accessors(ch));
                    continue;
                }
                "arrow_expression_clause" => {
                    if accessor_like {
                        parts.push("{ get; }".into());
                    }
                    break;
                }
                "block"
                | "declaration_list"
                | "enum_member_declaration_list"
                | "constructor_initializer"
                | "="
                | ";" => break,
                _ => {}
            }
            parts.push(self.src(ch).to_string());
        }
        normalize(&parts.join(" "))
    }

    fn accessors(&self, list: Node) -> String {
        let mut cur = list.walk();
        let items: Vec<String> = list
            .children(&mut cur)
            .filter(|c| c.kind() == "accessor_declaration")
            .map(|a| {
                let mut c2 = a.walk();
                let mods: Vec<&str> = a
                    .children(&mut c2)
                    .filter(|m| m.kind() == "modifier")
                    .map(|m| self.src(m).trim())
                    .collect();
                let name = self.field(a, "name").unwrap_or("");
                if mods.is_empty() {
                    format!("{name};")
                } else {
                    format!("{} {name};", mods.join(" "))
                }
            })
            .collect();
        format!("{{ {} }}", items.join(" "))
    }

    fn is_test_method(&self, n: Node) -> bool {
        let mut cur = n.walk();
        for list in n
            .children(&mut cur)
            .filter(|c| c.kind() == "attribute_list")
        {
            let mut c2 = list.walk();
            for attr in list.children(&mut c2).filter(|c| c.kind() == "attribute") {
                let name = self.field(attr, "name").unwrap_or("");
                let last = name.rsplit('.').next().unwrap_or(name);
                let last = last.strip_suffix("Attribute").unwrap_or(last);
                if TEST_ATTRIBUTES.contains(&last) {
                    return true;
                }
            }
        }
        false
    }

    fn set_returns(&mut self, idx: usize, ret: Option<Node>) {
        let Some(r) = ret else { return };
        let text = normalize(self.src(r));
        let is_option = text.ends_with('?') || text.starts_with("Nullable<");
        self.symbols[idx].returns = Some(ReturnContract {
            text,
            result: None,
            is_option,
        });
    }

    fn decls(&mut self, parent: Node, scope: &Scope) {
        let mut scope = scope.clone();
        let mut cur = parent.walk();
        let kids: Vec<Node> = parent.children(&mut cur).collect();
        for n in kids {
            if n.kind() == "file_scoped_namespace_declaration" {
                scope = self.namespace(n, &scope);
            } else {
                self.decl(n, &scope);
            }
        }
    }

    /// Declares the namespace's segments; returns the innermost scope.
    fn namespace(&mut self, n: Node, scope: &Scope) -> Scope {
        let name = self.field(n, "name").unwrap_or("").to_string();
        let mut cur = scope.clone();
        for seg in name.split('.').map(str::trim).filter(|s| !s.is_empty()) {
            let idx = self.push(
                SymbolKind::Namespace,
                seg,
                &cur,
                n,
                Visibility::Public,
                None,
                true,
            );
            cur = self.child_scope(&cur, idx, seg, cur.exported, Visibility::Crate);
        }
        cur
    }

    fn decl(&mut self, n: Node, scope: &Scope) {
        match n.kind() {
            "namespace_declaration" => {
                let inner = self.namespace(n, scope);
                if let Some(body) = n.child_by_field_name("body") {
                    self.decls(body, &inner);
                }
            }
            "class_declaration"
            | "struct_declaration"
            | "record_declaration"
            | "interface_declaration" => self.type_decl(n, scope),
            "enum_declaration" => self.enum_decl(n, scope),
            "delegate_declaration" => {
                let Some(name) = self.field(n, "name") else {
                    return;
                };
                let vis = visibility(&self.modifiers(n), scope.default_vis);
                let sig = self.header(n);
                self.push(SymbolKind::TypeAlias, name, scope, n, vis, Some(sig), false);
            }
            "method_declaration" => {
                let Some(name) = self.field(n, "name") else {
                    return;
                };
                let vis = visibility(&self.modifiers(n), scope.default_vis);
                let sig = self.header(n);
                let idx = self.push(SymbolKind::Method, name, scope, n, vis, Some(sig), false);
                self.set_returns(idx, n.child_by_field_name("returns"));
                self.mark_test(idx, n);
            }
            "operator_declaration" => {
                let op = n
                    .child_by_field_name("operator")
                    .map(|o| self.src(o))
                    .unwrap_or("");
                let name = format!("operator {op}");
                let vis = visibility(&self.modifiers(n), scope.default_vis);
                let sig = self.header(n);
                self.push(SymbolKind::Method, &name, scope, n, vis, Some(sig), false);
            }
            "constructor_declaration" => {
                let Some(name) = self.field(n, "name") else {
                    return;
                };
                let vis = visibility(&self.modifiers(n), scope.default_vis);
                let sig = self.header(n);
                self.push(
                    SymbolKind::Constructor,
                    name,
                    scope,
                    n,
                    vis,
                    Some(sig),
                    false,
                );
            }
            "property_declaration" => {
                let Some(name) = self.field(n, "name") else {
                    return;
                };
                let vis = visibility(&self.modifiers(n), scope.default_vis);
                let sig = self.header(n);
                let idx = self.push(SymbolKind::Property, name, scope, n, vis, Some(sig), false);
                self.set_returns(idx, n.child_by_field_name("type"));
            }
            "indexer_declaration" => {
                let vis = visibility(&self.modifiers(n), scope.default_vis);
                let sig = self.header(n);
                self.push(
                    SymbolKind::Property,
                    "this",
                    scope,
                    n,
                    vis,
                    Some(sig),
                    false,
                );
            }
            "event_declaration" => {
                let Some(name) = self.field(n, "name") else {
                    return;
                };
                let vis = visibility(&self.modifiers(n), scope.default_vis);
                let sig = self.header(n);
                self.push(SymbolKind::Property, name, scope, n, vis, Some(sig), false);
            }
            "field_declaration" | "event_field_declaration" => self.field_decl(n, scope),
            "using_directive" => self.using(n),
            "preproc_if" | "preproc_else" | "preproc_elif" => {
                let mut cur = n.walk();
                let kids: Vec<Node> = n.children(&mut cur).collect();
                for k in kids {
                    self.decl(k, scope);
                }
            }
            _ => {}
        }
    }

    fn mark_test(&mut self, idx: usize, n: Node) {
        if self.is_test_method(n) {
            self.symbols[idx].is_test = true;
            self.symbols[idx].is_public_api = false;
        }
    }

    fn type_decl(&mut self, n: Node, scope: &Scope) {
        let Some(name) = self.field(n, "name") else {
            return;
        };
        let mods = self.modifiers(n);
        let vis = visibility(&mods, scope.default_vis);
        let is_struct = n.kind() == "struct_declaration" || {
            let mut cur = n.walk();
            let found = n.children(&mut cur).any(|c| c.kind() == "struct");
            found
        };
        let (kind, def) = match n.kind() {
            "interface_declaration" => (SymbolKind::Interface, Visibility::Public),
            _ if is_struct => (SymbolKind::Struct, Visibility::Private),
            _ => (SymbolKind::Class, Visibility::Private),
        };
        let sig = self.header(n);
        let partial = mods.iter().any(|m| m == "partial");
        let idx = self.push(kind, name, scope, n, vis, Some(sig), partial);
        let inner = self.child_scope(
            scope,
            idx,
            name,
            scope.exported && vis == Visibility::Public,
            def,
        );
        if let Some(body) = n.child_by_field_name("body") {
            self.decls(body, &inner);
        }
    }

    fn enum_decl(&mut self, n: Node, scope: &Scope) {
        let Some(name) = self.field(n, "name") else {
            return;
        };
        let vis = visibility(&self.modifiers(n), scope.default_vis);
        let sig = self.header(n);
        let idx = self.push(SymbolKind::Enum, name, scope, n, vis, Some(sig), false);
        let inner = self.child_scope(
            scope,
            idx,
            name,
            scope.exported && vis == Visibility::Public,
            Visibility::Public,
        );
        let Some(body) = n.child_by_field_name("body") else {
            return;
        };
        let mut cur = body.walk();
        let kids: Vec<Node> = body.children(&mut cur).collect();
        for m in kids
            .into_iter()
            .filter(|m| m.kind() == "enum_member_declaration")
        {
            if let Some(mname) = self.field(m, "name") {
                self.push(
                    SymbolKind::Variant,
                    mname,
                    &inner,
                    m,
                    Visibility::Public,
                    None,
                    false,
                );
            }
        }
    }

    fn field_decl(&mut self, n: Node, scope: &Scope) {
        let is_event = n.kind() == "event_field_declaration";
        let mods = self.modifiers(n);
        let vis = visibility(&mods, scope.default_vis);
        let is_const = mods.iter().any(|m| m == "const");
        let kept: Vec<&str> = mods
            .iter()
            .map(String::as_str)
            .filter(|m| !is_access(m))
            .collect();
        let mut cur = n.walk();
        let Some(vd) = n
            .children(&mut cur)
            .find(|c| c.kind() == "variable_declaration")
        else {
            return;
        };
        let ty = self.field(vd, "type").unwrap_or("");
        let mut c2 = vd.walk();
        let decls: Vec<Node> = vd
            .children(&mut c2)
            .filter(|c| c.kind() == "variable_declarator")
            .collect();
        let single = decls.len() == 1;
        for d in decls {
            let Some(nm) = d.child_by_field_name("name") else {
                continue;
            };
            let name = self.src(nm);
            let mut sig = kept.join(" ");
            if is_event {
                sig.push_str(" event");
            }
            let sig = normalize(&format!("{sig} {ty} {name}"));
            let kind = if is_event {
                SymbolKind::Property
            } else if is_const {
                SymbolKind::Const
            } else {
                SymbolKind::Field
            };
            // The node whose span (and leading doc) belongs to the symbol.
            let span_node = if single { n } else { d };
            let idx = self.push(kind, name, scope, span_node, vis, Some(sig), false);
            if is_const {
                let mut c3 = d.walk();
                let value = d
                    .named_children(&mut c3)
                    .filter(|c| c.id() != nm.id())
                    .last();
                if let Some(v) = value {
                    let raw = self.src(v).trim().to_string();
                    if numeric_literal(&raw) {
                        let range = self.range(d.start_byte(), d.end_byte());
                        self.symbols[idx].facts.push(Fact {
                            kind: FactKind::ConstValue,
                            range,
                            detail: format!("{name} = {raw}"),
                        });
                    }
                }
            }
        }
    }

    fn using(&mut self, n: Node) {
        let text = self.src(n).trim().trim_end_matches(';').trim();
        let body = text
            .strip_prefix("global")
            .map(str::trim_start)
            .unwrap_or(text);
        let Some(body) = body.strip_prefix("using") else {
            return;
        };
        let body = body.trim();
        let is_static = body.starts_with("static ");
        let body = body.strip_prefix("static ").unwrap_or(body).trim();
        let (alias, target) = match body.split_once('=') {
            Some((a, t)) => (Some(a.trim().to_string()), t.trim()),
            None => (None, body),
        };
        let path: Vec<String> = target
            .split('.')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        if path.is_empty() {
            return;
        }
        self.imports.push(Import {
            glob: alias.is_none() && !is_static,
            path,
            alias,
            range: self.range(n.start_byte(), n.end_byte()),
        });
    }
}

/// `100`, `1_000`, `1.5f`, `-3`, `100L`.
fn numeric_literal(s: &str) -> bool {
    let t = s.trim_start_matches('-').replace('_', "");
    if t.starts_with("0x") || t.starts_with("0X") || t.is_empty() {
        return false;
    }
    let t = t.trim_end_matches(|c: char| {
        matches!(c, 'f' | 'F' | 'd' | 'D' | 'm' | 'M' | 'l' | 'L' | 'u' | 'U')
    });
    t.parse::<f64>().is_ok()
}

/// Strips XML doc tags: block tags (`summary`, `param`, ...) become line breaks, `see`
/// and `paramref` become their reference, other tags vanish. Entities are unescaped.
fn clean_xml(s: &str) -> String {
    let cs: Vec<char> = s.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < cs.len() {
        if cs[i] == '<'
            && cs
                .get(i + 1)
                .is_some_and(|c| c.is_alphabetic() || *c == '/')
        {
            if let Some(rel) = cs[i..].iter().position(|c| *c == '>') {
                let tag: String = cs[i + 1..i + rel].iter().collect();
                let closing = tag.starts_with('/');
                let body = tag.trim_start_matches('/').trim_end_matches('/');
                let name = body
                    .split_whitespace()
                    .next()
                    .unwrap_or("")
                    .to_ascii_lowercase();
                match name.as_str() {
                    "summary" | "remarks" | "returns" | "param" | "typeparam" | "example"
                    | "exception" | "value" | "para" | "list" | "item" | "description" | "term"
                    | "br" | "inheritdoc" | "code" => out.push('\n'),
                    "see" | "seealso" | "paramref" | "typeparamref" if !closing => {
                        for key in ["cref=\"", "name=\"", "langword=\""] {
                            if let Some(k) = body.find(key) {
                                let v = &body[k + key.len()..];
                                if let Some(q) = v.find('"') {
                                    let v = &v[..q];
                                    out.push_str(v.rsplit(':').next().unwrap_or(v));
                                }
                                break;
                            }
                        }
                    }
                    _ => {}
                }
                i += rel + 1;
                continue;
            }
        }
        out.push(cs[i]);
        i += 1;
    }
    let out = out
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&");
    out.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Requirement IDs declared by `Verifies: ID, ID` lines.
pub fn verifies_from_doc(doc: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in doc.lines() {
        let l = line.trim().trim_start_matches(['*', '-']).trim_start();
        let lower = l.to_ascii_lowercase();
        let rest = if let Some(r) = lower.strip_prefix("verifies:") {
            &l[l.len() - r.len()..]
        } else if let Some(r) = lower.strip_prefix("verifies ") {
            &l[l.len() - r.len()..]
        } else {
            continue;
        };
        for tok in rest.split(|c: char| c.is_whitespace() || matches!(c, ',' | ';')) {
            let t =
                tok.trim_matches(|c: char| matches!(c, '`' | '*' | '[' | ']' | '(' | ')' | ':'));
            let t = t.strip_suffix('.').unwrap_or(t);
            if concordance_core::doc::is_requirement_id(t) && !out.iter().any(|x| x == t) {
                out.push(t.to_string());
            }
        }
    }
    out
}

// ------------------------------------------------------------------ body facts

struct RawFact {
    kind: FactKind,
    span: (usize, usize),
    detail: String,
}

struct RawCall {
    target: CallTarget,
    span: (usize, usize),
}

fn is_literal(n: Option<Node>) -> bool {
    n.is_some_and(|n| matches!(n.kind(), "integer_literal" | "real_literal"))
}

/// Walks the whole tree (iteratively: long operator chains nest deeply).
fn collect(ctx: &Ctx, root: Node) -> (Vec<RawFact>, Vec<RawCall>) {
    let mut facts = Vec::new();
    let mut calls = Vec::new();
    let mut stack = vec![root];
    while let Some(n) = stack.pop() {
        let span = (n.start_byte(), n.end_byte());
        match n.kind() {
            "throw_statement" | "throw_expression" => facts.push(RawFact {
                kind: FactKind::PanicSite,
                span,
                detail: bounded(&normalize(ctx.src(n))),
            }),
            "unsafe_statement" => facts.push(RawFact {
                kind: FactKind::UnsafeCode,
                span,
                detail: "unsafe".into(),
            }),
            "modifier" if ctx.src(n).trim() == "unsafe" => facts.push(RawFact {
                kind: FactKind::UnsafeCode,
                span,
                detail: "unsafe".into(),
            }),
            "binary_expression" => {
                let op = n.child_by_field_name("operator").map(|o| o.kind());
                if matches!(op, Some("<" | "<=" | ">" | ">=" | "==" | "!="))
                    && (is_literal(n.child_by_field_name("left"))
                        || is_literal(n.child_by_field_name("right")))
                {
                    facts.push(RawFact {
                        kind: FactKind::LiteralComparison,
                        span,
                        detail: bounded(&normalize(ctx.src(n))),
                    });
                }
            }
            "invocation_expression" => {
                if let Some(f) = n.child_by_field_name("function") {
                    let callee: String = ctx.src(f).split_whitespace().collect();
                    let callee = callee.strip_prefix("System.").unwrap_or(&callee);
                    let lit_arg = n.child_by_field_name("arguments").is_some_and(|a| {
                        let mut cur = a.walk();
                        let found = a
                            .named_children(&mut cur)
                            .any(|arg| is_literal(arg.named_child(0)));
                        found
                    });
                    if callee == "Math.Clamp"
                        || (matches!(callee, "Math.Min" | "Math.Max") && lit_arg)
                    {
                        facts.push(RawFact {
                            kind: FactKind::Clamp,
                            span,
                            detail: bounded(&normalize(ctx.src(n))),
                        });
                    }
                    if matches!(
                        callee,
                        "Environment.GetEnvironmentVariable"
                            | "Environment.GetEnvironmentVariables"
                    ) {
                        facts.push(RawFact {
                            kind: FactKind::EnvRead,
                            span,
                            detail: bounded(&normalize(ctx.src(n))),
                        });
                    }
                    let target = match f.kind() {
                        "identifier" => Some(CallTarget::Path {
                            segments: vec![ctx.src(f).to_string()],
                        }),
                        "generic_name" => Some(CallTarget::Path {
                            segments: vec![ctx.src(f).split('<').next().unwrap_or("").to_string()],
                        }),
                        "member_access_expression" => {
                            let obj = f
                                .child_by_field_name("expression")
                                .map(|o| ctx.src(o).trim());
                            let name = f
                                .child_by_field_name("name")
                                .map(|o| ctx.src(o).split('<').next().unwrap_or("").to_string());
                            match (obj, name) {
                                (Some(o), Some(nm)) if is_pascal(o) => Some(CallTarget::Path {
                                    segments: vec![o.to_string(), nm],
                                }),
                                (_, Some(nm)) => Some(CallTarget::Method { name: nm }),
                                _ => None,
                            }
                        }
                        _ => None,
                    };
                    if let Some(target) = target {
                        calls.push(RawCall { target, span });
                    }
                }
            }
            _ => {}
        }
        let mut cur = n.walk();
        let kids: Vec<Node> = n.children(&mut cur).collect();
        stack.extend(kids.into_iter().rev());
    }
    (facts, calls)
}

// ------------------------------------------------------------------ entry points

fn header(path: &RepoPath, text: &str, ws: &WorkspaceInfo) -> (SourceFile, Option<RepoPath>) {
    let pkg = ws
        .crates
        .iter()
        .filter(|c| c.language == "csharp")
        .filter(|c| c.root_dir.as_ref().is_none_or(|d| path.starts_with_dir(d)))
        .max_by_key(|c| c.root_dir.as_ref().map_or(0, |d| d.as_str().len()));
    let root = pkg.and_then(|p| p.root_dir.clone());
    let file = SourceFile {
        id: FileId::from_key(&[path.as_str()]),
        path: path.clone(),
        language: "csharp".into(),
        crate_name: pkg.map(|p| p.name.clone()),
        module_path: module_path(path, root.as_ref()),
        content_hash: ContentHash::of_str(text),
        line_count: text.lines().count() as u32,
        parse_error: None,
    };
    (file, root)
}

pub fn failed(path: &RepoPath, text: &str, ws: &WorkspaceInfo, error: String) -> SourceIndex {
    let (mut file, _) = header(path, text, ws);
    file.parse_error = Some(error);
    SourceIndex {
        file,
        symbols: vec![],
        imports: vec![],
    }
}

/// Index a documented code block (no project).
pub fn index_snippet(path: &RepoPath, text: &str) -> SourceIndex {
    index(path, text, &WorkspaceInfo::default())
}

pub fn index(path: &RepoPath, text: &str, ws: &WorkspaceInfo) -> SourceIndex {
    let (file, _root) = header(path, text, ws);
    let mut parser = Parser::new();
    if parser
        .set_language(&tree_sitter_c_sharp::LANGUAGE.into())
        .is_err()
    {
        return failed(path, text, ws, "C# grammar could not be loaded".into());
    }
    let Some(tree) = parser.parse(text, None) else {
        return failed(path, text, ws, "parse failed".into());
    };
    let root = tree.root_node();
    let mut ctx = Ctx {
        path,
        text,
        lines: Lines::new(text),
        line_texts: text.lines().collect(),
        file: file.id.clone(),
        symbols: vec![],
        spans: vec![],
        imports: vec![],
        used: HashMap::new(),
    };
    let top = Scope {
        path: file.crate_name.iter().cloned().collect(),
        parent: None,
        exported: true,
        default_vis: Visibility::Crate,
    };
    ctx.decls(root, &top);

    let (facts, calls) = collect(&ctx, root);
    // Attribute facts and calls to the innermost containing symbol.
    let innermost = |spans: &[(usize, usize)], at: (usize, usize)| -> Option<usize> {
        spans
            .iter()
            .enumerate()
            .filter(|(_, (s, e))| *s <= at.0 && at.1 <= *e)
            .min_by_key(|(_, (s, e))| e - s)
            .map(|(i, _)| i)
    };
    for f in facts {
        if let Some(i) = innermost(&ctx.spans, f.span) {
            let range = ctx.range(f.span.0, f.span.1);
            ctx.symbols[i].facts.push(Fact {
                kind: f.kind,
                range,
                detail: f.detail,
            });
        }
    }
    for c in calls {
        if let Some(i) = innermost(&ctx.spans, c.span) {
            let range = ctx.range(c.span.0, c.span.1);
            ctx.symbols[i].calls.push(CallSite {
                target: c.target,
                range,
            });
        }
    }
    let mut file = file;
    if ctx.symbols.is_empty() && root.has_error() {
        file.parse_error = Some("syntax errors".into());
    }
    SourceIndex {
        file,
        symbols: ctx.symbols,
        imports: ctx.imports,
    }
}
