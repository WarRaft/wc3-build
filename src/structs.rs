//! Structs and interfaces lowered into what the game's JASS has (part of
//! [`crate::lower`]), as JassHelper does:
//!
//! - an instance's field `f` of struct `S` is the array `s__S_f`, indexed
//!   by the instance (a field array of size `k`: by `this * k + i`); a
//!   static field is the global `s__S_f`; a parent's field is the
//!   parent's;
//! - a method is the function `s__S_m` taking the instance first (`this`),
//!   a static one without it;
//! - instances (but for a struct `extends array`) come from a free list
//!   of their family's root (the interface, or the topmost struct):
//!   `s__S__allocate` takes the last one given back (`si__R_F`, each one's
//!   next in `si__R_V`) or a new one (`si__R_I`), past its size none (0),
//!   sets its type (`si__R_type`, in a family of several) and the fields
//!   of it and its parents that have a value; destroying runs `onDestroy`
//!   (the struct's, then its parents') and gives the instance back (one
//!   not taken is left alone), `deallocate` gives it back only.
//!   `S.create(...)` is the struct's own `create`, or `allocate`;
//! - a family of several (an interface's, a struct's with children)
//!   calls a method of an interface, a `stub` method and `destroy` by the
//!   instance's type: `wcb__d_R_m` puts the arguments into globals of its
//!   own and evaluates the trigger of the type (`wcb__dt_R_m[type]`),
//!   whose condition (`wcb__dthunk_S_m`) calls the struct's method and
//!   keeps what it returns; a type without one returns the interface's
//!   `defaults`. Every other call is the struct's (or the parent's that
//!   has it) straight; `super.m()` the parent's. `S.typeid` is the
//!   struct's type, `x.getType()` the instance's;
//! - a module's members are its struct's (`implement`); every static
//!   `onInit` runs at map initialization, its modules' before the
//!   struct's;
//! - a struct's or interface's type is `integer`; `S(x)` and
//!   `thistype(x)` are `x`;
//! - `F.evaluate(...)` and `F.execute(...)` of a function or a method go
//!   through its trigger, the arguments in globals (`wcb__call_F`,
//!   `wcb__exec_F`, which the JASS output makes).
//!
//! What a name is (`c.value`: which struct's) comes from the types:
//! parameters, locals, globals, fields, what functions and methods
//! return. Not lowered yet (an error each): `delegate`, operators,
//! function interfaces, a struct of more than 8190 instances.

use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap};

use crate::ir::*;

/// Instances a struct has without a size.
const DEFAULT_SIZE: i64 = 8191;

/// A field: its type, whether static, an instance's array's size.
type FieldInfo = (String, bool, Option<i64>);

#[derive(Clone)]
struct MethodInfo {
    is_static: bool,
    returns: Option<String>,
    params: Vec<Param>,
    stub: bool,
    /// An interface's: no body.
    abstract_: bool,
    defaults: Option<Expr>,
}

struct Info {
    name: String,
    interface: bool,
    /// `extends array`: no allocator.
    array: bool,
    /// The struct or interface it extends.
    parent: Option<String>,
    /// Field: its type, static, array size (instances' fields).
    fields: HashMap<String, FieldInfo>,
    methods: HashMap<String, MethodInfo>,
    /// Its type in its family (from 1), 0 for an interface.
    typeid: usize,
}

impl Info {
    fn field(&self, f: &str) -> String {
        format!("s__{}_{f}", self.name)
    }
    fn method(&self, m: &str) -> String {
        format!("s__{}_{m}", self.name)
    }
}

/// What lowering the structs gives the rest: the `onInit`s to run, by the
/// scope each struct is in, and what makes the families' triggers (to
/// run before them).
pub struct Lowered {
    pub inits: Vec<(Option<String>, String)>,
    pub setup: Vec<Stmt>,
}

struct Types {
    infos: HashMap<String, Info>,
    /// Each family of several (its root): its structs, by their types.
    families: HashMap<String, Vec<String>>,
}

impl Types {
    fn root<'a>(&'a self, t: &'a str) -> &'a str {
        let mut t = t;
        for _ in 0..64 {
            match self.infos.get(t).and_then(|i| i.parent.as_deref()) {
                Some(p) if self.infos.contains_key(p) => t = p,
                _ => break,
            }
        }
        t
    }

    fn several(&self, t: &str) -> bool {
        self.families.contains_key(self.root(t))
    }

    /// The type and its parents, itself first.
    fn chain(&self, t: &str) -> Vec<&Info> {
        let mut out = Vec::new();
        let mut cur = self.infos.get(t);
        while let Some(i) = cur {
            if out.len() > 64 {
                break;
            }
            out.push(i);
            cur = i.parent.as_deref().and_then(|p| self.infos.get(p));
        }
        out
    }

    /// The field `f` of `t` (or a parent's): whose it is.
    fn field(&self, t: &str, f: &str) -> Option<(&Info, &FieldInfo)> {
        self.chain(t).into_iter().find_map(|i| i.fields.get(f).map(|x| (i, x)))
    }

    /// The method `m` of `t`: the nearest that has it with a body, else
    /// the nearest that declares it.
    fn method(&self, t: &str, m: &str) -> Option<(&Info, &MethodInfo)> {
        let chain = self.chain(t);
        chain
            .iter()
            .find_map(|i| i.methods.get(m).filter(|x| !x.abstract_).map(|x| (*i, x)))
            .or_else(|| chain.iter().find_map(|i| i.methods.get(m).map(|x| (*i, x))))
    }

    /// Does a call of `m` on a `t` go by the instance's type?
    fn dispatches(&self, t: &str, m: &str) -> bool {
        if !self.several(t) {
            return false;
        }
        self.chain(t).iter().any(|i| i.methods.get(m).is_some_and(|x| x.stub || x.abstract_))
    }

    fn type_global(&self, t: &str) -> String {
        format!("si__{}_type", self.root(t))
    }
}

/// The structs and interfaces of `m` (modules taken in, names already
/// resolved) lowered: their globals and functions added, every member
/// written as they are, their types as `integer`.
pub fn lower(m: &mut Module, errors: &mut Vec<Error>) -> Lowered {
    let structs = std::mem::take(&mut m.structs);
    let interfaces = std::mem::take(&mut m.interfaces);
    let mut infos: HashMap<String, Info> = HashMap::new();
    let method_info = |me: &Method, owner: &str| MethodInfo {
        is_static: me.is_static,
        returns: me.function.sig.returns.as_ref().map(|r| if r == "thistype" { owner.to_string() } else { r.clone() }),
        params: me.function.sig.params.clone(),
        stub: me.stub,
        abstract_: me.abstract_,
        defaults: me.defaults.clone(),
    };
    for i in &interfaces {
        let methods = i.methods.iter().map(|me| (me.function.sig.name.clone(), method_info(me, &i.name))).collect();
        infos.insert(i.name.clone(), Info { name: i.name.clone(), interface: true, array: false, parent: None, fields: HashMap::new(), methods, typeid: 0 });
    }
    for s in &structs {
        let err = |errors: &mut Vec<Error>, what: &str| errors.push(Error { origin: s.origin.clone(), message: format!("struct {}: {what} isn't built into JASS yet", s.name) });
        match s.size.as_ref().map(literal_int) {
            Some(Some(n)) if n > DEFAULT_SIZE => err(errors, "more than 8190 instances"),
            Some(None) => err(errors, "a size that isn't a number"),
            _ => {}
        }
        let parent = s.extends.clone().filter(|e| e != "array");
        let mut info = Info { name: s.name.clone(), interface: false, array: s.extends.as_deref() == Some("array"), parent, fields: HashMap::new(), methods: HashMap::new(), typeid: 0 };
        for f in &s.fields {
            if f.delegate {
                err(errors, "`delegate`");
            }
            let k = match &f.array {
                Some(Some(e)) if !f.is_static => literal_int(e).or_else(|| {
                    err(errors, &format!("field {}'s size that isn't a number", f.name));
                    None
                }),
                Some(None) if !f.is_static => {
                    err(errors, &format!("field {}, an array without a size", f.name));
                    None
                }
                _ => None,
            };
            info.fields.insert(f.name.clone(), (f.ty.clone(), f.is_static, k));
        }
        for me in &s.methods {
            if me.function.sig.name.starts_with("operator") {
                err(errors, &format!("`method {}`", me.function.sig.name));
            }
            info.methods.insert(me.function.sig.name.clone(), method_info(me, &s.name));
        }
        infos.insert(s.name.clone(), info);
    }
    for s in &structs {
        if let Some(p) = &infos[&s.name].parent
            && !infos.contains_key(p)
        {
            errors.push(Error { origin: s.origin.clone(), message: format!("struct {} extends {p}, which is no struct or interface", s.name) });
        }
    }
    let mut types = Types { infos, families: HashMap::new() };
    // Families of several: an interface's, a struct's with children.
    let mut members: HashMap<String, Vec<String>> = HashMap::new();
    for s in &structs {
        members.entry(types.root(&s.name).to_string()).or_default().push(s.name.clone());
    }
    for (root, list) in members {
        if types.infos[&root].interface || list.len() > 1 {
            for (k, s) in list.iter().enumerate() {
                types.infos.get_mut(s).expect("a struct").typeid = k + 1;
            }
            types.families.insert(root, list);
        }
    }
    // An interface's methods, each struct's (but `defaults` ones).
    for s in &structs {
        let root = types.root(&s.name).to_string();
        if !types.infos[&root].interface {
            continue;
        }
        for (name, me) in &types.infos[&root].methods {
            if me.defaults.is_none() && types.method(&s.name, name).is_none_or(|(_, x)| x.abstract_) {
                errors.push(Error { origin: s.origin.clone(), message: format!("struct {} has no method {name} of interface {root}", s.name) });
            }
        }
    }

    // Globals, the allocators, the methods as functions.
    let mut inits = Vec::new();
    for s in &structs {
        let info = &types.infos[&s.name];
        for f in &s.fields {
            let mut g = Global::new(info.field(&f.name), f.ty.clone(), None, s.origin.clone());
            g.constant = f.constant;
            if f.is_static {
                g.array = f.array.is_some();
                g.init = f.init.clone();
            } else {
                g.array = true;
            }
            m.globals.push(g);
        }
        if !info.array {
            allocator(m, &types, &structs, s);
        }
        let mut own_inits = 0;
        for me in &s.methods {
            let mut f = me.function.clone();
            let name = &me.function.sig.name;
            // A module's `onInit` before its struct's.
            if name.starts_with("onInit__") && me.is_static && f.sig.params.is_empty() {
                let at = inits.len() - own_inits;
                inits.insert(at, (s.owner.scope.clone(), info.method(name)));
            } else if name == "onInit" && me.is_static && f.sig.params.is_empty() {
                inits.push((s.owner.scope.clone(), info.method(name)));
                own_inits += 1;
            }
            f.sig.name = info.method(&f.sig.name);
            if !me.is_static {
                f.sig.params.insert(0, Param { ty: s.name.clone(), name: "this".into() });
            }
            // A method of its struct: what `thistype` is.
            f.owner = Owner { scope: Some(s.name.clone()), access: Access::Private };
            m.functions.push(f);
        }
    }
    // The families' roots' allocators (an interface's) and destroyers.
    for (root, list) in &types.families {
        let info = &types.infos[root];
        if info.interface {
            free_list(m, root, &info.name, s_origin(&interfaces, root));
        }
        destroyer(m, &types, root, list, s_origin(&interfaces, root).or_else(|| structs.iter().find(|s| &s.name == root).map(|s| s.origin.clone())));
    }

    // Every function's members written as JASS has them.
    let sigs: HashMap<String, Option<String>> = m.functions.iter().map(|f| (f.sig.name.clone(), f.sig.returns.clone())).chain(m.natives.iter().map(|n| (n.sig.name.clone(), n.sig.returns.clone()))).collect();
    let globals: HashMap<String, String> = m.globals.iter().map(|g| (g.name.clone(), g.ty.clone())).collect();
    let dispatched: RefCell<BTreeSet<(String, String)>> = RefCell::new(BTreeSet::new());
    for f in &mut m.functions {
        // In a method `thistype` is its struct.
        let current = f.owner.scope.clone().filter(|s| types.infos.contains_key(s));
        let this_type = |t: &str| if t == "thistype" { current.clone().unwrap_or_default() } else { t.to_string() };
        for p in &mut f.sig.params {
            p.ty = this_type(&p.ty);
        }
        if let Some(r) = &mut f.sig.returns {
            *r = this_type(r);
        }
        let mut locals: HashMap<String, String> = f.sig.params.iter().map(|p| (p.name.clone(), p.ty.clone())).collect();
        local_types(&mut f.body, &this_type, &mut locals);
        let cx = Cx { types: &types, sigs: &sigs, globals: &globals, locals: &locals, current: current.as_deref(), file: &f.origin.file, dispatched: &dispatched };
        let body = std::mem::take(&mut f.body);
        f.body = cx.stmts(body, errors);
        if current.is_some() {
            f.owner = Owner::default();
        }
    }
    let mut setup = Vec::new();
    for (root, name) in dispatched.into_inner() {
        dispatcher(m, &types, &root, &name, &mut setup);
    }
    for (root, list) in &types.families {
        destroy_triggers(m, &types, root, list, &mut setup);
    }

    // Struct and interface types are integers.
    let integer = |t: &mut String| {
        if types.infos.contains_key(t.as_str()) {
            *t = "integer".into();
        }
    };
    for g in &mut m.globals {
        integer(&mut g.ty);
    }
    for f in &mut m.functions {
        f.sig.params.iter_mut().for_each(|p| integer(&mut p.ty));
        if let Some(r) = &mut f.sig.returns {
            integer(r);
        }
        local_types_integer(&mut f.body, &types.infos);
    }
    Lowered { inits, setup }
}

fn s_origin(interfaces: &[Interface], name: &str) -> Option<Origin> {
    interfaces.iter().find(|i| i.name == name).map(|i| i.origin.clone())
}

fn literal_int(e: &Expr) -> Option<i64> {
    match e {
        Expr::Literal(t) => t.parse().ok(),
        _ => None,
    }
}

fn this() -> Expr {
    Expr::var("this")
}

/// The free list of a family's root `root` (or a struct alone): its
/// globals.
fn free_list(m: &mut Module, root: &str, _name: &str, origin: Option<Origin>) {
    let o = origin.unwrap_or_else(|| Origin::file("the build"));
    m.globals.push(Global::new(format!("si__{root}_F"), "integer", Some(Expr::int(0)), o.clone()));
    m.globals.push(Global::new(format!("si__{root}_I"), "integer", Some(Expr::int(0)), o.clone()));
    let mut v = Global::new(format!("si__{root}_V"), "integer", None, o);
    v.array = true;
    m.globals.push(v);
}

/// `s__S__allocate` and `s__S_deallocate` (and `s__S_destroy` for a
/// struct alone), on its family's free list.
fn allocator(m: &mut Module, types: &Types, structs: &[Struct], s: &Struct) {
    let info = &types.infos[&s.name];
    let root = types.root(&s.name).to_string();
    let several = types.several(&s.name);
    if root == s.name {
        free_list(m, &root, &s.name, Some(s.origin.clone()));
        if several {
            let mut t = Global::new(types.type_global(&root), "integer", None, s.origin.clone());
            t.array = true;
            m.globals.push(t);
        }
    } else if several && types.infos[&root].interface && !m.globals.iter().any(|g| g.name == types.type_global(&root)) {
        let mut t = Global::new(types.type_global(&root), "integer", None, s.origin.clone());
        t.array = true;
        m.globals.push(t);
    }
    let (free, last, next) = (format!("si__{root}_F"), format!("si__{root}_I"), format!("si__{root}_V"));
    let size = structs.iter().find(|x| x.name == root).and_then(|x| x.size.as_ref()).and_then(literal_int).unwrap_or(DEFAULT_SIZE);
    let o = s.origin.clone();
    let ne = |a: Expr, b: Expr| Expr::binary(a, BinOp::NotEq, b);
    let mut alloc = vec![
        Stmt::local("integer", "this", Some(Expr::var(free.clone()))),
        Stmt::If {
            branches: vec![(ne(this(), Expr::int(0)), vec![Stmt::set(free.clone(), Expr::Index(next.clone(), Box::new(this())))])],
            otherwise: Some(vec![Stmt::set(last.clone(), Expr::binary(Expr::var(last.clone()), BinOp::Add, Expr::int(1))), Stmt::set("this", Expr::var(last.clone()))]),
            line: 0,
        },
        Stmt::If { branches: vec![(Expr::binary(this(), BinOp::GtEq, Expr::int(size)), vec![Stmt::Return { value: Some(Expr::int(0)), line: 0 }])], otherwise: None, line: 0 },
        Stmt::set_at(next.clone(), this(), Expr::int(-1)),
    ];
    if several {
        alloc.push(Stmt::set_at(types.type_global(&root), this(), Expr::int(info.typeid)));
    }
    // The fields with a value, its parents' first.
    for i in types.chain(&s.name).into_iter().rev() {
        let Some(x) = structs.iter().find(|x| x.name == i.name) else { continue };
        for f in x.fields.iter().filter(|f| !f.is_static && f.array.is_none()) {
            if let Some(init) = &f.init {
                alloc.push(Stmt::set_at(i.field(&f.name), this(), init.clone()));
            }
        }
    }
    alloc.push(Stmt::Return { value: Some(this()), line: 0 });
    let mut a = Function::new(info.method("_allocate"), alloc, o.clone());
    a.sig.returns = Some("integer".into());
    m.functions.push(a);

    let mut d = Function::new(info.method("deallocate"), taken(&root, give_back(&root)), o.clone());
    d.sig.params.push(Param { ty: "integer".into(), name: "this".into() });
    m.functions.push(d);
    if !several {
        let mut body = types.method(&s.name, "onDestroy").map(|(i, _)| vec![Stmt::call(i.method("onDestroy"), vec![this()])]).unwrap_or_default();
        body.extend(give_back(&root));
        let mut d = Function::new(info.method("destroy"), taken(&root, body), o);
        d.sig.params.push(Param { ty: "integer".into(), name: "this".into() });
        m.functions.push(d);
    }
}

fn give_back(root: &str) -> Vec<Stmt> {
    let (free, next) = (format!("si__{root}_F"), format!("si__{root}_V"));
    vec![Stmt::set_at(next, this(), Expr::var(free.clone())), Stmt::set(free, this())]
}

/// `body` for an instance taken (not 0, not given back already).
fn taken(root: &str, body: Vec<Stmt>) -> Vec<Stmt> {
    let next = format!("si__{root}_V");
    let cond = Expr::binary(
        Expr::binary(this(), BinOp::NotEq, Expr::int(0)),
        BinOp::And,
        Expr::binary(Expr::Index(next, Box::new(this())), BinOp::Eq, Expr::int(-1)),
    );
    vec![Stmt::If { branches: vec![(cond, body)], otherwise: None, line: 0 }]
}

/// A family's `s__R__destroy`: its type's `onDestroy`s run (through the
/// trigger of its type, when it has any), then it's given back.
fn destroyer(m: &mut Module, types: &Types, root: &str, list: &[String], origin: Option<Origin>) {
    let o = origin.unwrap_or_else(|| Origin::file("the build"));
    let triggers = format!("wcb__dt_{root}__destroy");
    let mut t = Global::new(triggers.clone(), "trigger", None, o.clone());
    t.array = true;
    m.globals.push(t);
    let arg = format!("wcb__da_{root}__destroy");
    m.globals.push(Global::new(arg.clone(), "integer", None, o.clone()));
    let _ = list;
    let ty = Expr::Index(types.type_global(root), Box::new(this()));
    let mut body = vec![Stmt::If {
        branches: vec![(
            Expr::binary(Expr::Index(triggers.clone(), Box::new(ty)), BinOp::NotEq, Expr::lit("null")),
            vec![Stmt::set(arg.clone(), this()), Stmt::call("TriggerEvaluate", vec![Expr::Index(triggers, Box::new(Expr::Index(types.type_global(root), Box::new(this()))))])],
        )],
        otherwise: None,
        line: 0,
    }];
    body.extend(give_back(root));
    let mut d = Function::new(format!("s__{root}__destroy"), taken(root, body), o);
    d.sig.params.push(Param { ty: "integer".into(), name: "this".into() });
    m.functions.push(d);
}

/// The family's types' triggers that run their `onDestroy`s (the
/// struct's, then its parents').
fn destroy_triggers(m: &mut Module, types: &Types, root: &str, list: &[String], setup: &mut Vec<Stmt>) {
    let triggers = format!("wcb__dt_{root}__destroy");
    let arg = format!("wcb__da_{root}__destroy");
    for s in list {
        let calls: Vec<Stmt> = types.chain(s).into_iter().filter(|i| i.methods.contains_key("onDestroy")).map(|i| Stmt::call(i.method("onDestroy"), vec![Expr::var(arg.clone())])).collect();
        if calls.is_empty() {
            continue;
        }
        let thunk = format!("wcb__dthunk_{s}__destroy");
        let mut body = calls;
        body.push(Stmt::Return { value: Some(Expr::lit("true")), line: 0 });
        let mut f = Function::new(thunk.clone(), body, Origin::file("the build"));
        f.sig.returns = Some("boolean".into());
        m.functions.push(f);
        register(setup, &triggers, types.infos[s].typeid, &thunk);
    }
}

fn register(setup: &mut Vec<Stmt>, triggers: &str, typeid: usize, thunk: &str) {
    let at = || Expr::Index(triggers.to_string(), Box::new(Expr::int(typeid)));
    setup.push(Stmt::set_at(triggers, Expr::int(typeid), Expr::call("CreateTrigger", vec![])));
    setup.push(Stmt::call("TriggerAddCondition", vec![at(), Expr::call("Condition", vec![Expr::code(thunk)])]));
}

/// `wcb__d_R_m`: the call of `m` by the instance's type, the arguments
/// through globals; each type's trigger, whose condition calls its
/// struct's method.
fn dispatcher(m: &mut Module, types: &Types, root: &str, name: &str, setup: &mut Vec<Stmt>) {
    // Its signature: the root's, else the first of the family's that has it.
    let Some((_, me)) = types.method(root, name).or_else(|| types.families[root].iter().find_map(|s| types.method(s, name))) else { return };
    let me = me.clone();
    let o = Origin::file("the build");
    let triggers = format!("wcb__dt_{root}_{name}");
    let arg = |k: usize| format!("wcb__da_{root}_{name}_{k}");
    let ret = format!("wcb__dr_{root}_{name}");
    let mut t = Global::new(triggers.clone(), "trigger", None, o.clone());
    t.array = true;
    m.globals.push(t);
    let mut params = vec![Param { ty: "integer".into(), name: "this".into() }];
    params.extend(me.params.iter().cloned());
    for (k, p) in params.iter().enumerate() {
        m.globals.push(Global::new(arg(k), p.ty.clone(), None, o.clone()));
    }
    if let Some(r) = &me.returns {
        m.globals.push(Global::new(ret.clone(), r.clone(), None, o.clone()));
    }
    let ty = || Expr::Index(types.type_global(root), Box::new(this()));
    let mut body: Vec<Stmt> = params.iter().enumerate().map(|(k, p)| Stmt::set(arg(k), Expr::var(p.name.clone()))).collect();
    // A type without the method: the interface's `defaults`.
    if let Some(d) = &me.defaults {
        body.push(Stmt::If {
            branches: vec![(Expr::binary(Expr::Index(triggers.clone(), Box::new(ty())), BinOp::Eq, Expr::lit("null")), vec![Stmt::Return { value: me.returns.as_ref().map(|_| d.clone()), line: 0 }])],
            otherwise: None,
            line: 0,
        });
    }
    body.push(Stmt::call("TriggerEvaluate", vec![Expr::Index(triggers.clone(), Box::new(ty()))]));
    if me.returns.is_some() {
        body.push(Stmt::Return { value: Some(Expr::var(ret.clone())), line: 0 });
    }
    let mut f = Function::new(format!("wcb__d_{root}_{name}"), body, o.clone());
    f.sig.params = params.clone();
    f.sig.returns = me.returns.clone();
    m.functions.push(f);
    for s in &types.families[root] {
        let Some((owner, x)) = types.method(s, name) else { continue };
        if x.abstract_ {
            continue;
        }
        let call = Call { name: owner.method(name), args: (0..params.len()).map(|k| Expr::var(arg(k))).collect() };
        let mut body = vec![match me.returns {
            Some(_) => Stmt::set(ret.clone(), Expr::Call(call)),
            None => Stmt::Call { call, line: 0 },
        }];
        body.push(Stmt::Return { value: Some(Expr::lit("true")), line: 0 });
        let thunk = format!("wcb__dthunk_{s}_{name}");
        let mut f = Function::new(thunk.clone(), body, o.clone());
        f.sig.returns = Some("boolean".into());
        m.functions.push(f);
        register(setup, &triggers, types.infos[s].typeid, &thunk);
    }
}

/// Locals' types (`thistype` made its struct), into `types`.
fn local_types(body: &mut [Stmt], this_type: &dyn Fn(&str) -> String, types: &mut HashMap<String, String>) {
    for s in body {
        match s {
            Stmt::Local { ty, name, .. } => {
                *ty = this_type(ty);
                types.insert(name.clone(), ty.clone());
            }
            Stmt::If { branches, otherwise, .. } => {
                branches.iter_mut().for_each(|(_, b)| local_types(b, this_type, types));
                if let Some(b) = otherwise {
                    local_types(b, this_type, types);
                }
            }
            Stmt::Loop { body, .. } => local_types(body, this_type, types),
            _ => {}
        }
    }
}

fn local_types_integer(body: &mut [Stmt], infos: &HashMap<String, Info>) {
    for s in body {
        match s {
            Stmt::Local { ty, .. } if infos.contains_key(ty.as_str()) => *ty = "integer".into(),
            Stmt::If { branches, otherwise, .. } => {
                branches.iter_mut().for_each(|(_, b)| local_types_integer(b, infos));
                if let Some(b) = otherwise {
                    local_types_integer(b, infos);
                }
            }
            Stmt::Loop { body, .. } => local_types_integer(body, infos),
            _ => {}
        }
    }
}

/// What a function's members are written in.
struct Cx<'a> {
    types: &'a Types,
    /// Functions: what each returns.
    sigs: &'a HashMap<String, Option<String>>,
    globals: &'a HashMap<String, String>,
    locals: &'a HashMap<String, String>,
    /// The struct of the method.
    current: Option<&'a str>,
    file: &'a str,
    /// The calls by type made: family root, method.
    dispatched: &'a RefCell<BTreeSet<(String, String)>>,
}

impl Cx<'_> {
    fn error(&self, errors: &mut Vec<Error>, line: usize, message: String) {
        errors.push(Error { origin: Origin { file: self.file.to_string(), line }, message });
    }

    /// The struct a name is as an object (`Data.x`, `thistype.x`).
    fn struct_named(&self, e: &Expr) -> Option<&Info> {
        match e {
            Expr::Var(n) if !self.locals.contains_key(n) && !self.globals.contains_key(n) => {
                let n = if n == "thistype" { self.current? } else { n.as_str() };
                self.types.infos.get(n)
            }
            _ => None,
        }
    }

    fn type_of_var(&self, n: &str) -> Option<String> {
        if n == "this" && !self.locals.contains_key("this") {
            return self.current.map(String::from);
        }
        self.locals.get(n).or_else(|| self.globals.get(n)).cloned()
    }

    fn stmts(&self, body: Vec<Stmt>, errors: &mut Vec<Error>) -> Vec<Stmt> {
        body.into_iter().map(|s| self.stmt(s, errors)).collect()
    }

    fn stmt(&self, s: Stmt, errors: &mut Vec<Error>) -> Stmt {
        match s {
            Stmt::Local { ty, name, array, init, line } => Stmt::Local { ty, name, array, init: init.map(|e| self.expr(e, line, errors).0), line },
            Stmt::Set { name, index, value, line } => Stmt::Set { name, index: index.map(|e| self.expr(e, line, errors).0), value: self.expr(value, line, errors).0, line },
            Stmt::Call { call, line } => match self.expr(Expr::Call(call), line, errors).0 {
                Expr::Call(call) => Stmt::Call { call, line },
                // A cast called for nothing.
                _ => Stmt::Call { call: Call { name: "DoNothing".into(), args: vec![] }, line },
            },
            Stmt::If { branches, otherwise, line } => Stmt::If {
                branches: branches.into_iter().map(|(c, b)| (self.expr(c, line, errors).0, self.stmts(b, errors))).collect(),
                otherwise: otherwise.map(|b| self.stmts(b, errors)),
                line,
            },
            Stmt::Loop { body, line } => Stmt::Loop { body: self.stmts(body, errors), line },
            Stmt::ExitWhen { cond, line } => Stmt::ExitWhen { cond: self.expr(cond, line, errors).0, line },
            Stmt::Return { value, line } => Stmt::Return { value: value.map(|e| self.expr(e, line, errors).0), line },
            Stmt::Debug { stmt, line } => Stmt::Debug { stmt: Box::new(self.stmt(*stmt, errors)), line },
            Stmt::SetMember { object, name, index, value, line } => {
                let value = self.expr(value, line, errors).0;
                let index = index.map(|e| self.expr(e, line, errors).0);
                match self.field(object, &name, index, line, errors) {
                    Some((array, index, _)) => Stmt::Set { name: array, index, value, line },
                    None => Stmt::Set { name: "wcb__error".into(), index: None, value, line },
                }
            }
            Stmt::CallMethod { call, line } => match self.expr(Expr::Method(Box::new(call)), line, errors).0 {
                Expr::Call(call) => Stmt::Call { call, line },
                _ => Stmt::Call { call: Call { name: "DoNothing".into(), args: vec![] }, line },
            },
            s @ Stmt::StaticIf { .. } => s,
        }
    }

    /// A field: the global it's in, the index (none for a static one's),
    /// its type.
    fn field(&self, object: Expr, name: &str, index: Option<Expr>, line: usize, errors: &mut Vec<Error>) -> Option<(String, Option<Expr>, String)> {
        let (t, instance) = match self.struct_named(&object) {
            Some(info) => (info.name.clone(), None),
            None => {
                let (o, t) = self.expr(object, line, errors);
                let Some(t) = t.filter(|t| self.types.infos.contains_key(t)) else {
                    self.error(errors, line, format!("`.{name}` of what isn't a struct (or whose type can't be told)"));
                    return None;
                };
                (t, Some(o))
            }
        };
        let Some((owner, (ty, is_static, k))) = self.types.field(&t, name) else {
            self.error(errors, line, format!("struct {t} has no field {name}"));
            return None;
        };
        let array = owner.field(name);
        let index = match (is_static, instance, index) {
            (true, _, i) => i,
            (false, Some(o), None) => Some(o),
            (false, Some(o), Some(i)) => Some(Expr::binary(Expr::binary(o, BinOp::Mul, Expr::int(k.unwrap_or(1))), BinOp::Add, i)),
            (false, None, _) => {
                self.error(errors, line, format!("{t}.{name}: an instance's field of the struct"));
                return None;
            }
        };
        Some((array, index, ty.clone()))
    }

    /// `e` as JASS has it, and its type when it can be told.
    fn expr(&self, e: Expr, line: usize, errors: &mut Vec<Error>) -> (Expr, Option<String>) {
        match e {
            Expr::Var(n) => {
                let t = self.type_of_var(&n);
                (Expr::Var(n), t)
            }
            Expr::Index(n, i) => {
                let t = self.type_of_var(&n);
                (Expr::Index(n, Box::new(self.expr(*i, line, errors).0)), t)
            }
            Expr::Call(Call { name, args }) => {
                let mut args: Vec<Expr> = args.into_iter().map(|a| self.expr(a, line, errors).0).collect();
                // A cast: `Data(x)`, `thistype(x)`.
                let cast = if name == "thistype" { self.current.map(String::from) } else { self.types.infos.contains_key(&name).then(|| name.clone()) };
                if let Some(t) = cast
                    && args.len() == 1
                {
                    return (args.remove(0), Some(t));
                }
                let t = self.sigs.get(&name).cloned().flatten();
                (Expr::Call(Call { name, args }), t)
            }
            Expr::Code(n) => match n.split_once('.') {
                Some((s, m)) => {
                    let s = if s == "thistype" { self.current.unwrap_or(s) } else { s };
                    match self.types.method(s, m) {
                        Some((owner, _)) => (Expr::Code(owner.method(m)), Some("code".into())),
                        None => {
                            self.error(errors, line, format!("function {n}: {s} has no method {m}"));
                            (Expr::Code(n), None)
                        }
                    }
                }
                None => (Expr::Code(n), Some("code".into())),
            },
            Expr::Member(object, name) => {
                // `S.typeid`: the struct's type.
                if name == "typeid"
                    && let Some(info) = self.struct_named(&object)
                {
                    return (Expr::int(info.typeid), Some("integer".into()));
                }
                match self.field(*object, &name, None, line, errors) {
                    Some((array, index, ty)) => (index.map_or_else(|| Expr::Var(array.clone()), |i| Expr::Index(array.clone(), Box::new(i))), Some(ty)),
                    None => (Expr::lit("0"), None),
                }
            }
            Expr::Element(a, i) => match *a {
                Expr::Member(object, name) => {
                    let i = self.expr(*i, line, errors).0;
                    match self.field(*object, &name, Some(i), line, errors) {
                        Some((array, Some(index), ty)) => (Expr::Index(array, Box::new(index)), Some(ty)),
                        _ => (Expr::lit("0"), None),
                    }
                }
                _ => {
                    self.error(errors, line, "an element of what isn't a field".into());
                    (Expr::lit("0"), None)
                }
            },
            Expr::Method(m) => self.method(*m, line, errors),
            Expr::Unary(op, x) => {
                let (x, t) = self.expr(*x, line, errors);
                (Expr::Unary(op, Box::new(x)), t)
            }
            Expr::Binary(a, op, b) => {
                let (a, t) = self.expr(*a, line, errors);
                let b = self.expr(*b, line, errors).0;
                (Expr::Binary(Box::new(a), op, Box::new(b)), t)
            }
            e @ Expr::Literal(_) => (e, None),
        }
    }

    fn method(&self, m: MethodCall, line: usize, errors: &mut Vec<Error>) -> (Expr, Option<String>) {
        let MethodCall { object, name, args } = m;
        let mut args: Vec<Expr> = args.into_iter().map(|a| self.expr(a, line, errors).0).collect();
        let call = |n: String, args: Vec<Expr>| Expr::Call(Call { name: n, args });
        // `.evaluate` and `.execute` of a function or a method: through its
        // trigger, the arguments in globals (the JASS output makes it:
        // `wcb__call_F`, `wcb__exec_F`).
        if matches!(name.as_str(), "evaluate" | "execute") {
            let through = |f: String, returns: Option<String>, args: Vec<Expr>| {
                let kind = if name == "evaluate" { "call" } else { "exec" };
                let returns = if name == "evaluate" { returns } else { None };
                (call(format!("{}{kind}_{f}", crate::jass::PREFIX), args), returns)
            };
            match &object {
                Expr::Var(f) if self.type_of_var(f).is_none() && self.sigs.contains_key(f) => {
                    return through(f.clone(), self.sigs[f].clone(), args);
                }
                Expr::Member(o, mname) => {
                    let (t, instance) = match self.struct_named(o) {
                        Some(info) => (Some(info.name.clone()), None),
                        None => {
                            let (o, t) = self.expr((**o).clone(), line, errors);
                            (t, Some(o))
                        }
                    };
                    if let Some((owner, x)) = t.as_deref().and_then(|t| self.types.method(t, mname)) {
                        if !x.is_static {
                            match instance {
                                Some(o) => args.insert(0, o),
                                None => {
                                    self.error(errors, line, format!("{}.{mname}: a method of an instance", owner.name));
                                    return (Expr::lit("0"), None);
                                }
                            }
                        }
                        return through(owner.method(mname), x.returns.clone(), args);
                    }
                }
                _ => {}
            }
        }
        // `super.m(...)`: the parent's, straight.
        if matches!(&object, Expr::Var(n) if n == "super") {
            let parent = self.current.and_then(|c| self.types.infos[c].parent.clone());
            return match parent.as_deref().and_then(|p| self.types.method(p, &name)) {
                Some((owner, x)) if !x.abstract_ => {
                    if !x.is_static {
                        args.insert(0, this());
                    }
                    (call(owner.method(&name), args), x.returns.clone())
                }
                _ => {
                    self.error(errors, line, format!("super.{name}: the parent has no method {name}"));
                    (Expr::lit("0"), None)
                }
            };
        }
        // A struct's own: static.
        if let Some(info) = self.struct_named(&object) {
            let f = match name.as_str() {
                "create" if self.types.method(&info.name, "create").is_none() && !info.array && !info.interface => info.method("_allocate"),
                "allocate" if !info.array && !info.interface => info.method("_allocate"),
                _ => match self.types.method(&info.name, &name) {
                    Some((owner, x)) if x.is_static => owner.method(&name),
                    _ => {
                        self.error(errors, line, format!("{}.{name}: no static method {name}", info.name));
                        return (Expr::lit("0"), None);
                    }
                },
            };
            let t = self.types.method(&info.name, &name).and_then(|(_, x)| x.returns.clone()).or_else(|| matches!(name.as_str(), "create" | "allocate").then(|| info.name.clone()));
            return (call(f, args), t);
        }
        let (o, t) = self.expr(object, line, errors);
        let Some(t) = t.filter(|t| self.types.infos.contains_key(t)) else {
            self.error(errors, line, format!(".{name}() of what isn't a struct (or whose type can't be told)"));
            return (Expr::lit("0"), None);
        };
        let several = self.types.several(&t);
        match name.as_str() {
            "destroy" if several => return (call(format!("s__{}__destroy", self.types.root(&t)), vec![o]), None),
            // A struct alone's own, unless it has a method of the name.
            "destroy" | "deallocate" if self.types.method(&t, &name).is_none() => {
                let info = &self.types.infos[&t];
                if !info.interface && !info.array {
                    return (call(info.method(&name), vec![o]), None);
                }
            }
            // `x.getType()`: the instance's type.
            "getType" if several => return (Expr::Index(self.types.type_global(&t), Box::new(o)), Some("integer".into())),
            _ => {}
        }
        let Some((owner, x)) = self.types.method(&t, &name) else {
            self.error(errors, line, format!("struct {t} has no method {name}"));
            return (Expr::lit("0"), None);
        };
        let returns = x.returns.clone();
        let is_static = x.is_static;
        if !is_static {
            args.insert(0, o);
        }
        if !is_static && self.types.dispatches(&t, &name) {
            let root = self.types.root(&t).to_string();
            self.dispatched.borrow_mut().insert((root.clone(), name.clone()));
            return (call(format!("wcb__d_{root}_{name}"), args), returns);
        }
        (call(owner.method(&name), args), returns)
    }
}

#[cfg(test)]
#[path = "structs.test.rs"]
pub(crate) mod tests;
