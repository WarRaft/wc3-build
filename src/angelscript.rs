//! UjAPI's AngelScript of a module made the game's JASS ([`crate::jass`]'s
//! steps but sorting: AngelScript calls a function wherever it is): what
//! `[build] script=as` writes (`war3map.as`, its entries `void main()` and
//! `void config()`).
//!
//! As decided with UjAPI's author:
//!
//! - the natives of `common.j` and its globals are in the namespace `Jass`
//!   (`Jass::CreateUnit`, `Jass::PLAYER_NEUTRAL_AGGRESSIVE`); a native the
//!   code declares itself too;
//! - every type extending `handle` is a global type of its own (a `uint`),
//!   its empty value `nil`; `integer`, `real`, `boolean`, `string` are
//!   `int`, `float`, `bool`, `string`; `function F` is `@F`;
//! - AngelScript has no `code`: UjAPI declares two funcdefs for callbacks,
//!   `void CallbackFunc()` and `bool BoolexprFunc()`. `Condition` and
//!   `Filter` take a `BoolexprFunc`, every other native a `CallbackFunc`;
//!   a `code` variable or parameter is a `BoolexprFunc@` when it's passed
//!   on to one that takes a `BoolexprFunc` ([`bool_code`]), else a
//!   `CallbackFunc@`. A function unlike what it's given to (JASS lets
//!   `Condition(function F)` have an `F` returning nothing) goes through
//!   one written for it (`wcb__bool_F` returning `false` after calling it,
//!   `wcb__void_F`);
//! - `'hfoo'` is written as in JASS;
//! - UjAPI ships no `Blizzard.j`: the functions and globals of it the code
//!   uses, and what they use, are taken ([`take_used`]) and written here
//!   too.
//!
//! - a struct is a class, an interface a base class, an object a handle
//!   (`S@`): see [`crate::structs::lower`] for what lowering keeps, and
//!   `Printer::classes` for what's written of them (the families'
//!   tables of objects by number, each struct's `s__S__allocate`).
//!
//! What JASS has that AngelScript writes otherwise:
//!
//! - an array is a JASS one, any index read and what's never set its
//!   type's empty value: a class of the namespace `WC3` for each type of
//!   element used (`WC3::int_array`), indexed as JASS does (`a[i] = x`) by
//!   its `get_opIndex`/`set_opIndex`, growing as it's set;
//! - a string is never null: `null` where a string goes is `""`;
//! - a variable is always given a value (its type's empty one);
//! - `and` binds looser than `or` in JASS, tighter in AngelScript:
//!   parentheses where the tree needs them;
//! - `$FF` is `0xFF`, `017` (octal) `15`, `.5` `0.5f`;
//! - `loop` is `while (true)`, `exitwhen c` `if (c) break;`; `debug` lines
//!   are left out, as the game does when it isn't debugging;
//! - a name that's a word of AngelScript's (`in`, `int`, `class` ...) gets
//!   the build's prefix (`wcb__in`).

use std::collections::{BTreeSet, HashMap, HashSet};
use std::fmt::Write as _;

use crate::ir::*;
use crate::lua::{Types, literal_type, locals};

/// What `common.j` declares, which the code calls and reads in `Jass::`.
#[derive(Debug, Clone, Default)]
pub struct Common {
    /// Natives and what they give back.
    pub natives: HashMap<String, Signature>,
    /// Globals and their types.
    pub globals: HashMap<String, String>,
}

impl Common {
    /// Of `common.j`'s module; the code's own natives are added by [`print()`].
    pub fn of(m: &Module) -> Common {
        Common {
            natives: m.natives.iter().map(|n| (n.sig.name.clone(), n.sig.clone())).collect(),
            globals: m.globals.iter().map(|g| (g.name.clone(), g.ty.clone())).collect(),
        }
    }
}

/// AngelScript's words, which a JASS name can be.
const WORDS: &[&str] = &[
    "abstract", "and", "auto", "bool", "break", "case", "cast", "catch", "class", "const", "continue", "default", "do", "double", "else", "enum", "explicit", "external",
    "false", "final", "float", "for", "from", "funcdef", "function", "get", "if", "import", "in", "inout", "int", "int16", "int32", "int64", "int8", "interface", "is",
    "mixin", "namespace", "nil", "not", "null", "or", "out", "override", "private", "property", "protected", "return", "set", "shared", "super", "switch", "this",
    "true", "try", "typedef", "uint", "uint16", "uint32", "uint64", "uint8", "void", "while", "xor", "Jass", "WC3", "array", "CallbackFunc", "BoolexprFunc",
];

/// A name of the code's, as AngelScript can have it.
fn ident(n: &str) -> String {
    if WORDS.contains(&n) { format!("{}{n}", crate::jass::PREFIX) } else { n.to_string() }
}

/// A JASS type as AngelScript writes it.
fn ty(t: &str) -> String {
    match t {
        "integer" => "int".into(),
        "real" => "float".into(),
        "boolean" => "bool".into(),
        "code" => "CallbackFunc@".into(),
        BOOL_CODE => "BoolexprFunc@".into(),
        _ => match t.strip_prefix(CLASS) {
            Some(c) => format!("{}@", ident(c)),
            None => t.into(),
        },
    }
}

/// A type's empty value.
fn empty(t: &str) -> &'static str {
    match t {
        "integer" => "0",
        "real" => "0.0f",
        "boolean" => "false",
        "string" => "\"\"",
        "code" | BOOL_CODE => "null",
        _ if t.starts_with(CLASS) => "null",
        _ => "nil",
    }
}

/// The class of `WC3` for an array of `t`.
fn array_class(t: &str) -> String {
    format!("{}_array", ty(t).trim_end_matches('@'))
}

/// The classes of `WC3` for arrays of `types`.
fn array_classes(types: &BTreeSet<String>) -> String {
    let mut out = String::from("namespace WC3 {\n");
    for t in types {
        let (class, elem, empty) = (array_class(t), ty(t), empty(t));
        let at = if is_code(t) || t.starts_with(CLASS) { "@" } else { "" };
        let _ = write!(
            out,
            "    // A JASS array: any index read, what's never set {empty}.
    class {class} {{
        private array<{elem}> v;
        {elem} get_opIndex(int i) const {{
            if (i < 0 || uint(i) >= v.length()) return {empty};
            return v[i];
        }}
        void set_opIndex(int i, {elem} x) {{
            if (i < 0) return;
            if (uint(i) >= v.length()) v.resize(uint(i) + 1 + v.length() / 2);
            {at}v[i] = {at}x;
        }}
    }}
"
        );
    }
    out += "}\n";
    out
}

/// What a class's type is marked with ([`mark_classes`]): `class:S`.
const CLASS: &str = "class:";

/// Marks every type that's a class's (a struct's or an interface's)
/// [`CLASS`], wherever a type is written.
fn mark_classes(m: &mut Module) {
    let classes: HashSet<String> = m.structs.iter().map(|s| s.name.clone()).chain(m.interfaces.iter().map(|i| i.name.clone())).collect();
    if classes.is_empty() {
        return;
    }
    let mark = |t: &mut String| {
        if classes.contains(t.as_str()) {
            *t = format!("{CLASS}{t}");
        }
    };
    fn body(b: &mut [Stmt], mark: &dyn Fn(&mut String)) {
        for s in b {
            match s {
                Stmt::Local { ty, .. } => mark(ty),
                Stmt::If { branches, otherwise, .. } => {
                    branches.iter_mut().for_each(|(_, b)| body(b, mark));
                    if let Some(b) = otherwise {
                        body(b, mark);
                    }
                }
                Stmt::Loop { body: b, .. } => body(b, mark),
                _ => {}
            }
        }
    }
    let function = |f: &mut Function| {
        f.sig.params.iter_mut().for_each(|p| mark(&mut p.ty));
        if let Some(r) = &mut f.sig.returns {
            mark(r);
        }
        body(&mut f.body, &mark);
    };
    m.globals.iter_mut().for_each(|g| mark(&mut g.ty));
    m.functions.iter_mut().for_each(function);
    for st in &mut m.structs {
        st.fields.iter_mut().for_each(|f| mark(&mut f.ty));
        st.methods.iter_mut().for_each(|me| function(&mut me.function));
    }
    for i in &mut m.interfaces {
        i.fields.iter_mut().for_each(|f| mark(&mut f.ty));
        i.methods.iter_mut().for_each(|me| function(&mut me.function));
    }
}

/// The type of a `code` that's a `BoolexprFunc` ([`bool_code`]).
const BOOL_CODE: &str = "code:bool";

/// The natives taking a `BoolexprFunc` where JASS has `code`.
const BOOL_SLOTS: &[&str] = &["Condition", "Filter"];

fn is_code(t: &str) -> bool {
    t == "code" || t == BOOL_CODE
}

/// Calls `f` with every call in `body`, statements' and expressions'.
fn each_call(body: &[Stmt], f: &mut impl FnMut(&Call)) {
    each_expr(body, &mut |e| {
        if let Expr::Call(c) = e {
            f(c);
        }
    });
    each_call_stmts(body, f);
    fn each_call_stmts(body: &[Stmt], f: &mut impl FnMut(&Call)) {
        for s in body {
            match s {
                Stmt::Call { call, .. } => f(call),
                Stmt::If { branches, otherwise, .. } => {
                    branches.iter().for_each(|(_, b)| each_call_stmts(b, f));
                    if let Some(b) = otherwise {
                        each_call_stmts(b, f);
                    }
                }
                Stmt::Loop { body, .. } => each_call_stmts(body, f),
                _ => {}
            }
        }
    }
}

/// Makes `code:bool` every `code` global, parameter and local passed
/// to what takes a `BoolexprFunc` (`Condition`, `Filter`, or a
/// parameter made one), until nothing more is.
pub fn bool_code(m: &mut Module) {
    fn retype(body: &mut [Stmt], name: &str) -> bool {
        let mut changed = false;
        for s in body {
            match s {
                Stmt::Local { ty, name: n, .. } if n == name && ty == "code" => {
                    *ty = BOOL_CODE.into();
                    changed = true;
                }
                Stmt::If { branches, otherwise, .. } => {
                    for (_, b) in branches.iter_mut() {
                        changed |= retype(b, name);
                    }
                    if let Some(b) = otherwise {
                        changed |= retype(b, name);
                    }
                }
                Stmt::Loop { body, .. } => changed |= retype(body, name),
                _ => {}
            }
        }
        changed
    }
    loop {
        let params: HashMap<String, Vec<String>> = m.functions.iter().map(|f| (f.sig.name.clone(), f.sig.params.iter().map(|p| p.ty.clone()).collect())).collect();
        let mut marks: Vec<(Option<usize>, String)> = Vec::new();
        for (fi, f) in m.functions.iter().enumerate() {
            let mut own: HashMap<String, String> = f.sig.params.iter().map(|p| (p.name.clone(), p.ty.clone())).collect();
            locals(&f.body, &mut own);
            each_call(&f.body, &mut |c| {
                for (i, a) in c.args.iter().enumerate() {
                    let slot = BOOL_SLOTS.contains(&c.name.as_str()) || params.get(&c.name).and_then(|p| p.get(i)).is_some_and(|t| t == BOOL_CODE);
                    if let (true, Expr::Var(n) | Expr::Index(n, _)) = (slot, a) {
                        marks.push((own.contains_key(n).then_some(fi), n.clone()));
                    }
                }
            });
        }
        let mut changed = false;
        for (fi, n) in marks {
            match fi {
                Some(fi) => {
                    let f = &mut m.functions[fi];
                    for p in f.sig.params.iter_mut().filter(|p| p.name == n && p.ty == "code") {
                        p.ty = BOOL_CODE.into();
                        changed = true;
                    }
                    changed |= retype(&mut f.body, &n);
                }
                None => {
                    for g in m.globals.iter_mut().filter(|g| g.name == n && g.ty == "code") {
                        g.ty = BOOL_CODE.into();
                        changed = true;
                    }
                }
            }
        }
        if !changed {
            break;
        }
    }
}

/// Every name `body` calls, names as code, reads or sets.
pub(crate) fn names_in(body: &[Stmt], out: &mut Vec<String>) {
    out.extend(called(body));
    each_expr(body, &mut |e| {
        if let Expr::Var(n) | Expr::Index(n, _) = e {
            out.push(n.clone());
        }
    });
    fn set(body: &[Stmt], out: &mut Vec<String>) {
        for s in body {
            match s {
                Stmt::Set { name, .. } => out.push(name.clone()),
                Stmt::If { branches, otherwise, .. } => {
                    branches.iter().for_each(|(_, b)| set(b, out));
                    if let Some(b) = otherwise {
                        set(b, out);
                    }
                }
                Stmt::Loop { body, .. } => set(body, out),
                Stmt::Debug { stmt, .. } => set(std::slice::from_ref(stmt), out),
                _ => {}
            }
        }
    }
    set(body, out);
}

/// The names `g`'s initial value reads and calls.
pub(crate) fn names_in_global(g: &Global, out: &mut Vec<String>) {
    if let Some(e) = &g.init {
        names_in(&[Stmt::Return { value: Some(e.clone()), line: 0 }], out);
    }
}

/// A JASS string literal's value (`"a\\b"` is `a\b`).
pub(crate) fn string_value(lit: &str) -> Option<String> {
    let inner = lit.strip_prefix('"')?.strip_suffix('"')?;
    let mut out = String::new();
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next()? {
            'n' => out.push('\n'),
            'r' => out.push('\r'),
            't' => out.push('\t'),
            'b' => out.push('\u{8}'),
            'f' => out.push('\u{c}'),
            c => out.push(c),
        }
    }
    Some(out)
}

/// The string `e` is before the game runs: literals, `+` of them, the
/// constants they make; `None` when it's told only as it runs.
fn constant_string(e: &Expr, constants: &HashMap<&str, &Expr>, depth: usize) -> Option<String> {
    if depth > 32 {
        return None;
    }
    match e {
        Expr::Literal(l) => string_value(l),
        Expr::Binary(a, BinOp::Add, b) => Some(constant_string(a, constants, depth + 1)? + &constant_string(b, constants, depth + 1)?),
        Expr::Var(n) => constant_string(constants.get(n.as_str())?, constants, depth + 1),
        _ => None,
    }
}

/// AngelScript has no `ExecuteFunc`: each `ExecuteFunc(name)` whose name
/// can be told before the game runs ([`constant_string`]) is a call of
/// that function, taking nothing, of the module's (or of `Blizzard.j`'s:
/// `blizzard`); any other is left as it is and is a problem each.
pub fn unroll_execute_func(m: &mut Module, blizzard: &Module) -> Vec<Error> {
    let constants: HashMap<String, Expr> = m.globals.iter().chain(&blizzard.globals).filter(|g| g.constant && !g.array).filter_map(|g| Some((g.name.clone(), g.init.clone()?))).collect();
    let constants: HashMap<&str, &Expr> = constants.iter().map(|(n, e)| (n.as_str(), e)).collect();
    let takes: HashMap<String, usize> = m.functions.iter().chain(&blizzard.functions).map(|f| (f.sig.name.clone(), f.sig.params.len())).collect();
    let mut problems = Vec::new();
    fn walk(body: &mut [Stmt], file: &str, constants: &HashMap<&str, &Expr>, takes: &HashMap<String, usize>, problems: &mut Vec<Error>) {
        for s in body {
            match s {
                Stmt::Call { call, line } if call.name == "ExecuteFunc" && call.args.len() == 1 => {
                    let origin = Origin { file: file.to_string(), line: *line };
                    match constant_string(&call.args[0], constants, 0) {
                        Some(name) if takes.get(&name) == Some(&0) => *call = Call { name, args: Vec::new() },
                        Some(name) if takes.contains_key(&name) => problems.push(Error { origin, message: format!("ExecuteFunc(\"{name}\"): {name} takes arguments, which ExecuteFunc can't give (AngelScript has no ExecuteFunc: left as it is)") }),
                        Some(name) => problems.push(Error { origin, message: format!("ExecuteFunc(\"{name}\"): there's no function {name} (AngelScript has no ExecuteFunc: left as it is)") }),
                        None => problems.push(Error {
                            origin,
                            message: "ExecuteFunc of a name told only as the game runs: AngelScript has no ExecuteFunc, and which function it calls can't be worked out (left as it is)".into(),
                        }),
                    }
                }
                Stmt::If { branches, otherwise, .. } => {
                    for (_, b) in branches.iter_mut() {
                        walk(b, file, constants, takes, problems);
                    }
                    if let Some(b) = otherwise {
                        walk(b, file, constants, takes, problems);
                    }
                }
                Stmt::Loop { body, .. } => walk(body, file, constants, takes, problems),
                Stmt::Debug { stmt, .. } => walk(std::slice::from_mut(&mut **stmt), file, constants, takes, problems),
                _ => {}
            }
        }
    }
    for f in &mut m.functions {
        let file = f.origin.file.clone();
        walk(&mut f.body, &file, &constants, &takes, &mut problems);
    }
    for st in &mut m.structs {
        for me in &mut st.methods {
            let file = me.function.origin.file.clone();
            walk(&mut me.function.body, &file, &constants, &takes, &mut problems);
        }
    }
    problems
}

/// Takes into `m` the functions and globals of `from` (`Blizzard.j`) that
/// it uses, and what they use (tree-shaking), in `from`'s order: the
/// globals before `m`'s, the functions after.
pub fn take_used(m: &mut Module, from: &Module) {
    let mut known: HashSet<String> = m.functions.iter().map(|f| f.sig.name.clone()).chain(m.globals.iter().map(|g| g.name.clone())).collect();
    let functions: HashMap<&str, usize> = from.functions.iter().enumerate().map(|(i, f)| (f.sig.name.as_str(), i)).collect();
    let globals: HashMap<&str, usize> = from.globals.iter().enumerate().map(|(i, g)| (g.name.as_str(), i)).collect();
    let mut queue = Vec::new();
    m.functions.iter().for_each(|f| names_in(&f.body, &mut queue));
    m.globals.iter().for_each(|g| names_in_global(g, &mut queue));
    for st in &m.structs {
        st.methods.iter().for_each(|me| names_in(&me.function.body, &mut queue));
        st.fields.iter().filter_map(|f| f.init.clone()).for_each(|e| names_in(&[Stmt::Return { value: Some(e), line: 0 }], &mut queue));
    }
    let (mut fs, mut gs) = (BTreeSet::new(), BTreeSet::new());
    while let Some(n) = queue.pop() {
        if known.contains(&n) {
            continue;
        }
        if let Some(&i) = functions.get(n.as_str()) {
            fs.insert(i);
            names_in(&from.functions[i].body, &mut queue);
        } else if let Some(&i) = globals.get(n.as_str()) {
            gs.insert(i);
            names_in_global(&from.globals[i], &mut queue);
        }
        known.insert(n);
    }
    let mut taken: Vec<Global> = gs.into_iter().map(|i| from.globals[i].clone()).collect();
    taken.append(&mut m.globals);
    m.globals = taken;
    m.functions.extend(fs.into_iter().map(|i| from.functions[i].clone()));
}

struct Printer<'a> {
    types: Types<'a>,
    common: &'a Common,
    /// Every function's parameters' types, the game's and the module's.
    params: HashMap<&'a str, Vec<&'a str>>,
    /// What the function printed gives back.
    returns: Option<&'a str>,
    /// The module's globals (taken from `Blizzard.j` too).
    globals: HashSet<&'a str>,
    /// What the class's method printed gives back.
    method_returns: Option<String>,
    arrays: BTreeSet<String>,
    /// Functions given where they don't fit: `(F, as a BoolexprFunc)`.
    wrappers: std::cell::RefCell<BTreeSet<(String, bool)>>,
    /// The classes: each one's parent, fields' types, methods' returns.
    parents: HashMap<String, String>,
    fields: HashMap<(String, String), String>,
    methods: HashMap<(String, String), Option<String>>,
    /// In a class's method: `this` is `this`.
    in_class: bool,
}

/// The AngelScript of `m` (made the game's JASS: lowered, merged, what it
/// uses of `Blizzard.j` taken in); `game` the game's natives and functions,
/// for what they take and give back; `common` what's in `Jass::`.
pub fn print(m: &Module, game: &[Signature], common: &Common) -> String {
    let mut m = m.clone();
    bool_code(&mut m);
    mark_classes(&mut m);
    let m = &m;
    let mut common = common.clone();
    for n in BOOL_SLOTS {
        if let Some(s) = common.natives.get_mut(*n) {
            s.params.iter_mut().filter(|p| p.ty == "code").for_each(|p| p.ty = BOOL_CODE.into());
        }
    }
    for n in &m.natives {
        common.natives.insert(n.sig.name.clone(), n.sig.clone());
    }
    let mut p = Printer { types: Types { globals: HashMap::new(), returns: HashMap::new(), locals: HashMap::new() }, common: &common, params: HashMap::new(), returns: None,
        arrays: BTreeSet::new(),
        wrappers: Default::default(),
        globals: m.globals.iter().map(|g| g.name.as_str()).collect(),
        parents: HashMap::new(),
        fields: HashMap::new(),
        methods: HashMap::new(),
        in_class: false,
        method_returns: None,
    };
    for st in &m.structs {
        if let Some(e) = st.extends.as_ref().filter(|e| *e != "array") {
            p.parents.insert(st.name.clone(), e.clone());
        }
        for f in st.fields.iter().filter(|f| !f.is_static) {
            p.fields.insert((st.name.clone(), f.name.clone()), f.ty.clone());
        }
        for me in &st.methods {
            p.methods.insert((st.name.clone(), me.function.sig.name.clone()), me.function.sig.returns.clone());
        }
    }
    for i in &m.interfaces {
        for f in i.fields.iter().filter(|f| !f.is_static) {
            p.fields.insert((i.name.clone(), f.name.clone()), f.ty.clone());
        }
        for me in &i.methods {
            p.methods.insert((i.name.clone(), me.function.sig.name.clone()), me.function.sig.returns.clone());
        }
    }
    for (name, t) in &common.globals {
        p.types.globals.insert(name, t);
    }
    for g in &m.globals {
        p.types.globals.insert(&g.name, &g.ty);
    }
    for s in game.iter().chain(common.natives.values()).chain(m.functions.iter().map(|f| &f.sig)) {
        p.types.returns.insert(&s.name, s.returns.as_deref());
        p.params.insert(&s.name, s.params.iter().map(|x| x.ty.as_str()).collect());
    }
    let mut body = String::new();
    for g in &m.globals {
        let line = if g.array {
            p.arrays.insert(g.ty.clone());
            format!("WC3::{} {};", array_class(&g.ty), ident(&g.name))
        } else {
            let value = g.init.as_ref().map_or_else(|| empty(&g.ty).to_string(), |e| p.value(e, Some(&g.ty)));
            let konst = if g.constant { "const " } else { "" };
            format!("{konst}{} {} = {value};", ty(&g.ty), ident(&g.name))
        };
        body += &line;
        body.push('\n');
    }
    for f in &m.functions {
        p.types.locals = f.sig.params.iter().map(|x| (x.name.clone(), x.ty.clone())).collect();
        locals(&f.body, &mut p.types.locals);
        p.returns = f.sig.returns.as_deref();
        let params: Vec<String> = f.sig.params.iter().map(|x| format!("{} {}", ty(&x.ty), ident(&x.name))).collect();
        let returns = f.sig.returns.as_deref().map_or_else(|| "void".to_string(), ty);
        let _ = writeln!(body, "\n{returns} {}({}) {{", ident(&f.sig.name), params.join(", "));
        p.statements(&f.body, 1, &mut body);
        body += "}\n";
    }
    // The classes (their methods may need arrays and wrappers too).
    let classes = p.classes(m);
    for (f, boolean) in p.wrappers.borrow().iter() {
        let call = format!("{}()", ident(f));
        let _ = if *boolean {
            writeln!(body, "\nbool {}bool_{f}() {{\n    {call};\n    return false;\n}}", crate::jass::PREFIX)
        } else {
            writeln!(body, "\nvoid {}void_{f}() {{\n    {call};\n}}", crate::jass::PREFIX)
        };
    }
    let mut out = String::from("// Made by wc3-build from JASS.\n");
    if !p.arrays.is_empty() {
        out += &array_classes(&p.arrays);
    }
    out += &classes;
    out.push('\n');
    out += &body;
    out
}

/// How tightly an operator binds in AngelScript.
fn precedence(op: BinOp) -> u8 {
    match op {
        BinOp::Or => 1,
        BinOp::And => 2,
        BinOp::Eq | BinOp::NotEq => 3,
        BinOp::Gt | BinOp::Lt | BinOp::GtEq | BinOp::LtEq => 4,
        BinOp::Add | BinOp::Sub => 5,
        BinOp::Mul | BinOp::Div | BinOp::Mod => 6,
    }
}

const UNARY: u8 = 7;

/// A string literal of JASS as AngelScript writes it: a line break in it
/// escaped, `\b` and `\f` by their codes.
fn string(t: &str) -> String {
    let mut out = String::with_capacity(t.len());
    let mut chars = t.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                Some('b') => out += "\\x08",
                Some('f') => out += "\\x0C",
                Some(n) => {
                    out.push('\\');
                    out.push(n);
                }
                None => out.push('\\'),
            },
            '\n' => out += "\\n",
            '\r' => out += "\\r",
            c => out.push(c),
        }
    }
    out
}

/// A literal as AngelScript writes it (`null` by [`Printer::value`]).
fn literal(t: &str) -> String {
    if t.starts_with('"') {
        return string(t);
    }
    if t.starts_with('\'') {
        return t.to_string();
    }
    if let Some(hex) = t.strip_prefix('$') {
        return format!("0x{hex}");
    }
    if t.len() > 1 && t.starts_with('0') && t.bytes().all(|b| (b'0'..=b'7').contains(&b)) {
        return i64::from_str_radix(&t[1..], 8).map_or(t.to_string(), |n| n.to_string());
    }
    if literal_type(t) == Some("real") {
        let mut s = t.to_string();
        if s.starts_with('.') {
            s.insert(0, '0');
        }
        if s.ends_with('.') {
            s.push('0');
        }
        s.push('f');
        return s;
    }
    if t == "null" { "nil".into() } else { t.to_string() }
}

/// What every family (its root `R`) has: its objects by their numbers
/// (`si__R_all`, 0 none), the numbers given back (`si__R_free`), and
/// `s__R__at`, `s__R__id`, `s__R__add`, `s__R__deallocate`,
/// `s__R__destroy` (its `onDestroy`s, then given back).
fn family(r: &str) -> String {
    let c = ident(r);
    format!(
        "
array<{c}@> si__{r}_all = {{null}};
array<int> si__{r}_free;

{c}@ s__{r}__at(int i) {{
    if (i <= 0 || uint(i) >= si__{r}_all.length()) return null;
    return si__{r}_all[i];
}}

int s__{r}__id({c}@ o) {{
    if (o is null) return 0;
    return o.wcb__id;
}}

void s__{r}__add({c}@ o) {{
    if (si__{r}_free.length() > 0) {{
        o.wcb__id = si__{r}_free[si__{r}_free.length() - 1];
        si__{r}_free.removeLast();
        @si__{r}_all[o.wcb__id] = o;
    }} else {{
        o.wcb__id = int(si__{r}_all.length());
        si__{r}_all.insertLast(o);
    }}
}}

void s__{r}__deallocate({c}@ o) {{
    if (o is null || o.wcb__id == 0) return;
    @si__{r}_all[o.wcb__id] = null;
    si__{r}_free.insertLast(o.wcb__id);
    o.wcb__id = 0;
}}

void s__{r}__destroy({c}@ o) {{
    if (o is null || o.wcb__id == 0) return;
    o.wcb__onDestroy();
    s__{r}__deallocate(o);
}}
"
    )
}

impl Printer<'_> {
    /// The topmost class `c` extends (itself without a parent).
    fn root_of(&self, c: &str) -> String {
        let mut c = c.to_string();
        for _ in 0..64 {
            match self.parents.get(&c) {
                Some(p) => c = p.clone(),
                None => break,
            }
        }
        c
    }

    /// A method's head and body, in its class.
    fn method(&mut self, class: &str, f: &Function, out: &mut String) {
        self.types.locals = f.sig.params.iter().map(|x| (x.name.clone(), x.ty.clone())).collect();
        self.types.locals.insert("this".into(), format!("{CLASS}{class}"));
        locals(&f.body, &mut self.types.locals);
        let returns = f.sig.returns.clone();
        self.returns = None;
        self.in_class = true;
        let params: Vec<String> = f.sig.params.iter().map(|x| format!("{} {}", ty(&x.ty), ident(&x.name))).collect();
        let head = returns.as_deref().map_or_else(|| "void".to_string(), ty);
        let _ = writeln!(out, "    {head} {}({}) {{", ident(&f.sig.name), params.join(", "));
        self.method_returns = returns;
        self.statements(&f.body, 2, out);
        self.method_returns = None;
        out.push_str("    }\n");
        self.in_class = false;
    }

    /// The classes of the structs and interfaces, the families' tables and
    /// each struct's allocator.
    fn classes(&mut self, m: &Module) -> String {
        if m.structs.is_empty() && m.interfaces.is_empty() {
            return String::new();
        }
        let mut out = String::new();
        // Types numbered as lowering numbers them: in a family of several
        // (an interface's, a struct's with children), from 1.
        let interfaces: HashSet<&str> = m.interfaces.iter().map(|i| i.name.as_str()).collect();
        let mut members: Vec<(String, Vec<String>)> = Vec::new();
        for st in &m.structs {
            let root = self.root_of(&st.name);
            match members.iter_mut().find(|(r, _)| *r == root) {
                Some((_, l)) => l.push(st.name.clone()),
                None => members.push((root, vec![st.name.clone()])),
            }
        }
        let typeid = |name: &str, root: &str| -> usize {
            let list = &members.iter().find(|(r, _)| r == root).expect("its family").1;
            if interfaces.contains(root) || list.len() > 1 { list.iter().position(|s| s == name).map_or(0, |k| k + 1) } else { 0 }
        };
        for i in &m.interfaces {
            let _ = writeln!(out, "\nclass {} {{\n    int wcb__id = 0;", ident(&i.name));
            // Its members, every struct extending it has.
            self.types.locals = HashMap::new();
            self.fields_of(&i.fields, &mut out);
            out.push_str("    int wcb__type() { return 0; }\n    void wcb__onDestroy() {}\n");
            for me in &i.methods {
                let f = &me.function;
                self.types.locals = f.sig.params.iter().map(|x| (x.name.clone(), x.ty.clone())).collect();
                let params: Vec<String> = f.sig.params.iter().map(|x| format!("{} {}", ty(&x.ty), ident(&x.name))).collect();
                let head = f.sig.returns.as_deref().map_or_else(|| "void".to_string(), ty);
                // What a struct without it gives back: `defaults`.
                let body = match (&f.sig.returns, &me.defaults) {
                    (None, _) => String::new(),
                    (Some(t), Some(d)) => format!(" return {}; ", self.value(d, Some(t))),
                    (Some(t), None) => format!(" return {}; ", empty(t)),
                };
                let _ = writeln!(out, "    {head} {}({}) {{{body}}}", ident(&f.sig.name), params.join(", "));
            }
            out.push_str("}\n");
        }
        for st in &m.structs {
            let root = self.root_of(&st.name);
            let parent = self.parents.get(&st.name).cloned();
            match &parent {
                Some(p) => {
                    let _ = writeln!(out, "\nclass {} : {} {{", ident(&st.name), ident(p));
                }
                None => {
                    let _ = writeln!(out, "\nclass {} {{\n    int wcb__id = 0;", ident(&st.name));
                }
            }
            self.types.locals = HashMap::new();
            self.fields_of(&st.fields, &mut out);
            let _ = writeln!(out, "    int wcb__type() {{ return {}; }}", typeid(&st.name, &root));
            // `onDestroy`: its own, then its parents'.
            let own = format!("onDestroy__{}", st.name);
            let has_own = st.methods.iter().any(|me| me.function.sig.name == own);
            match (has_own, &parent) {
                (true, Some(p)) => {
                    let _ = writeln!(out, "    void wcb__onDestroy() {{\n        {own}();\n        {}::wcb__onDestroy();\n    }}", ident(p));
                }
                (true, None) => {
                    let _ = writeln!(out, "    void wcb__onDestroy() {{\n        {own}();\n    }}");
                }
                (false, None) => out.push_str("    void wcb__onDestroy() {}\n"),
                (false, Some(_)) => {}
            }
            for me in &st.methods {
                self.method(&st.name, &me.function, &mut out);
            }
            out.push_str("}\n");
        }
        let mut roots: Vec<&str> = members.iter().map(|(r, _)| r.as_str()).collect();
        roots.dedup();
        for r in roots {
            out += &family(r);
        }
        for st in &m.structs {
            let r = self.root_of(&st.name);
            // Allocated through a parent's `create` (`s__S__allocate` made
            // of it): the object itself is `s__S__new`.
            let via = m.functions.iter().any(|f| f.sig.name == format!("s__{}__allocate", st.name));
            let name = if via { "new" } else { "allocate" };
            let _ = writeln!(out, "\n{1}@ s__{0}__{name}() {{\n    {1}@ o = {1}();\n    s__{r}__add(o);\n    return o;\n}}", st.name, ident(&st.name));
        }
        out
    }

    /// A class's instance members, with their values.
    fn fields_of(&mut self, fields: &[Field], out: &mut String) {
        for f in fields.iter().filter(|f| !f.is_static) {
            if f.array.is_some() {
                self.arrays.insert(f.ty.clone());
                let _ = writeln!(out, "    WC3::{} {};", array_class(&f.ty), ident(&f.name));
            } else {
                let value = f.init.as_ref().map_or_else(|| empty(&f.ty).to_string(), |e| self.value(e, Some(&f.ty)));
                let _ = writeln!(out, "    {} {} = {value};", ty(&f.ty), ident(&f.name));
            }
        }
    }

    /// The type of an object written (`a`, `a.b`, `a.m()`, `S(x)`): its
    /// class, when it's one.
    fn class_of(&self, e: &Expr) -> Option<String> {
        let t = match e {
            Expr::Var(n) | Expr::Index(n, _) => self.var_type(n),
            Expr::Member(o, f) => self.field_type(&self.class_of(o)?, f),
            Expr::Element(a, _) => match &**a {
                Expr::Member(o, f) => self.field_type(&self.class_of(o)?, f),
                _ => None,
            },
            Expr::Method(mc) => {
                let mut c = self.class_of(&mc.object)?;
                loop {
                    if let Some(r) = self.methods.get(&(c.clone(), mc.name.clone())) {
                        break r.clone();
                    }
                    c = self.parents.get(&c)?.clone();
                }
            }
            Expr::Call(c) => match c.name.strip_prefix("wcb__cast__") {
                Some(s) => Some(format!("{CLASS}{s}")),
                None => self.types.returns.get(c.name.as_str()).copied().flatten().map(String::from),
            },
            _ => None,
        }?;
        t.strip_prefix(CLASS).map(String::from)
    }

    /// The type of `class`'s (or a parent's) field `f`.
    fn field_type(&self, class: &str, f: &str) -> Option<String> {
        let mut c = class.to_string();
        for _ in 0..64 {
            if let Some(t) = self.fields.get(&(c.clone(), f.to_string())) {
                return Some(t.clone());
            }
            c = self.parents.get(&c)?.clone();
        }
        None
    }

    fn statements(&mut self, body: &[Stmt], depth: usize, out: &mut String) {
        for s in body {
            self.statement(s, depth, out);
        }
    }

    /// The type of a variable, local or global.
    fn var_type(&self, n: &str) -> Option<String> {
        self.types.locals.get(n).cloned().or_else(|| self.types.globals.get(n).map(|t| t.to_string()))
    }

    /// A variable's name: a global of `common.j`'s in `Jass::` (unless a
    /// local hides it).
    fn var(&self, n: &str) -> String {
        if self.in_class && n == "this" {
            return "this".into();
        }
        if !self.types.locals.contains_key(n) && !self.globals.contains(n) && self.common.globals.contains_key(n) { format!("Jass::{n}") } else { ident(n) }
    }

    fn call(&self, c: &Call) -> String {
        if c.name == "wcb__null" {
            return "null".into();
        }
        if let Some(t) = c.name.strip_prefix("wcb__cast__") {
            return format!("cast<{}>({})", ident(t), c.args.first().map_or(String::new(), |a| self.expr(a, 0)));
        }
        // `super.m(...)`: the parent's, `P::m(...)`.
        if let Some((class, m)) = c.name.split_once("::") {
            let args: Vec<String> = c.args.iter().map(|a| self.value(a, None)).collect();
            return format!("{}::{}({})", ident(class), ident(m), args.join(", "));
        }
        let name = if self.common.natives.contains_key(&c.name) { format!("Jass::{}", c.name) } else { ident(&c.name) };
        let params = self.params.get(c.name.as_str());
        let args: Vec<String> = c.args.iter().enumerate().map(|(i, a)| self.value(a, params.and_then(|p| p.get(i)).copied())).collect();
        format!("{name}({})", args.join(", "))
    }

    /// `e` where a value of type `want` goes: `null` as that type's.
    fn value(&self, e: &Expr, want: Option<&str>) -> String {
        match (e, want) {
            (Expr::Literal(l), Some(t)) if l == "null" => empty(t).to_string(),
            (Expr::Code(f), Some(t)) if is_code(t) => {
                let boolean = t == BOOL_CODE;
                let returns = self.types.returns.get(f.as_str()).copied().flatten();
                let fits = if boolean { returns == Some("boolean") } else { returns.is_none() };
                if fits {
                    format!("@{}", ident(f))
                } else {
                    self.wrappers.borrow_mut().insert((f.clone(), boolean));
                    format!("@{}{}_{f}", crate::jass::PREFIX, if boolean { "bool" } else { "void" })
                }
            }
            _ => self.expr(e, 0),
        }
    }

    fn statement(&mut self, s: &Stmt, depth: usize, out: &mut String) {
        let pad = "    ".repeat(depth);
        match s {
            Stmt::Local { ty: t, name, array, init, .. } => {
                if *array {
                    self.arrays.insert(t.clone());
                    let _ = writeln!(out, "{pad}WC3::{} {};", array_class(t), ident(name));
                } else {
                    let value = init.as_ref().map_or_else(|| empty(t).to_string(), |e| self.value(e, Some(t)));
                    let _ = writeln!(out, "{pad}{} {} = {value};", ty(t), ident(name));
                }
            }
            Stmt::Set { name, index, value, .. } => {
                let want = self.var_type(name);
                let index = index.as_ref().map_or(String::new(), |i| format!("[{}]", self.expr(i, 0)));
                let at = if want.as_deref().is_some_and(|t| is_code(t) || t.starts_with(CLASS)) && index.is_empty() { "@" } else { "" };
                let _ = writeln!(out, "{pad}{at}{}{index} = {};", self.var(name), self.value(value, want.as_deref()));
            }
            Stmt::Call { call: c, .. } => {
                let _ = writeln!(out, "{pad}{};", self.call(c));
            }
            Stmt::If { branches, otherwise, .. } => {
                for (i, (c, b)) in branches.iter().enumerate() {
                    let word = if i == 0 { format!("{pad}if") } else { " else if".to_string() };
                    let _ = writeln!(out, "{word} ({}) {{", self.expr(c, 0));
                    self.statements(b, depth + 1, out);
                    out.push_str(&pad);
                    out.push('}');
                }
                if let Some(b) = otherwise {
                    out.push_str(" else {\n");
                    self.statements(b, depth + 1, out);
                    out.push_str(&pad);
                    out.push('}');
                }
                out.push('\n');
            }
            Stmt::Loop { body, .. } => {
                let _ = writeln!(out, "{pad}while (true) {{");
                self.statements(body, depth + 1, out);
                let _ = writeln!(out, "{pad}}}");
            }
            Stmt::ExitWhen { cond, .. } => {
                let _ = writeln!(out, "{pad}if ({}) break;", self.expr(cond, 0));
            }
            Stmt::Return { value, .. } => match value {
                Some(v) => {
                    let want = self.method_returns.clone();
                    let _ = writeln!(out, "{pad}return {};", self.value(v, want.as_deref().or(self.returns)));
                }
                None => {
                    let _ = writeln!(out, "{pad}return;");
                }
            },
            // As the game when it isn't debugging.
            Stmt::Debug { .. } => {}
            // A class's field, and its method called.
            Stmt::SetMember { object, name, index, value, .. } => {
                let want = self.class_of(object).and_then(|c| self.field_type(&c, name));
                let at = if want.as_deref().is_some_and(|t| is_code(t) || t.starts_with(CLASS)) && index.is_none() { "@" } else { "" };
                let index = index.as_ref().map_or(String::new(), |i| format!("[{}]", self.expr(i, 0)));
                let _ = writeln!(out, "{pad}{at}{}.{}{index} = {};", self.expr(object, 8), ident(name), self.value(value, want.as_deref()));
            }
            Stmt::CallMethod { call, .. } => {
                let _ = writeln!(out, "{pad}{};", self.method_call(call));
            }
            Stmt::SetElement { .. } | Stmt::StaticIf { .. } => unreachable!("not lowered: build checks Module::not_lowered"),
        }
    }

    fn method_call(&self, c: &MethodCall) -> String {
        let args: Vec<String> = c.args.iter().map(|a| self.value(a, None)).collect();
        format!("{}.{}({})", self.expr(&c.object, 8), ident(&c.name), args.join(", "))
    }

    /// `e`, in parentheses when it binds looser than `min`.
    fn expr(&self, e: &Expr, min: u8) -> String {
        let (text, p) = match e {
            Expr::Literal(l) => (literal(l), 8),
            Expr::Var(n) => (self.var(n), 8),
            Expr::Index(n, i) => (format!("{}[{}]", self.var(n), self.expr(i, 0)), 8),
            // Objects compared: the same one.
            Expr::Call(c) if c.name == "wcb__same" && c.args.len() == 2 => (format!("{} is {}", self.expr(&c.args[0], 4), self.expr(&c.args[1], 4)), 3),
            Expr::Call(c) => (self.call(c), 8),
            Expr::Code(n) => (format!("@{}", ident(n)), 8),
            Expr::Member(o, n) => (format!("{}.{}", self.expr(o, 8), ident(n)), 8),
            Expr::Method(mc) => (self.method_call(mc), 8),
            Expr::Element(a, i) => (format!("{}[{}]", self.expr(a, 8), self.expr(i, 0)), 8),
            Expr::Unary(op, x) => {
                let inner = self.expr(x, UNARY);
                let text = match op {
                    UnaryOp::Not => format!("!{inner}"),
                    UnaryOp::Neg if inner.starts_with('-') => format!("-({inner})"),
                    UnaryOp::Neg => format!("-{inner}"),
                    UnaryOp::Plus => inner,
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
                    BinOp::Mod => "%",
                    BinOp::Eq => "==",
                    BinOp::NotEq => "!=",
                    BinOp::Gt => ">",
                    BinOp::Lt => "<",
                    BinOp::GtEq => ">=",
                    BinOp::LtEq => "<=",
                    BinOp::And => "&&",
                    BinOp::Or => "||",
                };
                // `null` compared: as the other side's type.
                let (ta, tb) = (self.types.of(a), self.types.of(b));
                let side = |x: &Expr, other: &Option<String>, min: u8| match x {
                    Expr::Literal(l) if l == "null" && other.is_some() => empty(other.as_deref().unwrap_or_default()).to_string(),
                    _ => self.expr(x, min),
                };
                let code = ta.as_deref().is_some_and(is_code) || tb.as_deref().is_some_and(is_code);
                let sign = match op {
                    BinOp::Eq if code => "is",
                    BinOp::NotEq if code => "!is",
                    _ => sign,
                };
                (format!("{} {sign} {}", side(a, &tb, p), side(b, &ta, p + 1)), p)
            }
        };
        if p < min { format!("({text})") } else { text }
    }
}

#[cfg(test)]
#[path = "angelscript.test.rs"]
mod tests;
