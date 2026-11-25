//! What vJASS and Zinc add, lowered into what the game's JASS has (the
//! build's stage 4), so the JASS output ([`crate::jass`]) gets a module of
//! JASS only. AngelScript (`lower_for` with `classes`) gets all of it
//! but structs and interfaces, which stay classes.
//!
//! - **Libraries and scopes.** Each library after those it `requires`
//!   (one it requires that isn't there is an error, unless `optional`; so
//!   is a cycle). A name is its scope's: a private one is
//!   `<scope>__<name>` (a scope in a scope: `<outer>_<inner>__<name>`), a
//!   public one `<scope>_<name>`, anyone else's as written. A name used in
//!   a scope is the scope's own, else its parent's ..., else everyone's;
//!   parameters and locals first.
//! - **Initializers** run at map initialization: a library's and a
//!   struct's `onInit` each in a thread of its own (`ExecuteFunc`), a
//!   scope's called straight (as JassHelper's in Diablo III beta; the
//!   others aren't seen in a map yet): the libraries' in the order
//!   above, then the scopes'. They're called right after
//!   `InitBlizzard()` (as JassHelper does: before `InitGlobals` sets the
//!   triggers' variables), or, in a module without it, are a `main` of
//!   their own after the first one.
//! - **`static if`**: its branch chosen by constants: `LIBRARY_<name>`
//!   (whether the library is there), `DEBUG_MODE` (false), a constant
//!   boolean global, and `not`, `and`, `or`, `==`, `!=` of them.
//! - **`%`**: `ModuloInteger(a, b)`, `ModuloReal` when a side is a real
//!   literal.
//! - A trigger's `InitTrig_<name>` that no code has (custom text that's a
//!   scope or a library) isn't called, as JassHelper does.
//!
//! Structs, interfaces, modules and members are lowered next (they need
//! the types); until then the JASS output says what's left.

use std::collections::{HashMap, HashSet};

use crate::ir::*;

const ORIGIN: &str = "the build";

/// `m` with its libraries and scopes, initializers, `static if`s and `%`
/// lowered.
pub fn lower(m: &Module) -> Result<Module, Vec<Error>> {
    lower_with(m, &[])
}

/// [`lower`], with the game's natives and functions (`game`: what a hook
/// of one takes and gives back).
pub fn lower_with(m: &Module, game: &[Signature]) -> Result<Module, Vec<Error>> {
    lower_for(m, game, false)
}

/// [`lower_with`]; with `classes` (AngelScript's) structs and interfaces
/// are kept as classes ([`crate::structs::lower`]).
pub fn lower_for(m: &Module, game: &[Signature], classes: bool) -> Result<Module, Vec<Error>> {
    let mut m = m.clone();
    let mut errors = Vec::new();
    let order = library_order(&m, &mut errors);
    if !errors.is_empty() {
        return Err(errors);
    }
    let scopes = Scopes::of(&m);
    let present: HashSet<String> = m.libraries.iter().map(|l| l.name.clone()).collect();
    mixins(&mut m, &mut errors);

    // Every name a scope declares, to what it becomes.
    let mut declared: HashMap<(String, String), String> = HashMap::new();
    let mut declare = |owner: &Owner, name: &str| {
        if let Some(s) = &owner.scope {
            let to = match owner.access {
                Access::Everyone => name.to_string(),
                Access::Public => format!("{s}_{name}"),
                Access::Private => format!("{}__{name}", scopes.path(s)),
            };
            declared.insert((s.clone(), name.to_string()), to);
        }
    };
    for g in &m.globals {
        declare(&g.owner, &g.name);
    }
    for f in &m.functions {
        declare(&f.owner, &f.sig.name);
    }
    for t in &m.types {
        declare(&t.owner, &t.name);
    }
    for s in &m.structs {
        declare(&s.owner, &s.name);
    }
    for i in &m.interfaces {
        declare(&i.owner, &i.name);
    }
    for t in &m.function_types {
        declare(&t.owner, &t.sig.name);
    }
    let resolve = |scope: Option<&str>, name: &str| -> String {
        let mut s = scope.map(String::from);
        while let Some(cur) = s {
            if let Some(to) = declared.get(&(cur.clone(), name.to_string())) {
                return to.clone();
            }
            s = scopes.parent.get(&cur).cloned().flatten();
        }
        name.to_string()
    };

    // Hooks, their names resolved in their scopes; made at the end, once
    // every function is what it becomes (`hooks`).
    let hooks: Vec<(String, String, Origin)> = std::mem::take(&mut m.hooks)
        .into_iter()
        .map(|h| {
            let scope = h.owner.scope.as_deref();
            let target = match h.target.split_once('.') {
                Some((s, method)) => format!("{}.{method}", resolve(scope, s)),
                None => resolve(scope, &h.target),
            };
            (resolve(scope, &h.name), target, h.origin)
        })
        .collect();

    // Initializers, resolved in their scopes, before names change: the
    // libraries' in their order, then the scopes'.
    let mut initializers: Vec<(String, Option<String>)> = Vec::new();
    let libraries: Vec<&Library> = order.iter().map(|&i| &m.libraries[i]).filter(|l| !l.scope).chain(m.libraries.iter().filter(|l| l.scope)).collect();
    for l in libraries {
        let Some(f) = &l.initializer else {
            initializers.push((l.name.clone(), None));
            continue;
        };
        let name = resolve(Some(&l.name), f);
        let found = m.functions.iter().find(|g| g.owner.scope.as_deref() == Some(l.name.as_str()) && g.sig.name == *f).or_else(|| m.functions.iter().find(|g| g.sig.name == name));
        match found {
            Some(g) if g.sig.params.is_empty() => initializers.push((l.name.clone(), Some(name))),
            Some(_) => errors.push(Error { origin: l.origin.clone(), message: format!("{}'s initializer {f} takes arguments: it can't run at initialization", l.name) }),
            None => errors.push(Error { origin: l.origin.clone(), message: format!("{}'s initializer {f} is declared nowhere", l.name) }),
        }
    }

    // Constant booleans, for `static if`.
    let mut constants: HashMap<String, bool> = HashMap::new();
    for g in m.globals.iter().filter(|g| g.constant && g.ty == "boolean") {
        if let Some(Expr::Literal(v)) = &g.init
            && let Ok(b) = v.parse::<bool>()
        {
            constants.insert(resolve(g.owner.scope.as_deref(), &g.name), b);
        }
    }

    // Function interfaces' names, as written: `I.f`'s `f` is a function.
    let ftypes: HashSet<String> = m.function_types.iter().map(|t| t.sig.name.clone()).collect();
    for g in &mut m.globals {
        let scope = g.owner.scope.clone();
        if let Some(e) = &mut g.init {
            names_in_expr(e, &|n| resolve(scope.as_deref(), n), &HashSet::new(), &ftypes);
            modulo(e);
        }
        g.ty = resolve(scope.as_deref(), &g.ty);
        g.name = resolve(scope.as_deref(), &g.name);
        g.owner = Owner::default();
    }
    for t in &mut m.types {
        let scope = t.owner.scope.clone();
        t.name = resolve(scope.as_deref(), &t.name);
        t.owner = Owner::default();
    }
    for f in &mut m.functions {
        let scope = f.owner.scope.clone();
        let r = |n: &str| resolve(scope.as_deref(), n);
        let mut own: HashSet<String> = f.sig.params.iter().map(|p| p.name.clone()).collect();
        locals(&f.body, &mut own);
        for p in &mut f.sig.params {
            p.ty = r(&p.ty);
        }
        if let Some(t) = &mut f.sig.returns {
            *t = r(t);
        }
        let library = |n: &str| present.contains(n);
        let constant = |n: &str| constants.get(&r(n)).copied();
        f.body = static_ifs(std::mem::take(&mut f.body), &f.origin.file, &library, &constant, &mut errors);
        names(&mut f.body, &r, &own, &ftypes);
        f.body.iter_mut().for_each(modulo_stmt);
        f.sig.name = r(&f.sig.name);
        f.owner = Owner::default();
    }
    for st in &mut m.structs {
        let scope = st.owner.scope.clone();
        let r = |n: &str| resolve(scope.as_deref(), n);
        st.name = r(&st.name);
        if let Some(e) = &mut st.extends {
            *e = r(e);
        }
        for f in &mut st.fields {
            f.ty = r(&f.ty);
            if let Some(e) = &mut f.init {
                names_in_expr(e, &r, &HashSet::new(), &ftypes);
                modulo(e);
            }
        }
        for me in &mut st.methods {
            let f = &mut me.function;
            let mut own: HashSet<String> = f.sig.params.iter().map(|p| p.name.clone()).collect();
            own.extend(["this".to_string(), "thistype".to_string()]);
            locals(&f.body, &mut own);
            for p in &mut f.sig.params {
                p.ty = r(&p.ty);
            }
            if let Some(t) = &mut f.sig.returns {
                *t = r(t);
            }
            let library = |n: &str| present.contains(n);
            let constant = |n: &str| constants.get(&r(n)).copied();
            f.body = static_ifs(std::mem::take(&mut f.body), &f.origin.file, &library, &constant, &mut errors);
            names(&mut f.body, &r, &own, &ftypes);
            f.body.iter_mut().for_each(modulo_stmt);
        }
        st.owner = Owner { scope, access: Access::Everyone };
    }
    for i in &mut m.interfaces {
        let scope = i.owner.scope.clone();
        let r = |n: &str| resolve(scope.as_deref(), n);
        i.name = r(&i.name);
        for me in &mut i.methods {
            for p in &mut me.function.sig.params {
                p.ty = r(&p.ty);
            }
            if let Some(t) = &mut me.function.sig.returns {
                *t = r(t);
            }
            if let Some(d) = &mut me.defaults {
                names_in_expr(d, &r, &HashSet::new(), &ftypes);
            }
        }
        i.owner = Owner { scope, access: Access::Everyone };
    }
    for t in &mut m.function_types {
        let scope = t.owner.scope.clone();
        let r = |n: &str| resolve(scope.as_deref(), n);
        t.sig.name = r(&t.sig.name);
        for p in &mut t.sig.params {
            p.ty = r(&p.ty);
        }
        if let Some(x) = &mut t.sig.returns {
            *x = r(x);
        }
        t.owner = Owner::default();
    }
    let lowered = crate::structs::lower(&mut m, game, classes, &mut errors);
    let struct_inits = lowered.inits;

    // Each library's structs' `onInit`s before its initializer; those of
    // no library first.
    // The families' triggers first: initializers may use them.
    let mut init: Vec<Stmt> = lowered.setup;
    let run = |f: &str| Stmt::call("ExecuteFunc", vec![Expr::string(f)]);
    for (scope, f) in &struct_inits {
        if scope.is_none() {
            init.push(run(f));
        }
    }
    for (lib, f) in &initializers {
        for (scope, g) in &struct_inits {
            if scope.as_deref() == Some(lib.as_str()) {
                init.push(run(g));
            }
        }
        // A scope's called straight, as JassHelper does (Diablo III beta's);
        // a library's in a thread of its own.
        if let Some(f) = f {
            let scope = m.libraries.iter().any(|l| &l.name == lib && l.scope);
            init.push(if scope { Stmt::call(f.clone(), Vec::new()) } else { run(f) });
        }
    }
    m.libraries.clear();
    // A trigger's `InitTrig_<name>` its code doesn't have (custom text
    // that's a scope or a library, with an initializer of its own): not
    // called, as JassHelper does.
    let defined: HashSet<String> = m.functions.iter().map(|f| f.sig.name.clone()).collect();
    for f in m.functions.iter_mut().filter(|f| f.sig.name == "main") {
        f.body.retain(|s| !matches!(s, Stmt::Call { call, .. } if call.name.starts_with("InitTrig_") && call.args.is_empty() && !defined.contains(&call.name)));
    }
    if !init.is_empty() {
        // Right after `InitBlizzard()`, as JassHelper (before the triggers'
        // variables are set: `InitGlobals`); else a `main` of their own
        // after the first.
        let blizzard = m.functions.iter_mut().filter(|f| f.sig.name == "main").find_map(|f| {
            let at = f.body.iter().position(|s| matches!(s, Stmt::Call { call, .. } if call.name == "InitBlizzard"))?;
            Some((f, at))
        });
        match blizzard {
            Some((f, at)) => {
                f.body.splice(at + 1..at + 1, init);
            }
            None => {
                let main = Function::new("main", init, Origin::file(ORIGIN));
                let at = m.functions.iter().position(|f| f.sig.name == "main").map_or(0, |i| i + 1);
                m.functions.insert(at, main);
            }
        }
    }
    apply_hooks(&mut m, hooks, game, &mut errors);
    if errors.is_empty() { Ok(m) } else { Err(errors) }
}

/// The hooks (each a name, its target, where it's written), as JassHelper
/// makes them: for each name hooked, `h__<name>` taking what it takes
/// calls each target (a struct's method `S.m`: its function) in turn,
/// then the name itself, and gives back what that gives; every call of
/// the name but in the targets themselves (a hook calling what it hooks
/// would run itself) is a call of it. A target must take what the name
/// takes.
fn apply_hooks(m: &mut Module, hooks: Vec<(String, String, Origin)>, game: &[Signature], errors: &mut Vec<Error>) {
    if hooks.is_empty() {
        return;
    }
    let sig_of = |m: &Module, n: &str| m.functions.iter().map(|f| &f.sig).chain(m.natives.iter().map(|x| &x.sig)).chain(game).find(|s| s.name == n).cloned();
    let mut hooked: Vec<(String, Signature, Vec<String>)> = Vec::new();
    for (name, target, origin) in hooks {
        let Some(sig) = sig_of(m, &name) else {
            errors.push(Error { origin, message: format!("hook {name}: no native or function {name}") });
            continue;
        };
        // `S.m`: the struct's method, as the structs made it.
        let function = match target.split_once('.') {
            Some((s, method)) => format!("s__{s}_{method}"),
            None => target.clone(),
        };
        let Some(t) = sig_of(m, &function) else {
            errors.push(Error { origin, message: format!("hook {name} {target}: no function or static method {target}") });
            continue;
        };
        if t.params.iter().map(|p| &p.ty).ne(sig.params.iter().map(|p| &p.ty)) {
            errors.push(Error { origin, message: format!("hook {name} {target}: {target} doesn't take what {name} takes") });
            continue;
        }
        match hooked.iter_mut().find(|(n, _, _)| *n == name) {
            Some((_, _, targets)) => targets.push(function),
            None => hooked.push((name, sig, vec![function])),
        }
    }
    let targets: HashSet<String> = hooked.iter().flat_map(|(_, _, t)| t.iter().cloned()).collect();
    let wrapper = |n: &str| format!("h__{n}");
    let names: HashMap<String, String> = hooked.iter().map(|(n, _, _)| (n.clone(), wrapper(n))).collect();
    for f in m.functions.iter_mut().filter(|f| !targets.contains(&f.sig.name)) {
        rename_calls(&mut f.body, &names);
    }
    for (name, sig, targets) in hooked {
        let args: Vec<Expr> = sig.params.iter().map(|p| Expr::var(p.name.clone())).collect();
        let mut body: Vec<Stmt> = targets.into_iter().map(|t| Stmt::call(t, args.clone())).collect();
        let call = Call { name: name.clone(), args };
        body.push(match &sig.returns {
            Some(_) => Stmt::Return { value: Some(Expr::Call(call)), line: 0 },
            None => Stmt::Call { call, line: 0 },
        });
        let mut f = Function::new(wrapper(&name), body, Origin::file(ORIGIN));
        f.sig.params = sig.params.clone();
        f.sig.returns = sig.returns.clone();
        m.functions.push(f);
    }
}

/// Every call of a name of `names` in `body` a call of what it maps to.
fn rename_calls(body: &mut [Stmt], names: &HashMap<String, String>) {
    fn expr(e: &mut Expr, names: &HashMap<String, String>) {
        if let Expr::Call(c) = e
            && let Some(to) = names.get(&c.name)
        {
            c.name = to.clone();
        }
        for c in e.children_mut() {
            expr(c, names);
        }
    }
    for s in body {
        match s {
            Stmt::Call { call, .. } => {
                if let Some(to) = names.get(&call.name) {
                    call.name = to.clone();
                }
                call.args.iter_mut().for_each(|a| expr(a, names));
            }
            Stmt::Local { init, .. } => init.iter_mut().for_each(|e| expr(e, names)),
            Stmt::Set { index, value, .. } => {
                index.iter_mut().for_each(|e| expr(e, names));
                expr(value, names);
            }
            Stmt::If { branches, otherwise, .. } => {
                for (c, b) in branches {
                    expr(c, names);
                    rename_calls(b, names);
                }
                if let Some(b) = otherwise {
                    rename_calls(b, names);
                }
            }
            Stmt::Loop { body, .. } => rename_calls(body, names),
            Stmt::ExitWhen { cond, .. } => expr(cond, names),
            Stmt::Return { value, .. } => value.iter_mut().for_each(|e| expr(e, names)),
            Stmt::Debug { stmt, .. } => rename_calls(std::slice::from_mut(&mut **stmt), names),
            other => {
                let (es, bs) = other.oop_parts_mut();
                es.into_iter().for_each(|e| expr(e, names));
                bs.into_iter().for_each(|b| rename_calls(b, names));
            }
        }
    }
}

/// Each struct's modules taken in (theirs too): their fields and
/// methods, a module's `onInit` named `onInit__<module>`, so each runs.
fn mixins(m: &mut Module, errors: &mut Vec<Error>) {
    let mixins: HashMap<String, Mixin> = m.mixins.drain(..).map(|x| (x.name.clone(), x)).collect();
    for s in &mut m.structs {
        let mut todo: Vec<(String, bool)> = std::mem::take(&mut s.mixins);
        let mut seen: HashSet<String> = HashSet::new();
        while let Some((name, optional)) = todo.pop() {
            if !seen.insert(name.clone()) {
                continue;
            }
            match mixins.get(&name) {
                Some(x) => {
                    s.fields.extend(x.fields.iter().cloned());
                    for me in &x.methods {
                        let mut me = me.clone();
                        if me.function.sig.name == "onInit" {
                            me.function.sig.name = format!("onInit__{name}");
                        }
                        s.methods.push(me);
                    }
                    todo.extend(x.mixins.iter().cloned());
                }
                None if optional => {}
                None => errors.push(Error { origin: s.origin.clone(), message: format!("struct {} implements {name}, which isn't there", s.name) }),
            }
        }
    }
}

/// Each scope's parent, and its path of names.
struct Scopes {
    parent: HashMap<String, Option<String>>,
}

impl Scopes {
    fn of(m: &Module) -> Scopes {
        Scopes { parent: m.libraries.iter().map(|l| (l.name.clone(), l.parent.clone())).collect() }
    }

    /// `Outer_Inner` for a scope in a scope.
    fn path(&self, scope: &str) -> String {
        let mut names = vec![scope.to_string()];
        let mut s = self.parent.get(scope).cloned().flatten();
        while let Some(p) = s {
            s = self.parent.get(&p).cloned().flatten();
            names.push(p);
        }
        names.reverse();
        names.join("_")
    }
}

/// The libraries' indices, each after those it requires (in the order
/// written otherwise); errors for one required that isn't there, a
/// cycle, a name given twice.
fn library_order(m: &Module, errors: &mut Vec<Error>) -> Vec<usize> {
    let mut index: HashMap<&str, usize> = HashMap::new();
    for (i, l) in m.libraries.iter().enumerate() {
        if let Some(&j) = index.get(l.name.as_str()) {
            errors.push(Error { origin: l.origin.clone(), message: format!("{} again (the first at {}:{})", l.name, m.libraries[j].origin.file, m.libraries[j].origin.line) });
        }
        index.insert(&l.name, i);
    }
    let mut deps: Vec<Vec<usize>> = Vec::new();
    for l in &m.libraries {
        let mut d = Vec::new();
        for (r, optional) in &l.requires {
            match index.get(r.as_str()) {
                Some(&j) if !m.libraries[j].scope => d.push(j),
                Some(_) => errors.push(Error { origin: l.origin.clone(), message: format!("{} requires {r}, a scope: only a library can be required", l.name) }),
                None if *optional => {}
                None => errors.push(Error { origin: l.origin.clone(), message: format!("{} requires {r}, which isn't there", l.name) }),
            }
        }
        deps.push(d);
    }
    // 0 unseen, 1 being visited, 2 done.
    let mut state = vec![0u8; deps.len()];
    let mut order = Vec::new();
    fn visit(i: usize, deps: &[Vec<usize>], state: &mut [u8], order: &mut Vec<usize>) -> Option<usize> {
        match state[i] {
            2 => return None,
            1 => return Some(i),
            _ => {}
        }
        state[i] = 1;
        for &d in &deps[i] {
            if let Some(c) = visit(d, deps, state, order) {
                return Some(c);
            }
        }
        state[i] = 2;
        order.push(i);
        None
    }
    for i in 0..deps.len() {
        if let Some(c) = visit(i, &deps, &mut state, &mut order) {
            let l = &m.libraries[c];
            errors.push(Error { origin: l.origin.clone(), message: format!("{} requires itself, through the libraries it requires", l.name) });
            break;
        }
    }
    order
}

/// The names of `body`'s locals.
fn locals(body: &[Stmt], out: &mut HashSet<String>) {
    for s in body {
        match s {
            Stmt::Local { name, .. } => {
                out.insert(name.clone());
            }
            Stmt::If { branches, otherwise, .. } => {
                branches.iter().for_each(|(_, b)| locals(b, out));
                if let Some(b) = otherwise {
                    locals(b, out);
                }
            }
            Stmt::Loop { body, .. } => locals(body, out),
            Stmt::StaticIf { then, otherwise, .. } => {
                locals(then, out);
                if let Some(b) = otherwise {
                    locals(b, out);
                }
            }
            _ => {}
        }
    }
}

/// Every name `body` uses that isn't `own` (its parameters and locals) as
/// `to` says; types of locals too.
fn names(body: &mut [Stmt], to: &dyn Fn(&str) -> String, own: &HashSet<String>, ftypes: &HashSet<String>) {
    let name = |n: &mut String| {
        if !own.contains(n.as_str()) {
            *n = to(n);
        }
    };
    for s in body {
        match s {
            Stmt::Local { ty, init, .. } => {
                *ty = to(ty);
                if let Some(e) = init {
                    names_in_expr(e, to, own, ftypes);
                }
            }
            Stmt::Set { name: n, index, value, .. } => {
                name(n);
                if let Some(i) = index {
                    names_in_expr(i, to, own, ftypes);
                }
                names_in_expr(value, to, own, ftypes);
            }
            Stmt::Call { call, .. } => {
                call.name = to(&call.name);
                call.args.iter_mut().for_each(|a| names_in_expr(a, to, own, ftypes));
            }
            Stmt::If { branches, otherwise, .. } => {
                for (c, b) in branches {
                    names_in_expr(c, to, own, ftypes);
                    names(b, to, own, ftypes);
                }
                if let Some(b) = otherwise {
                    names(b, to, own, ftypes);
                }
            }
            Stmt::Loop { body, .. } => names(body, to, own, ftypes),
            Stmt::ExitWhen { cond, .. } => names_in_expr(cond, to, own, ftypes),
            Stmt::Return { value, .. } => {
                if let Some(e) = value {
                    names_in_expr(e, to, own, ftypes);
                }
            }
            Stmt::Debug { stmt, .. } => names(std::slice::from_mut(stmt), to, own, ftypes),
            other => {
                let (es, bs) = other.oop_parts_mut();
                es.into_iter().for_each(|e| names_in_expr(e, to, own, ftypes));
                bs.into_iter().for_each(|b| names(b, to, own, ftypes));
            }
        }
    }
}

fn names_in_expr(e: &mut Expr, to: &dyn Fn(&str) -> String, own: &HashSet<String>, ftypes: &HashSet<String>) {
    match e {
        Expr::Var(n) | Expr::Index(n, _) if !own.contains(n.as_str()) => *n = to(n),
        Expr::Call(c) => c.name = to(&c.name),
        // `I.f` of a function interface `I`: the function, by its name there.
        Expr::Member(o, f) if matches!(&**o, Expr::Var(i) if ftypes.contains(i) && !own.contains(i)) => *f = to(f),
        // `function Data.onTimer`: the struct's name.
        Expr::Code(n) => {
            *n = match n.split_once('.') {
                Some((s, m)) => format!("{}.{m}", to(s)),
                None => to(n),
            }
        }
        _ => {}
    }
    for c in e.children_mut() {
        names_in_expr(c, to, own, ftypes);
    }
}

/// `static if`s made into the branch their constants choose.
fn static_ifs(body: Vec<Stmt>, file: &str, library: &dyn Fn(&str) -> bool, constant: &dyn Fn(&str) -> Option<bool>, errors: &mut Vec<Error>) -> Vec<Stmt> {
    let mut out = Vec::new();
    for s in body {
        match s {
            Stmt::StaticIf { cond, then, otherwise, line } => match constant_bool(&cond, library, constant) {
                Some(true) => out.extend(static_ifs(then, file, library, constant, errors)),
                Some(false) => out.extend(static_ifs(otherwise.unwrap_or_default(), file, library, constant, errors)),
                None => {
                    errors.push(Error { origin: Origin { file: file.into(), line }, message: "a `static if` of what isn't a constant boolean".into() });
                }
            },
            Stmt::If { branches, otherwise, line } => out.push(Stmt::If {
                branches: branches.into_iter().map(|(c, b)| (c, static_ifs(b, file, library, constant, errors))).collect(),
                otherwise: otherwise.map(|b| static_ifs(b, file, library, constant, errors)),
                line,
            }),
            Stmt::Loop { body, line } => out.push(Stmt::Loop { body: static_ifs(body, file, library, constant, errors), line }),
            s => out.push(s),
        }
    }
    out
}

/// What a `static if`'s condition is.
fn constant_bool(e: &Expr, library: &dyn Fn(&str) -> bool, constant: &dyn Fn(&str) -> Option<bool>) -> Option<bool> {
    let f = |x: &Expr| constant_bool(x, library, constant);
    match e {
        Expr::Literal(v) => v.parse().ok(),
        Expr::Var(n) if n == "DEBUG_MODE" => Some(false),
        Expr::Var(n) => match n.strip_prefix("LIBRARY_") {
            Some(l) => Some(library(l)),
            None => constant(n),
        },
        Expr::Unary(UnaryOp::Not, x) => f(x).map(|b| !b),
        Expr::Binary(a, op, b) => {
            let (a, b) = (f(a)?, f(b)?);
            match op {
                BinOp::And => Some(a && b),
                BinOp::Or => Some(a || b),
                BinOp::Eq => Some(a == b),
                BinOp::NotEq => Some(a != b),
                _ => None,
            }
        }
        _ => None,
    }
}

/// `a % b` as `ModuloInteger(a, b)` (`ModuloReal` when a side is a real).
fn modulo(e: &mut Expr) {
    for c in e.children_mut() {
        modulo(c);
    }
    if let Expr::Binary(a, BinOp::Mod, b) = e {
        let real = |x: &Expr| matches!(x, Expr::Literal(t) if t.contains('.'));
        let f = if real(a) || real(b) { "ModuloReal" } else { "ModuloInteger" };
        *e = Expr::call(f, vec![(**a).clone(), (**b).clone()]);
    }
}

fn modulo_stmt(s: &mut Stmt) {
    let mut each = |e: &mut Expr| modulo(e);
    match s {
        Stmt::Local { init, .. } => init.iter_mut().for_each(&mut each),
        Stmt::Set { index, value, .. } => {
            index.iter_mut().for_each(&mut each);
            each(value);
        }
        Stmt::Call { call, .. } => call.args.iter_mut().for_each(each),
        Stmt::If { branches, otherwise, .. } => {
            for (c, b) in branches {
                each(c);
                b.iter_mut().for_each(modulo_stmt);
            }
            if let Some(b) = otherwise {
                b.iter_mut().for_each(modulo_stmt);
            }
        }
        Stmt::Loop { body, .. } => body.iter_mut().for_each(modulo_stmt),
        Stmt::ExitWhen { cond, .. } => each(cond),
        Stmt::Return { value, .. } => value.iter_mut().for_each(each),
        Stmt::Debug { stmt, .. } => modulo_stmt(stmt),
        other => {
            let (es, bs) = other.oop_parts_mut();
            es.into_iter().for_each(modulo);
            bs.into_iter().for_each(|b| b.iter_mut().for_each(modulo_stmt));
        }
    }
}

#[cfg(test)]
#[path = "lower.test.rs"]
mod tests;
