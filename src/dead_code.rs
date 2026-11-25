//! Dead code (`[build] dead_code=1`, [`crate::jass::Options::dead_code`]):
//! the functions nothing reachable calls or names as code (from `main`,
//! `config`, what `ExecuteFunc("name")` names, a class's methods), and the
//! globals nothing reachable reads or sets, left out. An `ExecuteFunc` of
//! a name told only as the game runs could call any function: then every
//! function is kept.

use std::collections::{HashMap, HashSet};

use crate::angelscript::{names_in, names_in_global, string_value};
use crate::ir::*;

/// The functions `ExecuteFunc` names in `body`; `None` when one is named
/// only as the game runs.
fn executed(body: &[Stmt], out: &mut HashSet<String>) -> bool {
    let mut known = true;
    each_call(body, &mut |c| {
        if c.name == "ExecuteFunc" {
            match c.args.first() {
                Some(Expr::Literal(l)) if string_value(l).is_some() => {
                    out.insert(string_value(l).expect("checked"));
                }
                _ => known = false,
            }
        }
    });
    known
}

/// Calls `f` with every call in `body`, statements' and expressions'.
fn each_call(body: &[Stmt], f: &mut impl FnMut(&Call)) {
    each_expr(body, &mut |e| {
        if let Expr::Call(c) = e {
            f(c);
        }
    });
    fn stmts(body: &[Stmt], f: &mut impl FnMut(&Call)) {
        for s in body {
            match s {
                Stmt::Call { call, .. } => f(call),
                Stmt::If { branches, otherwise, .. } => {
                    branches.iter().for_each(|(_, b)| stmts(b, f));
                    if let Some(b) = otherwise {
                        stmts(b, f);
                    }
                }
                Stmt::Loop { body, .. } => stmts(body, f),
                Stmt::Debug { stmt, .. } => stmts(std::slice::from_ref(stmt), f),
                _ => {}
            }
        }
    }
    stmts(body, f);
}

/// Leaves out what nothing reachable uses: how many functions and globals.
pub fn dead_code(m: &mut Module) -> (usize, usize) {
    let functions: HashMap<&str, usize> = m.functions.iter().enumerate().map(|(i, f)| (f.sig.name.as_str(), i)).collect();
    let globals: HashMap<&str, usize> = m.globals.iter().enumerate().map(|(i, g)| (g.name.as_str(), i)).collect();
    let mut by_name = HashSet::new();
    let mut known = true;
    for f in &m.functions {
        known &= executed(&f.body, &mut by_name);
    }
    for st in &m.structs {
        for me in &st.methods {
            known &= executed(&me.function.body, &mut by_name);
        }
    }
    let mut queue: Vec<String> = Vec::new();
    if known {
        queue.extend(["main", "config"].map(String::from));
        queue.extend(by_name);
    } else {
        queue.extend(m.functions.iter().map(|f| f.sig.name.clone()));
    }
    // A class's methods: any may be called (by its type).
    for st in &m.structs {
        st.methods.iter().for_each(|me| names_in(&me.function.body, &mut queue));
        st.fields.iter().filter_map(|f| f.init.clone()).for_each(|e| names_in(&[Stmt::Return { value: Some(e), line: 0 }], &mut queue));
    }
    for i in &m.interfaces {
        i.methods.iter().filter_map(|me| me.defaults.clone()).for_each(|e| names_in(&[Stmt::Return { value: Some(e), line: 0 }], &mut queue));
    }
    let (mut live_f, mut live_g) = (vec![false; m.functions.len()], vec![false; m.globals.len()]);
    let mut seen = HashSet::new();
    while let Some(n) = queue.pop() {
        if !seen.insert(n.clone()) {
            continue;
        }
        if let Some(&i) = functions.get(n.as_str()) {
            live_f[i] = true;
            names_in(&m.functions[i].body, &mut queue);
        }
        if let Some(&i) = globals.get(n.as_str()) {
            live_g[i] = true;
            names_in_global(&m.globals[i], &mut queue);
        }
    }
    let (before_f, before_g) = (m.functions.len(), m.globals.len());
    let mut k = 0;
    m.functions.retain(|_| {
        k += 1;
        live_f[k - 1]
    });
    let mut k = 0;
    m.globals.retain(|_| {
        k += 1;
        live_g[k - 1]
    });
    (before_f - m.functions.len(), before_g - m.globals.len())
}

#[cfg(test)]
#[path = "dead_code.test.rs"]
mod tests;
