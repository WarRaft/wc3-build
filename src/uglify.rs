//! Shorter names: every name the module declares but the game doesn't
//! need as it is renamed to the shortest free one (`a`, `b`, ..., `Z`,
//! `a0` ...), the names used most getting the shortest. The finished tree
//! knows every name and every use of it, so nothing is missed:
//!
//! - **Renamed**: the module's functions (not `main` and `config`, which
//!   the game calls), globals and types; each function's parameters and
//!   locals, which take the shortest names again in each function,
//!   clear of what that function uses.
//! - **Kept**: natives (the engine's names), and whatever the module uses
//!   without declaring (the game's `common.j`, `Blizzard.j`): their names,
//!   and the language's words, are never given.
//! - **Names in text**: a literal naming a function in `ExecuteFunc("F")`,
//!   or a global in `TriggerRegisterVariableEvent(t, "udg_x", ...)`, is
//!   renamed with it. Where the name isn't a literal it can't be known, so
//!   the functions it may name (those taking nothing) keep their names, and
//!   the build says so ([`Renamed::kept`]).

use std::collections::{HashMap, HashSet};

use crate::ir::*;

/// JASS's words: never a name.
const WORDS: [&str; 37] = [
    "and", "array", "call", "constant", "debug", "else", "elseif", "endfunction", "endglobals", "endif", "endloop", "exitwhen",
    "extends", "false", "function", "globals", "if", "local", "loop", "native", "not", "nothing", "null", "or", "return", "returns",
    "set", "takes", "then", "true", "type", "boolean", "code", "handle", "integer", "real", "string",
];

/// What shortening did.
#[derive(Debug, Default, PartialEq)]
pub struct Renamed {
    /// Each name the module's top level had, and what it's called now.
    pub names: HashMap<String, String>,
    /// Why names were kept that would have been shortened.
    pub kept: Vec<String>,
}

/// The shortest names, in order: `a` ... `z`, `A` ... `Z`, then a letter
/// and a letter or digit, and so on.
struct Names {
    n: usize,
}

const FIRST: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ";
const REST: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";

impl Iterator for Names {
    type Item = String;
    fn next(&mut self) -> Option<String> {
        let mut n = self.n;
        self.n += 1;
        // n in base: first a letter, then letters and digits.
        if n < FIRST.len() {
            return Some((FIRST[n] as char).to_string());
        }
        n -= FIRST.len();
        let mut tail = Vec::new();
        let mut width = 1;
        let mut count = FIRST.len() * REST.len();
        while n >= count {
            n -= count;
            width += 1;
            count *= REST.len();
        }
        for _ in 0..width {
            tail.push(REST[n % REST.len()] as char);
            n /= REST.len();
        }
        let mut s = (FIRST[n] as char).to_string();
        s.extend(tail.into_iter().rev());
        Some(s)
    }
}

/// The text of a string literal (`"Name"`), when it's a plain one.
fn literal_text(e: &Expr) -> Option<&str> {
    match e {
        Expr::Literal(t) => t.strip_prefix('"')?.strip_suffix('"').filter(|s| !s.contains(['\\', '"'])),
        _ => None,
    }
}

/// Every name used in `body` (variables, functions, code values, types of
/// locals), once per use.
fn uses(body: &[Stmt], out: &mut Vec<String>) {
    for s in body {
        match s {
            Stmt::Local { ty, name, .. } => {
                out.push(ty.clone());
                out.push(name.clone());
            }
            Stmt::Set { name, .. } => out.push(name.clone()),
            Stmt::Call { call, .. } => out.push(call.name.clone()),
            Stmt::If { branches, otherwise, .. } => {
                branches.iter().for_each(|(_, b)| uses(b, out));
                if let Some(b) = otherwise {
                    uses(b, out);
                }
            }
            Stmt::Loop { body, .. } => uses(body, out),
            Stmt::Debug { stmt, .. } => uses(std::slice::from_ref(stmt), out),
            _ => {}
        }
    }
    each_expr(body, &mut |e| match e {
        Expr::Var(n) | Expr::Index(n, _) | Expr::Code(n) => out.push(n.clone()),
        Expr::Call(c) => out.push(c.name.clone()),
        Expr::Literal(_) | Expr::Unary(..) | Expr::Binary(..) => {}
    });
}

/// The names in text a call gives: (function or global, the literal).
fn named_in_text(body: &[Stmt], functions: &mut Vec<String>, globals: &mut Vec<String>, dynamic: &mut bool) {
    let mut call = |c: &Call| match c.name.as_str() {
        "ExecuteFunc" => match c.args.first().and_then(literal_text) {
            Some(n) => functions.push(n.to_string()),
            None => *dynamic = true,
        },
        "TriggerRegisterVariableEvent" => {
            if let Some(n) = c.args.get(1).and_then(literal_text) {
                globals.push(n.to_string());
            }
        }
        _ => {}
    };
    fn statements(body: &[Stmt], f: &mut dyn FnMut(&Call)) {
        for s in body {
            match s {
                Stmt::Call { call, .. } => f(call),
                Stmt::If { branches, otherwise, .. } => {
                    branches.iter().for_each(|(_, b)| statements(b, f));
                    if let Some(b) = otherwise {
                        statements(b, f);
                    }
                }
                Stmt::Loop { body, .. } => statements(body, f),
                Stmt::Debug { stmt, .. } => statements(std::slice::from_ref(stmt), f),
                _ => {}
            }
        }
    }
    statements(body, &mut call);
    each_expr(body, &mut |e| {
        if let Expr::Call(c) = e {
            call(c);
        }
    });
}

/// Renames `m`'s names to the shortest free ones (see the module docs).
pub fn uglify(m: &mut Module) -> Renamed {
    let mut renamed = Renamed::default();
    // Locals and parameters first take names nothing else can have, so a
    // top-level name given later can't be taken for one of them.
    for f in &mut m.functions {
        let mut own = Vec::new();
        locals_and_params(f, &mut own);
        own.dedup();
        let to: HashMap<String, String> = own.into_iter().enumerate().map(|(i, n)| (n, format!("\u{0}{i}"))).collect();
        for p in &mut f.sig.params {
            if let Some(t) = to.get(&p.name) {
                p.name = t.clone();
            }
        }
        rename_locals_in(&mut f.body, &to);
    }
    // What the module declares at its top level, and what it uses.
    let natives: HashSet<String> = m.natives.iter().map(|n| n.sig.name.clone()).collect();
    let mut declared: Vec<String> = Vec::new();
    declared.extend(m.types.iter().map(|t| t.name.clone()));
    declared.extend(m.globals.iter().map(|g| g.name.clone()));
    declared.extend(m.functions.iter().map(|f| f.sig.name.clone()));
    let declared_set: HashSet<&str> = declared.iter().map(String::as_str).collect();

    // Every use, and the names used without being declared: at the top
    // level, or in a function, not one of its own locals or parameters.
    let mut all_uses = Vec::new();
    let mut external: HashSet<String> = HashSet::new();
    let mut top_uses = Vec::new();
    for t in &m.types {
        top_uses.push(t.extends.clone());
    }
    for g in &m.globals {
        top_uses.push(g.ty.clone());
        if let Some(e) = &g.init {
            uses(&[Stmt::Return { value: Some(e.clone()), line: 0 }], &mut top_uses);
        }
    }
    for s in m.natives.iter().map(|n| &n.sig).chain(m.functions.iter().map(|f| &f.sig)) {
        top_uses.extend(s.params.iter().map(|p| p.ty.clone()));
        top_uses.extend(s.returns.clone());
    }
    external.extend(top_uses.iter().filter(|n| !declared_set.contains(n.as_str())).cloned());
    all_uses.extend(top_uses);
    let (mut in_text_functions, mut in_text_globals, mut dynamic) = (Vec::new(), Vec::new(), false);
    for f in &m.functions {
        let mut own = Vec::new();
        locals_and_params(f, &mut own);
        let mut used = Vec::new();
        uses(&f.body, &mut used);
        external.extend(used.iter().filter(|n| !declared_set.contains(n.as_str()) && !own.contains(n)).cloned());
        all_uses.extend(used.into_iter().filter(|n| !own.contains(n)));
        named_in_text(&f.body, &mut in_text_functions, &mut in_text_globals, &mut dynamic);
    }
    for g in &m.globals {
        if let Some(e) = &g.init {
            named_in_text(&[Stmt::Return { value: Some(e.clone()), line: 0 }], &mut in_text_functions, &mut in_text_globals, &mut dynamic);
        }
    }
    let mut count: HashMap<String, usize> = HashMap::new();
    for n in all_uses.iter().chain(&in_text_functions).chain(&in_text_globals) {
        *count.entry(n.clone()).or_default() += 1;
    }

    // Kept: the entry points; with a name not known in ExecuteFunc, every
    // function that takes nothing.
    let mut keep: HashSet<String> = ["main", "config"].iter().map(|s| s.to_string()).collect();
    if dynamic {
        let named: Vec<String> = m.functions.iter().filter(|f| f.sig.params.is_empty()).map(|f| f.sig.name.clone()).collect();
        renamed.kept.push(format!(
            "ExecuteFunc is given a name that isn't a literal: the {} functions that take nothing keep their names",
            named.len()
        ));
        keep.extend(named);
    }
    // Never given: the words, what the module uses without declaring (the
    // game's), natives, and what's kept.
    let mut taken: HashSet<String> = WORDS.iter().map(|s| s.to_string()).collect();
    taken.extend(external);
    taken.extend(natives.iter().cloned());
    taken.extend(keep.iter().cloned());

    let mut order: Vec<&String> = declared.iter().filter(|n| !keep.contains(*n) && !natives.contains(*n)).collect();
    order.sort_by_key(|n| std::cmp::Reverse(count.get(*n).copied().unwrap_or(0)));
    order.dedup();
    let mut fresh = Names { n: 0 }.filter(|n| !taken.contains(n));
    for n in order {
        if let Some(new) = fresh.next() {
            renamed.names.insert(n.clone(), new);
        }
    }

    apply_top(m, &renamed.names);
    // Each function's locals and parameters: the shortest names clear of
    // everything else the function names.
    for f in &mut m.functions {
        rename_locals(f);
    }
    renamed
}

/// The names of `f`'s parameters and locals.
fn locals_and_params(f: &Function, out: &mut Vec<String>) {
    out.extend(f.sig.params.iter().map(|p| p.name.clone()));
    fn locals(body: &[Stmt], out: &mut Vec<String>) {
        for s in body {
            match s {
                Stmt::Local { name, .. } => out.push(name.clone()),
                Stmt::If { branches, otherwise, .. } => {
                    branches.iter().for_each(|(_, b)| locals(b, out));
                    if let Some(b) = otherwise {
                        locals(b, out);
                    }
                }
                Stmt::Loop { body, .. } => locals(body, out),
                _ => {}
            }
        }
    }
    locals(&f.body, out);
}

/// Every top-level name `to` names renamed, wherever it's used (a type's
/// too, and in text where a call names it).
fn apply_top(m: &mut Module, to: &HashMap<String, String>) {
    let r = |n: &mut String| {
        if let Some(t) = to.get(n.as_str()) {
            *n = t.clone();
        }
    };
    for t in &mut m.types {
        r(&mut t.name);
        r(&mut t.extends);
    }
    for g in &mut m.globals {
        r(&mut g.name);
        r(&mut g.ty);
        if let Some(e) = &mut g.init {
            rename_expr(e, to, None);
        }
    }
    for s in m.natives.iter_mut().map(|n| &mut n.sig).chain(m.functions.iter_mut().map(|f| &mut f.sig)) {
        r(&mut s.name);
        for p in &mut s.params {
            r(&mut p.ty);
        }
        if let Some(t) = &mut s.returns {
            r(t);
        }
    }
    for f in &mut m.functions {
        // A parameter or local of the same name as a top-level one is its
        // own: not renamed here.
        let mut own = Vec::new();
        locals_and_params(f, &mut own);
        rename_body(&mut f.body, to, Some(&own.into_iter().collect()), true);
    }
}

/// Renames in `body`: variables and calls by `to`, but the names `own`
/// (a function's locals and parameters) as variables; types of locals
/// too when `types`.
fn rename_body(body: &mut [Stmt], to: &HashMap<String, String>, own: Option<&HashSet<String>>, types: bool) {
    let var = |n: &mut String| {
        if own.is_some_and(|o| o.contains(n.as_str())) {
            return;
        }
        if let Some(t) = to.get(n.as_str()) {
            *n = t.clone();
        }
    };
    for s in body {
        match s {
            Stmt::Local { ty, init, .. } => {
                if types && let Some(t) = to.get(ty.as_str()) {
                    *ty = t.clone();
                }
                if let Some(e) = init {
                    rename_expr(e, to, own);
                }
            }
            Stmt::Set { name, index, value, .. } => {
                var(name);
                if let Some(i) = index {
                    rename_expr(i, to, own);
                }
                rename_expr(value, to, own);
            }
            Stmt::Call { call, .. } => rename_call(call, to, own),
            Stmt::If { branches, otherwise, .. } => {
                for (c, b) in branches.iter_mut() {
                    rename_expr(c, to, own);
                    rename_body(b, to, own, types);
                }
                if let Some(b) = otherwise {
                    rename_body(b, to, own, types);
                }
            }
            Stmt::Loop { body, .. } => rename_body(body, to, own, types),
            Stmt::ExitWhen { cond, .. } => rename_expr(cond, to, own),
            Stmt::Return { value, .. } => {
                if let Some(e) = value {
                    rename_expr(e, to, own);
                }
            }
            Stmt::Debug { stmt, .. } => rename_body(std::slice::from_mut(stmt), to, own, types),
        }
    }
}

fn rename_call(c: &mut Call, to: &HashMap<String, String>, own: Option<&HashSet<String>>) {
    // Names in text: the function `ExecuteFunc` runs, the global
    // `TriggerRegisterVariableEvent` watches.
    let at = match c.name.as_str() {
        "ExecuteFunc" => Some(0),
        "TriggerRegisterVariableEvent" => Some(1),
        _ => None,
    };
    if let Some(i) = at
        && let Some(arg) = c.args.get_mut(i)
        && let Some(n) = literal_text(arg)
        && let Some(t) = to.get(n)
    {
        *arg = Expr::Literal(format!("\"{t}\""));
    }
    if let Some(t) = to.get(c.name.as_str()) {
        c.name = t.clone();
    }
    for a in &mut c.args {
        rename_expr(a, to, own);
    }
}

fn rename_expr(e: &mut Expr, to: &HashMap<String, String>, own: Option<&HashSet<String>>) {
    let mine = |n: &str| own.is_some_and(|o| o.contains(n));
    match e {
        Expr::Var(n) => {
            if !mine(n)
                && let Some(t) = to.get(n.as_str())
            {
                *n = t.clone();
            }
        }
        Expr::Index(n, i) => {
            if !mine(n)
                && let Some(t) = to.get(n.as_str())
            {
                *n = t.clone();
            }
            rename_expr(i, to, own);
        }
        Expr::Code(n) => {
            if let Some(t) = to.get(n.as_str()) {
                *n = t.clone();
            }
        }
        Expr::Call(c) => rename_call(c, to, own),
        Expr::Unary(_, x) => rename_expr(x, to, own),
        Expr::Binary(a, _, b) => {
            rename_expr(a, to, own);
            rename_expr(b, to, own);
        }
        Expr::Literal(_) => {}
    }
}

/// `f`'s parameters and locals renamed to the shortest names clear of the
/// words and of every other name `f` uses, the most used first.
fn rename_locals(f: &mut Function) {
    let mut own = Vec::new();
    locals_and_params(f, &mut own);
    if own.is_empty() {
        return;
    }
    let own_set: HashSet<String> = own.iter().cloned().collect();
    let mut used = Vec::new();
    uses(&f.body, &mut used);
    used.extend(f.sig.params.iter().map(|p| p.ty.clone()));
    used.extend(f.sig.returns.clone());
    used.push(f.sig.name.clone());
    let mut count: HashMap<&str, usize> = HashMap::new();
    for n in &used {
        *count.entry(n.as_str()).or_default() += 1;
    }
    let mut taken: HashSet<String> = WORDS.iter().map(|s| s.to_string()).collect();
    taken.extend(used.iter().filter(|n| !own_set.contains(*n)).cloned());
    let mut order: Vec<String> = own.clone();
    order.sort_by_key(|n| std::cmp::Reverse(count.get(n.as_str()).copied().unwrap_or(0)));
    order.dedup();
    let mut fresh = Names { n: 0 }.filter(|n| !taken.contains(n));
    let to: HashMap<String, String> = order.into_iter().filter_map(|n| fresh.next().map(|t| (n, t))).collect();
    for p in &mut f.sig.params {
        if let Some(t) = to.get(&p.name) {
            p.name = t.clone();
        }
    }
    rename_locals_in(&mut f.body, &to);
}

/// Locals declared, set and read in `body`, by `to` (only locals and
/// parameters are in it: everything else keeps its name).
fn rename_locals_in(body: &mut [Stmt], to: &HashMap<String, String>) {
    // Once each, declarations too: a name renamed isn't renamed again
    // (`d` to `a`, `distance` to `d`). Variables only: a local never names
    // a function.
    fn vars(body: &mut [Stmt], to: &HashMap<String, String>) {
        let v = |n: &mut String| {
            if let Some(t) = to.get(n.as_str()) {
                *n = t.clone();
            }
        };
        fn expr(e: &mut Expr, to: &HashMap<String, String>) {
            match e {
                Expr::Var(n) => {
                    if let Some(t) = to.get(n.as_str()) {
                        *n = t.clone();
                    }
                }
                Expr::Index(n, i) => {
                    if let Some(t) = to.get(n.as_str()) {
                        *n = t.clone();
                    }
                    expr(i, to);
                }
                Expr::Call(c) => c.args.iter_mut().for_each(|a| expr(a, to)),
                Expr::Unary(_, x) => expr(x, to),
                Expr::Binary(a, _, b) => {
                    expr(a, to);
                    expr(b, to);
                }
                Expr::Literal(_) | Expr::Code(_) => {}
            }
        }
        for s in body {
            match s {
                Stmt::Local { name, init, .. } => {
                    v(name);
                    if let Some(e) = init {
                        expr(e, to);
                    }
                }
                Stmt::Set { name, index, value, .. } => {
                    v(name);
                    if let Some(i) = index {
                        expr(i, to);
                    }
                    expr(value, to);
                }
                Stmt::Call { call, .. } => call.args.iter_mut().for_each(|a| expr(a, to)),
                Stmt::If { branches, otherwise, .. } => {
                    for (c, b) in branches.iter_mut() {
                        expr(c, to);
                        vars(b, to);
                    }
                    if let Some(b) = otherwise {
                        vars(b, to);
                    }
                }
                Stmt::Loop { body, .. } => vars(body, to),
                Stmt::ExitWhen { cond, .. } => expr(cond, to),
                Stmt::Return { value, .. } => {
                    if let Some(e) = value {
                        expr(e, to);
                    }
                }
                Stmt::Debug { stmt, .. } => vars(std::slice::from_mut(stmt), to),
            }
        }
    }
    vars(body, to);
}

#[cfg(test)]
#[path = "uglify.test.rs"]
mod tests;
