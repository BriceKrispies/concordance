//! Syntactic facts and call sites found inside one body.
//!
//! LIMITATION: macro arguments are token streams, not expressions, so nothing inside
//! `assert!(x > 100)`, `println!("{}", a.unwrap())` etc. is analysed; only the macro
//! invocation itself (location and path) is recorded. Nested `fn`/`impl`/... items are not
//! entered; they are returned in [`Scan::nested`] and indexed as symbols of their own.

use crate::signature::{normalize_tokens, tokens_string};
use crate::spans::Locator;
use concordance_core::code::{CallSite, CallTarget, Fact, FactKind};
use concordance_core::SourceRange;
use syn::visit::{self, Visit};
use syn::{BinOp, Block, Expr, ExprBinary, ExprCall, ExprMethodCall, Item, Lit, Macro, Stmt, UnOp};

const MAX_CALLS: usize = 200;
const MAX_TRY_FACTS: usize = 20;
const MAX_DETAIL: usize = 200;

pub(crate) struct Scan<'ast> {
    pub facts: Vec<Fact>,
    pub calls: Vec<CallSite>,
    pub nested: Vec<&'ast Item>,
}

pub(crate) fn mk_fact(kind: FactKind, range: SourceRange, detail: String) -> Fact {
    Fact {
        kind,
        range,
        detail: truncate(detail),
    }
}

fn truncate(s: String) -> String {
    if s.chars().count() <= MAX_DETAIL {
        s
    } else {
        s.chars().take(MAX_DETAIL).collect()
    }
}

pub(crate) fn scan_block<'ast>(loc: &Locator, b: &'ast Block) -> Scan<'ast> {
    let mut s = Scanner::new(loc);
    s.visit_block(b);
    s.out
}

pub(crate) fn scan_expr<'ast>(loc: &Locator, e: &'ast Expr) -> Scan<'ast> {
    let mut s = Scanner::new(loc);
    s.visit_expr(e);
    s.out
}

struct Scanner<'a, 'ast> {
    loc: &'a Locator,
    out: Scan<'ast>,
    tries: usize,
}

impl<'a, 'ast> Scanner<'a, 'ast> {
    fn new(loc: &'a Locator) -> Self {
        Scanner {
            loc,
            out: Scan {
                facts: vec![],
                calls: vec![],
                nested: vec![],
            },
            tries: 0,
        }
    }

    fn fact(&mut self, kind: FactKind, range: SourceRange, detail: String) {
        self.out.facts.push(mk_fact(kind, range, detail));
    }

    fn call(&mut self, target: CallTarget, range: SourceRange) {
        if self.out.calls.len() < MAX_CALLS {
            self.out.calls.push(CallSite { target, range });
        }
    }
}

fn is_numeric_literal(e: &Expr) -> bool {
    match e {
        Expr::Lit(l) => matches!(l.lit, Lit::Int(_) | Lit::Float(_)),
        Expr::Unary(u) => matches!(u.op, UnOp::Neg(_)) && is_numeric_literal(&u.expr),
        Expr::Paren(p) => is_numeric_literal(&p.expr),
        Expr::Group(g) => is_numeric_literal(&g.expr),
        Expr::Cast(c) => is_numeric_literal(&c.expr),
        _ => false,
    }
}

fn is_screaming(s: &str) -> bool {
    s.chars().any(|c| c.is_ascii_uppercase())
        && s.chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

fn is_const_path(e: &Expr) -> bool {
    match e {
        Expr::Path(p) if p.qself.is_none() => p
            .path
            .segments
            .last()
            .is_some_and(|s| is_screaming(&s.ident.to_string())),
        Expr::Paren(p) => is_const_path(&p.expr),
        _ => false,
    }
}

fn is_lit_or_const(e: &Expr) -> bool {
    is_numeric_literal(e) || is_const_path(e)
}

fn is_err_call(e: &Expr) -> bool {
    matches!(e, Expr::Call(c) if matches!(&*c.func, Expr::Path(p) if p.path.is_ident("Err")))
}

fn path_segments(p: &syn::Path) -> Vec<String> {
    p.segments
        .iter()
        .map(|s| crate::signature::ident_name(&s.ident))
        .collect()
}

impl<'a, 'ast> Visit<'ast> for Scanner<'a, 'ast> {
    fn visit_item(&mut self, i: &'ast Item) {
        self.out.nested.push(i);
    }

    fn visit_block(&mut self, b: &'ast Block) {
        for s in &b.stmts {
            if let Stmt::Expr(e, _) = s {
                if is_err_call(e) {
                    let (r, d) = (self.loc.of(e), tokens_string(e));
                    self.fact(FactKind::ErrorReturn, r, d);
                }
            }
        }
        visit::visit_block(self, b);
    }

    fn visit_arm(&mut self, a: &'ast syn::Arm) {
        if is_err_call(&a.body) {
            let (r, d) = (self.loc.of(&*a.body), tokens_string(&*a.body));
            self.fact(FactKind::ErrorReturn, r, d);
        }
        visit::visit_arm(self, a);
    }

    fn visit_expr_return(&mut self, i: &'ast syn::ExprReturn) {
        if i.expr.as_deref().is_some_and(is_err_call) {
            let (r, d) = (self.loc.of(i), tokens_string(i));
            self.fact(FactKind::ErrorReturn, r, d);
        }
        visit::visit_expr_return(self, i);
    }

    fn visit_expr_try(&mut self, i: &'ast syn::ExprTry) {
        if self.tries < MAX_TRY_FACTS {
            let (r, d) = (self.loc.of(i), tokens_string(i));
            self.fact(FactKind::ErrorReturn, r, d);
        }
        self.tries += 1;
        visit::visit_expr_try(self, i);
    }

    fn visit_expr_unsafe(&mut self, i: &'ast syn::ExprUnsafe) {
        let (r, d) = (self.loc.of(i), tokens_string(i));
        self.fact(FactKind::UnsafeCode, r, d);
        visit::visit_expr_unsafe(self, i);
    }

    fn visit_expr_binary(&mut self, i: &'ast ExprBinary) {
        let op = match i.op {
            BinOp::Lt(_) => Some("<"),
            BinOp::Le(_) => Some("<="),
            BinOp::Gt(_) => Some(">"),
            BinOp::Ge(_) => Some(">="),
            BinOp::Eq(_) => Some("=="),
            BinOp::Ne(_) => Some("!="),
            _ => None,
        };
        if let Some(op) = op {
            if is_lit_or_const(&i.left) || is_lit_or_const(&i.right) {
                let d = format!(
                    "{} {} {}",
                    tokens_string(&*i.left),
                    op,
                    tokens_string(&*i.right)
                );
                let r = self.loc.of(i);
                self.fact(FactKind::LiteralComparison, r, d);
            }
        }
        visit::visit_expr_binary(self, i);
    }

    fn visit_expr_call(&mut self, i: &'ast ExprCall) {
        if let Expr::Path(p) = &*i.func {
            let segs = path_segments(&p.path);
            let start = self.loc.of(&p.path);
            let end = i.paren_token.span.close().end();
            let range = self.loc.range(start_lc(&start), end);
            self.call(
                CallTarget::Path {
                    segments: segs.clone(),
                },
                range,
            );
            let n = segs.len();
            let last = segs.last().map(String::as_str).unwrap_or("");
            if n >= 2 && segs[n - 2] == "env" && matches!(last, "var" | "var_os") {
                let (r, d) = (self.loc.of(i), tokens_string(i));
                self.fact(FactKind::EnvRead, r, d);
            }
            if matches!(last, "min" | "max") && i.args.iter().any(is_lit_or_const) {
                let (r, d) = (self.loc.of(i), tokens_string(i));
                self.fact(FactKind::Clamp, r, d);
            }
        }
        visit::visit_expr_call(self, i);
    }

    fn visit_expr_method_call(&mut self, i: &'ast ExprMethodCall) {
        let name = crate::signature::ident_name(&i.method);
        let start = i.method.span().start();
        let range = self.loc.range(start, i.paren_token.span.close().end());
        self.call(CallTarget::Method { name: name.clone() }, range);
        let fact = match name.as_str() {
            "unwrap" | "expect" => Some(FactKind::PanicSite),
            "min" | "max" if i.args.iter().any(is_lit_or_const) => Some(FactKind::Clamp),
            "clamp" => Some(FactKind::Clamp),
            _ => None,
        };
        if let Some(kind) = fact {
            let (r, d) = (self.loc.of(i), tokens_string(i));
            self.fact(kind, r, d);
        }
        visit::visit_expr_method_call(self, i);
    }

    fn visit_macro(&mut self, m: &'ast Macro) {
        let segs = path_segments(&m.path);
        let name = segs.last().cloned().unwrap_or_default();
        let path = segs.join("::");
        let range = self.loc.of(m);
        let args = || normalize_tokens(&m.tokens.to_string());
        match name.as_str() {
            "panic" | "unreachable" | "todo" | "unimplemented" | "assert" | "assert_eq"
            | "assert_ne" => {
                self.fact(FactKind::PanicSite, range, format!("{path}!({})", args()));
            }
            "env" | "option_env" => {
                self.fact(FactKind::EnvRead, range, format!("{path}!({})", args()))
            }
            "cfg" => {
                let a = args();
                if has_word(&a, "feature") {
                    self.fact(FactKind::FeatureGate, range, format!("cfg!({a})"));
                } else {
                    self.fact(FactKind::MacroInvocation, range, path);
                }
            }
            "bail" => self.fact(FactKind::ErrorReturn, range, format!("{path}!({})", args())),
            "println" | "print" | "eprintln" | "eprint" | "format" | "write" | "writeln"
            | "vec" | "format_args" => {}
            _ => self.fact(FactKind::MacroInvocation, range, path),
        }
        visit::visit_macro(self, m);
    }
}

fn start_lc(r: &SourceRange) -> proc_macro2::LineColumn {
    proc_macro2::LineColumn {
        line: r.start_line as usize,
        column: r.start_col.saturating_sub(1) as usize,
    }
}

pub(crate) fn has_word(s: &str, w: &str) -> bool {
    s.split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .any(|x| x == w)
}
