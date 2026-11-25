//! Inlining (`[build] inline=1`, [`crate::jass::Options::inline`]), as
//! JassHelper does (its manual, Script optimization): a call of a
//! one-line function is what the line does, its arguments in place of its
//! parameters. Only where nothing can change: the line is a `return` of a
//! value (where the call is a value), or (where it's a statement) a `set`
//! (not of one of its parameters), a `call`, or a `return` of one call;
//! every parameter is read once and only once, in the order they're
//! given, and any call in the line comes after the last of them. Not of
//! `main` or `config`, of a function calling itself, nor where the caller
//! has a local named as a name the line reads. Run on the finished module,
//! before names are shortened, for every output.

use std::collections::{HashMap, HashSet};

use crate::angelscript::names_in;
use crate::ir::*;

/// A one-line function that can be inlined: its parameters and its line.
#[derive(Clone)]
struct Line {
    params: Vec<String>,
    stmt: Stmt,
}

/// What happens as an expression is evaluated, in order: a parameter
/// read (its place), a call.
enum Event {
    Param(usize),
    Call,
}

fn events(e: &Expr, params: &[String], out: &mut Vec<Event>) {
    match e {
        Expr::Var(n) => {
            if let Some(k) = params.iter().position(|p| p == n) {
                out.push(Event::Param(k));
            }
        }
        Expr::Index(_, i) => events(i, params, out),
        Expr::Call(c) => {
            c.args.iter().for_each(|a| events(a, params, out));
            out.push(Event::Call);
        }
        Expr::Unary(_, x) => events(x, params, out),
        Expr::Binary(a, _, b) => {
            events(a, params, out);
            events(b, params, out);
        }
        _ => {}
    }
}

/// Is every parameter read once, in their order, every call after them?
fn in_order(exprs: &[&Expr], params: &[String]) -> bool {
    let mut ev = Vec::new();
    for e in exprs {
        events(e, params, &mut ev);
    }
    let reads: Vec<usize> = ev.iter().filter_map(|e| if let Event::Param(k) = e { Some(*k) } else { None }).collect();
    if reads != (0..params.len()).collect::<Vec<_>>() {
        return false;
    }
    let last = ev.iter().rposition(|e| matches!(e, Event::Param(_)));
    last.is_none_or(|l| !ev[..l].iter().any(|e| matches!(e, Event::Call)))
}

/// Does `e` have anything but what [`events`] walks (a member, code ...)?
fn plain(e: &Expr) -> bool {
    match e {
        Expr::Literal(_) | Expr::Var(_) => true,
        Expr::Index(_, i) | Expr::Unary(_, i) => plain(i),
        Expr::Call(c) => c.args.iter().all(plain),
        Expr::Binary(a, _, b) => plain(a) && plain(b),
        _ => false,
    }
}

/// The module's one-line functions that can be inlined.
fn lines(m: &Module) -> HashMap<String, Line> {
    let mut out = HashMap::new();
    for f in &m.functions {
        if matches!(f.sig.name.as_str(), "main" | "config") || f.body.len() != 1 {
            continue;
        }
        let params: Vec<String> = f.sig.params.iter().map(|p| p.name.clone()).collect();
        let stmt = &f.body[0];
        let exprs: Vec<&Expr> = match stmt {
            Stmt::Return { value: Some(e), .. } => vec![e],
            Stmt::Set { name, index, value, .. } if !params.contains(name) => index.iter().chain(std::iter::once(value)).collect(),
            Stmt::Call { call, .. } => call.args.iter().collect(),
            _ => continue,
        };
        if !exprs.iter().all(|e| plain(e)) || !in_order(&exprs, &params) || called(&f.body).contains(&f.sig.name) {
            continue;
        }
        out.insert(f.sig.name.clone(), Line { params, stmt: stmt.clone() });
    }
    out
}

/// `e` with each parameter replaced by its argument.
fn substitute(e: &Expr, to: &HashMap<&str, &Expr>) -> Expr {
    match e {
        Expr::Var(n) => to.get(n.as_str()).map_or_else(|| e.clone(), |a| (*a).clone()),
        _ => {
            let mut e = e.clone();
            for x in e.children_mut() {
                *x = substitute(x, to);
            }
            e
        }
    }
}

/// The names a line reads or sets but its parameters.
fn free_names(l: &Line) -> Vec<String> {
    let mut out = Vec::new();
    names_in(std::slice::from_ref(&l.stmt), &mut out);
    out.retain(|n| !l.params.contains(n));
    out
}

/// Inlines what can be in every function; how many calls.
fn inline_once(m: &mut Module) -> usize {
    let lines = lines(m);
    if lines.is_empty() {
        return 0;
    }
    let mut count = 0;
    for f in &mut m.functions {
        let mut own: HashSet<String> = f.sig.params.iter().map(|p| p.name.clone()).collect();
        local_names(&f.body, &mut own);
        let me = f.sig.name.clone();
        // A line inlined here: not its own, nor one reading a name a local hides.
        let usable = |name: &str| -> Option<Line> {
            let l = lines.get(name)?;
            (name != me && !free_names(l).iter().any(|n| own.contains(n))).then(|| l.clone())
        };
        count += inline_body(&mut f.body, &usable);
    }
    count
}

fn local_names(body: &[Stmt], out: &mut HashSet<String>) {
    for s in body {
        match s {
            Stmt::Local { name, .. } => {
                out.insert(name.clone());
            }
            Stmt::If { branches, otherwise, .. } => {
                branches.iter().for_each(|(_, b)| local_names(b, out));
                if let Some(b) = otherwise {
                    local_names(b, out);
                }
            }
            Stmt::Loop { body, .. } => local_names(body, out),
            _ => {}
        }
    }
}

/// The arguments of `call` by `l`'s parameters.
fn args<'a>(l: &'a Line, call: &'a Call) -> Option<HashMap<&'a str, &'a Expr>> {
    (call.args.len() == l.params.len()).then(|| l.params.iter().map(String::as_str).zip(&call.args).collect())
}

fn inline_expr(e: &mut Expr, usable: &dyn Fn(&str) -> Option<Line>) -> usize {
    let mut n = 0;
    for x in e.children_mut() {
        n += inline_expr(x, usable);
    }
    let made = match &*e {
        Expr::Call(c) => usable(&c.name).and_then(|l| match &l.stmt {
            Stmt::Return { value: Some(v), .. } => args(&l, c).map(|to| substitute(v, &to)),
            _ => None,
        }),
        _ => None,
    };
    if let Some(made) = made {
        *e = made;
        n += 1;
    }
    n
}

fn inline_body(body: &mut [Stmt], usable: &dyn Fn(&str) -> Option<Line>) -> usize {
    let mut n = 0;
    for s in body {
        match s {
            Stmt::Local { init, .. } => n += init.iter_mut().map(|e| inline_expr(e, usable)).sum::<usize>(),
            Stmt::Set { index, value, .. } => {
                n += index.iter_mut().map(|e| inline_expr(e, usable)).sum::<usize>();
                n += inline_expr(value, usable);
            }
            Stmt::Call { call, line } => {
                n += call.args.iter_mut().map(|e| inline_expr(e, usable)).sum::<usize>();
                // The call a statement: what the line does.
                let made = usable(&call.name).and_then(|l| {
                    let to = args(&l, call)?;
                    match &l.stmt {
                        Stmt::Set { name, index, value, .. } => Some(Stmt::Set { name: name.clone(), index: index.as_ref().map(|e| substitute(e, &to)), value: substitute(value, &to), line: *line }),
                        Stmt::Call { call: inner, .. } => Some(Stmt::Call { call: Call { name: inner.name.clone(), args: inner.args.iter().map(|a| substitute(a, &to)).collect() }, line: *line }),
                        Stmt::Return { value: Some(Expr::Call(inner)), .. } => Some(Stmt::Call { call: Call { name: inner.name.clone(), args: inner.args.iter().map(|a| substitute(a, &to)).collect() }, line: *line }),
                        _ => None,
                    }
                });
                if let Some(made) = made {
                    *s = made;
                    n += 1;
                }
            }
            Stmt::If { branches, otherwise, .. } => {
                for (c, b) in branches.iter_mut() {
                    n += inline_expr(c, usable);
                    n += inline_body(b, usable);
                }
                if let Some(b) = otherwise {
                    n += inline_body(b, usable);
                }
            }
            Stmt::Loop { body, .. } => n += inline_body(body, usable),
            Stmt::ExitWhen { cond, .. } => n += inline_expr(cond, usable),
            Stmt::Return { value, .. } => n += value.iter_mut().map(|e| inline_expr(e, usable)).sum::<usize>(),
            Stmt::Debug { stmt, .. } => n += inline_body(std::slice::from_mut(&mut **stmt), usable),
            _ => {}
        }
    }
    n
}

/// `m`'s calls inlined ([`self`]), until nothing more is (a line made of
/// an inlined one): how many.
pub fn inline(m: &mut Module) -> usize {
    let mut total = 0;
    for _ in 0..8 {
        let n = inline_once(m);
        total += n;
        if n == 0 {
            break;
        }
    }
    total
}

#[cfg(test)]
#[path = "inline.test.rs"]
mod tests;
