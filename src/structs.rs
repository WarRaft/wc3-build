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
//!   `thistype(x)` are `x`; `S[i]` of an array struct (`extends array`,
//!   without an `operator []`) is `i`;
//! - `F.name`, `S.m.name`: the name the function is built with, a string
//!   (for `ExecuteFunc`);
//! - `F.evaluate(...)` and `F.execute(...)` of a function or a method go
//!   through its trigger, the arguments in globals (`wcb__call_F`,
//!   `wcb__exec_F`, which the JASS output makes);
//! - a function interface `I` is `integer`: `I.f` is `f`'s number among
//!   the functions taken as `I`'s (from 1), and `x.evaluate(...)` /
//!   `x.execute(...)` (`wcb__fi_I`, `wcb__fx_I`) put the arguments into
//!   `I`'s globals and evaluate or execute the trigger of that number
//!   (`wcb__fit_I[x]`), which calls the function; `f` must take and
//!   return what `I` does;
//! - operators are methods named as JassHelper's: `a.x` and `set a.x =`
//!   of a struct without a field `x` call `s__S__get_x` / `s__S__set_x`,
//!   `a[i]` and `set a[i] =` of a struct (not an array; `S[i]` static)
//!   `s__S__getindex` / `s__S__setindex`, `a < b` `s__S__lt(a, b)` (`>`
//!   turned round, `<=` and `>=` the other's `not`);
//! - a `delegate` field gives its struct what its type has: `a.x`,
//!   `a.m()` of a struct without them are `a.d.x`, `a.d.m()` (through a
//!   delegate's delegates too).
//!
//! What a name is (`c.value`: which struct's) comes from the types:
//! parameters, locals, globals, fields, what functions and methods
//! return. Not lowered yet (an error each): an operator of an interface
//! or a `stub` one, a `delegate` static or an array, a struct of more
//! than 8190 instances.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use crate::ir::*;

/// Instances a struct has without a size.
const DEFAULT_SIZE: i64 = 8191;

/// A field: its type, whether static, an instance's array's size,
/// whether an array.
type FieldInfo = (String, bool, Option<i64>, bool);

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
    /// Its `delegate` fields: name, type.
    delegates: Vec<(String, String)>,
    /// Its type in its family (from 1), 0 for an interface.
    typeid: usize,
}

impl Info {
    fn field(&self, f: &str) -> String {
        format!("s__{}_{f}", self.name)
    }
    /// A method's function; an operator's as JassHelper names it.
    fn method(&self, m: &str) -> String {
        let n = &self.name;
        match m.strip_prefix("operator ") {
            Some("[]") => format!("s__{n}__getindex"),
            Some("[]=") => format!("s__{n}__setindex"),
            Some("<") => format!("s__{n}__lt"),
            Some(x) => match x.strip_suffix('=') {
                Some(x) => format!("s__{n}__set_{x}"),
                None => format!("s__{n}__get_{x}"),
            },
            None => format!("s__{n}_{m}"),
        }
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
    /// Function interfaces: their signatures.
    functions: HashMap<String, Signature>,
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
        chain.iter().find_map(|i| i.methods.get(m).filter(|x| !x.abstract_).map(|x| (*i, x))).or_else(|| chain.iter().find_map(|i| i.methods.get(m).map(|x| (*i, x))))
    }

    /// Does a call of `m` on a `t` go by the instance's type?
    fn dispatches(&self, t: &str, m: &str) -> bool {
        if !self.several(t) {
            return false;
        }
        self.chain(t).iter().any(|i| i.methods.get(m).is_some_and(|x| x.stub || x.abstract_))
    }

    /// The `delegate` field of `t` (or a parent) whose type (or its
    /// delegates') has what `has` looks for: its name and type.
    fn delegate(&self, t: &str, has: &dyn Fn(&str) -> bool) -> Option<(String, String)> {
        fn reaches(types: &Types, t: &str, has: &dyn Fn(&str) -> bool, depth: usize) -> bool {
            has(t) || (depth < 8 && types.chain(t).iter().any(|i| i.delegates.iter().any(|(_, d)| reaches(types, d, has, depth + 1))))
        }
        self.chain(t).iter().find_map(|i| i.delegates.iter().find(|(_, d)| reaches(self, d, has, 0)).cloned())
    }

    fn type_global(&self, t: &str) -> String {
        format!("si__{}_type", self.root(t))
    }
}

/// The structs and interfaces of `m` (modules taken in, names already
/// resolved) lowered: their globals and functions added, every member
/// written as they are, their types as `integer`.
///
/// With `classes` (AngelScript's, see [`crate::angelscript`]) they're kept
/// as classes: `m.structs` and `m.interfaces` stay, each instance method
/// in its struct with its body lowered, the instance fields in it; a
/// struct's type stays its name (an object). Static fields and methods
/// are globals and functions as for JASS. `a.x`, `set a.x = v` and
/// `a.m(...)` of an instance stay members (`onDestroy` is
/// `onDestroy__S`, an operator `wcb__get_x`, `wcb__set_x`,
/// `wcb__getindex`, `wcb__setindex`, `wcb__lt`); `super.m(...)` is
/// `P::m(...)`; `x.destroy()` and `x.deallocate()` are
/// `s__R__destroy(x)` and `s__R__deallocate(x)` of the family's root `R`,
/// `S.allocate()` (and `S.create()` without one of its own)
/// `s__S__allocate()`, `x.getType()` `x.wcb__type()`. Where an object
/// and an integer meet, its number: `s__R__id(x)` where an integer goes,
/// `wcb__cast__S(s__R__at(i))` where an `S` goes from an integer,
/// `wcb__cast__S(x)` from a parent; `0` where an object goes is
/// `wcb__null()`, `a == b` of objects `wcb__same(a, b)`. Instance 0 is no
/// object: `S(0).x` is an error. A struct `extends array` is no class: its
/// fields are arrays and its methods functions, as for JASS. `x.m.execute(...)`
/// of an object runs `wcb__x_S_m(x, ...)` (made here: it calls `x.m(...)`)
/// as any function's `.execute`; `x.m.evaluate(...)` is `x.m(...)`.
pub fn lower(m: &mut Module, game: &[Signature], classes: bool, errors: &mut Vec<Error>) -> Lowered {
    // A global's initial value and a member's default are set before any
    // code runs, so they can't use a struct (`pair.create()`): a constant,
    // or set it in an initializer.
    let uses_struct = |e: &Expr| {
        let mut found = false;
        each_expr(&[Stmt::Return { value: Some(e.clone()), line: 0 }], &mut |x| found |= matches!(x, Expr::Member(..) | Expr::Method(_) | Expr::Element(..)));
        found
    };
    for g in &m.globals {
        if g.init.as_ref().is_some_and(uses_struct) {
            errors.push(Error { origin: g.origin.clone(), message: format!("the global {}'s initial value uses a struct (a.b, S.create() ...): globals are set before any code runs, so give it a constant and set it in an initializer", g.name) });
        }
    }
    for s in &m.structs {
        for f in s.fields.iter().filter(|f| f.init.as_ref().is_some_and(uses_struct)) {
            errors.push(Error { origin: s.origin.clone(), message: format!("{}'s member {}'s default value uses a struct (a.b, S.create() ...): a default is a constant; set it in create instead", s.name, f.name) });
        }
    }
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
        infos.insert(i.name.clone(), Info { name: i.name.clone(), interface: true, array: false, parent: None, fields: HashMap::new(), methods, delegates: Vec::new(), typeid: 0 });
    }
    for s in &structs {
        let err = |errors: &mut Vec<Error>, what: &str| errors.push(Error { origin: s.origin.clone(), message: format!("struct {}: {what} isn't built into JASS yet", s.name) });
        match s.size.as_ref().map(literal_int) {
            Some(Some(n)) if n > DEFAULT_SIZE && !classes => err(errors, "more than 8190 instances"),
            Some(None) => err(errors, "a size that isn't a number"),
            _ => {}
        }
        let parent = s.extends.clone().filter(|e| e != "array");
        let mut info =
            Info { name: s.name.clone(), interface: false, array: s.extends.as_deref() == Some("array"), parent, fields: HashMap::new(), methods: HashMap::new(), delegates: Vec::new(), typeid: 0 };
        for f in &s.fields {
            if f.delegate {
                if f.is_static || f.array.is_some() {
                    err(errors, &format!("`delegate` {}, static or an array", f.name));
                } else {
                    info.delegates.push((f.name.clone(), f.ty.clone()));
                }
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
            info.fields.insert(f.name.clone(), (f.ty.clone(), f.is_static, k, f.array.is_some()));
        }
        for me in &s.methods {
            if me.function.sig.name.starts_with("operator") && me.stub {
                err(errors, &format!("`stub method {}`", me.function.sig.name));
            }
            info.methods.insert(me.function.sig.name.clone(), method_info(me, &s.name));
        }
        if info.array {
            array_struct_rules(s, errors);
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
    let functions: HashMap<String, Signature> = std::mem::take(&mut m.function_types).into_iter().map(|t| (t.sig.name.clone(), t.sig)).collect();
    let mut types = Types { infos, functions, families: HashMap::new() };
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
    // Classes': each instance method's function, its struct and name.
    let mut instance: HashMap<String, (String, String)> = HashMap::new();
    for s in &structs {
        let info = &types.infos[&s.name];
        // A struct `extends array` is its fields' arrays, as for JASS: its
        // instances are integers, no objects.
        let class = classes && !info.array;
        for f in s.fields.iter().filter(|f| !class || f.is_static) {
            let mut g = Global::new(info.field(&f.name), f.ty.clone(), None, s.origin.clone());
            g.constant = f.constant;
            if f.is_static {
                g.array = f.array.is_some();
                g.size = f.array.clone().flatten();
                g.init = f.init.clone();
            } else {
                g.array = true;
            }
            m.globals.push(g);
        }
        if !info.array && !classes {
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
            if class && !me.is_static {
                // Lowered with the functions, then back into its class.
                instance.insert(f.sig.name.clone(), (s.name.clone(), class_method(&me.function.sig.name, &s.name)));
            } else if !me.is_static {
                f.sig.params.insert(0, Param { ty: s.name.clone(), name: "this".into() });
            }
            // A method of its struct: what `thistype` is.
            f.owner = Owner { scope: Some(s.name.clone()), access: Access::Private };
            m.functions.push(f);
        }
    }
    // The families' roots' allocators (an interface's) and destroyers.
    for (root, list) in types.families.iter().filter(|_| !classes) {
        let info = &types.infos[root];
        if info.interface {
            free_list(m, root, &info.name, s_origin(&interfaces, root));
        }
        destroyer(m, &types, root, list, s_origin(&interfaces, root).or_else(|| structs.iter().find(|s| &s.name == root).map(|s| s.origin.clone())));
    }

    // Every function's members written as JASS has them.
    let sigs: HashMap<String, Option<String>> = game
        .iter()
        .filter(|_| classes)
        .map(|s| (s.name.clone(), s.returns.clone()))
        .chain(m.functions.iter().map(|f| (f.sig.name.clone(), f.sig.returns.clone())))
        .chain(m.natives.iter().map(|n| (n.sig.name.clone(), n.sig.returns.clone())))
        .collect();
    let params: HashMap<String, Vec<String>> = m.functions.iter().map(|f| (f.sig.name.clone(), f.sig.params.iter().map(|p| p.ty.clone()).collect())).collect();
    // Classes': what every function and native takes, for an object's number.
    let takes: HashMap<String, Vec<String>> = if classes {
        game.iter().chain(m.natives.iter().map(|n| &n.sig)).chain(m.functions.iter().map(|f| &f.sig)).map(|s| (s.name.clone(), s.params.iter().map(|p| p.ty.clone()).collect())).collect()
    } else {
        HashMap::new()
    };
    let globals: HashMap<String, String> = m.globals.iter().map(|g| (g.name.clone(), g.ty.clone())).collect();
    let global_arrays: HashSet<String> = m.globals.iter().filter(|g| g.array).map(|g| g.name.clone()).collect();
    let sizes: HashMap<String, Expr> = m.globals.iter().filter_map(|g| Some((g.name.clone(), g.size.clone()?))).collect();
    let dispatched: RefCell<BTreeSet<(String, String)>> = RefCell::new(BTreeSet::new());
    let executed: RefCell<BTreeSet<(String, String)>> = RefCell::new(BTreeSet::new());
    let pointers = Pointers::default();
    for f in &mut m.functions {
        // In a method `thistype` is its struct.
        let current = f.owner.scope.clone().filter(|s| types.infos.contains_key(s));
        let this_type = |t: &str| {
            if t == "thistype" { current.clone().unwrap_or_default() } else { t.to_string() }
        };
        for p in &mut f.sig.params {
            p.ty = this_type(&p.ty);
        }
        if let Some(r) = &mut f.sig.returns {
            *r = this_type(r);
        }
        let mut locals: HashMap<String, String> = f.sig.params.iter().map(|p| (p.name.clone(), p.ty.clone())).collect();
        let mut arrays = HashSet::new();
        local_types(&mut f.body, &this_type, &mut locals, &mut arrays);
        let returns = f.sig.returns.clone();
        let cx = Cx {
            sizes: &sizes,
            executed: &executed,
            classes,
            takes: &takes,
            returns: returns.as_deref(),
            types: &types,
            sigs: &sigs,
            params: &params,
            globals: &globals,
            locals: &locals,
            arrays: &arrays,
            global_arrays: &global_arrays,
            current: current.as_deref(),
            file: &f.origin.file,
            dispatched: &dispatched,
            pointers: &pointers,
        };
        let body = std::mem::take(&mut f.body);
        f.body = cx.stmts(body, errors);
        if current.is_some() {
            f.owner = Owner::default();
        }
    }
    let mut setup = Vec::new();
    if classes {
        // The instance methods back into their classes, the classes kept
        // (an array struct's are functions: it's no class).
        let mut structs: Vec<Struct> = structs.into_iter().filter(|s| !types.infos[&s.name].array).collect();
        let mut interfaces = interfaces;
        let mut i = 0;
        while i < m.functions.len() {
            let Some((owner, name)) = instance.get(&m.functions[i].sig.name).cloned() else {
                i += 1;
                continue;
            };
            let mut f = m.functions.remove(i);
            f.sig.name = name.clone();
            let st = structs.iter_mut().find(|s| s.name == owner).expect("its struct");
            if let Some(me) = st.methods.iter_mut().find(|me| !me.is_static && class_method(&me.function.sig.name, &owner) == name) {
                me.function = f;
            }
        }
        // A child allocated through a parent's own `create`: that `create`
        // made to make the child (AngelScript makes an object of its class).
        let names: Vec<String> = structs.iter().map(|s| s.name.clone()).collect();
        for c in &names {
            if let Some((p, _)) = parent_create(&types, &structs, c) {
                via_create(m, &types, &structs, c, &p.name.clone(), &format!("s__{c}__allocate"));
            }
        }
        for st in &mut structs {
            st.methods.retain(|me| !me.is_static);
            for f in &mut st.fields {
                f.ty = if f.ty == "thistype" { st.name.clone() } else { f.ty.clone() };
            }
        }
        for i in &mut interfaces {
            for me in &mut i.methods {
                me.function.sig.name = class_method(&me.function.sig.name, &i.name);
            }
        }
        m.structs = structs;
        m.interfaces = interfaces;
        // An object's method run by `.execute`: a function taking it first.
        for (owner, name) in executed.into_inner() {
            let Some((_, x)) = types.method(&owner, &name) else { continue };
            let this_type = |t: &str| if t == "thistype" { owner.clone() } else { t.to_string() };
            let mut params = vec![Param { ty: owner.clone(), name: format!("{}o", crate::jass::PREFIX) }];
            params.extend(x.params.iter().map(|p| Param { ty: this_type(&p.ty), name: p.name.clone() }));
            let call = MethodCall { object: Expr::var(format!("{}o", crate::jass::PREFIX)), name: class_method(&name, &owner), args: x.params.iter().map(|p| Expr::var(p.name.clone())).collect() };
            m.functions.push(Function {
                sig: Signature { name: executor(&owner, &name), params, returns: None, constant: false },
                body: vec![Stmt::CallMethod { call, line: 0 }],
                owner: Owner::default(),
                origin: Origin::file("the build"),
            });
        }
        function_pointers(m, &types, &pointers, &mut setup);
        // Function interface types and array structs are integers; a
        // struct's stays an object.
        let integer = |t: &mut String| {
            if types.functions.contains_key(t.as_str()) || types.infos.get(t.as_str()).is_some_and(|i| i.array) {
                *t = "integer".into();
            }
        };
        m.globals.iter_mut().for_each(|g| integer(&mut g.ty));
        for f in &mut m.functions {
            f.sig.params.iter_mut().for_each(|p| integer(&mut p.ty));
            if let Some(r) = &mut f.sig.returns {
                integer(r);
            }
            local_types_integer(&mut f.body, &|t| types.functions.contains_key(t) || types.infos.get(t).is_some_and(|i| i.array));
        }
        return Lowered { inits, setup };
    }
    for (root, name) in dispatched.into_inner() {
        dispatcher(m, &types, &root, &name, &mut setup);
    }
    for (root, list) in &types.families {
        destroy_triggers(m, &types, root, list, &mut setup);
    }
    function_pointers(m, &types, &pointers, &mut setup);

    // Struct, interface and function interface types are integers.
    let integer = |t: &mut String| {
        if types.infos.contains_key(t.as_str()) || types.functions.contains_key(t.as_str()) {
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
        local_types_integer(&mut f.body, &|t| types.infos.contains_key(t) || types.functions.contains_key(t));
    }
    Lowered { inits, setup }
}

/// What an array struct (`extends array`: its instances only numbers,
/// nothing creates or destroys them) can't have, its modules taken in: a
/// member's default value, an array member, `onDestroy`. An error each,
/// said as the editor says it (`jass_rs::array_struct`).
fn array_struct_rules(s: &Struct, errors: &mut Vec<Error>) {
    use jass_rs::array_struct as says;
    let mut err = |message: String| errors.push(Error { origin: s.origin.clone(), message });
    for f in s.fields.iter().filter(|f| !f.is_static) {
        if f.init.is_some() {
            err(says::default_value(&s.name, &f.name, None));
        }
        if f.array.is_some() {
            err(says::array_member(&s.name, &f.name, None));
        }
    }
    if s.methods.iter().any(|me| !me.is_static && me.function.sig.name == "onDestroy") {
        err(says::on_destroy(&s.name, None));
    }
}

/// Why `what` (`allocate`, `create`, `destroy`, `deallocate`) isn't there
/// for the array struct `s`.
fn no_allocation(s: &str, what: &str) -> String {
    jass_rs::array_struct::call(s, what)
}

/// The function an object's method `.execute` runs (classes'): it takes
/// the object first.
fn executor(owner: &str, m: &str) -> String {
    format!("{}x_{owner}_{}", crate::jass::PREFIX, class_method(m, owner))
}

/// An instance method's name in its class: an operator's as
/// [`lower`] says, `onDestroy` its struct's own.
pub(crate) fn class_method(m: &str, owner: &str) -> String {
    match m.strip_prefix("operator ") {
        Some("[]") => "wcb__getindex".into(),
        Some("[]=") => "wcb__setindex".into(),
        Some("<") => "wcb__lt".into(),
        Some(x) => match x.strip_suffix('=') {
            Some(x) => format!("wcb__set_{x}"),
            None => format!("wcb__get_{x}"),
        },
        None if m == "onDestroy" => format!("onDestroy__{owner}"),
        None => m.into(),
    }
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
/// The nearest parent of `s` (not `s`) with a `create` of its own, and
/// that `create`.
fn parent_create<'a>(types: &'a Types, structs: &[Struct], s: &str) -> Option<(&'a Info, &'a MethodInfo)> {
    types.chain(s).into_iter().skip(1).find_map(|i| {
        let own = structs.iter().any(|x| x.name == i.name && x.methods.iter().any(|me| me.is_static && me.function.sig.name == "create"));
        own.then(|| i.methods.get("create").map(|x| (i, x))).flatten()
    })
}

/// Classes': the function `name`, `p`'s own `create` made to make a `c`
/// (a child of `p`): its allocation `s__c__new()` (the printer's, an
/// object of `c`), or, when `p` is itself allocated through a parent's
/// `create`, that one made so too.
fn via_create(m: &mut Module, types: &Types, structs: &[Struct], c: &str, p: &str, name: &str) {
    let Some(mut f) = m.functions.iter().find(|f| f.sig.name == format!("s__{p}_create")).cloned() else { return };
    let alloc = format!("s__{p}__allocate");
    let to = match parent_create(types, structs, p) {
        Some((gp, _)) => {
            let via = format!("s__{c}__via_{}", gp.name);
            via_create(m, types, structs, c, &gp.name.clone(), &via);
            via
        }
        None => format!("s__{c}__new"),
    };
    let mut to_map = HashMap::new();
    to_map.insert(alloc, to);
    retarget_calls(&mut f.body, &to_map);
    f.sig.name = name.to_string();
    m.functions.push(f);
}

/// Calls of the functions `to` names, made of the ones it names for them.
fn retarget_calls(body: &mut [Stmt], to: &HashMap<String, String>) {
    fn expr(e: &mut Expr, to: &HashMap<String, String>) {
        if let Expr::Call(c) = e
            && let Some(t) = to.get(&c.name)
        {
            c.name = t.clone();
        }
        for x in e.children_mut() {
            expr(x, to);
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
                    retarget_calls(b, to);
                }
                if let Some(b) = otherwise {
                    retarget_calls(b, to);
                }
            }
            Stmt::Loop { body, .. } => retarget_calls(body, to),
            Stmt::ExitWhen { cond, .. } => expr(cond, to),
            Stmt::Return { value, .. } => value.iter_mut().for_each(|e| expr(e, to)),
            Stmt::Debug { stmt, .. } => retarget_calls(std::slice::from_mut(&mut **stmt), to),
            other => {
                let (es, bs) = other.oop_parts_mut();
                es.into_iter().for_each(|e| expr(e, to));
                bs.into_iter().for_each(|b| retarget_calls(b, to));
            }
        }
    }
}

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
    // A parent with a `create` of its own: allocating is that (as
    // JassHelper does), taking what it takes; the instance made this
    // struct's after.
    if let Some((p, create)) = parent_create(types, structs, &s.name) {
        let params = create.params.clone();
        let mut alloc = vec![
            Stmt::local("integer", "this", Some(Expr::call(p.method("create"), params.iter().map(|x| Expr::var(x.name.clone())).collect()))),
            Stmt::If { branches: vec![(Expr::binary(this(), BinOp::Eq, Expr::int(0)), vec![Stmt::Return { value: Some(Expr::int(0)), line: 0 }])], otherwise: None, line: 0 },
        ];
        if several {
            alloc.push(Stmt::set_at(types.type_global(&root), this(), Expr::int(info.typeid)));
        }
        // The fields with a value of the structs below the parent.
        for i in types.chain(&s.name).into_iter().take_while(|i| i.name != p.name).collect::<Vec<_>>().into_iter().rev() {
            let Some(x) = structs.iter().find(|x| x.name == i.name) else { continue };
            for f in x.fields.iter().filter(|f| !f.is_static && f.array.is_none()) {
                if let Some(init) = &f.init {
                    alloc.push(Stmt::set_at(i.field(&f.name), this(), init.clone()));
                }
            }
        }
        alloc.push(Stmt::Return { value: Some(this()), line: 0 });
        let mut a = Function::new(info.method("_allocate"), alloc, s.origin.clone());
        a.sig.params = params;
        a.sig.returns = Some("integer".into());
        m.functions.push(a);
        let mut d = Function::new(info.method("deallocate"), taken(&root, give_back(&root)), s.origin.clone());
        d.sig.params.push(Param { ty: "integer".into(), name: "this".into() });
        m.functions.push(d);
        return;
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
        let Some(x) = structs.iter().find(|x| x.name == i.name) else {
            continue;
        };
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
    let cond = Expr::binary(Expr::binary(this(), BinOp::NotEq, Expr::int(0)), BinOp::And, Expr::binary(Expr::Index(next, Box::new(this())), BinOp::Eq, Expr::int(-1)));
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
    let Some((_, me)) = types.method(root, name).or_else(|| types.families[root].iter().find_map(|s| types.method(s, name))) else {
        return;
    };
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
        let Some((owner, x)) = types.method(s, name) else {
            continue;
        };
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

/// Each function interface's runner, `wcb__fi_I` (`.evaluate`) and
/// `wcb__fx_I` (`.execute`): the arguments into its globals, then the
/// trigger of the function's number evaluated (its condition calls it and
/// keeps what it returns) or executed (its action calls it, a thread of
/// its own); the triggers made before anything runs.
fn function_pointers(m: &mut Module, types: &Types, pointers: &Pointers, setup: &mut Vec<Stmt>) {
    let o = Origin::file("the build");
    let runs = pointers.runs.borrow();
    for (i, list) in pointers.functions.borrow().iter() {
        let sig = &types.functions[i];
        let (evaluate, execute) = runs.get(i).copied().unwrap_or_default();
        let triggers = format!("wcb__fit_{i}");
        let arg = |k: usize| format!("wcb__fia_{i}_{k}");
        let ret = format!("wcb__fir_{i}");
        let mut t = Global::new(triggers.clone(), "trigger", None, o.clone());
        t.array = true;
        m.globals.push(t);
        for (k, p) in sig.params.iter().enumerate() {
            m.globals.push(Global::new(arg(k), p.ty.clone(), None, o.clone()));
        }
        if let Some(r) = &sig.returns {
            m.globals.push(Global::new(ret.clone(), r.clone(), None, o.clone()));
        }
        let mut params = vec![Param { ty: "integer".into(), name: "wcb__f".into() }];
        params.extend(sig.params.iter().cloned());
        let set_args: Vec<Stmt> = sig.params.iter().enumerate().map(|(k, p)| Stmt::set(arg(k), Expr::var(p.name.clone()))).collect();
        let at = || Expr::Index(triggers.clone(), Box::new(Expr::var("wcb__f")));
        if evaluate {
            let mut body = set_args.clone();
            body.push(Stmt::call("TriggerEvaluate", vec![at()]));
            if sig.returns.is_some() {
                body.push(Stmt::Return { value: Some(Expr::var(ret.clone())), line: 0 });
            }
            let mut f = Function::new(format!("wcb__fi_{i}"), body, o.clone());
            f.sig.params = params.clone();
            f.sig.returns = sig.returns.clone();
            m.functions.push(f);
        }
        if execute {
            let mut body = set_args.clone();
            body.push(Stmt::call("TriggerExecute", vec![at()]));
            let mut f = Function::new(format!("wcb__fx_{i}"), body, o.clone());
            f.sig.params = params.clone();
            m.functions.push(f);
        }
        for (k, name) in list.iter().enumerate() {
            let number = k + 1;
            let call = Call { name: name.clone(), args: (0..sig.params.len()).map(|j| Expr::var(arg(j))).collect() };
            let at = || Expr::Index(triggers.clone(), Box::new(Expr::int(number)));
            setup.push(Stmt::set_at(triggers.clone(), Expr::int(number), Expr::call("CreateTrigger", vec![])));
            if evaluate || !execute {
                let thunk = format!("wcb__fithunk_{i}_{number}");
                let mut body = vec![match sig.returns {
                    Some(_) => Stmt::set(ret.clone(), Expr::Call(call.clone())),
                    None => Stmt::Call { call: call.clone(), line: 0 },
                }];
                body.push(Stmt::Return { value: Some(Expr::lit("true")), line: 0 });
                let mut f = Function::new(thunk.clone(), body, o.clone());
                f.sig.returns = Some("boolean".into());
                m.functions.push(f);
                setup.push(Stmt::call("TriggerAddCondition", vec![at(), Expr::call("Condition", vec![Expr::code(thunk)])]));
            }
            if execute {
                let action = format!("wcb__fiaction_{i}_{number}");
                m.functions.push(Function::new(action.clone(), vec![Stmt::Call { call, line: 0 }], o.clone()));
                setup.push(Stmt::call("TriggerAddAction", vec![at(), Expr::code(action)]));
            }
        }
    }
}

/// Locals' types (`thistype` made its struct), into `types`.
fn local_types(body: &mut [Stmt], this_type: &dyn Fn(&str) -> String, types: &mut HashMap<String, String>, arrays: &mut HashSet<String>) {
    for s in body {
        match s {
            Stmt::Local { ty, name, array, .. } => {
                *ty = this_type(ty);
                types.insert(name.clone(), ty.clone());
                if *array {
                    arrays.insert(name.clone());
                }
            }
            Stmt::If { branches, otherwise, .. } => {
                branches.iter_mut().for_each(|(_, b)| local_types(b, this_type, types, arrays));
                if let Some(b) = otherwise {
                    local_types(b, this_type, types, arrays);
                }
            }
            Stmt::Loop { body, .. } => local_types(body, this_type, types, arrays),
            _ => {}
        }
    }
}

fn local_types_integer(body: &mut [Stmt], integer: &dyn Fn(&str) -> bool) {
    for s in body {
        match s {
            Stmt::Local { ty, .. } if integer(ty) => *ty = "integer".into(),
            Stmt::If { branches, otherwise, .. } => {
                branches.iter_mut().for_each(|(_, b)| local_types_integer(b, integer));
                if let Some(b) = otherwise {
                    local_types_integer(b, integer);
                }
            }
            Stmt::Loop { body, .. } => local_types_integer(body, integer),
            _ => {}
        }
    }
}

/// What a function's members are written in.
struct Cx<'a> {
    /// AngelScript's classes ([`lower`]).
    classes: bool,
    /// Classes': what every function and native takes.
    takes: &'a HashMap<String, Vec<String>>,
    /// What the function gives back.
    returns: Option<&'a str>,
    /// The sized global arrays' sizes (`a.size`).
    sizes: &'a HashMap<String, Expr>,
    /// Classes': the objects' methods run by `.execute` (struct, method).
    executed: &'a RefCell<BTreeSet<(String, String)>>,
    types: &'a Types,
    /// Functions: what each returns.
    sigs: &'a HashMap<String, Option<String>>,
    /// Functions' parameters' types.
    params: &'a HashMap<String, Vec<String>>,
    globals: &'a HashMap<String, String>,
    locals: &'a HashMap<String, String>,
    /// The locals and globals that are arrays.
    arrays: &'a HashSet<String>,
    global_arrays: &'a HashSet<String>,
    /// The struct of the method.
    current: Option<&'a str>,
    file: &'a str,
    /// The calls by type made: family root, method.
    dispatched: &'a RefCell<BTreeSet<(String, String)>>,
    pointers: &'a Pointers,
}

/// Function interfaces' functions taken (`I.f`), and how they're run.
#[derive(Default)]
struct Pointers {
    /// Each interface's functions, by their numbers (from 1).
    functions: RefCell<BTreeMap<String, Vec<String>>>,
    /// Each interface's runs: `.evaluate`, `.execute`.
    runs: RefCell<BTreeMap<String, (bool, bool)>>,
}

impl Pointers {
    /// `f`'s number in `i`.
    fn number(&self, i: &str, f: &str) -> usize {
        let mut all = self.functions.borrow_mut();
        let list = all.entry(i.to_string()).or_default();
        match list.iter().position(|g| g == f) {
            Some(k) => k + 1,
            None => {
                list.push(f.to_string());
                list.len()
            }
        }
    }
}

fn cast(t: &str, e: Expr) -> Expr {
    Expr::call(format!("wcb__cast__{t}"), vec![e])
}

fn null_object() -> Expr {
    Expr::call("wcb__null", vec![])
}

fn is_zero(e: &Expr) -> bool {
    matches!(e, Expr::Literal(l) if l == "0" || l == "null")
}

impl Cx<'_> {
    fn error(&self, errors: &mut Vec<Error>, line: usize, message: String) {
        errors.push(Error { origin: Origin { file: self.file.to_string(), line }, message });
    }

    /// Is `t` a class (AngelScript's)?
    fn is_class(&self, t: &str) -> bool {
        self.classes && self.types.infos.get(t).is_some_and(|i| !i.array)
    }

    /// `e` (of type `have`) where a `want` goes: with classes, an object's
    /// number where an integer goes and back, a cast to a child, `0` as no
    /// object; as it is otherwise.
    fn coerce(&self, e: Expr, have: Option<&str>, want: Option<&str>) -> Expr {
        if !self.classes {
            return e;
        }
        let have_c = have.filter(|t| self.is_class(t));
        let want_c = want.filter(|t| self.is_class(t));
        match (have_c, want_c) {
            (Some(h), Some(w)) => {
                if self.types.chain(h).iter().any(|i| i.name == w) {
                    e
                } else {
                    cast(w, e)
                }
            }
            (Some(h), None) if want.is_some() => Expr::call(format!("s__{}__id", self.types.root(h)), vec![e]),
            (None, Some(_)) if is_zero(&e) => null_object(),
            (None, Some(w)) if have == Some("integer") || matches!(e, Expr::Literal(_)) => cast(w, Expr::call(format!("s__{}__at", self.types.root(w)), vec![e])),
            _ => e,
        }
    }

    /// Arguments (of types `have`) where `want` go ([`Cx::coerce`]).
    fn coerce_args(&self, args: Vec<Expr>, have: &[Option<String>], want: Option<&Vec<String>>) -> Vec<Expr> {
        if !self.classes {
            return args;
        }
        args.into_iter().enumerate().map(|(i, a)| self.coerce(a, have.get(i).cloned().flatten().as_deref(), want.and_then(|w| w.get(i)).map(String::as_str))).collect()
    }

    /// `e` where an integer goes (an index): an object's number.
    fn integer(&self, e: Expr, line: usize, errors: &mut Vec<Error>) -> Expr {
        let (e, t) = self.expr(e, line, errors);
        self.coerce(e, t.as_deref(), Some("integer"))
    }

    /// The function `e` names as an object (`F`, `S.m`, `thistype.m`): its
    /// name as built; `None` when it isn't one, or (classes') it's an
    /// object's method, no function.
    fn function_name(&self, e: &Expr) -> Option<String> {
        match e {
            Expr::Var(f) if self.type_of_var(f).is_none() && self.sigs.contains_key(f) => Some(f.clone()),
            Expr::Member(o, m) => {
                let info = self.struct_named(o)?;
                let (owner, x) = self.types.method(&info.name, m)?;
                (!self.is_class(&owner.name) || x.is_static).then(|| owner.method(m))
            }
            _ => None,
        }
    }

    /// The size of the sized array `e` names: a global (`a`), a member
    /// (`x.a`, `S.a`); `None` when it names none.
    fn size_of(&self, e: &Expr) -> Option<Expr> {
        match e {
            Expr::Var(n) if !self.locals.contains_key(n) => self.sizes.get(n).cloned(),
            Expr::Member(o, f) => {
                let t = match self.struct_named(o) {
                    Some(info) => info.name.clone(),
                    None => self.type_of_expr(o)?,
                };
                let (owner, (_, is_static, k, array)) = self.types.field(&t, f)?;
                if !*array {
                    return None;
                }
                if *is_static { self.sizes.get(&owner.field(f)).cloned() } else { k.map(Expr::int) }
            }
            _ => None,
        }
    }

    /// The type of `e`, when it's a name (a variable, `this`) or a member
    /// of one, without lowering it.
    fn type_of_expr(&self, e: &Expr) -> Option<String> {
        match e {
            Expr::Var(n) => self.type_of_var(n),
            Expr::Member(o, f) => {
                let t = match self.struct_named(o) {
                    Some(info) => info.name.clone(),
                    None => self.type_of_expr(o)?,
                };
                self.types.field(&t, f).map(|(_, x)| x.0.clone())
            }
            _ => None,
        }
    }

    /// `S.m` naming an object's method (classes': no function): whose, and
    /// which.
    fn object_method(&self, e: &Expr) -> Option<(String, String)> {
        let Expr::Member(o, m) = e else { return None };
        let info = self.struct_named(o)?;
        let (owner, x) = self.types.method(&info.name, m)?;
        (self.is_class(&owner.name) && !x.is_static).then(|| (owner.name.clone(), m.clone()))
    }

    /// Arguments lowered, and their types.
    fn args(&self, args: Vec<Expr>, line: usize, errors: &mut Vec<Error>) -> (Vec<Expr>, Vec<Option<String>>) {
        args.into_iter().map(|a| self.expr(a, line, errors)).unzip()
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

    fn is_array(&self, n: &str) -> bool {
        if self.locals.contains_key(n) { self.arrays.contains(n) } else { self.global_arrays.contains(n) }
    }

    /// Has `t` (or a parent) the operator `op` (`x`, `x=`, `[]`, `[]=`, `<`)?
    fn has_operator(&self, t: &str, op: &str) -> bool {
        self.types.infos.contains_key(t) && self.types.method(t, &format!("operator {op}")).is_some()
    }

    /// A call of `t`'s operator `op` (which it has), of `instance` but for
    /// a static one.
    fn operator(&self, t: &str, instance: Option<Expr>, op: &str, mut args: Vec<Expr>, line: usize, errors: &mut Vec<Error>) -> (Expr, Option<String>) {
        let name = format!("operator {op}");
        let (owner, x) = self.types.method(t, &name).expect("has_operator");
        if self.is_class(t) && !x.is_static {
            return match instance {
                Some(o) => (Expr::Method(Box::new(MethodCall { object: o, name: class_method(&name, &owner.name), args })), x.returns.clone()),
                None => {
                    self.error(errors, line, format!("{t}'s `operator {op}`: an instance's"));
                    (Expr::lit("0"), None)
                }
            };
        }
        if self.types.dispatches(t, &name) {
            self.error(errors, line, format!("{t}'s `operator {op}` by the instance's type (an interface's) isn't built into JASS yet"));
            return (Expr::lit("0"), None);
        }
        if !x.is_static {
            match instance {
                Some(o) => args.insert(0, o),
                None => {
                    self.error(errors, line, format!("{t}'s `operator {op}`: an instance's"));
                    return (Expr::lit("0"), None);
                }
            }
        }
        (Expr::Call(Call { name: owner.method(&name), args }), x.returns.clone())
    }

    /// What `.name` is of: a struct by its name (no instance), or an
    /// instance (its type, and it).
    fn object(&self, object: Expr, name: &str, line: usize, errors: &mut Vec<Error>) -> Option<(String, Option<Expr>)> {
        if let Some(info) = self.struct_named(&object) {
            return Some((info.name.clone(), None));
        }
        let (o, t) = self.expr(object, line, errors);
        if self.classes && o == null_object() {
            self.error(errors, line, format!("`.{name}` of instance 0, which in AngelScript is no object (null): a static field instead"));
            return None;
        }
        match t.filter(|t| self.types.infos.contains_key(t)) {
            Some(t) => Some((t, Some(o))),
            None => {
                self.error(errors, line, format!("`.{name}` of what isn't a struct (or whose type can't be told)"));
                None
            }
        }
    }

    /// `.name` read: a field, or the operator `name`.
    fn member(&self, t: String, instance: Option<Expr>, name: &str, line: usize, errors: &mut Vec<Error>) -> (Expr, Option<String>) {
        if self.types.field(&t, name).is_none() && self.has_operator(&t, name) {
            return self.operator(&t, instance, name, vec![], line, errors);
        }
        // What a `delegate` has: `a.x` is `a.d.x`.
        if self.types.field(&t, name).is_none()
            && instance.is_some()
            && let Some((d, dt)) = self.types.delegate(&t, &|u| self.types.field(u, name).is_some() || self.has_operator(u, name))
        {
            let v = self.member(t, instance, &d, line, errors).0;
            return self.member(dt, Some(v), name, line, errors);
        }
        if self.is_class(&t)
            && let (Some((_, (ty, false, _, _))), Some(o)) = (self.types.field(&t, name), &instance)
        {
            return (Expr::Member(Box::new(o.clone()), name.to_string()), Some(ty.clone()));
        }
        match self.field_of(t, instance, name, None, line, errors) {
            Some((array, index, ty)) => (index.map_or_else(|| Expr::Var(array.clone()), |i| Expr::Index(array.clone(), Box::new(i))), Some(ty)),
            None => (Expr::lit("0"), None),
        }
    }

    /// `v[i]` of what isn't an array: its type's `operator []`.
    fn element(&self, v: Expr, t: Option<String>, i: Expr, line: usize, errors: &mut Vec<Error>) -> (Expr, Option<String>) {
        match t.filter(|t| self.has_operator(t, "[]")) {
            Some(t) => self.operator(&t, Some(v), "[]", vec![i], line, errors),
            None => {
                self.error(errors, line, "an element of what's no array and has no `operator []`".into());
                (Expr::lit("0"), None)
            }
        }
    }

    /// `set v[i] = value` of what isn't an array: its type's `operator []=`.
    fn set_element(&self, v: Expr, t: Option<String>, i: Expr, value: Expr, line: usize, errors: &mut Vec<Error>) -> Stmt {
        match t.filter(|t| self.has_operator(t, "[]=")) {
            Some(t) => self.call_stmt(self.operator(&t, Some(v), "[]=", vec![i, value], line, errors).0, line),
            None => {
                self.error(errors, line, "an element set of what's no array and has no `operator []=`".into());
                Stmt::Call { call: Call { name: "DoNothing".into(), args: vec![] }, line }
            }
        }
    }

    fn call_stmt(&self, e: Expr, line: usize) -> Stmt {
        match e {
            Expr::Call(call) => Stmt::Call { call, line },
            // A class's method (AngelScript's).
            Expr::Method(call) => Stmt::CallMethod { call: *call, line },
            _ => Stmt::Call { call: Call { name: "DoNothing".into(), args: vec![] }, line },
        }
    }

    /// `x[i]` of a name: an array's element, else the operator `[]` of
    /// the variable's struct (or of the struct so named, static).
    fn indexed(&self, n: &str) -> Option<(String, Option<Expr>)> {
        if self.is_array(n) {
            return None;
        }
        if let Some(t) = self.type_of_var(n) {
            return Some((t, Some(Expr::Var(n.to_string()))));
        }
        self.struct_named(&Expr::Var(n.to_string())).map(|i| (i.name.clone(), None))
    }

    fn stmts(&self, body: Vec<Stmt>, errors: &mut Vec<Error>) -> Vec<Stmt> {
        body.into_iter().map(|s| self.stmt(s, errors)).collect()
    }

    fn stmt(&self, s: Stmt, errors: &mut Vec<Error>) -> Stmt {
        match s {
            Stmt::Local { ty, name, array, init, line } => {
                let init = init.map(|e| {
                    let (e, t) = self.expr(e, line, errors);
                    self.coerce(e, t.as_deref(), Some(&ty))
                });
                Stmt::Local { ty, name, array, init, line }
            }
            Stmt::Set { name, index: Some(i), value, line } if self.indexed(&name).is_some_and(|(t, _)| self.has_operator(&t, "[]=")) => {
                let (t, o) = self.indexed(&name).unwrap();
                let i = self.expr(i, line, errors).0;
                let value = self.expr(value, line, errors).0;
                self.call_stmt(self.operator(&t, o, "[]=", vec![i, value], line, errors).0, line)
            }
            Stmt::Set { name, index, value, line } => {
                let index = index.map(|e| self.integer(e, line, errors));
                let (value, t) = self.expr(value, line, errors);
                let value = self.coerce(value, t.as_deref(), self.type_of_var(&name).as_deref());
                Stmt::Set { name, index, value, line }
            }
            // A cast called for nothing is nothing.
            Stmt::Call { call, line } => self.call_stmt(self.expr(Expr::Call(call), line, errors).0, line),
            Stmt::If { branches, otherwise, line } => {
                Stmt::If { branches: branches.into_iter().map(|(c, b)| (self.expr(c, line, errors).0, self.stmts(b, errors))).collect(), otherwise: otherwise.map(|b| self.stmts(b, errors)), line }
            }
            Stmt::Loop { body, line } => Stmt::Loop { body: self.stmts(body, errors), line },
            Stmt::ExitWhen { cond, line } => Stmt::ExitWhen { cond: self.expr(cond, line, errors).0, line },
            Stmt::Return { value, line } => Stmt::Return {
                value: value.map(|e| {
                    let (e, t) = self.expr(e, line, errors);
                    self.coerce(e, t.as_deref(), self.returns)
                }),
                line,
            },
            Stmt::Debug { stmt, line } => Stmt::Debug { stmt: Box::new(self.stmt(*stmt, errors)), line },
            Stmt::SetMember { object, name, index, value, line } => {
                let (value, vt) = self.expr(value, line, errors);
                let index = index.map(|e| self.integer(e, line, errors));
                match self.object(object, &name, line, errors) {
                    Some((t, instance)) => {
                        let ty = self.types.field(&t, &name).map(|(_, f)| f.0.clone());
                        let value = self.coerce(value, vt.as_deref(), ty.as_deref());
                        self.set_member(t, instance, &name, index, value, line, errors)
                    }
                    None => Stmt::Set { name: "wcb__error".into(), index: None, value, line },
                }
            }
            Stmt::CallMethod { call, line } => self.call_stmt(self.expr(Expr::Method(Box::new(call)), line, errors).0, line),
            s @ Stmt::StaticIf { .. } => s,
        }
    }

    /// `set a.name[index] = value`: a field, the operator `name=` or
    /// `[]=`, or a `delegate`'s.
    #[allow(clippy::too_many_arguments)]
    fn set_member(&self, t: String, instance: Option<Expr>, name: &str, index: Option<Expr>, value: Expr, line: usize, errors: &mut Vec<Error>) -> Stmt {
        let error = |value| Stmt::Set { name: "wcb__error".into(), index: None, value, line };
        if self.types.field(&t, name).is_none()
            && !self.has_operator(&t, &format!("{name}="))
            && instance.is_some()
            && let Some((d, dt)) = self.types.delegate(&t, &|u| self.types.field(u, name).is_some() || self.has_operator(u, &format!("{name}=")))
        {
            let v = self.member(t, instance, &d, line, errors).0;
            return self.set_member(dt, Some(v), name, index, value, line, errors);
        }
        if self.is_class(&t)
            && let (Some((_, (_, false, _, _))), Some(o)) = (self.types.field(&t, name), &instance)
        {
            return Stmt::SetMember { object: o.clone(), name: name.to_string(), index, value, line };
        }
        {
            let field = self.types.field(&t, name).map(|(_, f)| f.3);
            match (field, index) {
                // `set a.x = v`: the operator `x=`.
                (None, None) if self.has_operator(&t, &format!("{name}=")) => self.call_stmt(self.operator(&t, instance, &format!("{name}="), vec![value], line, errors).0, line),
                // `set a.x[i] = v` of what isn't an array field.
                (None | Some(false), Some(i)) => {
                    let (v, vt) = self.member(t, instance, name, line, errors);
                    self.set_element(v, vt, i, value, line, errors)
                }
                (_, index) => match self.field_of(t, instance, name, index, line, errors) {
                    Some((array, index, _)) => Stmt::Set { name: array, index, value, line },
                    None => error(value),
                },
            }
        }
    }

    /// A field: the global it's in, the index (none for a static one's),
    /// its type.
    fn field_of(&self, t: String, instance: Option<Expr>, name: &str, index: Option<Expr>, line: usize, errors: &mut Vec<Error>) -> Option<(String, Option<Expr>, String)> {
        let Some((owner, (ty, is_static, k, _))) = self.types.field(&t, name) else {
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
                let i = self.integer(*i, line, errors);
                // `x[i]` of a struct with `operator []`, `S[i]` of a static one.
                if let Some((t, o)) = self.indexed(&n).filter(|(t, _)| self.has_operator(t, "[]")) {
                    return self.operator(&t, o, "[]", vec![i], line, errors);
                }
                // `S[i]` of an array struct (`extends array`): its instance `i`.
                if !self.is_array(&n)
                    && self.type_of_var(&n).is_none()
                    && let Some(info) = self.struct_named(&Expr::Var(n.clone())).filter(|info| info.array)
                {
                    return (i, Some(info.name.clone()));
                }
                let t = self.type_of_var(&n);
                (Expr::Index(n, Box::new(i)), t)
            }
            Expr::Call(Call { name, args }) => {
                let (mut args, have) = self.args(args, line, errors);
                // A cast: `Data(x)`, `thistype(x)`.
                let cast = if name == "thistype" { self.current.map(String::from) } else { self.types.infos.contains_key(&name).then(|| name.clone()) };
                if let Some(t) = cast
                    && args.len() == 1
                {
                    // From what can't be told: an integer, as a cast mostly is.
                    let e = self.coerce(args.remove(0), have[0].as_deref().or(Some("integer")), Some(&t));
                    return (e, Some(t));
                }
                let args = self.coerce_args(args, &have, self.takes.get(&name));
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
            // `F.name`, `S.m.name`: the name the function is built with, for
            // `ExecuteFunc` and the like.
            Expr::Member(object, name) if name == "name" && self.function_name(&object).is_some() => {
                let f = self.function_name(&object).expect("checked");
                (Expr::string(&f), Some("string".into()))
            }
            // `a.size` of a sized array: a global's, a member's.
            Expr::Member(object, name) if name == "size" && self.size_of(&object).is_some() => (self.size_of(&object).expect("checked"), Some("integer".into())),
            // An object's method has no function to name (AngelScript's).
            Expr::Member(object, name) if name == "name" && self.object_method(&object).is_some() => {
                let (owner, m) = self.object_method(&object).expect("checked");
                self.error(errors, line, format!("{owner}.{m}.name: in AngelScript an object's method is no function of its own, so it has no name to give (ExecuteFunc can't run it): a static method instead"));
                (Expr::string(""), Some("string".into()))
            }
            // `I.f`: the number of `f` among `I`'s functions.
            Expr::Member(object, name) if matches!(&*object, Expr::Var(i) if self.types.functions.contains_key(i) && self.type_of_var(i).is_none()) => {
                let Expr::Var(i) = *object else { unreachable!() };
                let Some(params) = self.params.get(&name) else {
                    self.error(errors, line, format!("{i}.{name}: no function {name}"));
                    return (Expr::lit("0"), None);
                };
                let sig = &self.types.functions[&i];
                if params.len() != sig.params.len() || params.iter().zip(&sig.params).any(|(a, b)| *a != b.ty) || self.sigs[&name] != sig.returns {
                    self.error(errors, line, format!("{i}.{name}: {name} doesn't take and return what {i} does"));
                }
                (Expr::int(self.pointers.number(&i, &name)), Some(i))
            }
            Expr::Member(object, name) => {
                // `S.typeid`: the struct's type.
                if name == "typeid"
                    && let Some(info) = self.struct_named(&object)
                {
                    return (Expr::int(info.typeid), Some("integer".into()));
                }
                match self.object(*object, &name, line, errors) {
                    Some((t, instance)) => self.member(t, instance, &name, line, errors),
                    None => (Expr::lit("0"), None),
                }
            }
            Expr::Element(a, i) => {
                let i = self.integer(*i, line, errors);
                match *a {
                    Expr::Member(object, name) => {
                        let Some((t, instance)) = self.object(*object, &name, line, errors) else {
                            return (Expr::lit("0"), None);
                        };
                        // An array field's element.
                        if self.is_class(&t)
                            && let (Some((_, (ty, false, _, true))), Some(o)) = (self.types.field(&t, &name), &instance)
                        {
                            return (Expr::Element(Box::new(Expr::Member(Box::new(o.clone()), name.clone())), Box::new(i)), Some(ty.clone()));
                        }
                        if self.types.field(&t, &name).is_some_and(|(_, f)| f.3) {
                            return match self.field_of(t, instance, &name, Some(i), line, errors) {
                                Some((array, Some(index), ty)) => (Expr::Index(array, Box::new(index)), Some(ty)),
                                _ => (Expr::lit("0"), None),
                            };
                        }
                        let (v, vt) = self.member(t, instance, &name, line, errors);
                        self.element(v, vt, i, line, errors)
                    }
                    a => {
                        let (v, vt) = self.expr(a, line, errors);
                        self.element(v, vt, i, line, errors)
                    }
                }
            }
            Expr::Method(m) => self.method(*m, line, errors),
            Expr::Unary(op, x) => {
                let (x, t) = self.expr(*x, line, errors);
                (Expr::Unary(op, Box::new(x)), t)
            }
            Expr::Binary(a, op, b) => {
                let (a, t) = self.expr(*a, line, errors);
                let (b, tb) = self.expr(*b, line, errors);
                let tb_kept = tb.clone();
                // `a < b` of structs with `operator <` (`a > b` is `b < a`,
                // `a <= b` is `not (b < a)`, `a >= b` is `not (a < b)`).
                if matches!(op, BinOp::Lt | BinOp::Gt | BinOp::LtEq | BinOp::GtEq)
                    && let Some(st) = t.clone().filter(|t| self.has_operator(t, "<")).or_else(|| tb.filter(|t| self.has_operator(t, "<")))
                {
                    let (x, y) = if matches!(op, BinOp::Lt | BinOp::GtEq) { (a, b) } else { (b, a) };
                    let lt = self.operator(&st, Some(x), "<", vec![y], line, errors).0;
                    let e = if matches!(op, BinOp::LtEq | BinOp::GtEq) { Expr::Unary(UnaryOp::Not, Box::new(lt)) } else { lt };
                    return (e, Some("boolean".into()));
                }
                let tb = tb_kept;
                let t2 = if matches!(op, BinOp::Eq | BinOp::NotEq | BinOp::Lt | BinOp::Gt | BinOp::LtEq | BinOp::GtEq | BinOp::And | BinOp::Or) { Some("boolean".into()) } else { t.clone() };
                if self.classes {
                    let (ca, cb) = (t.as_deref().is_some_and(|x| self.is_class(x)), tb.as_deref().is_some_and(|x| self.is_class(x)));
                    if ca || cb {
                        // Objects compared: the same one; with 0, none.
                        if matches!(op, BinOp::Eq | BinOp::NotEq) && ((ca && cb) || (ca && is_zero(&b)) || (cb && is_zero(&a))) {
                            let side = |e: Expr| if is_zero(&e) { null_object() } else { e };
                            let same = Expr::call("wcb__same", vec![side(a), side(b)]);
                            let e = if op == BinOp::NotEq { Expr::Unary(UnaryOp::Not, Box::new(same)) } else { same };
                            return (e, Some("boolean".into()));
                        }
                        // Their numbers otherwise.
                        let a = self.coerce(a, t.as_deref(), Some("integer"));
                        let b = self.coerce(b, tb.as_deref(), Some("integer"));
                        let t2 = if t2.as_deref().is_some_and(|x| self.is_class(x)) { Some("integer".into()) } else { t2 };
                        return (Expr::Binary(Box::new(a), op, Box::new(b)), t2);
                    }
                }
                (Expr::Binary(Box::new(a), op, Box::new(b)), t2)
            }
            e @ Expr::Literal(_) => (e, None),
        }
    }

    fn method(&self, m: MethodCall, line: usize, errors: &mut Vec<Error>) -> (Expr, Option<String>) {
        let MethodCall { object, name, args } = m;
        let (mut args, have) = self.args(args, line, errors);
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
                        // An object's method: `.evaluate` called straight
                        // (in the same thread, as evaluate runs it);
                        // `.execute` through a function taking the object
                        // first (`wcb__x_S_m`), run as any function's.
                        if self.is_class(&owner.name) && !x.is_static {
                            let want: Vec<String> = x.params.iter().map(|p| p.ty.clone()).collect();
                            let args = self.coerce_args(args, &have, Some(&want));
                            return match (instance, name.as_str()) {
                                (Some(o), "evaluate") => (Expr::Method(Box::new(MethodCall { object: o, name: class_method(mname, &owner.name), args })), x.returns.clone()),
                                (Some(o), _) => {
                                    self.executed.borrow_mut().insert((owner.name.clone(), mname.clone()));
                                    let mut args = args;
                                    args.insert(0, o);
                                    (call(format!("{}exec_{}", crate::jass::PREFIX, executor(&owner.name, mname)), args), None)
                                }
                                (None, _) => {
                                    self.error(errors, line, format!("{}.{mname}: a method of an instance", owner.name));
                                    (Expr::lit("0"), None)
                                }
                            };
                        }
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
                    let want: Vec<String> = x.params.iter().map(|p| p.ty.clone()).collect();
                    let args = self.coerce_args(args, &have, Some(&want));
                    if self.is_class(&owner.name) && !x.is_static {
                        return (call(format!("{}::{}", owner.name, class_method(&name, &owner.name)), args), x.returns.clone());
                    }
                    let mut args = args;
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
        // `S.destroy(x)`, `S.deallocate(x)`: `x.destroy()`, unless S has a
        // static method so named.
        if let Some(info) = self.struct_named(&object)
            && matches!(name.as_str(), "destroy" | "deallocate")
            && args.len() == 1
            && self.types.method(&info.name, &name).is_none_or(|(_, x)| !x.is_static)
        {
            let t = info.name.clone();
            let o = self.coerce(args.remove(0), have[0].as_deref(), Some(&t));
            return self.instance_method(o, t, name, Vec::new(), line, errors);
        }
        // A struct's own: static.
        if let Some(info) = self.struct_named(&object) {
            let f = match name.as_str() {
                "create" | "allocate" if info.array && self.types.method(&info.name, &name).is_none() => {
                    self.error(errors, line, no_allocation(&info.name, &name));
                    return (Expr::lit("0"), None);
                }
                // Through a parent's own `create`: in AngelScript an object's
                // class is fixed when it's made, and that makes a parent.
                // AngelScript: `s__S__allocate`, the parent's `create` made to
                // make an S (`via_create`), cast to S.
                "allocate" | "create" if self.is_class(&info.name) && !info.methods.contains_key("create") && self.types.chain(&info.name).into_iter().skip(1).any(|i| i.methods.get("create").is_some_and(|x| x.is_static && !x.abstract_)) => {
                    let want: Option<Vec<String>> = self.types.method(&info.name, "create").map(|(_, x)| x.params.iter().map(|p| p.ty.clone()).collect());
                    let args = self.coerce_args(args, &have, want.as_ref());
                    return (cast(&info.name, call(info.method("_allocate"), args)), Some(info.name.clone()));
                }
                // A child without a `create` of its own: its `allocate`, which
                // goes through a parent's own `create`, if it has one.
                "create" if !info.methods.contains_key("create") && !info.array && !info.interface => info.method("_allocate"),
                "allocate" if !info.array && !info.interface => info.method("_allocate"),
                _ => match self.types.method(&info.name, &name) {
                    Some((owner, x)) if x.is_static => owner.method(&name),
                    _ => {
                        self.error(errors, line, format!("{}.{name}: no static method {name}", info.name));
                        return (Expr::lit("0"), None);
                    }
                },
            };
            // `create`, `allocate` not its own: an instance of it.
            let t = if matches!(name.as_str(), "create" | "allocate") && !info.methods.contains_key(&name) {
                Some(info.name.clone())
            } else {
                self.types.method(&info.name, &name).and_then(|(_, x)| x.returns.clone())
            };
            let want: Option<Vec<String>> = self.types.method(&info.name, &name).map(|(_, x)| x.params.iter().map(|p| p.ty.clone()).collect());
            let args = self.coerce_args(args, &have, want.as_ref());
            return (call(f, args), t);
        }
        let (o, t) = self.expr(object, line, errors);
        // A function interface's: run by its number, the arguments in
        // globals (`wcb__fi_I`, `wcb__fx_I`).
        if let Some(i) = t.as_ref().filter(|t| self.types.functions.contains_key(*t)) {
            let returns = self.types.functions[i].returns.clone();
            args.insert(0, o);
            let mut runs = self.pointers.runs.borrow_mut();
            let r = runs.entry(i.clone()).or_default();
            return match name.as_str() {
                "evaluate" => {
                    r.0 = true;
                    (call(format!("wcb__fi_{i}"), args), returns)
                }
                "execute" => {
                    r.1 = true;
                    (call(format!("wcb__fx_{i}"), args), None)
                }
                _ => {
                    drop(runs);
                    self.error(errors, line, format!(".{name}() of a function interface: only evaluate and execute"));
                    (Expr::lit("0"), None)
                }
            };
        }
        let Some(t) = t.filter(|t| self.types.infos.contains_key(t)) else {
            self.error(errors, line, format!(".{name}() of what isn't a struct (or whose type can't be told)"));
            return (Expr::lit("0"), None);
        };
        let args = match self.types.method(&t, &name) {
            Some((_, x)) => {
                let want: Vec<String> = x.params.iter().map(|p| p.ty.clone()).collect();
                self.coerce_args(args, &have, Some(&want))
            }
            None => args,
        };
        self.instance_method(o, t, name, args, line, errors)
    }

    /// `o.name(args)` of an instance `o` of `t`.
    fn instance_method(&self, o: Expr, t: String, name: String, mut args: Vec<Expr>, line: usize, errors: &mut Vec<Error>) -> (Expr, Option<String>) {
        let call = |name: String, args: Vec<Expr>| Expr::Call(Call { name, args });
        let several = self.types.several(&t);
        if self.is_class(&t) {
            let root = self.types.root(&t).to_string();
            match name.as_str() {
                "destroy" | "deallocate" if self.types.method(&t, &name).is_none() => return (call(format!("s__{root}__{name}"), vec![o]), None),
                "getType" if self.types.method(&t, &name).is_none() => {
                    return (Expr::Method(Box::new(MethodCall { object: o, name: "wcb__type".into(), args: vec![] })), Some("integer".into()));
                }
                _ => {}
            }
        }
        match name.as_str() {
            "destroy" if several => {
                return (call(format!("s__{}__destroy", self.types.root(&t)), vec![o]), None);
            }
            // A struct alone's own, unless it has a method of the name.
            "destroy" | "deallocate" if self.types.method(&t, &name).is_none() => {
                let info = &self.types.infos[&t];
                if info.array {
                    self.error(errors, line, no_allocation(&t, &name));
                    return (Expr::lit("0"), None);
                }
                if !info.interface {
                    return (call(info.method(&name), vec![o]), None);
                }
            }
            // `x.getType()`: the instance's type.
            "getType" if several => {
                return (Expr::Index(self.types.type_global(&t), Box::new(o)), Some("integer".into()));
            }
            _ => {}
        }
        // What a `delegate` has: `a.m()` is `a.d.m()`.
        if self.types.method(&t, &name).is_none()
            && let Some((d, dt)) = self.types.delegate(&t, &|u| self.types.method(u, &name).is_some())
        {
            let v = self.member(t, Some(o), &d, line, errors).0;
            return self.instance_method(v, dt, name, args, line, errors);
        }
        let Some((owner, x)) = self.types.method(&t, &name) else {
            self.error(errors, line, format!("struct {t} has no method {name}"));
            return (Expr::lit("0"), None);
        };
        let returns = x.returns.clone();
        let is_static = x.is_static;
        if self.is_class(&t) {
            return match is_static {
                true => (call(owner.method(&name), args), returns),
                false => (Expr::Method(Box::new(MethodCall { object: o, name: class_method(&name, &owner.name), args })), returns),
            };
        }
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
