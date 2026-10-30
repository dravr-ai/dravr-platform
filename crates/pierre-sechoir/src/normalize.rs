// ABOUTME: Turns a Rust function body into a set of structural fingerprints
// ABOUTME: Keeps what is called (functions, methods, macros, operators); drops locals, literals and types
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Normalization follows unclebob's dryer: two bodies that call the same
//! operations in the same shape match, whatever their locals are named. Every
//! subtree is hashed bottom-up, and the set of those hashes is the body's
//! fingerprint set.
//!
//! The names of called functions and methods are kept on purpose. Replacing
//! them with placeholders would make `x.save()` and `x.delete()` the same,
//! burying real copies under look-alikes.

use std::collections::HashSet;
use std::hash::DefaultHasher;

use syn::punctuated::Punctuated;
use syn::visit::{self, Visit};
use syn::{BinOp, Block, Expr, Macro, Pat, Stmt, Token, UnOp};

/// A normalized body: its distinct subtree fingerprints and its node count.
#[derive(Debug)]
pub(crate) struct Shape {
    /// Distinct subtree hashes, sorted ascending.
    pub(crate) prints: Vec<u64>,
    /// Every node in the normalized tree, repeats included.
    pub(crate) nodes: usize,
}

/// Normalize one function body.
pub(crate) fn shape_of(body: &Block) -> Shape {
    let mut builder = Builder::default();
    builder.visit_block(body);
    let mut prints: Vec<u64> = builder.prints.into_iter().collect();
    prints.sort_unstable();
    Shape {
        prints,
        nodes: builder.nodes,
    }
}

/// Builds the normalized tree depth-first, hashing each node as it closes.
#[derive(Debug, Default)]
struct Builder {
    /// Open nodes: their label and the hashes of their finished children.
    stack: Vec<(String, Vec<u64>)>,
    prints: HashSet<u64>,
    nodes: usize,
}

impl Builder {
    fn open(&mut self, label: String) {
        self.stack.push((label, Vec::new()));
    }

    fn close(&mut self) {
        use std::hash::{Hash, Hasher};

        let Some((label, children)) = self.stack.pop() else {
            return;
        };
        let mut hasher = DefaultHasher::new();
        label.hash(&mut hasher);
        children.hash(&mut hasher);
        let hash = hasher.finish();
        self.nodes += 1;
        self.prints.insert(hash);
        if let Some(parent) = self.stack.last_mut() {
            parent.1.push(hash);
        }
    }

    /// A macro keeps its name. Most macros (`format!`, `vec!`, `assert!`)
    /// take comma-separated expressions, and those are walked like code; a
    /// body that does not parse that way stays one opaque node.
    fn macro_call(&mut self, mac: &Macro) {
        self.open(format!("mac:{}", last_segment(&mac.path)));
        let arguments = Punctuated::<Expr, Token![,]>::parse_terminated;
        if let Ok(args) = mac.parse_body_with(arguments) {
            for arg in &args {
                self.visit_expr(arg);
            }
        }
        self.close();
    }
}

fn last_segment(path: &syn::Path) -> String {
    path.segments
        .last()
        .map(|s| s.ident.to_string())
        .unwrap_or_default()
}

/// Path segments kept from a callee: `Type::function` or `module::function`.
const CALLEE_SEGMENTS: usize = 2;

/// The last [`CALLEE_SEGMENTS`] segments name what is called without tying
/// the match to where it was imported from.
fn callee_name(func: &Expr) -> String {
    let Expr::Path(path) = func else {
        return "dynamic".to_owned();
    };
    let segments: Vec<String> = path
        .path
        .segments
        .iter()
        .map(|s| s.ident.to_string())
        .collect();
    segments[segments.len().saturating_sub(CALLEE_SEGMENTS)..].join("::")
}

const fn binary_operator(op: &BinOp) -> &'static str {
    match op {
        BinOp::Add(_) | BinOp::AddAssign(_) => "+",
        BinOp::Sub(_) | BinOp::SubAssign(_) => "-",
        BinOp::Mul(_) | BinOp::MulAssign(_) => "*",
        BinOp::Div(_) | BinOp::DivAssign(_) => "/",
        BinOp::Rem(_) | BinOp::RemAssign(_) => "%",
        BinOp::And(_) => "&&",
        BinOp::Or(_) => "||",
        BinOp::Eq(_) => "==",
        BinOp::Ne(_) => "!=",
        BinOp::Lt(_) | BinOp::Le(_) | BinOp::Gt(_) | BinOp::Ge(_) => "compare",
        _ => "bitwise",
    }
}

const fn unary_operator(op: &UnOp) -> &'static str {
    match op {
        UnOp::Deref(_) => "*",
        UnOp::Not(_) => "!",
        UnOp::Neg(_) => "-",
        _ => "unary",
    }
}

/// The label of an expression node. Calls carry their callee, operators their
/// operator; everything else is the kind of expression alone.
fn expr_label(expr: &Expr) -> String {
    match expr {
        Expr::Binary(b) => format!("binary:{}", binary_operator(&b.op)),
        Expr::Call(c) => format!("call:{}", callee_name(&c.func)),
        Expr::MethodCall(m) => format!("method:{}", m.method),
        Expr::Struct(s) => format!("struct:{}", last_segment(&s.path)),
        Expr::Unary(u) => format!("unary:{}", unary_operator(&u.op)),
        other => expr_kind(other).to_owned(),
    }
}

const fn expr_kind(expr: &Expr) -> &'static str {
    match expr {
        Expr::Array(_) => "array",
        Expr::Assign(_) => "assign",
        Expr::Async(_) => "async",
        Expr::Await(_) => "await",
        Expr::Block(_) => "block-expr",
        Expr::Break(_) => "break",
        Expr::Cast(_) => "cast",
        Expr::Closure(_) => "closure",
        Expr::Continue(_) => "continue",
        Expr::Field(_) => "field",
        Expr::ForLoop(_) => "for",
        Expr::If(_) => "if",
        Expr::Index(_) => "index",
        Expr::Let(_) => "let-expr",
        Expr::Lit(_) => "literal",
        Expr::Loop(_) => "loop",
        Expr::Match(_) => "match",
        Expr::Path(_) => "path",
        Expr::Range(_) => "range",
        Expr::Reference(_) => "reference",
        Expr::Return(_) => "return",
        Expr::Try(_) => "try",
        Expr::Tuple(_) => "tuple",
        Expr::Unsafe(_) => "unsafe",
        Expr::While(_) => "while",
        _ => "expr",
    }
}

fn pattern_label(pat: &Pat) -> String {
    match pat {
        Pat::Ident(_) => "pat:binding".to_owned(),
        Pat::TupleStruct(t) => format!("pat:{}", last_segment(&t.path)),
        Pat::Struct(s) => format!("pat:{}", last_segment(&s.path)),
        Pat::Path(p) => format!("pat:{}", last_segment(&p.path)),
        Pat::Wild(_) => "pat:_".to_owned(),
        Pat::Lit(_) => "pat:literal".to_owned(),
        _ => "pat".to_owned(),
    }
}

impl<'ast> Visit<'ast> for Builder {
    fn visit_expr(&mut self, expr: &'ast Expr) {
        // Parentheses and invisible groups change nothing about what runs.
        match expr {
            Expr::Paren(p) => return self.visit_expr(&p.expr),
            Expr::Group(g) => return self.visit_expr(&g.expr),
            Expr::Macro(m) => return self.macro_call(&m.mac),
            _ => {}
        }
        self.open(expr_label(expr));
        match expr {
            // The callee is in the label already; only the arguments remain.
            Expr::Call(call) => call.args.iter().for_each(|a| self.visit_expr(a)),
            Expr::MethodCall(call) => {
                self.visit_expr(&call.receiver);
                call.args.iter().for_each(|a| self.visit_expr(a));
            }
            Expr::Lit(_) | Expr::Path(_) => {}
            _ => visit::visit_expr(self, expr),
        }
        self.close();
    }

    fn visit_stmt(&mut self, stmt: &'ast Stmt) {
        match stmt {
            Stmt::Local(local) => {
                self.open("let".to_owned());
                visit::visit_local(self, local);
                self.close();
            }
            Stmt::Macro(m) => self.macro_call(&m.mac),
            // A nested item is a definition, not a step of this body.
            Stmt::Item(_) => {}
            Stmt::Expr(expr, _) => self.visit_expr(expr),
        }
    }

    fn visit_block(&mut self, block: &'ast Block) {
        self.open("block".to_owned());
        visit::visit_block(self, block);
        self.close();
    }

    fn visit_pat(&mut self, pat: &'ast Pat) {
        self.open(pattern_label(pat));
        if !matches!(pat, Pat::Ident(_) | Pat::Lit(_)) {
            visit::visit_pat(self, pat);
        }
        self.close();
    }

    // Types are incidental to what a body does.
    fn visit_type(&mut self, _: &'ast syn::Type) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shape(code: &str) -> Shape {
        let block: Block = syn::parse_str(code).unwrap();
        shape_of(&block)
    }

    #[test]
    fn renamed_locals_and_changed_literals_match() {
        let left = shape("{ let ys = xs.iter().filter(|x| x > 3).count(); ys + 1 }");
        let right = shape("{ let kept = items.iter().filter(|i| i > 99).count(); kept + 7 }");
        assert_eq!(
            left.prints, right.prints,
            "only names and literals differ, so the shapes match"
        );
        assert_eq!(
            left.nodes, right.nodes,
            "matching shapes have the same node count"
        );
    }

    #[test]
    fn a_different_method_breaks_the_match() {
        let save = shape("{ store.save(record).await }");
        let delete = shape("{ store.delete(record).await }");
        assert_ne!(
            save.prints, delete.prints,
            "the called method's name is kept"
        );
    }

    #[test]
    fn macro_arguments_are_walked() {
        let left = shape(r#"{ println!("{}", a.len()); }"#);
        let right = shape(r#"{ println!("{}", a.capacity()); }"#);
        assert_ne!(
            left.prints, right.prints,
            "code inside a macro call is compared too"
        );
    }

    #[test]
    fn parentheses_do_not_change_the_shape() {
        assert_eq!(
            shape("{ (a + b) }").prints,
            shape("{ a + b }").prints,
            "parens are transparent"
        );
    }

    #[test]
    fn nested_items_are_not_part_of_the_body() {
        let with_item = shape("{ fn helper() -> u8 { 1 } run(x) }");
        let without = shape("{ run(x) }");
        assert_eq!(
            with_item.prints, without.prints,
            "a nested fn adds nothing to the enclosing body"
        );
    }
}
