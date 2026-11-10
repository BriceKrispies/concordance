//! One TypeScript/JavaScript file -> `SourceIndex`.

use crate::signature::normalize;
use concordance_core::code::*;
use concordance_core::{ContentHash, FileId, RepoPath, SourceRange, SymbolId};
use oxc_allocator::Allocator;
use oxc_ast::ast::*;
use oxc_ast_visit::{walk, Visit};
use oxc_parser::Parser;
use oxc_span::{GetSpan, SourceType, Span};
use std::collections::HashMap;

const MAX_DETAIL: usize = 200;

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
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
}

/// Module path of a file inside its package: `src/codec/header.ts` -> `["codec","header"]`.
fn module_path(path: &RepoPath, pkg_root: Option<&RepoPath>) -> Vec<String> {
    let rel = match pkg_root {
        Some(r) if path.starts_with_dir(r) => path.as_str()[r.as_str().len()..]
            .trim_start_matches('/')
            .to_string(),
        _ => path.as_str().to_string(),
    };
    let mut segs: Vec<String> = rel.split('/').map(str::to_string).collect();
    if segs.first().is_some_and(|s| s == "src" || s == "lib") && segs.len() > 1 {
        segs.remove(0);
    }
    if let Some(last) = segs.last_mut() {
        let stem = match last.rfind('.') {
            Some(i) => last[..i].to_string(),
            None => last.clone(),
        };
        *last = stem;
    }
    if segs.last().is_some_and(|s| s == "index") {
        segs.pop();
    }
    segs
}

fn source_type(path: &RepoPath) -> SourceType {
    let ext = path.extension().unwrap_or("ts");
    SourceType::from_extension(ext).unwrap_or_else(|_| SourceType::ts())
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
    /// Every enclosing declaration is exported.
    exported: bool,
}

struct Ctx<'t> {
    path: &'t RepoPath,
    text: &'t str,
    lines: Lines,
    file: FileId,
    symbols: Vec<Symbol>,
    /// Byte span of each symbol (same order as `symbols`), for fact attribution.
    spans: Vec<(u32, u32)>,
    imports: Vec<Import>,
    used: HashMap<String, usize>,
}

impl<'t> Ctx<'t> {
    fn src(&self, span: Span) -> &'t str {
        self.text
            .get(span.start as usize..span.end as usize)
            .unwrap_or("")
    }

    fn range(&self, start: u32, end: u32) -> SourceRange {
        let (sl, sc) = self.lines.pos(self.text, start as usize);
        let (el, ec) = self
            .lines
            .pos(self.text, end.saturating_sub(1).max(start) as usize);
        SourceRange {
            path: self.path.clone(),
            start_line: sl,
            start_col: sc,
            end_line: el,
            end_col: ec,
        }
    }

    /// Doc text from the comments directly above byte offset `at`: a `/** ... */`
    /// block and/or consecutive `//` lines, with comment markers removed.
    fn leading_doc(&self, at: u32) -> Option<String> {
        let (line, _) = self.lines.pos(self.text, at as usize);
        let all: Vec<&str> = self.text.lines().collect();
        let mut k = line as usize - 1; // index of the declaration line
        let mut collected: Vec<String> = Vec::new();
        while k > 0 {
            let l = all[k - 1].trim();
            if l.starts_with("//") {
                collected.push(l.trim_start_matches('/').trim().to_string());
                k -= 1;
            } else if l.ends_with("*/") {
                let mut block: Vec<String> = Vec::new();
                let mut j = k - 1;
                loop {
                    let b = all[j].trim();
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
            } else if l.starts_with('@') {
                k -= 1; // decorator line between doc and declaration
            } else {
                break;
            }
        }
        collected.reverse();
        let doc = collected.join("\n").trim().to_string();
        (!doc.is_empty()).then_some(doc)
    }

    #[allow(clippy::too_many_arguments)]
    fn push(
        &mut self,
        kind: SymbolKind,
        name: &str,
        scope: &Scope,
        outer: Span,
        vis: Visibility,
        signature: Option<String>,
        doc_at: u32,
    ) -> SymbolId {
        let mut path = scope.path.clone();
        path.push(name.to_string());
        let key = format!("{}\u{0}{}", kind.as_str(), path.join("::"));
        let n = self.used.entry(key).or_insert(0);
        *n += 1;
        let occ = if *n == 1 {
            String::new()
        } else {
            format!("#{n}")
        };
        let id = SymbolId::from_key(&[kind.as_str(), &path.join("::"), &occ]);
        let doc = self.leading_doc(doc_at);
        let verifies = doc.as_deref().map(verifies_from_doc).unwrap_or_default();
        let body = self.src(outer);
        self.symbols.push(Symbol {
            id: id.clone(),
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
            range: self.range(outer.start, outer.end),
            content_hash: ContentHash::of_str(&normalize(body)),
            impl_info: None,
            returns: None,
            facts: vec![],
            calls: vec![],
            is_test: false,
            verifies,
        });
        self.spans.push((outer.start, outer.end));
        id
    }

    fn ret_text(&self, rt: Option<&TSTypeAnnotation<'_>>) -> String {
        rt.map(|r| {
            format!(
                ": {}",
                self.src(r.span).trim().trim_start_matches(':').trim()
            )
        })
        .unwrap_or_default()
    }

    fn tp_text(&self, tp: Option<&TSTypeParameterDeclaration<'_>>) -> String {
        tp.map(|t| self.src(t.span).to_string()).unwrap_or_default()
    }

    fn set_returns(&mut self, id: &SymbolId, rt: Option<&TSTypeAnnotation<'_>>) {
        let Some(r) = rt else { return };
        let text = normalize(self.src(r.span).trim().trim_start_matches(':'));
        let is_option = text.contains("undefined") || text.contains("null");
        if let Some(s) = self.symbols.iter_mut().rev().find(|s| &s.id == id) {
            s.returns = Some(ReturnContract {
                text,
                result: None,
                is_option,
            });
        }
    }

    fn function(&mut self, f: &Function<'_>, scope: &Scope, exported: bool, outer: Span) {
        let Some(id) = &f.id else { return };
        let name = id.name.as_str();
        let sig = format!(
            "{}function {}{}{}{}",
            if f.r#async { "async " } else { "" },
            name,
            self.tp_text(f.type_parameters.as_deref()),
            self.src(f.params.span),
            self.ret_text(f.return_type.as_deref())
        );
        let vis = if exported {
            Visibility::Public
        } else {
            Visibility::Private
        };
        let sid = self.push(
            SymbolKind::Function,
            name,
            scope,
            outer,
            vis,
            Some(normalize(&sig)),
            outer.start,
        );
        self.set_returns(&sid, f.return_type.as_deref());
    }

    fn class(&mut self, c: &Class<'_>, scope: &Scope, exported: bool, outer: Span) {
        let Some(id) = &c.id else { return };
        let name = id.name.as_str();
        let header = self
            .text
            .get(c.span.start as usize..c.body.span.start as usize)
            .unwrap_or("");
        let vis = if exported {
            Visibility::Public
        } else {
            Visibility::Private
        };
        let cid = self.push(
            SymbolKind::Class,
            name,
            scope,
            outer,
            vis,
            Some(normalize(header)),
            outer.start,
        );
        let inner = Scope {
            path: {
                let mut p = scope.path.clone();
                p.push(name.to_string());
                p
            },
            parent: Some(cid),
            exported: scope.exported && exported,
        };
        for el in &c.body.body {
            match el {
                ClassElement::MethodDefinition(m) => {
                    let Some(mname) = m.key.static_name() else {
                        continue;
                    };
                    let private = matches!(
                        m.accessibility,
                        Some(TSAccessibility::Private) | Some(TSAccessibility::Protected)
                    ) || matches!(m.key, PropertyKey::PrivateIdentifier(_));
                    let vis = if private {
                        Visibility::Private
                    } else {
                        Visibility::Public
                    };
                    let f = &m.value;
                    let (kind, sig) = match m.kind {
                        MethodDefinitionKind::Constructor => (
                            SymbolKind::Constructor,
                            format!("constructor{}", self.src(f.params.span)),
                        ),
                        k => {
                            let mut mods = String::new();
                            if m.r#static {
                                mods.push_str("static ");
                            }
                            if f.r#async {
                                mods.push_str("async ");
                            }
                            match k {
                                MethodDefinitionKind::Get => mods.push_str("get "),
                                MethodDefinitionKind::Set => mods.push_str("set "),
                                _ => {}
                            }
                            (
                                SymbolKind::Method,
                                format!(
                                    "{mods}{mname}{}{}{}",
                                    self.tp_text(f.type_parameters.as_deref()),
                                    self.src(f.params.span),
                                    self.ret_text(f.return_type.as_deref())
                                ),
                            )
                        }
                    };
                    let sid = self.push(
                        kind,
                        &mname,
                        &inner,
                        m.span,
                        vis,
                        Some(normalize(&sig)),
                        m.span.start,
                    );
                    self.set_returns(&sid, f.return_type.as_deref());
                }
                ClassElement::PropertyDefinition(p) => {
                    let Some(pname) = p.key.static_name() else {
                        continue;
                    };
                    let private = matches!(
                        p.accessibility,
                        Some(TSAccessibility::Private) | Some(TSAccessibility::Protected)
                    ) || matches!(p.key, PropertyKey::PrivateIdentifier(_));
                    let vis = if private {
                        Visibility::Private
                    } else {
                        Visibility::Public
                    };
                    let sig = format!("{pname}{}", self.ret_text(p.type_annotation.as_deref()));
                    self.push(
                        SymbolKind::Property,
                        &pname,
                        &inner,
                        p.span,
                        vis,
                        Some(normalize(&sig)),
                        p.span.start,
                    );
                }
                _ => {}
            }
        }
    }

    fn interface(
        &mut self,
        i: &TSInterfaceDeclaration<'_>,
        scope: &Scope,
        exported: bool,
        outer: Span,
    ) {
        let name = i.id.name.as_str();
        let header = self
            .text
            .get(i.span.start as usize..i.body.span.start as usize)
            .unwrap_or("");
        let vis = if exported {
            Visibility::Public
        } else {
            Visibility::Private
        };
        let iid = self.push(
            SymbolKind::Interface,
            name,
            scope,
            outer,
            vis,
            Some(normalize(header)),
            outer.start,
        );
        let inner = Scope {
            path: {
                let mut p = scope.path.clone();
                p.push(name.to_string());
                p
            },
            parent: Some(iid),
            exported: scope.exported && exported,
        };
        for sig in &i.body.body {
            match sig {
                TSSignature::TSMethodSignature(m) => {
                    let Some(mname) = m.key.static_name() else {
                        continue;
                    };
                    let s = format!(
                        "{mname}{}{}{}{}",
                        if m.optional { "?" } else { "" },
                        self.tp_text(m.type_parameters.as_deref()),
                        self.src(m.params.span),
                        self.ret_text(m.return_type.as_deref())
                    );
                    self.push(
                        SymbolKind::Method,
                        &mname,
                        &inner,
                        m.span,
                        Visibility::Public,
                        Some(normalize(&s)),
                        m.span.start,
                    );
                }
                TSSignature::TSPropertySignature(p) => {
                    let Some(pname) = p.key.static_name() else {
                        continue;
                    };
                    let s = format!(
                        "{pname}{}{}",
                        if p.optional { "?" } else { "" },
                        self.ret_text(p.type_annotation.as_deref())
                    );
                    self.push(
                        SymbolKind::Property,
                        &pname,
                        &inner,
                        p.span,
                        Visibility::Public,
                        Some(normalize(&s)),
                        p.span.start,
                    );
                }
                _ => {}
            }
        }
    }

    fn var_decl(
        &mut self,
        v: &VariableDeclaration<'_>,
        scope: &Scope,
        exported: bool,
        outer: Span,
    ) {
        let single = v.declarations.len() == 1;
        for d in &v.declarations {
            let Some(name) = d.id.get_identifier_name() else {
                continue;
            };
            let name = name.as_str();
            let vis = if exported {
                Visibility::Public
            } else {
                Visibility::Private
            };
            let span = if single { outer } else { d.span };
            match &d.init {
                Some(Expression::ArrowFunctionExpression(a)) => {
                    let sig = format!(
                        "{}function {}{}{}{}",
                        if a.r#async { "async " } else { "" },
                        name,
                        self.tp_text(a.type_parameters.as_deref()),
                        self.src(a.params.span),
                        self.ret_text(a.return_type.as_deref())
                    );
                    let sid = self.push(
                        SymbolKind::Function,
                        name,
                        scope,
                        span,
                        vis,
                        Some(normalize(&sig)),
                        span.start,
                    );
                    self.set_returns(&sid, a.return_type.as_deref());
                }
                Some(Expression::FunctionExpression(f)) => {
                    let sig = format!(
                        "{}function {}{}{}{}",
                        if f.r#async { "async " } else { "" },
                        name,
                        self.tp_text(f.type_parameters.as_deref()),
                        self.src(f.params.span),
                        self.ret_text(f.return_type.as_deref())
                    );
                    self.push(
                        SymbolKind::Function,
                        name,
                        scope,
                        span,
                        vis,
                        Some(normalize(&sig)),
                        span.start,
                    );
                }
                init => {
                    if v.kind != VariableDeclarationKind::Const {
                        continue;
                    }
                    let sig = format!(
                        "const {name}{}",
                        self.ret_text(d.type_annotation.as_deref())
                    );
                    let sid = self.push(
                        SymbolKind::Const,
                        name,
                        scope,
                        span,
                        vis,
                        Some(normalize(&sig)),
                        span.start,
                    );
                    if let Some(e) = init {
                        let value = self.src(e.span()).trim().to_string();
                        let numeric = value
                            .trim_start_matches('-')
                            .replace('_', "")
                            .parse::<f64>()
                            .is_ok();
                        if numeric {
                            let range = self.range(d.span.start, d.span.end);
                            if let Some(s) = self.symbols.iter_mut().rev().find(|s| s.id == sid) {
                                s.facts.push(Fact {
                                    kind: FactKind::ConstValue,
                                    range,
                                    detail: format!("{name} = {value}"),
                                });
                            }
                        }
                    }
                }
            }
        }
    }

    fn decl(&mut self, d: &Declaration<'_>, scope: &Scope, exported: bool, outer: Span) {
        match d {
            Declaration::FunctionDeclaration(f) => self.function(f, scope, exported, outer),
            Declaration::ClassDeclaration(c) => self.class(c, scope, exported, outer),
            Declaration::TSInterfaceDeclaration(i) => self.interface(i, scope, exported, outer),
            Declaration::VariableDeclaration(v) => self.var_decl(v, scope, exported, outer),
            Declaration::TSTypeAliasDeclaration(t) => {
                let vis = if exported {
                    Visibility::Public
                } else {
                    Visibility::Private
                };
                let sig = normalize(self.src(t.span));
                self.push(
                    SymbolKind::TypeAlias,
                    t.id.name.as_str(),
                    scope,
                    outer,
                    vis,
                    Some(sig),
                    outer.start,
                );
            }
            Declaration::TSEnumDeclaration(e) => {
                let vis = if exported {
                    Visibility::Public
                } else {
                    Visibility::Private
                };
                let header = self
                    .text
                    .get(e.span.start as usize..e.body.span.start as usize)
                    .unwrap_or("");
                let name = e.id.name.as_str();
                let eid = self.push(
                    SymbolKind::Enum,
                    name,
                    scope,
                    outer,
                    vis,
                    Some(normalize(header)),
                    outer.start,
                );
                let inner = Scope {
                    path: {
                        let mut p = scope.path.clone();
                        p.push(name.to_string());
                        p
                    },
                    parent: Some(eid),
                    exported: scope.exported && exported,
                };
                for m in &e.body.members {
                    let mname = match &m.id {
                        TSEnumMemberName::Identifier(i) => i.name.as_str().to_string(),
                        other => self
                            .src(other.span())
                            .trim_matches(['"', '\'', '`'])
                            .to_string(),
                    };
                    self.push(
                        SymbolKind::Variant,
                        &mname,
                        &inner,
                        m.span,
                        Visibility::Public,
                        None,
                        m.span.start,
                    );
                }
            }
            Declaration::TSNamespaceDeclaration(n) => {
                let vis = if exported {
                    Visibility::Public
                } else {
                    Visibility::Private
                };
                let name = n.id.name.as_str();
                let nid = self.push(
                    SymbolKind::Namespace,
                    name,
                    scope,
                    outer,
                    vis,
                    None,
                    outer.start,
                );
                let inner = Scope {
                    path: {
                        let mut p = scope.path.clone();
                        p.push(name.to_string());
                        p
                    },
                    parent: Some(nid),
                    exported: scope.exported && exported,
                };
                self.namespace_body(&n.body, &inner);
            }
            _ => {}
        }
    }

    fn namespace_body(&mut self, body: &TSNamespaceDeclarationBody<'_>, scope: &Scope) {
        match body {
            TSNamespaceDeclarationBody::TSModuleBlock(b) => self.statements(&b.body, scope),
            TSNamespaceDeclarationBody::TSNamespaceDeclaration(n) => {
                self.decl_namespace(n, scope);
            }
        }
    }

    fn decl_namespace(&mut self, n: &TSNamespaceDeclaration<'_>, scope: &Scope) {
        let name = n.id.name.as_str();
        let nid = self.push(
            SymbolKind::Namespace,
            name,
            scope,
            n.span,
            Visibility::Public,
            None,
            n.span.start,
        );
        let inner = Scope {
            path: {
                let mut p = scope.path.clone();
                p.push(name.to_string());
                p
            },
            parent: Some(nid),
            exported: scope.exported,
        };
        self.namespace_body(&n.body, &inner);
    }

    fn statements(&mut self, stmts: &[Statement<'_>], scope: &Scope) {
        for st in stmts {
            match st {
                Statement::ExportDeclaration(e) => self.decl(&e.declaration, scope, true, e.span),
                Statement::ExportDefaultDeclaration(e) => match &e.declaration {
                    ExportDefaultDeclarationKind::FunctionDeclaration(f) => {
                        self.function(f, scope, true, e.span)
                    }
                    ExportDefaultDeclarationKind::ClassDeclaration(c) => {
                        self.class(c, scope, true, e.span)
                    }
                    ExportDefaultDeclarationKind::TSInterfaceDeclaration(i) => {
                        self.interface(i, scope, true, e.span)
                    }
                    _ => {}
                },
                Statement::ImportDeclaration(i) => self.import(i),
                other => {
                    if let Some(d) = other.as_declaration() {
                        self.decl(d, scope, false, d.span());
                    }
                }
            }
        }
    }

    fn import(&mut self, i: &ImportDeclaration<'_>) {
        let from = self.src(i.source.span).trim_matches(['"', '\'']);
        let mut base: Vec<String> = from
            .split('/')
            .filter(|s| !s.is_empty() && *s != "." && *s != "..")
            .map(|s| {
                s.trim_end_matches(".js")
                    .trim_end_matches(".ts")
                    .to_string()
            })
            .collect();
        if from.starts_with('@') && base.len() >= 2 {
            // `@scope/pkg/sub` -> package `pkg`
            base.remove(0);
        }
        if let Some(first) = base.first_mut() {
            if !from.starts_with('.') {
                *first = crate::workspace::normalize_name(first);
            }
        }
        let range = self.range(i.span.start, i.span.end);
        match &i.specifiers {
            None => self.imports.push(Import {
                path: base,
                alias: None,
                glob: true,
                range,
            }),
            Some(specs) => {
                for s in specs {
                    match s {
                        ImportDeclarationSpecifier::ImportSpecifier(sp) => {
                            let imported = self
                                .src(sp.imported.span())
                                .trim_matches(['"', '\''])
                                .to_string();
                            let local = sp.local.name.as_str().to_string();
                            let mut p = base.clone();
                            p.push(imported.clone());
                            self.imports.push(Import {
                                path: p,
                                alias: (local != imported).then_some(local),
                                glob: false,
                                range: range.clone(),
                            });
                        }
                        ImportDeclarationSpecifier::ImportDefaultSpecifier(sp) => {
                            self.imports.push(Import {
                                path: base.clone(),
                                alias: Some(sp.local.name.as_str().to_string()),
                                glob: false,
                                range: range.clone(),
                            });
                        }
                        ImportDeclarationSpecifier::ImportNamespaceSpecifier(sp) => {
                            self.imports.push(Import {
                                path: base.clone(),
                                alias: Some(sp.local.name.as_str().to_string()),
                                glob: true,
                                range: range.clone(),
                            });
                        }
                    }
                }
            }
        }
    }
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
    span: Span,
    detail: String,
}

struct RawCall {
    target: CallTarget,
    span: Span,
}

struct RawTest {
    describes: Vec<String>,
    title: String,
    span: Span,
}

struct Collector<'t> {
    text: &'t str,
    facts: Vec<RawFact>,
    calls: Vec<RawCall>,
    tests: Vec<RawTest>,
    describes: Vec<String>,
}

impl<'t> Collector<'t> {
    fn src(&self, span: Span) -> &'t str {
        self.text
            .get(span.start as usize..span.end as usize)
            .unwrap_or("")
    }

    fn title(&self, args: &[Argument<'_>]) -> Option<String> {
        let first = args.first()?;
        let s = self.src(first.span()).trim();
        let q = s.chars().next()?;
        (matches!(q, '"' | '\'' | '`') && s.len() >= 2 && s.ends_with(q) && !s.contains("${"))
            .then(|| s[1..s.len() - 1].to_string())
    }
}

impl<'a> Visit<'a> for Collector<'_> {
    fn visit_throw_statement(&mut self, it: &ThrowStatement<'a>) {
        let detail = bounded(&normalize(self.src(it.span)));
        self.facts.push(RawFact {
            kind: FactKind::PanicSite,
            span: it.span,
            detail,
        });
        walk::walk_throw_statement(self, it);
    }

    fn visit_ts_any_keyword(&mut self, it: &TSAnyKeyword) {
        self.facts.push(RawFact {
            kind: FactKind::UnsafeCode,
            span: it.span,
            detail: "any".into(),
        });
    }

    fn visit_static_member_expression(&mut self, it: &StaticMemberExpression<'a>) {
        let s = self.src(it.span);
        if s.starts_with("process.env.") {
            self.facts.push(RawFact {
                kind: FactKind::EnvRead,
                span: it.span,
                detail: bounded(s),
            });
        }
        walk::walk_static_member_expression(self, it);
    }

    fn visit_computed_member_expression(&mut self, it: &ComputedMemberExpression<'a>) {
        let s = self.src(it.span);
        if s.starts_with("process.env[") {
            self.facts.push(RawFact {
                kind: FactKind::EnvRead,
                span: it.span,
                detail: bounded(s),
            });
        }
        walk::walk_computed_member_expression(self, it);
    }

    fn visit_binary_expression(&mut self, it: &BinaryExpression<'a>) {
        let lit = |e: &Expression<'_>| matches!(e, Expression::NumericLiteral(_));
        if it.operator.is_compare() && (lit(&it.left) || lit(&it.right)) {
            self.facts.push(RawFact {
                kind: FactKind::LiteralComparison,
                span: it.span,
                detail: bounded(&normalize(self.src(it.span))),
            });
        }
        walk::walk_binary_expression(self, it);
    }

    fn visit_call_expression(&mut self, it: &CallExpression<'a>) {
        let callee = self.src(it.callee.span()).trim();
        let root = callee.split(['.', '(']).next().unwrap_or("");
        // Test structure.
        if matches!(root, "describe" | "suite" | "context") {
            if let Some(t) = self.title(&it.arguments) {
                self.describes.push(t);
                walk::walk_call_expression(self, it);
                self.describes.pop();
                return;
            }
        }
        if matches!(root, "it" | "test") && !callee.contains(".each") {
            if let Some(t) = self.title(&it.arguments) {
                self.tests.push(RawTest {
                    describes: self.describes.clone(),
                    title: t,
                    span: it.span,
                });
            }
        }
        // Clamps.
        if matches!(callee, "Math.min" | "Math.max")
            && it
                .arguments
                .iter()
                .any(|a| matches!(a, Argument::NumericLiteral(_)))
        {
            self.facts.push(RawFact {
                kind: FactKind::Clamp,
                span: it.span,
                detail: bounded(&normalize(self.src(it.span))),
            });
        }
        // Call edges.
        let target = match &it.callee {
            Expression::Identifier(id) => Some(CallTarget::Path {
                segments: vec![id.name.as_str().to_string()],
            }),
            Expression::StaticMemberExpression(m) => {
                let obj = self.src(m.object.span()).trim();
                let prop = m.property.name.as_str().to_string();
                if is_pascal(obj) {
                    Some(CallTarget::Path {
                        segments: vec![obj.to_string(), prop],
                    })
                } else {
                    Some(CallTarget::Method { name: prop })
                }
            }
            _ => None,
        };
        if let Some(target) = target {
            self.calls.push(RawCall {
                target,
                span: it.span,
            });
        }
        walk::walk_call_expression(self, it);
    }
}

// ------------------------------------------------------------------ entry points

fn header(path: &RepoPath, text: &str, ws: &WorkspaceInfo) -> (SourceFile, Option<RepoPath>) {
    let pkg = ws
        .crates
        .iter()
        .filter(|c| c.language == "typescript")
        .filter(|c| c.root_dir.as_ref().is_none_or(|d| path.starts_with_dir(d)))
        .max_by_key(|c| c.root_dir.as_ref().map_or(0, |d| d.as_str().len()));
    let root = pkg.and_then(|p| p.root_dir.clone());
    let file = SourceFile {
        id: FileId::from_key(&[path.as_str()]),
        path: path.clone(),
        language: "typescript".into(),
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

/// Index a documented code block (no package, no module path).
pub fn index_snippet(path: &RepoPath, text: &str) -> SourceIndex {
    let mut idx = index(path, text, &WorkspaceInfo::default());
    // Snippet symbols are addressed by their own names only.
    for s in &mut idx.symbols {
        let drop = idx.file.module_path.len();
        if s.path.len() > drop {
            s.path.drain(..drop);
        }
    }
    idx
}

pub fn index(path: &RepoPath, text: &str, ws: &WorkspaceInfo) -> SourceIndex {
    let (file, _root) = header(path, text, ws);
    let allocator = Allocator::default();
    let ret = Parser::new(&allocator, text, source_type(path)).parse();
    if ret.fatal_error {
        let msg = ret
            .diagnostics
            .first()
            .map(|e| e.to_string())
            .unwrap_or_else(|| "parse failed".into());
        return failed(path, text, ws, msg);
    }
    let mut base: Vec<String> = file.crate_name.iter().cloned().collect();
    base.extend(file.module_path.iter().cloned());
    let mut ctx = Ctx {
        path,
        text,
        lines: Lines::new(text),
        file: file.id.clone(),
        symbols: vec![],
        spans: vec![],
        imports: vec![],
        used: HashMap::new(),
    };
    let top = Scope {
        path: base.clone(),
        parent: None,
        exported: true,
    };
    ctx.statements(&ret.program.body, &top);

    let mut col = Collector {
        text,
        facts: vec![],
        calls: vec![],
        tests: vec![],
        describes: vec![],
    };
    col.visit_program(&ret.program);

    // Tests become symbols (innermost scope: the file).
    for t in &col.tests {
        let mut scope = top.clone();
        scope.path.extend(t.describes.iter().cloned());
        let id = ctx.push(
            SymbolKind::Function,
            &t.title,
            &scope,
            t.span,
            Visibility::Private,
            None,
            t.span.start,
        );
        if let Some(s) = ctx.symbols.iter_mut().rev().find(|s| s.id == id) {
            s.is_test = true;
        }
    }
    // `@ts-ignore` / `@ts-expect-error` comments.
    for c in ret.program.comments.iter() {
        let s = ctx.src(c.span);
        if s.contains("@ts-ignore") || s.contains("@ts-expect-error") || s.contains("@ts-nocheck") {
            col.facts.push(RawFact {
                kind: FactKind::UnsafeCode,
                span: c.span,
                detail: bounded(s.trim()),
            });
        }
    }
    // Attribute facts and calls to the innermost containing symbol.
    let innermost = |spans: &[(u32, u32)], at: Span| -> Option<usize> {
        spans
            .iter()
            .enumerate()
            .filter(|(_, (s, e))| *s <= at.start && at.end <= *e)
            .min_by_key(|(_, (s, e))| e - s)
            .map(|(i, _)| i)
    };
    for f in col.facts {
        if let Some(i) = innermost(&ctx.spans, f.span) {
            let range = ctx.range(f.span.start, f.span.end);
            ctx.symbols[i].facts.push(Fact {
                kind: f.kind,
                range,
                detail: f.detail,
            });
        }
    }
    for c in col.calls {
        if let Some(i) = innermost(&ctx.spans, c.span) {
            let range = ctx.range(c.span.start, c.span.end);
            ctx.symbols[i].calls.push(CallSite {
                target: c.target,
                range,
            });
        }
    }
    let parse_error = ret.diagnostics.first().map(|e| e.to_string());
    let mut file = file;
    if ctx.symbols.is_empty() {
        file.parse_error = parse_error;
    }
    SourceIndex {
        file,
        symbols: ctx.symbols,
        imports: ctx.imports,
    }
}
