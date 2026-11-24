//! The game's JASS out of a [`Module`]: what the game reads, whatever the
//! sources let themselves (see the README, "Outputs").
//!
//! 1. Every `main` and every `config` (taking and returning nothing) is
//!    merged into one, their statements in the order of the files; a
//!    local of a later one named as an earlier one's is renamed.
//! 2. Globals go into `globals`, each after those its initial value reads.
//!    An initial value that calls a function of the module (which the
//!    game hasn't read yet at `globals`) is set at the start of `main`
//!    instead; a constant can't be, and is an error.
//! 3. Functions are sorted so that each comes after those it calls or
//!    names as code (`main` and `config` last). A call that can't be sorted
//!    (A calls B, B calls A) goes through a trigger, as JassHelper's
//!    `.evaluate`: `wcb__call_F` (above every function) puts the
//!    arguments into globals of `F`'s own, evaluates `wcb__trig_F`, whose
//!    condition `wcb__thunk_F` (after `F`) calls `F` with them and keeps
//!    what it returns. JASS runs one thing at a time, and an evaluated
//!    function takes no arguments, so no two calls race for those globals.
//!    The triggers are made at the start of `config` and of `main`.
//!    Lowering's `F.evaluate(...)` (`wcb__call_F`) goes through it
//!    wherever `F` is, and `F.execute(...)` (`wcb__exec_F`) too, but runs
//!    the trigger's action (`wcb__action_F`, a thread of its own, which
//!    reads the arguments before it can wait).
//! 4. Locals go to the top of their function: one declared after a
//!    statement is declared there and set where it was.
//! 5. `set`, `call`, `local` and `takes nothing returns nothing` are
//!    written; parentheses where the game's precedence needs them.

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;

use crate::ir::*;

/// The prefix of every name the build makes.
pub const PREFIX: &str = "wcb__";

/// How a build makes its JASS.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Options {
    /// Shorter names ([`crate::uglify`]): everything the game doesn't need
    /// as it is renamed `a`, `b` ...
    pub uglify: bool,
    /// The game's natives and functions (`common.j`, `Blizzard.j`): what
    /// the code calls without declaring, for what a hook of one takes and
    /// gives back.
    pub game: Vec<Signature>,
}

/// What a build made: the script, and what it says about it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Built {
    pub script: String,
    /// Why some names weren't shortened, when they were asked to be.
    pub notes: Vec<String>,
}

/// The game's JASS of `m`, or why it can't be made.
pub fn build(m: &Module) -> Result<String, Vec<Error>> {
    build_with(m, &Options::default()).map(|b| b.script)
}

/// [`build`], as `options` say.
pub fn build_with(m: &Module, options: &Options) -> Result<Built, Vec<Error>> {
    let m = crate::lower::lower_with(m, &options.game)?;
    let errors = m.not_lowered();
    if !errors.is_empty() {
        return Err(errors);
    }
    let mut m = m;
    let mut errors = Vec::new();
    merge_entries(&mut m);
    let deferred = order_globals(&mut m, &mut errors);
    if !errors.is_empty() {
        return Err(errors);
    }
    if !deferred.is_empty() {
        let main = entry(&mut m, "main");
        let mut body = deferred;
        body.append(&mut main.body);
        main.body = body;
    }
    let order = sort_functions(&m);
    let functions: Vec<Function> = order.into_iter().map(|i| m.functions[i].clone()).collect();
    let (functions, globals) = evaluate_forward_calls(functions);
    m.globals.extend(globals);
    m.functions = functions.into_iter().map(hoist_locals).collect();
    let mut notes = Vec::new();
    if options.uglify {
        notes = crate::uglify::uglify(&mut m).kept;
    }
    Ok(Built { script: print(&m), notes })
}

/// Is `f` a `main` or a `config` the build merges?
fn is_entry(f: &Function) -> bool {
    (f.sig.name == "main" || f.sig.name == "config") && f.sig.params.is_empty() && f.sig.returns.is_none()
}

/// The module's `name` (`main`, `config`), made if it has none.
fn entry<'a>(m: &'a mut Module, name: &str) -> &'a mut Function {
    if !m.functions.iter().any(|f| f.sig.name == name) {
        m.functions.push(Function {
            sig: Signature { name: name.into(), params: Vec::new(), returns: None, constant: false },
            body: Vec::new(),
            owner: Owner::default(),
            origin: Origin { file: String::new(), line: 0 },
        });
    }
    m.functions.iter_mut().find(|f| f.sig.name == name).expect("just made")
}

/// Every `main` into the first one, every `config` into the first one.
fn merge_entries(m: &mut Module) {
    for name in ["main", "config"] {
        let parts: Vec<usize> = (0..m.functions.len()).filter(|&i| m.functions[i].sig.name == name && is_entry(&m.functions[i])).collect();
        let Some((&first, rest)) = parts.split_first() else { continue };
        let mut taken: HashSet<String> = HashSet::new();
        locals_of(&m.functions[first].body, &mut |n| {
            taken.insert(n.to_string());
        });
        let mut body = std::mem::take(&mut m.functions[first].body);
        for (k, &i) in rest.iter().enumerate() {
            let mut part = m.functions[i].body.clone();
            let mut renames = HashMap::new();
            locals_of(&part, &mut |n| {
                if taken.contains(n) {
                    let mut new = format!("{n}__{}", k + 2);
                    while taken.contains(&new) {
                        new.push('_');
                    }
                    renames.insert(n.to_string(), new);
                }
            });
            if !renames.is_empty() {
                rename(&mut part, &renames);
            }
            locals_of(&part, &mut |n| {
                taken.insert(n.to_string());
            });
            body.extend(part);
        }
        m.functions[first].body = body;
        let rest: HashSet<usize> = rest.iter().copied().collect();
        let mut i = 0;
        m.functions.retain(|_| {
            i += 1;
            !rest.contains(&(i - 1))
        });
    }
}

/// Calls `f` with the name of every local declared in `body`.
fn locals_of(body: &[Stmt], f: &mut impl FnMut(&str)) {
    for s in body {
        match s {
            Stmt::Local { name, .. } => f(name),
            Stmt::If { branches, otherwise, .. } => {
                branches.iter().for_each(|(_, b)| locals_of(b, f));
                if let Some(b) = otherwise {
                    locals_of(b, f);
                }
            }
            Stmt::Loop { body, .. } => locals_of(body, f),
            _ => {}
        }
    }
}

/// Renames variables in `body` (locals declared, set and read).
fn rename(body: &mut [Stmt], to: &HashMap<String, String>) {
    fn expr(e: &mut Expr, to: &HashMap<String, String>) {
        match e {
            Expr::Var(n) => {
                if let Some(t) = to.get(n) {
                    *n = t.clone();
                }
            }
            Expr::Index(n, i) => {
                if let Some(t) = to.get(n) {
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
            other => other.children_mut().into_iter().for_each(|x| expr(x, to)),
        }
    }
    for s in body {
        match s {
            Stmt::Local { name, init, .. } => {
                if let Some(t) = to.get(name) {
                    *name = t.clone();
                }
                init.iter_mut().for_each(|e| expr(e, to));
            }
            Stmt::Set { name, index, value, .. } => {
                if let Some(t) = to.get(name) {
                    *name = t.clone();
                }
                index.iter_mut().for_each(|e| expr(e, to));
                expr(value, to);
            }
            Stmt::Call { call, .. } => call.args.iter_mut().for_each(|a| expr(a, to)),
            Stmt::If { branches, otherwise, .. } => {
                for (c, b) in branches.iter_mut() {
                    expr(c, to);
                    rename(b, to);
                }
                if let Some(b) = otherwise {
                    rename(b, to);
                }
            }
            Stmt::Loop { body, .. } => rename(body, to),
            Stmt::ExitWhen { cond, .. } => expr(cond, to),
            Stmt::Return { value, .. } => value.iter_mut().for_each(|e| expr(e, to)),
            Stmt::Debug { stmt, .. } => rename(std::slice::from_mut(stmt), to),
            other => {
                let (es, bs) = other.oop_parts_mut();
                es.into_iter().for_each(|e| expr(e, to));
                bs.into_iter().for_each(|b| rename(b, to));
            }
        }
    }
}

/// The variables an expression reads.
fn read_vars(e: &Expr, out: &mut Vec<String>) {
    let mut f = |e: &Expr| match e {
        Expr::Var(n) | Expr::Index(n, _) => out.push(n.clone()),
        _ => {}
    };
    each_expr(&[Stmt::Return { value: Some(e.clone()), line: 0 }], &mut f);
}

/// Sorts the globals so that each comes after those its initial value
/// reads, and takes out the initial values that call the module's
/// functions: the statements that set them, for the start of `main`.
fn order_globals(m: &mut Module, errors: &mut Vec<Error>) -> Vec<Stmt> {
    let functions: HashSet<&str> = m.functions.iter().map(|f| f.sig.name.as_str()).collect();
    let mut deferred = Vec::new();
    for g in &mut m.globals {
        let Some(init) = &g.init else { continue };
        let calls = called_in(init);
        let Some(f) = calls.iter().find(|c| functions.contains(c.as_str())) else { continue };
        if g.constant {
            errors.push(Error {
                origin: g.origin.clone(),
                message: format!("the constant '{}' is set by calling '{f}', a function of the map's code, which the game reads after the globals: set it without calling one, or make it a variable", g.name),
            });
            continue;
        }
        let init = g.init.take().expect("checked");
        deferred.push(Stmt::Set { name: g.name.clone(), index: None, value: init, line: g.origin.line });
    }
    // After what each reads, in their order otherwise.
    let index: HashMap<String, usize> = m.globals.iter().enumerate().map(|(i, g)| (g.name.clone(), i)).collect();
    let deps: Vec<Vec<usize>> = m
        .globals
        .iter()
        .map(|g| {
            let mut vars = Vec::new();
            if let Some(e) = &g.init {
                read_vars(e, &mut vars);
            }
            vars.iter().filter_map(|v| index.get(v).copied()).collect()
        })
        .collect();
    let mut state = vec![0u8; m.globals.len()];
    let mut order = Vec::new();
    fn visit(i: usize, deps: &[Vec<usize>], state: &mut [u8], order: &mut Vec<usize>, cycle: &mut Option<usize>) {
        match state[i] {
            2 => return,
            1 => {
                *cycle = Some(i);
                return;
            }
            _ => {}
        }
        state[i] = 1;
        for &d in &deps[i] {
            visit(d, deps, state, order, cycle);
        }
        state[i] = 2;
        order.push(i);
    }
    let mut cycle = None;
    for i in 0..m.globals.len() {
        visit(i, &deps, &mut state, &mut order, &mut cycle);
    }
    if let Some(i) = cycle {
        let g = &m.globals[i];
        errors.push(Error { origin: g.origin.clone(), message: format!("the initial value of '{}' reads itself, through other globals", g.name) });
    }
    let old = std::mem::take(&mut m.globals);
    let mut slots: Vec<Option<Global>> = old.into_iter().map(Some).collect();
    m.globals = order.into_iter().filter_map(|i| slots[i].take()).collect();
    deferred
}

/// The functions' order: each after those it calls or names as code,
/// `config` and `main` last; within a cycle, as written.
fn sort_functions(m: &Module) -> Vec<usize> {
    let n = m.functions.len();
    let index: HashMap<&str, usize> = m.functions.iter().enumerate().map(|(i, f)| (f.sig.name.as_str(), i)).collect();
    let edges: Vec<Vec<usize>> = m
        .functions
        .iter()
        .map(|f| {
            let mut out: Vec<usize> = called(&f.body).iter().filter_map(|c| index.get(c.as_str()).copied()).collect();
            out.dedup();
            out
        })
        .collect();
    // Tarjan's strongly connected components: each comes out after every
    // component it reaches, so callees first.
    struct T<'a> {
        edges: &'a [Vec<usize>],
        index: Vec<Option<usize>>,
        low: Vec<usize>,
        on: Vec<bool>,
        stack: Vec<usize>,
        next: usize,
        out: Vec<Vec<usize>>,
    }
    impl T<'_> {
        fn visit(&mut self, v: usize) {
            self.index[v] = Some(self.next);
            self.low[v] = self.next;
            self.next += 1;
            self.stack.push(v);
            self.on[v] = true;
            for k in 0..self.edges[v].len() {
                let w = self.edges[v][k];
                match self.index[w] {
                    None => {
                        self.visit(w);
                        self.low[v] = self.low[v].min(self.low[w]);
                    }
                    Some(iw) if self.on[w] => self.low[v] = self.low[v].min(iw),
                    Some(_) => {}
                }
            }
            if Some(self.low[v]) == self.index[v] {
                let mut c = Vec::new();
                loop {
                    let w = self.stack.pop().expect("on the stack");
                    self.on[w] = false;
                    c.push(w);
                    if w == v {
                        break;
                    }
                }
                c.sort();
                self.out.push(c);
            }
        }
    }
    let mut t = T { edges: &edges, index: vec![None; n], low: vec![0; n], on: vec![false; n], stack: Vec::new(), next: 0, out: Vec::new() };
    for v in 0..n {
        if t.index[v].is_none() {
            t.visit(v);
        }
    }
    let mut order: Vec<usize> = t.out.into_iter().flatten().collect();
    // The entry points last: the game calls them.
    for name in ["config", "main"] {
        if let Some(p) = order.iter().position(|&i| m.functions[i].sig.name == name && is_entry(&m.functions[i])) {
            let i = order.remove(p);
            order.push(i);
        }
    }
    order
}

/// Every call (or code value) of a function below the caller goes
/// through that function's trigger (see the module docs): the functions
/// with what that takes added, and the globals it needs.
fn evaluate_forward_calls(functions: Vec<Function>) -> (Vec<Function>, Vec<Global>) {
    let at: HashMap<String, usize> = functions.iter().enumerate().map(|(i, f)| (f.sig.name.clone(), i)).collect();
    let mut forward: Vec<usize> = Vec::new();
    // Run in a thread of their own (`wcb__exec_F`, `.execute`).
    let mut executed: Vec<usize> = Vec::new();
    for (i, f) in functions.iter().enumerate() {
        for c in called(&f.body) {
            // Called through the trigger wherever they are (lowering's
            // `.evaluate`, `.execute`).
            let (through, exec) = match (c.strip_prefix(&format!("{PREFIX}call_")), c.strip_prefix(&format!("{PREFIX}exec_"))) {
                (Some(n), _) => (at.get(n).copied(), false),
                (_, Some(n)) => (at.get(n).copied(), true),
                _ => (None, false),
            };
            if let Some(j) = through {
                if !forward.contains(&j) {
                    forward.push(j);
                }
                if exec && !executed.contains(&j) {
                    executed.push(j);
                }
                continue;
            }
            if let Some(&j) = at.get(&c)
                && j > i
                && !forward.contains(&j)
            {
                forward.push(j);
            }
        }
    }
    if forward.is_empty() {
        return (functions, Vec::new());
    }
    forward.sort();
    let names: HashMap<String, String> = forward.iter().map(|&j| (functions[j].sig.name.clone(), format!("{PREFIX}call_{}", functions[j].sig.name))).collect();
    let mut globals = Vec::new();
    let mut callers = Vec::new();
    let mut thunks: HashMap<usize, Function> = HashMap::new();
    let mut actions: HashMap<usize, Function> = HashMap::new();
    let mut setup = Vec::new();
    let origin = |f: &Function| f.origin.clone();
    for &j in &forward {
        let f = &functions[j];
        let name = &f.sig.name;
        let trig = format!("{PREFIX}trig_{name}");
        let arg = |k: usize| format!("{PREFIX}arg_{name}_{k}");
        let ret = format!("{PREFIX}ret_{name}");
        globals.push(Global { name: trig.clone(), ty: "trigger".into(), array: false, constant: false, init: None, owner: Owner::default(), origin: origin(f) });
        for (k, p) in f.sig.params.iter().enumerate() {
            globals.push(Global { name: arg(k), ty: p.ty.clone(), array: false, constant: false, init: None, owner: Owner::default(), origin: origin(f) });
        }
        if let Some(r) = &f.sig.returns {
            globals.push(Global { name: ret.clone(), ty: r.clone(), array: false, constant: false, init: None, owner: Owner::default(), origin: origin(f) });
        }
        // Above every function: the arguments into the globals, the
        // trigger evaluated, what it returned.
        let mut body: Vec<Stmt> = f
            .sig
            .params
            .iter()
            .enumerate()
            .map(|(k, p)| Stmt::Set { name: arg(k), index: None, value: Expr::Var(p.name.clone()), line: 0 })
            .collect();
        body.push(Stmt::Call { call: Call { name: "TriggerEvaluate".into(), args: vec![Expr::Var(trig.clone())] }, line: 0 });
        if f.sig.returns.is_some() {
            body.push(Stmt::Return { value: Some(Expr::Var(ret.clone())), line: 0 });
        }
        callers.push(Function {
            sig: Signature { name: names[name].clone(), params: f.sig.params.clone(), returns: f.sig.returns.clone(), constant: false },
            body,
            owner: Owner::default(),
            origin: origin(f),
        });
        // `.execute`: the arguments into the globals, the trigger's action
        // run (a thread of its own, which reads them before it can wait).
        if executed.contains(&j) {
            let mut body: Vec<Stmt> = f.sig.params.iter().enumerate().map(|(k, p)| Stmt::Set { name: arg(k), index: None, value: Expr::Var(p.name.clone()), line: 0 }).collect();
            body.push(Stmt::Call { call: Call { name: "TriggerExecute".into(), args: vec![Expr::Var(trig.clone())] }, line: 0 });
            callers.push(Function {
                sig: Signature { name: format!("{PREFIX}exec_{name}"), params: f.sig.params.clone(), returns: None, constant: false },
                body,
                owner: Owner::default(),
                origin: origin(f),
            });
        }
        // After it: its trigger's condition calls it.
        let call = Call { name: name.clone(), args: (0..f.sig.params.len()).map(|k| Expr::Var(arg(k))).collect() };
        let mut body = vec![match f.sig.returns {
            Some(_) => Stmt::Set { name: ret.clone(), index: None, value: Expr::Call(call), line: 0 },
            None => Stmt::Call { call, line: 0 },
        }];
        let mut action = body.clone();
        body.push(Stmt::Return { value: Some(Expr::Literal("true".into())), line: 0 });
        let thunk = format!("{PREFIX}thunk_{name}");
        thunks.insert(
            j,
            Function {
                sig: Signature { name: thunk.clone(), params: Vec::new(), returns: Some("boolean".into()), constant: false },
                body,
                owner: Owner::default(),
                origin: origin(f),
            },
        );
        let action_name = format!("{PREFIX}action_{name}");
        if executed.contains(&j) {
            // An action keeps nothing it returns.
            if let Some(Stmt::Set { value: Expr::Call(call), .. }) = action.first().cloned() {
                action[0] = Stmt::Call { call, line: 0 };
            }
            actions.insert(j, Function::new(action_name.clone(), action, origin(f)));
        }
        setup.push(Stmt::Set { name: trig.clone(), index: None, value: Expr::Call(Call { name: "CreateTrigger".into(), args: Vec::new() }), line: 0 });
        setup.push(Stmt::Call {
            call: Call {
                name: "TriggerAddCondition".into(),
                args: vec![Expr::Var(trig.clone()), Expr::Call(Call { name: "Condition".into(), args: vec![Expr::Code(thunk)] })],
            },
            line: 0,
        });
        if executed.contains(&j) {
            setup.push(Stmt::call("TriggerAddAction", vec![Expr::Var(trig), Expr::Code(action_name)]));
        }
    }
    // The calls below go through the callers above.
    let mut out = callers;
    for (i, mut f) in functions.into_iter().enumerate() {
        let below: HashMap<String, String> =
            names.iter().filter(|(n, _)| at[n.as_str()] > i).map(|(n, c)| (n.clone(), c.clone())).collect();
        if !below.is_empty() {
            retarget(&mut f.body, &below);
        }
        if is_entry(&f) {
            let mut body = setup.clone();
            body.append(&mut f.body);
            f.body = body;
        }
        out.push(f);
        if let Some(t) = thunks.remove(&i) {
            out.push(t);
        }
        if let Some(a) = actions.remove(&i) {
            out.push(a);
        }
    }
    // Without a main, one of its own makes them.
    if !out.iter().any(|f| f.sig.name == "main" && is_entry(f)) {
        out.push(Function {
            sig: Signature { name: "main".into(), params: Vec::new(), returns: None, constant: false },
            body: setup,
            owner: Owner::default(),
            origin: Origin { file: String::new(), line: 0 },
        });
    }
    (out, globals)
}

/// Calls and code values of the functions `to` names, made of the ones
/// it names for them.
fn retarget(body: &mut [Stmt], to: &HashMap<String, String>) {
    fn expr(e: &mut Expr, to: &HashMap<String, String>) {
        match e {
            Expr::Call(c) => {
                if let Some(t) = to.get(&c.name) {
                    c.name = t.clone();
                }
                c.args.iter_mut().for_each(|a| expr(a, to));
            }
            Expr::Code(n) => {
                if let Some(t) = to.get(n) {
                    *n = t.clone();
                }
            }
            Expr::Index(_, i) => expr(i, to),
            Expr::Unary(_, x) => expr(x, to),
            Expr::Binary(a, _, b) => {
                expr(a, to);
                expr(b, to);
            }
            other => other.children_mut().into_iter().for_each(|x| expr(x, to)),
        }
    }
    for s in body {
        match s {
            Stmt::Local { init, .. } => init.iter_mut().for_each(|e| expr(e, to)),
            Stmt::Set { index, value, .. } => {
                index.iter_mut().for_each(|e| expr(e, to));
                expr(value, to);
            }
            Stmt::Call { call, .. } => {
                if let Some(t) = to.get(&call.name) {
                    call.name = t.clone();
                }
                call.args.iter_mut().for_each(|a| expr(a, to));
            }
            Stmt::If { branches, otherwise, .. } => {
                for (c, b) in branches.iter_mut() {
                    expr(c, to);
                    retarget(b, to);
                }
                if let Some(b) = otherwise {
                    retarget(b, to);
                }
            }
            Stmt::Loop { body, .. } => retarget(body, to),
            Stmt::ExitWhen { cond, .. } => expr(cond, to),
            Stmt::Return { value, .. } => value.iter_mut().for_each(|e| expr(e, to)),
            Stmt::Debug { stmt, .. } => retarget(std::slice::from_mut(stmt), to),
            other => {
                let (es, bs) = other.oop_parts_mut();
                es.into_iter().for_each(|e| expr(e, to));
                bs.into_iter().for_each(|b| retarget(b, to));
            }
        }
    }
}

/// `f` with its locals at its top: those declared before its first other
/// statement stay as they are; any other is declared at the top and set
/// where it was (or, without a value, only declared).
fn hoist_locals(mut f: Function) -> Function {
    let lead = f.body.iter().take_while(|s| matches!(s, Stmt::Local { .. })).count();
    let mut top: Vec<Stmt> = f.body.drain(..lead).collect();
    fn lift(body: Vec<Stmt>, top: &mut Vec<Stmt>) -> Vec<Stmt> {
        let mut out = Vec::new();
        for s in body {
            match s {
                Stmt::Local { ty, name, array, init, line } => {
                    top.push(Stmt::Local { ty, name: name.clone(), array, init: None, line });
                    if let Some(value) = init {
                        out.push(Stmt::Set { name, index: None, value, line });
                    }
                }
                Stmt::If { branches, otherwise, line } => out.push(Stmt::If {
                    branches: branches.into_iter().map(|(c, b)| (c, lift(b, top))).collect(),
                    otherwise: otherwise.map(|b| lift(b, top)),
                    line,
                }),
                Stmt::Loop { body, line } => out.push(Stmt::Loop { body: lift(body, top), line }),
                other => out.push(other),
            }
        }
        out
    }
    let rest = std::mem::take(&mut f.body);
    let rest = lift(rest, &mut top);
    top.extend(rest);
    f.body = top;
    f
}

// ------------------------------------------------------------- printing

fn print(m: &Module) -> String {
    let mut out = String::new();
    for t in &m.types {
        let _ = writeln!(out, "type {} extends {}", t.name, t.extends);
    }
    if !m.types.is_empty() {
        out.push('\n');
    }
    out += "globals\n";
    for g in &m.globals {
        let constant = if g.constant { "constant " } else { "" };
        let array = if g.array { " array" } else { "" };
        let _ = write!(out, "    {constant}{}{array} {}", g.ty, g.name);
        if let Some(e) = &g.init {
            let _ = write!(out, " = {}", expr(e, 0));
        }
        out.push('\n');
    }
    out += "endglobals\n";
    for n in &m.natives {
        let constant = if n.sig.constant { "constant " } else { "" };
        let _ = writeln!(out, "{constant}native {}", signature(&n.sig));
    }
    for f in &m.functions {
        out.push('\n');
        let constant = if f.sig.constant { "constant " } else { "" };
        let _ = writeln!(out, "{constant}function {}", signature(&f.sig));
        statements(&f.body, 1, &mut out);
        out += "endfunction\n";
    }
    out
}

fn signature(s: &Signature) -> String {
    let takes = if s.params.is_empty() {
        "nothing".to_string()
    } else {
        s.params.iter().map(|p| format!("{} {}", p.ty, p.name)).collect::<Vec<_>>().join(", ")
    };
    format!("{} takes {takes} returns {}", s.name, s.returns.as_deref().unwrap_or("nothing"))
}

fn statements(body: &[Stmt], depth: usize, out: &mut String) {
    for s in body {
        statement(s, depth, out);
    }
}

fn call(c: &Call) -> String {
    format!("{}({})", c.name, c.args.iter().map(|a| expr(a, 0)).collect::<Vec<_>>().join(", "))
}

fn statement(s: &Stmt, depth: usize, out: &mut String) {
    let pad = "    ".repeat(depth);
    match s {
        Stmt::Local { ty, name, array, init, .. } => {
            let array = if *array { " array" } else { "" };
            let _ = write!(out, "{pad}local {ty}{array} {name}");
            if let Some(e) = init {
                let _ = write!(out, " = {}", expr(e, 0));
            }
            out.push('\n');
        }
        Stmt::Set { name, index, value, .. } => {
            let index = index.as_ref().map_or(String::new(), |i| format!("[{}]", expr(i, 0)));
            let _ = writeln!(out, "{pad}set {name}{index} = {}", expr(value, 0));
        }
        Stmt::Call { call: c, .. } => {
            let _ = writeln!(out, "{pad}call {}", call(c));
        }
        Stmt::If { branches, otherwise, .. } => {
            for (i, (c, b)) in branches.iter().enumerate() {
                let word = if i == 0 { "if" } else { "elseif" };
                let _ = writeln!(out, "{pad}{word} {} then", expr(c, 0));
                statements(b, depth + 1, out);
            }
            if let Some(b) = otherwise {
                let _ = writeln!(out, "{pad}else");
                statements(b, depth + 1, out);
            }
            let _ = writeln!(out, "{pad}endif");
        }
        Stmt::Loop { body, .. } => {
            let _ = writeln!(out, "{pad}loop");
            statements(body, depth + 1, out);
            let _ = writeln!(out, "{pad}endloop");
        }
        Stmt::ExitWhen { cond, .. } => {
            let _ = writeln!(out, "{pad}exitwhen {}", expr(cond, 0));
        }
        Stmt::Return { value, .. } => match value {
            Some(v) => {
                let _ = writeln!(out, "{pad}return {}", expr(v, 0));
            }
            None => {
                let _ = writeln!(out, "{pad}return");
            }
        },
        Stmt::Debug { stmt, .. } => {
            let mut inner = String::new();
            statement(stmt, depth, &mut inner);
            let _ = write!(out, "{pad}debug {}", inner.trim_start());
        }
        Stmt::SetMember { .. } | Stmt::CallMethod { .. } | Stmt::StaticIf { .. } => unreachable!("not lowered: build checks Module::not_lowered"),
    }
}

/// How tightly an operator binds, as the game reads JASS (`and` looser
/// than `or`).
fn precedence(op: BinOp) -> u8 {
    match op {
        BinOp::And => 1,
        BinOp::Or => 2,
        BinOp::Eq | BinOp::NotEq => 3,
        BinOp::Gt | BinOp::Lt | BinOp::GtEq | BinOp::LtEq => 4,
        BinOp::Add | BinOp::Sub => 5,
        BinOp::Mul | BinOp::Div | BinOp::Mod => 6,
    }
}

const UNARY: u8 = 7;

/// `e`, in parentheses when it binds looser than `min`.
fn expr(e: &Expr, min: u8) -> String {
    let (text, p) = match e {
        Expr::Literal(t) => (t.clone(), 8),
        Expr::Var(n) => (n.clone(), 8),
        Expr::Index(n, i) => (format!("{n}[{}]", expr(i, 0)), 8),
        Expr::Call(c) => (call(c), 8),
        Expr::Code(n) => (format!("function {n}"), 8),
        Expr::Member(..) | Expr::Method(_) | Expr::Element(..) => unreachable!("not lowered: build checks Module::not_lowered"),
        Expr::Unary(op, x) => {
            let inner = expr(x, UNARY);
            let text = match op {
                UnaryOp::Not => format!("not {inner}"),
                // `- -x` reads as two signs, never as a comment.
                UnaryOp::Neg if inner.starts_with('-') => format!("- {inner}"),
                UnaryOp::Neg => format!("-{inner}"),
                UnaryOp::Plus => format!("+{inner}"),
            };
            (text, UNARY)
        }
        Expr::Binary(a, op, b) => {
            let p = precedence(*op);
            let sign = match op {
                BinOp::Add => "+",
                BinOp::Sub => "-",
                BinOp::Mul => "*",
                BinOp::Div => "/",
                BinOp::Eq => "==",
                BinOp::NotEq => "!=",
                BinOp::Gt => ">",
                BinOp::Lt => "<",
                BinOp::GtEq => ">=",
                BinOp::LtEq => "<=",
                BinOp::And => "and",
                BinOp::Mod => unreachable!("not lowered: build checks Module::not_lowered"),
                BinOp::Or => "or",
            };
            // Left to right: the right side binds tighter.
            (format!("{} {sign} {}", expr(a, p), expr(b, p + 1)), p)
        }
    };
    if p < min { format!("({text})") } else { text }
}

#[cfg(test)]
#[path = "jass.test.rs"]
mod tests;
