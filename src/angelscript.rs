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
    /// Of `common.j`'s module; the code's own natives are added by [`print`].
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
        _ => t.into(),
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
        let at = if is_code(t) { "@" } else { "" };
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

/// Makes [`BOOL_CODE`] every `code` global, parameter and local passed
/// to what takes a `BoolexprFunc` (a native of [`BOOL_SLOTS`], or a
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
fn names_in(body: &[Stmt], out: &mut Vec<String>) {
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
fn names_in_global(g: &Global, out: &mut Vec<String>) {
    if let Some(e) = &g.init {
        names_in(&[Stmt::Return { value: Some(e.clone()), line: 0 }], out);
    }
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
    arrays: BTreeSet<String>,
    /// Functions given where they don't fit: `(F, as a BoolexprFunc)`.
    wrappers: std::cell::RefCell<BTreeSet<(String, bool)>>,
}

/// The AngelScript of `m` (made the game's JASS: lowered, merged, what it
/// uses of `Blizzard.j` taken in); `game` the game's natives and functions,
/// for what they take and give back; `common` what's in `Jass::`.
pub fn print(m: &Module, game: &[Signature], common: &Common) -> String {
    let mut m = m.clone();
    bool_code(&mut m);
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
    let mut p = Printer { types: Types { globals: HashMap::new(), returns: HashMap::new(), locals: HashMap::new() }, common: &common, params: HashMap::new(), returns: None, arrays: BTreeSet::new(), wrappers: Default::default(), globals: m.globals.iter().map(|g| g.name.as_str()).collect() };
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

impl Printer<'_> {
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
        if !self.types.locals.contains_key(n) && !self.globals.contains(n) && self.common.globals.contains_key(n) { format!("Jass::{n}") } else { ident(n) }
    }

    fn call(&self, c: &Call) -> String {
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
                let at = if want.as_deref().is_some_and(is_code) && index.is_empty() { "@" } else { "" };
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
                    let _ = writeln!(out, "{pad}return {};", self.value(v, self.returns));
                }
                None => {
                    let _ = writeln!(out, "{pad}return;");
                }
            },
            // As the game when it isn't debugging.
            Stmt::Debug { .. } => {}
            Stmt::SetMember { .. } | Stmt::CallMethod { .. } | Stmt::StaticIf { .. } => unreachable!("not lowered: build checks Module::not_lowered"),
        }
    }

    /// `e`, in parentheses when it binds looser than `min`.
    fn expr(&self, e: &Expr, min: u8) -> String {
        let (text, p) = match e {
            Expr::Literal(l) => (literal(l), 8),
            Expr::Var(n) => (self.var(n), 8),
            Expr::Index(n, i) => (format!("{}[{}]", self.var(n), self.expr(i, 0)), 8),
            Expr::Call(c) => (self.call(c), 8),
            Expr::Code(n) => (format!("@{}", ident(n)), 8),
            Expr::Member(..) | Expr::Method(_) | Expr::Element(..) => unreachable!("not lowered: build checks Module::not_lowered"),
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
