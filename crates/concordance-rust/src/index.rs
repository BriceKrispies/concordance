//! `syn` AST -> [`SourceIndex`]: symbols, signatures, facts, calls and imports.
//!
//! Approximations (all syntactic, no name resolution):
//! * Symbol ranges include outer attributes and doc comments.
//! * File modules (`src/a.rs`) cannot see their `mod a;` declaration, so they are treated
//!   as `pub` when computing `is_public_api`.
//! * Items of a trait impl inherit the visibility of the self type when it is declared in
//!   the same file; otherwise they are assumed `Public`.

#![allow(clippy::needless_lifetimes)]

use crate::facts::{has_word, mk_fact, scan_block, scan_expr};
use crate::modpath::module_path;
use crate::signature::{fn_header, ident_name, item_header, normalize_tokens, tokens_string};
use crate::spans::Locator;
use concordance_core::code::*;
use concordance_core::{ContentHash, FileId, RepoPath, SymbolId};
use proc_macro2::TokenStream;
use quote::{quote, ToTokens};
use std::collections::{HashMap, HashSet};
use syn::{Attribute, Block, Expr, ImplItem, Item, Lit, Signature, TraitItem, Type, UseTree};

/// Header `SourceFile` for a file; shared by the success and failure paths.
fn file_header(path: &RepoPath, text: &str, ws: &WorkspaceInfo) -> SourceFile {
    let krate = ws.crate_for(path);
    SourceFile {
        id: FileId::from_key(&[path.as_str()]),
        path: path.clone(),
        language: "rust".to_string(),
        crate_name: krate.map(|c| c.name.clone()),
        module_path: module_path(path, krate),
        content_hash: ContentHash::of_str(text),
        line_count: text.lines().count() as u32,
        parse_error: None,
    }
}

/// Index result for a file that could not be parsed (or indexed).
pub(crate) fn failed(path: &RepoPath, text: &str, ws: &WorkspaceInfo, msg: String) -> SourceIndex {
    let mut file = file_header(path, text, ws);
    file.parse_error = Some(msg);
    SourceIndex {
        file,
        symbols: vec![],
        imports: vec![],
    }
}

pub(crate) fn index(path: &RepoPath, text: &str, ws: &WorkspaceInfo) -> SourceIndex {
    let parsed = match syn::parse_file(text) {
        Ok(f) => f,
        Err(e) => {
            let line = e.span().start().line;
            return failed(path, text, ws, format!("line {line}: {e}"));
        }
    };
    let file = file_header(path, text, ws);
    let mut base = vec![file
        .crate_name
        .clone()
        .unwrap_or_else(|| "crate".to_string())];
    base.extend(file.module_path.iter().cloned());
    let mut w = Walker {
        loc: Locator::new(path),
        file: file.id.clone(),
        symbols: vec![],
        imports: vec![],
        used: HashSet::new(),
        type_vis: HashMap::new(),
    };
    w.collect_type_vis(&parsed.items);
    let sc = Scope {
        path: base,
        parent: None,
        cfg: vec![],
        api: true,
        inherit: None,
        trait_key: String::new(),
        impl_info: None,
    };
    w.items(&parsed.items, &sc);
    SourceIndex {
        file,
        symbols: w.symbols,
        imports: w.imports,
    }
}

#[derive(Clone)]
struct Scope {
    /// Prefix for the paths of items declared directly in this scope.
    path: Vec<String>,
    parent: Option<SymbolId>,
    cfg: Vec<String>,
    /// Every enclosing module/type is public.
    api: bool,
    /// Forced visibility for items (trait items, trait-impl items, variants).
    inherit: Option<Visibility>,
    /// Trait path for trait-impl items (id disambiguator).
    trait_key: String,
    impl_info: Option<ImplInfo>,
}

struct Spec<'s> {
    kind: SymbolKind,
    name: String,
    path: Vec<String>,
    vis: Visibility,
    api: bool,
    signature: Option<String>,
    attrs: &'s [Attribute],
    tokens: TokenStream,
    returns: Option<ReturnContract>,
    facts: Vec<Fact>,
    calls: Vec<CallSite>,
    impl_info: Option<ImplInfo>,
    trait_key: String,
}

struct Walker {
    loc: Locator,
    file: FileId,
    symbols: Vec<Symbol>,
    imports: Vec<Import>,
    used: HashSet<String>,
    type_vis: HashMap<String, Visibility>,
}

fn kind_str(k: SymbolKind) -> &'static str {
    k.as_str()
}

fn conv_vis(v: &syn::Visibility) -> Visibility {
    match v {
        syn::Visibility::Public(_) => Visibility::Public,
        syn::Visibility::Inherited => Visibility::Private,
        syn::Visibility::Restricted(r) => {
            if r.in_token.is_none() && r.path.is_ident("crate") {
                Visibility::Crate
            } else if r.in_token.is_none() && r.path.is_ident("self") {
                Visibility::Private
            } else {
                Visibility::Restricted
            }
        }
    }
}

fn type_last(ty: &Type) -> String {
    match ty {
        Type::Path(p) => p
            .path
            .segments
            .last()
            .map_or_else(|| tokens_string(ty), |s| ident_name(&s.ident)),
        Type::Reference(r) => type_last(&r.elem),
        Type::Paren(p) => type_last(&p.elem),
        Type::Group(g) => type_last(&g.elem),
        _ => tokens_string(ty),
    }
}

struct AttrInfo {
    doc: Option<String>,
    attributes: Vec<String>,
    cfgs: Vec<String>,
    gates: Vec<Fact>,
    is_test: bool,
}

/// Requirement IDs declared by `Verifies: ID, ID` lines of a doc comment.
pub fn verifies_from_doc(doc: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in doc.lines() {
        let l = line.trim().trim_start_matches(['*', '-']).trim_start();
        let lower = l.to_ascii_lowercase();
        let rest = if lower.starts_with("verifies:") {
            &l["verifies:".len()..]
        } else if lower.starts_with("verifies ") {
            &l["verifies ".len()..]
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

fn attr_info(loc: &Locator, attrs: &[Attribute]) -> AttrInfo {
    let mut docs: Vec<String> = vec![];
    let mut info = AttrInfo {
        doc: None,
        attributes: vec![],
        cfgs: vec![],
        gates: vec![],
        is_test: false,
    };
    for a in attrs {
        if !matches!(a.style, syn::AttrStyle::Outer) {
            continue;
        }
        if a.path().is_ident("doc") {
            if let syn::Meta::NameValue(nv) = &a.meta {
                if let Expr::Lit(syn::ExprLit {
                    lit: Lit::Str(s), ..
                }) = &nv.value
                {
                    let v = s.value();
                    docs.push(v.strip_prefix(' ').unwrap_or(&v).to_string());
                    continue;
                }
            }
        } else if a.path().is_ident("cfg") {
            if let syn::Meta::List(l) = &a.meta {
                let pred = normalize_tokens(&l.tokens.to_string());
                if has_word(&pred, "feature") {
                    info.gates
                        .push(mk_fact(FactKind::FeatureGate, loc.of(a), pred.clone()));
                }
                info.cfgs.push(pred);
                continue;
            }
        }
        if a.path()
            .segments
            .last()
            .is_some_and(|s| s.ident == "test" || s.ident == "rstest")
        {
            info.is_test = true;
        }
        info.attributes.push(tokens_string(a));
    }
    if !docs.is_empty() {
        info.doc = Some(docs.join("\n"));
    }
    info
}

fn return_contract(sig: &Signature) -> Option<ReturnContract> {
    let syn::ReturnType::Type(_, ty) = &sig.output else {
        return None;
    };
    let text = tokens_string(&**ty);
    let mut result = None;
    let mut is_option = false;
    if let Type::Path(p) = &**ty {
        if let Some(last) = p.path.segments.last() {
            let name = last.ident.to_string();
            let args: Vec<String> = match &last.arguments {
                syn::PathArguments::AngleBracketed(a) => a
                    .args
                    .iter()
                    .filter_map(|g| match g {
                        syn::GenericArgument::Type(t) => Some(tokens_string(t)),
                        _ => None,
                    })
                    .collect(),
                _ => vec![],
            };
            if name == "Result" {
                match args.len() {
                    2 => result = Some((args[0].clone(), args[1].clone())),
                    1 => result = Some((args[0].clone(), "?".to_string())),
                    _ => {}
                }
            } else if name == "Option" {
                is_option = true;
            }
        }
    }
    Some(ReturnContract {
        text,
        result,
        is_option,
    })
}

/// `const X: T = <literal>`: the initializer is a (possibly negated / cast) literal.
fn is_literalish(e: &Expr) -> bool {
    match e {
        Expr::Lit(_) => true,
        Expr::Unary(u) => matches!(u.op, syn::UnOp::Neg(_)) && is_literalish(&u.expr),
        Expr::Paren(p) => is_literalish(&p.expr),
        Expr::Group(g) => is_literalish(&g.expr),
        Expr::Cast(c) => is_literalish(&c.expr),
        _ => false,
    }
}

#[allow(clippy::type_complexity)]
fn flatten_use(
    tree: &UseTree,
    prefix: &mut Vec<String>,
    out: &mut Vec<(
        Vec<String>,
        Option<String>,
        bool,
        proc_macro2::Span,
        proc_macro2::Span,
    )>,
) {
    match tree {
        UseTree::Path(p) => {
            prefix.push(ident_name(&p.ident));
            flatten_use(&p.tree, prefix, out);
            prefix.pop();
        }
        UseTree::Name(n) => {
            let mut path = prefix.clone();
            let name = ident_name(&n.ident);
            if name != "self" {
                path.push(name);
            }
            out.push((path, None, false, n.ident.span(), n.ident.span()));
        }
        UseTree::Rename(r) => {
            let mut path = prefix.clone();
            let name = ident_name(&r.ident);
            if name != "self" {
                path.push(name);
            }
            out.push((
                path,
                Some(ident_name(&r.rename)),
                false,
                r.ident.span(),
                r.rename.span(),
            ));
        }
        UseTree::Glob(g) => out.push((
            prefix.clone(),
            None,
            true,
            g.star_token.spans[0],
            g.star_token.spans[0],
        )),
        UseTree::Group(g) => {
            for t in &g.items {
                flatten_use(t, prefix, out);
            }
        }
    }
}

impl Walker {
    fn collect_type_vis(&mut self, items: &[Item]) {
        for it in items {
            let (name, vis) = match it {
                Item::Struct(s) => (ident_name(&s.ident), &s.vis),
                Item::Enum(s) => (ident_name(&s.ident), &s.vis),
                Item::Union(s) => (ident_name(&s.ident), &s.vis),
                Item::Type(s) => (ident_name(&s.ident), &s.vis),
                Item::Trait(s) => (ident_name(&s.ident), &s.vis),
                Item::Mod(m) => {
                    if let Some((_, inner)) = &m.content {
                        self.collect_type_vis(inner);
                    }
                    continue;
                }
                _ => continue,
            };
            self.type_vis.insert(name, conv_vis(vis));
        }
    }

    fn make_id(
        &mut self,
        kind: SymbolKind,
        path: &[String],
        trait_key: &str,
        cfg: &[String],
    ) -> SymbolId {
        let k = kind_str(kind);
        let p = path.join("::");
        let first = format!("{k}\u{0}{p}\u{0}{trait_key}");
        if self.used.insert(first) {
            return SymbolId::from_key(&[k, &p, trait_key]);
        }
        if !cfg.is_empty() {
            let d = format!("{trait_key}|cfg:{}", cfg.join(" && "));
            if self.used.insert(format!("{k}\u{0}{p}\u{0}{d}")) {
                return SymbolId::from_key(&[k, &p, &d]);
            }
        }
        let mut n = 2u32;
        loop {
            let d = format!("{trait_key}|cfg:{}|#{n}", cfg.join(" && "));
            if self.used.insert(format!("{k}\u{0}{p}\u{0}{d}")) {
                return SymbolId::from_key(&[k, &p, &d]);
            }
            n += 1;
        }
    }

    fn vis(&self, sc: &Scope, v: &syn::Visibility) -> Visibility {
        sc.inherit.unwrap_or_else(|| conv_vis(v))
    }

    /// Record a symbol; returns its id and the cfg list that applies to its children.
    fn add(&mut self, sc: &Scope, s: Spec<'_>) -> (SymbolId, Vec<String>) {
        let info = attr_info(&self.loc, s.attrs);
        let mut cfg = sc.cfg.clone();
        cfg.extend(info.cfgs);
        let range = self.loc.of_tokens(s.tokens.clone());
        let content_hash = ContentHash::of_str(&normalize_tokens(&s.tokens.to_string()));
        let id = self.make_id(s.kind, &s.path, &s.trait_key, &cfg);
        let mut facts = info.gates;
        facts.extend(s.facts);
        let verifies = info
            .doc
            .as_deref()
            .map(verifies_from_doc)
            .unwrap_or_default();
        self.symbols.push(Symbol {
            id: id.clone(),
            file: self.file.clone(),
            kind: s.kind,
            name: s.name,
            path: s.path,
            parent: sc.parent.clone(),
            visibility: s.vis,
            is_public_api: s.api && s.vis == Visibility::Public,
            signature_hash: s.signature.as_deref().map(ContentHash::of_str),
            signature: s.signature,
            doc: info.doc,
            attributes: info.attributes,
            cfg: cfg.clone(),
            range,
            content_hash,
            impl_info: s.impl_info,
            returns: s.returns,
            facts,
            calls: s.calls,
            is_test: info.is_test && s.kind == SymbolKind::Function,
            verifies,
        });
        (id, cfg)
    }

    #[allow(clippy::too_many_arguments)]
    fn spec<'s>(
        &self,
        sc: &Scope,
        kind: SymbolKind,
        name: String,
        vis: Visibility,
        api: bool,
        signature: Option<String>,
        attrs: &'s [Attribute],
        tokens: TokenStream,
    ) -> Spec<'s> {
        let mut path = sc.path.clone();
        path.push(name.clone());
        Spec {
            kind,
            name,
            path,
            vis,
            api,
            signature,
            attrs,
            tokens,
            returns: None,
            facts: vec![],
            calls: vec![],
            impl_info: sc.impl_info.clone(),
            trait_key: sc.trait_key.clone(),
        }
    }

    fn child_scope(&self, sc: &Scope, path: Vec<String>, id: SymbolId, cfg: Vec<String>) -> Scope {
        Scope {
            path,
            parent: Some(id),
            cfg,
            api: sc.api,
            inherit: None,
            trait_key: String::new(),
            impl_info: None,
        }
    }

    fn items(&mut self, items: &[Item], sc: &Scope) {
        for it in items {
            self.item(it, sc);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn function<'ast>(
        &mut self,
        sc: &Scope,
        kind: SymbolKind,
        syn_vis: &syn::Visibility,
        sig: &Signature,
        attrs: &[Attribute],
        block: Option<&'ast Block>,
        tokens: TokenStream,
    ) {
        let vis = self.vis(sc, syn_vis);
        let name = ident_name(&sig.ident);
        let mut spec = self.spec(
            sc,
            kind,
            name.clone(),
            vis,
            sc.api,
            Some(fn_header(syn_vis, sig)),
            attrs,
            tokens,
        );
        spec.returns = return_contract(sig);
        if sig.unsafety.is_some() {
            spec.facts.push(mk_fact(
                FactKind::UnsafeCode,
                self.loc.of(&sig.unsafety),
                format!("unsafe fn {name}"),
            ));
        }
        let scan = block.map(|b| scan_block(&self.loc, b));
        let mut nested = vec![];
        if let Some(s) = scan {
            spec.facts.extend(s.facts);
            spec.calls = s.calls;
            nested = s.nested;
        }
        let path = spec.path.clone();
        let (id, cfg) = self.add(sc, spec);
        // Items nested in the body: indexed under the function's path; never public API.
        let mut child = self.child_scope(sc, path, id, cfg);
        child.api = false;
        for n in nested {
            self.item(n, &child);
        }
    }

    /// Symbol without a body of its own (const, static, type, struct, ...); `expr` is an
    /// initializer to scan for facts.
    #[allow(clippy::too_many_arguments)]
    fn leaf<'ast>(
        &mut self,
        sc: &Scope,
        kind: SymbolKind,
        name: String,
        syn_vis: &syn::Visibility,
        signature: Option<String>,
        attrs: &[Attribute],
        tokens: TokenStream,
        expr: Option<&'ast Expr>,
    ) -> (SymbolId, Vec<String>, Visibility, bool) {
        let vis = self.vis(sc, syn_vis);
        let mut spec = self.spec(
            sc,
            kind,
            name.clone(),
            vis,
            sc.api,
            signature,
            attrs,
            tokens,
        );
        if let Some(e) = expr {
            if matches!(kind, SymbolKind::Const | SymbolKind::Static) && is_literalish(e) {
                spec.facts.push(mk_fact(
                    FactKind::ConstValue,
                    self.loc.of_tokens(spec.tokens.clone()),
                    format!("{name} = {}", tokens_string(e)),
                ));
            }
            let s = scan_expr(&self.loc, e);
            spec.facts.extend(s.facts);
            spec.calls = s.calls;
        }
        let (id, cfg) = self.add(sc, spec);
        (id, cfg, vis, sc.api && vis == Visibility::Public)
    }

    fn item<'ast>(&mut self, item: &'ast Item, sc: &Scope) {
        match item {
            Item::Fn(f) => self.function(
                sc,
                SymbolKind::Function,
                &f.vis,
                &f.sig,
                &f.attrs,
                Some(&f.block),
                f.to_token_stream(),
            ),
            Item::Mod(m) => {
                let name = ident_name(&m.ident);
                let (id, cfg, vis, api) = self.leaf(
                    sc,
                    SymbolKind::Module,
                    name.clone(),
                    &m.vis,
                    None,
                    &m.attrs,
                    m.to_token_stream(),
                    None,
                );
                if let Some((_, inner)) = &m.content {
                    let mut path = sc.path.clone();
                    path.push(name);
                    let mut child = self.child_scope(sc, path, id, cfg);
                    child.api = api && vis == Visibility::Public;
                    self.items(inner, &child);
                }
            }
            Item::Struct(s) => {
                let sig = item_header(item).map(|x| x.1);
                let name = ident_name(&s.ident);
                let (id, cfg, vis, api) = self.leaf(
                    sc,
                    SymbolKind::Struct,
                    name.clone(),
                    &s.vis,
                    sig,
                    &s.attrs,
                    s.to_token_stream(),
                    None,
                );
                // Named fields are symbols too, so documentation can reference
                // `Type::field` and get exact resolution.
                if let syn::Fields::Named(named) = &s.fields {
                    let mut path = sc.path.clone();
                    path.push(name);
                    let mut child = self.child_scope(sc, path, id, cfg);
                    child.api = api && vis == Visibility::Public;
                    for f in &named.named {
                        let Some(ident) = &f.ident else { continue };
                        let mut bare = f.clone();
                        bare.attrs.clear();
                        let sig = Some(tokens_string(&bare));
                        self.leaf(
                            &child,
                            SymbolKind::Field,
                            ident_name(ident),
                            &f.vis,
                            sig,
                            &f.attrs,
                            f.to_token_stream(),
                            None,
                        );
                    }
                }
            }
            Item::Union(s) => {
                let sig = item_header(item).map(|x| x.1);
                self.leaf(
                    sc,
                    SymbolKind::Union,
                    ident_name(&s.ident),
                    &s.vis,
                    sig,
                    &s.attrs,
                    s.to_token_stream(),
                    None,
                );
            }
            Item::Enum(e) => {
                let sig = item_header(item).map(|x| x.1);
                let name = ident_name(&e.ident);
                let (id, cfg, vis, api) = self.leaf(
                    sc,
                    SymbolKind::Enum,
                    name.clone(),
                    &e.vis,
                    sig,
                    &e.attrs,
                    e.to_token_stream(),
                    None,
                );
                let mut path = sc.path.clone();
                path.push(name);
                let mut child = self.child_scope(sc, path, id, cfg);
                child.inherit = Some(vis);
                child.api = api;
                for v in &e.variants {
                    let mut bare = v.clone();
                    bare.attrs.clear();
                    let sig = Some(tokens_string(&bare));
                    self.leaf(
                        &child,
                        SymbolKind::Variant,
                        ident_name(&v.ident),
                        &syn::Visibility::Inherited,
                        sig,
                        &v.attrs,
                        v.to_token_stream(),
                        None,
                    );
                }
            }
            Item::Trait(t) => {
                let sig = item_header(item).map(|x| x.1);
                let name = ident_name(&t.ident);
                let (id, cfg, vis, api) = self.leaf(
                    sc,
                    SymbolKind::Trait,
                    name.clone(),
                    &t.vis,
                    sig,
                    &t.attrs,
                    t.to_token_stream(),
                    None,
                );
                if t.unsafety.is_some() {
                    if let Some(sym) = self.symbols.iter_mut().find(|s| s.id == id) {
                        sym.facts.push(mk_fact(
                            FactKind::UnsafeCode,
                            self.loc.of(&t.unsafety),
                            format!("unsafe trait {name}"),
                        ));
                    }
                }
                let mut path = sc.path.clone();
                path.push(name);
                let mut child = self.child_scope(sc, path, id, cfg);
                child.inherit = Some(vis);
                child.api = api;
                for ti in &t.items {
                    match ti {
                        TraitItem::Fn(f) => self.function(
                            &child,
                            SymbolKind::TraitMethod,
                            &syn::Visibility::Inherited,
                            &f.sig,
                            &f.attrs,
                            f.default.as_ref(),
                            f.to_token_stream(),
                        ),
                        TraitItem::Const(c) => {
                            let (i, ty) = (&c.ident, &c.ty);
                            let sig = Some(tokens_string(&quote!(const #i : #ty)));
                            self.leaf(
                                &child,
                                SymbolKind::Const,
                                ident_name(i),
                                &syn::Visibility::Inherited,
                                sig,
                                &c.attrs,
                                c.to_token_stream(),
                                c.default.as_ref().map(|d| &d.1),
                            );
                        }
                        TraitItem::Type(ty) => {
                            let (i, g, c, b) =
                                (&ty.ident, &ty.generics, &ty.colon_token, &ty.bounds);
                            let sig = Some(tokens_string(&quote!(type #i #g #c #b)));
                            self.leaf(
                                &child,
                                SymbolKind::TypeAlias,
                                ident_name(i),
                                &syn::Visibility::Inherited,
                                sig,
                                &ty.attrs,
                                ty.to_token_stream(),
                                None,
                            );
                        }
                        _ => {}
                    }
                }
            }
            Item::Type(t) => {
                let sig = item_header(item).map(|x| x.1);
                self.leaf(
                    sc,
                    SymbolKind::TypeAlias,
                    ident_name(&t.ident),
                    &t.vis,
                    sig,
                    &t.attrs,
                    t.to_token_stream(),
                    None,
                );
            }
            Item::Const(c) => {
                let sig = item_header(item).map(|x| x.1);
                self.leaf(
                    sc,
                    SymbolKind::Const,
                    ident_name(&c.ident),
                    &c.vis,
                    sig,
                    &c.attrs,
                    c.to_token_stream(),
                    Some(&c.expr),
                );
            }
            Item::Static(c) => {
                let sig = item_header(item).map(|x| x.1);
                self.leaf(
                    sc,
                    SymbolKind::Static,
                    ident_name(&c.ident),
                    &c.vis,
                    sig,
                    &c.attrs,
                    c.to_token_stream(),
                    Some(&c.expr),
                );
            }
            Item::Macro(m) => {
                if let (true, Some(id)) = (m.mac.path.is_ident("macro_rules"), &m.ident) {
                    let exported = m.attrs.iter().any(|a| a.path().is_ident("macro_export"));
                    let v = if exported {
                        syn::parse_quote!(pub)
                    } else {
                        syn::Visibility::Inherited
                    };
                    self.leaf(
                        sc,
                        SymbolKind::MacroRules,
                        ident_name(id),
                        &v,
                        None,
                        &m.attrs,
                        m.to_token_stream(),
                        None,
                    );
                }
            }
            Item::Use(u) => {
                let mut flat = vec![];
                flatten_use(&u.tree, &mut vec![], &mut flat);
                for (path, alias, glob, s, e) in flat {
                    self.imports.push(Import {
                        path,
                        alias,
                        glob,
                        range: self.loc.range(s.start(), e.end()),
                    });
                }
            }
            Item::Impl(imp) => self.impl_block(imp, sc),
            _ => {}
        }
    }

    fn impl_block<'ast>(&mut self, imp: &'ast syn::ItemImpl, sc: &Scope) {
        let self_ty = tokens_string(&*imp.self_ty);
        let trait_path = imp.trait_.as_ref().map(|(_, p, _)| tokens_string(p));
        let name = match &trait_path {
            Some(t) => format!("impl {t} for {self_ty}"),
            None => format!("impl {self_ty}"),
        };
        let last = type_last(&imp.self_ty);
        let ty_vis = self.type_vis.get(&last).copied();
        let ty_public = ty_vis.is_none_or(|v| v == Visibility::Public);
        let impl_info = ImplInfo {
            self_ty,
            trait_path: trait_path.clone(),
        };

        let (vis, api) = if trait_path.is_some() {
            (ty_vis.unwrap_or(Visibility::Public), sc.api && ty_public)
        } else {
            (Visibility::Private, false)
        };
        let mut path = sc.path.clone();
        path.push(format!("<{name}>"));
        let mut spec = self.spec(
            sc,
            SymbolKind::Impl,
            name,
            vis,
            api,
            None,
            &imp.attrs,
            imp.to_token_stream(),
        );
        spec.path = path;
        spec.impl_info = Some(impl_info.clone());
        if imp.unsafety.is_some() {
            spec.facts.push(mk_fact(
                FactKind::UnsafeCode,
                self.loc.of(&imp.unsafety),
                format!("unsafe {}", spec.name),
            ));
        }
        let (id, cfg) = self.add(sc, spec);

        let mut ipath = sc.path.clone();
        ipath.push(last);
        let mut isc = self.child_scope(sc, ipath, id, cfg);
        isc.api = sc.api && ty_public;
        isc.impl_info = Some(impl_info);
        if let Some(t) = trait_path {
            isc.inherit = Some(ty_vis.unwrap_or(Visibility::Public));
            isc.trait_key = t;
        }
        for it in &imp.items {
            match it {
                ImplItem::Fn(f) => {
                    let kind = if f.sig.receiver().is_some() {
                        SymbolKind::Method
                    } else {
                        SymbolKind::AssociatedFunction
                    };
                    self.function(
                        &isc,
                        kind,
                        &f.vis,
                        &f.sig,
                        &f.attrs,
                        Some(&f.block),
                        f.to_token_stream(),
                    );
                }
                ImplItem::Const(c) => {
                    let (v, i, ty) = (&c.vis, &c.ident, &c.ty);
                    let sig = Some(tokens_string(&quote!(#v const #i : #ty)));
                    self.leaf(
                        &isc,
                        SymbolKind::Const,
                        ident_name(i),
                        v,
                        sig,
                        &c.attrs,
                        c.to_token_stream(),
                        Some(&c.expr),
                    );
                }
                ImplItem::Type(t) => {
                    let (v, i, g, ty) = (&t.vis, &t.ident, &t.generics, &t.ty);
                    let sig = Some(tokens_string(&quote!(#v type #i #g = #ty)));
                    self.leaf(
                        &isc,
                        SymbolKind::TypeAlias,
                        ident_name(i),
                        v,
                        sig,
                        &t.attrs,
                        t.to_token_stream(),
                        None,
                    );
                }
                _ => {}
            }
        }
    }
}
