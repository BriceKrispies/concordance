//! One Lean file -> `SourceIndex`, from the token stream of [`crate::lex`].
//!
//! A *command* starts at a declaration keyword (after its modifiers and attributes) or at
//! one of the other top-level keywords (`namespace`, `section`, `end`, `open`, ...) that
//! begins a line. A declaration's range runs from its first modifier up to the next command.

use crate::lex::{lex, name_segments, Comment, CommentKind, Lexed, TokKind};
use concordance_core::code::*;
use concordance_core::{ContentHash, FileId, RepoPath, SourceRange, SymbolId};
use std::collections::HashMap;

const DECL_KW: &[&str] = &[
    "def",
    "theorem",
    "lemma",
    "abbrev",
    "structure",
    "inductive",
    "class",
    "instance",
    "axiom",
    "opaque",
    "example",
];

/// Keywords that end a declaration when they start a line in column 1.
const OTHER_KW: &[&str] = &[
    "open",
    "variable",
    "universe",
    "universes",
    "set_option",
    "attribute",
    "export",
    "macro",
    "macro_rules",
    "syntax",
    "notation",
    "infix",
    "infixl",
    "infixr",
    "prefix",
    "postfix",
    "elab",
    "elab_rules",
    "initialize",
    "builtin_initialize",
    "deriving",
    "declare_syntax_cat",
    "run_cmd",
    "compile_inductive",
    "module",
    "register_option",
    "register_simp_attr",
];

const MODIFIERS: &[&str] = &[
    "private",
    "protected",
    "noncomputable",
    "partial",
    "unsafe",
    "nonrec",
    "local",
    "scoped",
];

const NOT_FIELD: &[&str] = &[
    "have", "let", "show", "obtain", "suffices", "from", "deriving", "calc", "exact", "fun",
    "match", "if", "then", "else", "by", "at", "with", "do",
];

const MAX_DETAIL: usize = 200;

#[derive(Clone, Debug, PartialEq, Eq)]
enum CmdKind {
    Decl(&'static str),
    Namespace,
    Section,
    End,
    Mutual,
    Import,
    Other,
}

struct Cmd {
    /// First token, modifiers and attributes included.
    first: usize,
    /// The keyword token.
    kw: usize,
    kind: CmdKind,
}

fn bounded(s: &str) -> String {
    if s.chars().count() <= MAX_DETAIL {
        s.to_string()
    } else {
        let t: String = s.chars().take(MAX_DETAIL).collect();
        format!("{t}…")
    }
}

struct Src<'t> {
    text: &'t str,
    lx: Lexed,
}

impl<'t> Src<'t> {
    fn t(&self, i: usize) -> &str {
        let k = &self.lx.toks[i];
        &self.text[k.start..k.end]
    }
    fn is(&self, i: usize, s: &str) -> bool {
        i < self.lx.toks.len() && self.t(i) == s
    }
    fn is_ident(&self, i: usize) -> bool {
        self.lx
            .toks
            .get(i)
            .is_some_and(|t| t.kind == TokKind::Ident)
    }
    /// Tokens `a..b` joined with one space wherever the source had a gap (whitespace
    /// or comments), so spacing and comments never matter.
    fn join(&self, a: usize, b: usize) -> String {
        let mut out = String::new();
        let mut prev_end: Option<usize> = None;
        for i in a..b.min(self.lx.toks.len()) {
            let k = &self.lx.toks[i];
            if prev_end.is_some_and(|e| e < k.start) {
                out.push(' ');
            }
            out.push_str(&self.text[k.start..k.end]);
            prev_end = Some(k.end);
        }
        out
    }
}

fn opens(s: &str) -> bool {
    matches!(s, "(" | "[" | "{" | "⟨" | "⦃" | "⟦" | "‹")
}
fn closes(s: &str) -> bool {
    matches!(s, ")" | "]" | "}" | "⟩" | "⦄" | "⟧" | "›")
}

/// Module path of a file inside its package: `Volume/Mixer.lean` -> `["Volume","Mixer"]`.
fn module_path(path: &RepoPath, pkg_root: Option<&RepoPath>) -> Vec<String> {
    let rel = match pkg_root {
        Some(r) if path.starts_with_dir(r) => path.as_str()[r.as_str().len()..]
            .trim_start_matches('/')
            .to_string(),
        _ => path.as_str().to_string(),
    };
    let mut segs: Vec<String> = rel.split('/').map(str::to_string).collect();
    if let Some(last) = segs.last_mut() {
        if let Some(i) = last.rfind('.') {
            last.truncate(i);
        }
    }
    segs
}

fn header(path: &RepoPath, text: &str, ws: &WorkspaceInfo) -> SourceFile {
    let pkg = ws
        .crates
        .iter()
        .filter(|c| c.language == "lean")
        .filter(|c| c.root_dir.as_ref().is_none_or(|d| path.starts_with_dir(d)))
        .max_by_key(|c| c.root_dir.as_ref().map_or(0, |d| d.as_str().len()));
    SourceFile {
        id: FileId::from_key(&[path.as_str()]),
        path: path.clone(),
        language: "lean".into(),
        crate_name: pkg.map(|p| p.name.clone()),
        module_path: module_path(path, pkg.and_then(|p| p.root_dir.as_ref())),
        content_hash: ContentHash::of_str(text),
        line_count: text.lines().count() as u32,
        parse_error: None,
    }
}

pub fn failed(path: &RepoPath, text: &str, ws: &WorkspaceInfo, error: String) -> SourceIndex {
    let mut file = header(path, text, ws);
    file.parse_error = Some(error);
    SourceIndex {
        file,
        symbols: vec![],
        imports: vec![],
    }
}

/// Index a documented code block (no package, no module path).
pub fn index_snippet(path: &RepoPath, text: &str) -> SourceIndex {
    let mut idx = index(path, text, &WorkspaceInfo::default());
    for s in &mut idx.symbols {
        let drop = idx.file.module_path.len();
        if s.path.len() > drop {
            s.path.drain(..drop);
        }
    }
    idx
}

/// `Verifies: ID, ID` lines in doc text.
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

pub fn index(path: &RepoPath, text: &str, ws: &WorkspaceInfo) -> SourceIndex {
    let file = header(path, text, ws);
    let src = Src {
        text,
        lx: lex(text),
    };
    let mut base: Vec<String> = file.crate_name.iter().cloned().collect();
    base.extend(file.module_path.iter().cloned());
    let mut cx = Cx {
        src,
        path,
        file: file.id.clone(),
        symbols: vec![],
        imports: vec![],
        used: HashMap::new(),
    };
    cx.run(base);
    SourceIndex {
        file,
        symbols: cx.symbols,
        imports: cx.imports,
    }
}

struct Cx<'t> {
    src: Src<'t>,
    path: &'t RepoPath,
    file: FileId,
    symbols: Vec<Symbol>,
    imports: Vec<Import>,
    used: HashMap<String, usize>,
}

impl<'t> Cx<'t> {
    fn range(&self, start: usize, end: usize) -> SourceRange {
        let (sl, sc) = self.src.lx.pos(self.src.text, start);
        let (el, ec) = self
            .src
            .lx
            .pos(self.src.text, end.saturating_sub(1).max(start));
        SourceRange {
            path: self.path.clone(),
            start_line: sl,
            start_col: sc,
            end_line: el,
            end_col: ec,
        }
    }

    fn tok_range(&self, a: usize, b: usize) -> SourceRange {
        self.range(self.src.lx.toks[a].start, self.src.lx.toks[b].end)
    }

    /// Where commands start.
    fn commands(&self) -> Vec<Cmd> {
        let n = self.src.lx.toks.len();
        let mut out = Vec::new();
        for i in 0..n {
            let tk = &self.src.lx.toks[i];
            let word = self.src.t(i);
            if tk.kind == TokKind::Sym && word == "#" && tk.first_on_line && tk.col == 1 {
                out.push(Cmd {
                    first: i,
                    kw: i,
                    kind: CmdKind::Other,
                });
                continue;
            }
            if tk.kind != TokKind::Ident {
                continue;
            }
            if let Some(kw) = DECL_KW.iter().find(|k| **k == word) {
                // `deriving instance ...` and friends: the keyword must open the command.
                let first = self.walk_back_modifiers(i);
                if self.src.lx.toks[first].first_on_line {
                    out.push(Cmd {
                        first,
                        kw: i,
                        kind: CmdKind::Decl(kw),
                    });
                }
                continue;
            }
            if !tk.first_on_line {
                continue;
            }
            let kind = match word {
                "namespace" => CmdKind::Namespace,
                "section" => CmdKind::Section,
                "end" => CmdKind::End,
                "mutual" => CmdKind::Mutual,
                "import" => CmdKind::Import,
                w if tk.col == 1 && OTHER_KW.contains(&w) => CmdKind::Other,
                _ => continue,
            };
            out.push(Cmd {
                first: i,
                kw: i,
                kind,
            });
        }
        out
    }

    /// Index of the first modifier/attribute token before keyword `kw`.
    fn walk_back_modifiers(&self, kw: usize) -> usize {
        let mut first = kw;
        while first > 0 {
            let p = first - 1;
            let w = self.src.t(p);
            if self.src.lx.toks[p].kind == TokKind::Ident && MODIFIERS.contains(&w) {
                first = p;
            } else if w == "]" {
                // Attribute `@[...]`.
                let mut depth = 0usize;
                let mut j = p;
                let open = loop {
                    let wj = self.src.t(j);
                    if closes(wj) {
                        depth += 1;
                    } else if opens(wj) {
                        depth -= 1;
                        if depth == 0 {
                            break Some(j);
                        }
                    }
                    if j == 0 {
                        break None;
                    }
                    j -= 1;
                };
                match open {
                    Some(o) if o > 0 && self.src.t(o - 1) == "@" && self.src.t(o) == "[" => {
                        first = o - 1;
                    }
                    _ => break,
                }
            } else {
                break;
            }
        }
        first
    }

    /// Doc text from the comments directly above (or beside) the command starting at
    /// token `first`: a `/-- -/` block and/or consecutive `--` lines.
    fn leading_doc(&self, first: usize) -> Option<String> {
        let tk = &self.src.lx.toks[first];
        let prev_end = if first == 0 {
            0
        } else {
            self.src.lx.toks[first - 1].end
        };
        let mut limit = tk.start;
        let mut cur_line = tk.line;
        let mut parts: Vec<String> = Vec::new();
        loop {
            let comments = &self.src.lx.comments;
            let k = comments.partition_point(|c| c.end <= limit);
            let c: Option<&Comment> = k
                .checked_sub(1)
                .map(|k| &comments[k])
                .filter(|c| c.start >= prev_end);
            let Some(c) = c else { break };
            if !c.own_line || c.kind == CommentKind::ModuleDoc || c.end_line + 1 < cur_line {
                break;
            }
            parts.push(clean_comment(c));
            limit = c.start;
            cur_line = c.start_line;
        }
        parts.reverse();
        let doc = parts.join("\n").trim().to_string();
        (!doc.is_empty()).then_some(doc)
    }

    #[allow(clippy::too_many_arguments)]
    fn push(
        &mut self,
        kind: SymbolKind,
        name: &str,
        path: Vec<String>,
        parent: Option<SymbolId>,
        vis: Visibility,
        signature: Option<String>,
        range: SourceRange,
        hash_of: &str,
        doc: Option<String>,
    ) -> usize {
        let key = format!("{}\u{0}{}", kind.as_str(), path.join("::"));
        let n = self.used.entry(key).or_insert(0);
        *n += 1;
        let occ = if *n == 1 {
            String::new()
        } else {
            format!("#{n}")
        };
        let id = SymbolId::from_key(&[kind.as_str(), &path.join("::"), &occ]);
        let verifies = doc.as_deref().map(verifies_from_doc).unwrap_or_default();
        self.symbols.push(Symbol {
            id,
            file: self.file.clone(),
            kind,
            name: name.to_string(),
            path,
            parent,
            visibility: vis,
            is_public_api: vis == Visibility::Public,
            signature_hash: signature.as_deref().map(ContentHash::of_str),
            signature,
            doc,
            attributes: vec![],
            cfg: vec![],
            range,
            content_hash: ContentHash::of_str(hash_of),
            impl_info: None,
            returns: None,
            facts: vec![],
            calls: vec![],
            is_test: false,
            verifies,
        });
        self.symbols.len() - 1
    }

    fn run(&mut self, base: Vec<String>) {
        let cmds = self.commands();
        let ntok = self.src.lx.toks.len();
        // Namespace nesting: segments pushed per open scope.
        let mut ns: Vec<String> = Vec::new();
        let mut stack: Vec<usize> = Vec::new();
        for (ci, cmd) in cmds.iter().enumerate() {
            let end_tok = cmds.get(ci + 1).map_or(ntok, |c| c.first);
            match &cmd.kind {
                CmdKind::Namespace => {
                    let segs = if self.src.is_ident(cmd.kw + 1) {
                        name_segments(self.src.t(cmd.kw + 1))
                    } else {
                        vec![]
                    };
                    stack.push(segs.len());
                    ns.extend(segs);
                }
                CmdKind::Section | CmdKind::Mutual => stack.push(0),
                CmdKind::End => {
                    if let Some(n) = stack.pop() {
                        ns.truncate(ns.len().saturating_sub(n));
                    }
                }
                CmdKind::Import => {
                    if self.src.is_ident(cmd.kw + 1) {
                        let path = name_segments(self.src.t(cmd.kw + 1));
                        let range = self.tok_range(cmd.kw, cmd.kw + 1);
                        self.imports.push(Import {
                            path,
                            alias: None,
                            glob: false,
                            range,
                        });
                    }
                }
                CmdKind::Other => {}
                CmdKind::Decl(kw) => {
                    let mut scope = base.clone();
                    scope.extend(ns.iter().cloned());
                    self.decl(cmd, kw, end_tok, &scope);
                }
            }
        }
    }

    fn decl(&mut self, cmd: &Cmd, kw: &str, end_tok: usize, scope: &[String]) {
        let src = &self.src;
        let mut kw_idx = cmd.kw;
        let mut kw = kw;
        if kw == "class" && src.is(kw_idx + 1, "inductive") {
            kw_idx += 1;
            kw = "inductive";
        }
        let last = end_tok - 1;
        // Modifiers and attributes.
        let mut attributes: Vec<String> = Vec::new();
        let mut vis = Visibility::Public;
        let mut unsafe_tok: Option<usize> = None;
        {
            let mut i = cmd.first;
            while i < cmd.kw {
                let w = src.t(i);
                if w == "@" && src.is(i + 1, "[") {
                    let mut depth = 0usize;
                    let mut j = i + 1;
                    while j < cmd.kw {
                        if opens(src.t(j)) {
                            depth += 1;
                        } else if closes(src.t(j)) {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        j += 1;
                    }
                    attributes.push(src.join(i, j + 1));
                    i = j + 1;
                    continue;
                }
                match w {
                    "private" => vis = Visibility::Private,
                    "protected" => vis = Visibility::Restricted,
                    "unsafe" => unsafe_tok = Some(i),
                    _ => {}
                }
                if MODIFIERS.contains(&w) {
                    attributes.push(w.to_string());
                }
                i += 1;
            }
        }
        // Name.
        let mut ni = kw_idx + 1;
        if kw == "instance" && src.is(ni, "(") && src.is(ni + 1, "priority") {
            let mut depth = 0usize;
            while ni <= last {
                if opens(src.t(ni)) {
                    depth += 1;
                } else if closes(src.t(ni)) {
                    depth -= 1;
                    if depth == 0 {
                        ni += 1;
                        break;
                    }
                }
                ni += 1;
            }
        }
        // Header end: first `:=`, `where` or `|` outside brackets.
        let mut depth = 0usize;
        let mut hend = end_tok;
        for j in kw_idx + 1..end_tok {
            let w = src.t(j);
            if opens(w) {
                depth += 1;
            } else if closes(w) {
                depth = depth.saturating_sub(1);
            } else if depth == 0
                && (w == ":="
                    || w == "|"
                    || (w == "where" && src.lx.toks[j].kind == TokKind::Ident))
            {
                hend = j;
                break;
            }
        }
        let name_segs: Vec<String>;
        match kw {
            "example" => name_segs = vec!["example".to_string()],
            "instance" if !(src.is_ident(ni) && ni < hend) => {
                // Anonymous: mimic Lean's auto name from the instance head.
                let colon = (kw_idx + 1..hend).find(|j| {
                    src.t(*j) == ":" && {
                        // Depth 0 colon.
                        let mut d = 0i32;
                        for k in kw_idx + 1..*j {
                            if opens(src.t(k)) {
                                d += 1;
                            } else if closes(src.t(k)) {
                                d -= 1;
                            }
                        }
                        d == 0
                    }
                });
                let mut name = String::from("inst");
                if let Some(c) = colon {
                    for j in c + 1..hend {
                        if src.lx.toks[j].kind == TokKind::Ident {
                            for seg in name_segments(src.t(j)) {
                                name.push_str(&seg);
                            }
                        }
                    }
                }
                name_segs = vec![name];
            }
            _ => {
                if !src.is_ident(ni) {
                    return;
                }
                name_segs = name_segments(src.t(ni));
                if name_segs.is_empty() {
                    return;
                }
            }
        }
        let name = name_segs.last().cloned().unwrap_or_default();
        let mut path = scope.to_vec();
        path.extend(name_segs.iter().cloned());

        let signature = src.join(kw_idx, hend);
        let kind = match kw {
            "structure" => SymbolKind::Struct,
            "class" => SymbolKind::Trait,
            "inductive" => SymbolKind::Enum,
            "instance" => SymbolKind::Impl,
            "axiom" | "opaque" => SymbolKind::Const,
            _ => SymbolKind::Function,
        };
        let range = self.tok_range(cmd.first, last);
        let doc = self.leading_doc(cmd.first);
        let hash_of = self.src.join(cmd.first, end_tok);
        let sig = Some(signature);
        let pidx = self.push(
            kind,
            &name,
            path.clone(),
            None,
            vis,
            sig,
            range,
            &hash_of,
            doc,
        );
        let pid = self.symbols[pidx].id.clone();
        self.symbols[pidx].attributes = attributes;
        if kind == SymbolKind::Impl {
            let colon = (kw_idx + 1..hend).find(|j| self.src.t(*j) == ":");
            if let Some(c) = colon {
                let head = if self.src.is_ident(c + 1) {
                    self.src.t(c + 1).to_string()
                } else {
                    String::new()
                };
                self.symbols[pidx].impl_info = Some(ImplInfo {
                    self_ty: self.src.join(c + 2, hend),
                    trait_path: (!head.is_empty()).then_some(head),
                });
            }
        }
        if matches!(kw, "theorem" | "lemma") && !self.symbols[pidx].verifies.is_empty() {
            self.symbols[pidx].is_test = true;
        }

        // Facts.
        let mut facts: Vec<Fact> = Vec::new();
        if let Some(u) = unsafe_tok {
            facts.push(Fact {
                kind: FactKind::UnsafeCode,
                range: self.tok_range(u, u),
                detail: "unsafe".into(),
            });
        }
        let mut mentioned: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        // Mentions go after the other facts, so a bounded list of facts keeps `sorry` first.
        let mut mentions: Vec<Fact> = Vec::new();
        for j in cmd.first..end_tok {
            if self.src.lx.toks[j].kind != TokKind::Ident {
                continue;
            }
            let w = self.src.t(j);
            if matches!(kw, "theorem" | "lemma" | "def" | "abbrev" | "instance")
                && mentioned.len() < 200
                && w.starts_with(|c: char| c.is_alphabetic() || c == '_')
                && mentioned.insert(w.to_string())
            {
                mentions.push(Fact {
                    kind: FactKind::Mention,
                    range: self.tok_range(j, j),
                    detail: bounded(w),
                });
            }
            let (k, d) = match w {
                "sorry" => (FactKind::Unproven, "sorry"),
                "panic!" => (FactKind::PanicSite, "panic!"),
                "unreachable!" => (FactKind::PanicSite, "unreachable!"),
                "unsafe" if j >= cmd.kw => (FactKind::UnsafeCode, "unsafe"),
                _ => continue,
            };
            facts.push(Fact {
                kind: k,
                range: self.tok_range(j, j),
                detail: bounded(d),
            });
        }
        facts.extend(mentions);
        self.symbols[pidx].facts = facts;

        match kind {
            SymbolKind::Struct | SymbolKind::Trait => {
                self.fields(&path, &pid, vis, hend, end_tok);
            }
            SymbolKind::Enum => self.ctors(&path, &pid, vis, hend, end_tok),
            _ => {}
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn member(
        &mut self,
        kind: SymbolKind,
        name: &str,
        parent_path: &[String],
        parent: &SymbolId,
        vis: Visibility,
        sig: String,
        (a, b): (usize, usize),
    ) {
        let mut path = parent_path.to_vec();
        path.push(name.to_string());
        let range = self.tok_range(a, b);
        let doc = self.leading_doc(a);
        let hash_of = self.src.join(a, b + 1);
        self.push(
            kind,
            name,
            path,
            Some(parent.clone()),
            vis,
            Some(sig),
            range,
            &hash_of,
            doc,
        );
    }

    /// Structure / class fields: line-first identifiers (all at one column) followed by
    /// a `:` or binders, after `where`.
    fn fields(
        &mut self,
        path: &[String],
        pid: &SymbolId,
        vis: Visibility,
        hend: usize,
        end_tok: usize,
    ) {
        if !(hend < end_tok && self.src.t(hend) == "where") {
            return;
        }
        // (first token, names, colon/binder index)
        let mut found: Vec<(usize, Vec<String>, usize)> = Vec::new();
        let mut field_col: Option<u32> = None;
        let mut fields_end = end_tok;
        let mut depth = 0usize;
        let mut j = hend + 1;
        while j < end_tok {
            let w = self.src.t(j).to_string();
            if opens(&w) {
                depth += 1;
            } else if closes(&w) {
                depth = depth.saturating_sub(1);
            }
            let tk = &self.src.lx.toks[j];
            // A `deriving` clause ends the fields: the last field's type stops before it.
            if depth == 0 && tk.first_on_line && w == "deriving" {
                fields_end = j;
                break;
            }
            if depth == 0
                && tk.first_on_line
                && tk.kind == TokKind::Ident
                && tk.col > 1
                && !NOT_FIELD.contains(&w.as_str())
                && field_col.is_none_or(|c| c == tk.col)
            {
                let mut k = j;
                let mut names = Vec::new();
                while k < end_tok && self.src.is_ident(k) && !self.src.lx.toks[k].first_on_line
                    || k == j
                {
                    names.extend(name_segments(self.src.t(k)));
                    k += 1;
                }
                let is_ctor = self.src.is(k, ":") && self.src.is(k + 1, ":");
                let ok = k < end_tok
                    && !is_ctor
                    && (self.src.is(k, ":") || matches!(self.src.t(k), "(" | "[" | "{"));
                if ok {
                    field_col = Some(tk.col);
                    found.push((j, names, k));
                }
            }
            j += 1;
        }
        for (n, (start, names, k)) in found.iter().enumerate() {
            let mut stop = found.get(n + 1).map_or(fields_end, |f| f.0);
            // Cut at a `:=` default value outside brackets.
            let mut d = 0usize;
            for q in *k..stop {
                let w = self.src.t(q);
                if opens(w) {
                    d += 1;
                } else if closes(w) {
                    d = d.saturating_sub(1);
                } else if d == 0 && w == ":=" {
                    stop = q;
                    break;
                }
            }
            if stop <= *start {
                continue;
            }
            let rest = self.src.join(*k, stop);
            for name in names {
                self.member(
                    SymbolKind::Field,
                    name,
                    path,
                    pid,
                    vis,
                    format!("{name} {rest}"),
                    (*start, stop - 1),
                );
            }
        }
    }

    /// Inductive constructors: `| name ...` at bracket depth 0.
    fn ctors(
        &mut self,
        path: &[String],
        pid: &SymbolId,
        vis: Visibility,
        hend: usize,
        end_tok: usize,
    ) {
        let mut starts: Vec<usize> = Vec::new();
        let mut depth = 0usize;
        let mut stop = end_tok;
        for j in hend..end_tok {
            let w = self.src.t(j);
            if opens(w) {
                depth += 1;
            } else if closes(w) {
                depth = depth.saturating_sub(1);
            } else if depth == 0 {
                if w == "|" && self.src.is_ident(j + 1) {
                    starts.push(j + 1);
                } else if w == "deriving" && self.src.lx.toks[j].first_on_line {
                    stop = j;
                    break;
                }
            }
        }
        for (n, s) in starts.iter().enumerate() {
            // The constructor ends before the next `|` (one token before its name).
            let e = starts.get(n + 1).map_or(stop, |nx| nx - 1);
            if e <= *s {
                continue;
            }
            let names = name_segments(self.src.t(*s));
            let Some(name) = names.last() else { continue };
            let sig = self.src.join(*s, e);
            self.member(SymbolKind::Variant, name, path, pid, vis, sig, (*s, e - 1));
        }
    }
}

fn clean_comment(c: &Comment) -> String {
    match c.kind {
        CommentKind::Line => c.text.trim().to_string(),
        _ => c
            .text
            .lines()
            .map(|l| l.trim())
            .collect::<Vec<_>>()
            .join("\n")
            .trim()
            .to_string(),
    }
}
