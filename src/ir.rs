//! The model of a map's code that every source goes into and every output
//! is made from (see the README): what JASS has (types, natives, globals,
//! functions) and what vJASS and Zinc add, as items of their own
//! (libraries and scopes, structs with fields and methods, interfaces,
//! modules), so an output that has classes (AngelScript) gets them as
//! they are; only the JASS output lowers them. A name inside a library is
//! kept as it's written there ([`Owner`] says whose it is); `a.b` and
//! `a.b()` are kept as they are, for what `a` is takes the types to know.
//! Each language's own forms that the game's JASS has too are made into
//! those (Zinc's `while`, `for`, `+=` into `loop`, `exitwhen`, `set`).
//!
//! Each item knows where it was written ([`Origin`]), for the errors of a
//! build. A literal is kept as it was written (`'hfoo'`, `$FF`, `.5`), so
//! what's built reads as its source.

/// Where something was written: a file (as the build was given it) and a
/// line, from 1.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Origin {
    pub file: String,
    pub line: usize,
}

/// A map's code.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Module {
    pub types: Vec<TypeDef>,
    pub natives: Vec<Native>,
    pub globals: Vec<Global>,
    pub functions: Vec<Function>,
    /// Libraries and scopes, in the order they were written (a scope in
    /// a library after it).
    pub libraries: Vec<Library>,
    pub structs: Vec<Struct>,
    pub interfaces: Vec<Interface>,
    /// vJASS's and Zinc's modules: members a struct takes in where it
    /// names one (`implement`, Zinc's `module M;`).
    pub mixins: Vec<Mixin>,
    /// vJASS's function interfaces: types of functions.
    pub function_types: Vec<FunctionType>,
    /// vJASS's hooks, as written.
    pub hooks: Vec<Hook>,
}

/// vJASS's `hook Name Target`: before every call of `Name`, `Target` (a
/// function, or a struct's static method `S.m`) with the same arguments.
#[derive(Debug, Clone, PartialEq)]
pub struct Hook {
    pub name: String,
    pub target: String,
    pub owner: Owner,
    pub origin: Origin,
}

/// vJASS's `function interface F takes ... returns ...`.
#[derive(Debug, Clone, PartialEq)]
pub struct FunctionType {
    pub sig: Signature,
    pub owner: Owner,
    pub origin: Origin,
}

/// Who an item is: the library or scope it's in, and who sees it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Owner {
    /// The library or scope (a scope in a scope: the innermost), none at
    /// the top.
    pub scope: Option<String>,
    pub access: Access,
}

/// Who sees an item of a library or scope.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Access {
    /// Everyone, by its name as written (vJASS's default; Zinc's
    /// `public`; anything outside a library).
    #[default]
    Everyone,
    /// vJASS's `public`: its scope by its name, everyone else as
    /// `Scope_name`.
    Public,
    /// Its scope only (vJASS's `private`; Zinc's default).
    Private,
}

/// A library or a scope.
#[derive(Debug, Clone, PartialEq)]
pub struct Library {
    pub name: String,
    /// A scope (vJASS's `scope`): no `requires`, and it may be in a
    /// library or another scope.
    pub scope: bool,
    /// The library or scope it's in.
    pub parent: Option<String>,
    /// What it needs before it: the name, and whether only if it's there
    /// (`optional`).
    pub requires: Vec<(String, bool)>,
    /// The function run at map initialization (`initializer f`; Zinc's
    /// `onInit`), as named in it.
    pub initializer: Option<String>,
    pub origin: Origin,
}

/// A struct: its fields and methods. `this` and `thistype` in a method are
/// the instance and the struct.
#[derive(Debug, Clone, PartialEq)]
pub struct Struct {
    pub name: String,
    pub owner: Owner,
    /// `array`, a struct or an interface.
    pub extends: Option<String>,
    /// How many instances (`struct S[20000]`), as written.
    pub size: Option<Expr>,
    pub fields: Vec<Field>,
    pub methods: Vec<Method>,
    /// The modules it takes in, and whether only if they're there.
    pub mixins: Vec<(String, bool)>,
    pub origin: Origin,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    pub name: String,
    pub ty: String,
    /// An array of the instance or of the struct; its size as written.
    pub array: Option<Option<Expr>>,
    pub is_static: bool,
    pub constant: bool,
    /// Set only inside the struct.
    pub readonly: bool,
    /// Its members are the struct's too (`delegate`).
    pub delegate: bool,
    pub private: bool,
    pub init: Option<Expr>,
    pub origin: Origin,
}

/// A method: a function with `this` (but a static one), named as written
/// (an operator `operator []`, `operator x=`).
#[derive(Debug, Clone, PartialEq)]
pub struct Method {
    pub function: Function,
    pub is_static: bool,
    /// A child may have its own (`stub`).
    pub stub: bool,
    pub private: bool,
    /// An interface's that has no body: its signature only.
    pub abstract_: bool,
    /// An interface's: what it returns when a struct has none of its own
    /// (`defaults`).
    pub defaults: Option<Expr>,
}

/// An interface: the methods a struct extending it has.
#[derive(Debug, Clone, PartialEq)]
pub struct Interface {
    pub name: String,
    pub owner: Owner,
    /// Its members: every struct extending it has them, as a parent's.
    pub fields: Vec<Field>,
    /// Its family's index space (`interface A[20000]`), as written.
    pub size: Option<Expr>,
    pub methods: Vec<Method>,
    pub origin: Origin,
}

/// A module: fields and methods a struct takes in.
#[derive(Debug, Clone, PartialEq)]
pub struct Mixin {
    pub name: String,
    pub owner: Owner,
    pub fields: Vec<Field>,
    pub methods: Vec<Method>,
    pub mixins: Vec<(String, bool)>,
    pub origin: Origin,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TypeDef {
    pub name: String,
    pub extends: String,
    /// A dynamic array type (`type T extends integer array [16]`): its
    /// size, as written.
    pub size: Option<Expr>,
    pub owner: Owner,
    pub origin: Origin,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Param {
    pub ty: String,
    pub name: String,
}

/// What a function takes and returns (`None`: nothing).
#[derive(Debug, Clone, PartialEq)]
pub struct Signature {
    pub name: String,
    pub params: Vec<Param>,
    pub returns: Option<String>,
    pub constant: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Native {
    pub sig: Signature,
    pub origin: Origin,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Global {
    pub name: String,
    pub ty: String,
    pub array: bool,
    /// A sized array's size, as written (`integer array a[500]`): what
    /// `a.size` is.
    pub size: Option<Expr>,
    pub constant: bool,
    pub init: Option<Expr>,
    pub owner: Owner,
    pub origin: Origin,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Function {
    pub sig: Signature,
    pub body: Vec<Stmt>,
    pub owner: Owner,
    pub origin: Origin,
}

/// A statement; `line` is its line in its function's file.
#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    Local { ty: String, name: String, array: bool, init: Option<Expr>, line: usize },
    Set { name: String, index: Option<Expr>, value: Expr, line: usize },
    Call { call: Call, line: usize },
    If { branches: Vec<(Expr, Vec<Stmt>)>, otherwise: Option<Vec<Stmt>>, line: usize },
    Loop { body: Vec<Stmt>, line: usize },
    ExitWhen { cond: Expr, line: usize },
    Return { value: Option<Expr>, line: usize },
    /// Run only in debug mode: a `Set`, `Call`, `If` or `Loop`.
    Debug { stmt: Box<Stmt>, line: usize },
    /// `object.name = value` (`object.name[index]`): a field of an
    /// instance, or of a struct (`Data.count`).
    SetMember { object: Expr, name: String, index: Option<Expr>, value: Expr, line: usize },
    /// `object.name(args)`: a method, of an instance or a struct.
    CallMethod { call: MethodCall, line: usize },
    /// `static if`: a branch kept or dropped by constants when built.
    StaticIf { cond: Expr, then: Vec<Stmt>, otherwise: Option<Vec<Stmt>>, line: usize },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Call {
    pub name: String,
    pub args: Vec<Expr>,
}

/// `object.name(args)`: `object` an instance, a struct (`Data.create`)
/// or a function (`F.evaluate`).
#[derive(Debug, Clone, PartialEq)]
pub struct MethodCall {
    pub object: Expr,
    pub name: String,
    pub args: Vec<Expr>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    /// A number, a string, `true`, `false` or `null`, as it was written.
    Literal(String),
    Var(String),
    Index(String, Box<Expr>),
    Call(Call),
    /// `function Name`: a code value.
    Code(String),
    Unary(UnaryOp, Box<Expr>),
    Binary(Box<Expr>, BinOp, Box<Expr>),
    /// `object.name`: a field, of an instance or a struct; `this` and
    /// `thistype` are names (`Var`).
    Member(Box<Expr>, String),
    /// `object.name(args)`.
    Method(Box<MethodCall>),
    /// An element of what isn't a name (`this.list[i]`).
    Element(Box<Expr>, Box<Expr>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Neg,
    Plus,
    Not,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Eq,
    NotEq,
    Gt,
    Lt,
    GtEq,
    LtEq,
    And,
    Or,
    /// An integer's remainder (Zinc's `%`).
    Mod,
}

/// A problem that stops a build: where, and what.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    pub origin: Origin,
    pub message: String,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}: {}", self.origin.file, self.origin.line, self.message)
    }
}

/// Walks every expression of statements, outermost first.
pub fn each_expr(body: &[Stmt], f: &mut impl FnMut(&Expr)) {
    fn expr(e: &Expr, f: &mut impl FnMut(&Expr)) {
        f(e);
        match e {
            Expr::Index(_, i) => expr(i, f),
            Expr::Call(c) => c.args.iter().for_each(|a| expr(a, f)),
            Expr::Unary(_, x) => expr(x, f),
            Expr::Binary(a, _, b) => {
                expr(a, f);
                expr(b, f);
            }
            Expr::Member(x, _) => expr(x, f),
            Expr::Method(m) => {
                expr(&m.object, f);
                m.args.iter().for_each(|a| expr(a, f));
            }
            Expr::Element(a, i) => {
                expr(a, f);
                expr(i, f);
            }
            Expr::Literal(_) | Expr::Var(_) | Expr::Code(_) => {}
        }
    }
    for s in body {
        match s {
            Stmt::Local { init, .. } => init.iter().for_each(|e| expr(e, f)),
            Stmt::Set { index, value, .. } => {
                index.iter().for_each(|e| expr(e, f));
                expr(value, f);
            }
            Stmt::Call { call, .. } => call.args.iter().for_each(|a| expr(a, f)),
            Stmt::If { branches, otherwise, .. } => {
                for (c, b) in branches {
                    expr(c, f);
                    each_expr(b, f);
                }
                if let Some(b) = otherwise {
                    each_expr(b, f);
                }
            }
            Stmt::Loop { body, .. } => each_expr(body, f),
            Stmt::ExitWhen { cond, .. } => expr(cond, f),
            Stmt::Return { value, .. } => value.iter().for_each(|e| expr(e, f)),
            Stmt::Debug { stmt, .. } => each_expr(std::slice::from_ref(stmt), f),
            Stmt::SetMember { object, index, value, .. } => {
                expr(object, f);
                index.iter().for_each(|e| expr(e, f));
                expr(value, f);
            }
            Stmt::CallMethod { call, .. } => {
                expr(&call.object, f);
                call.args.iter().for_each(|a| expr(a, f));
            }
            Stmt::StaticIf { cond, then, otherwise, .. } => {
                expr(cond, f);
                each_expr(then, f);
                if let Some(b) = otherwise {
                    each_expr(b, f);
                }
            }
        }
    }
}

impl Expr {
    /// The expressions directly in this one, to change them.
    pub fn children_mut(&mut self) -> Vec<&mut Expr> {
        match self {
            Expr::Index(_, i) => vec![&mut **i],
            Expr::Call(c) => c.args.iter_mut().collect(),
            Expr::Unary(_, x) | Expr::Member(x, _) => vec![&mut **x],
            Expr::Binary(a, _, b) | Expr::Element(a, b) => vec![&mut **a, &mut **b],
            Expr::Method(m) => std::iter::once(&mut m.object).chain(m.args.iter_mut()).collect(),
            Expr::Literal(_) | Expr::Var(_) | Expr::Code(_) => Vec::new(),
        }
    }
}

impl Stmt {
    /// A statement of vJASS's or Zinc's (`SetMember`, `CallMethod`,
    /// `StaticIf`): its expressions and its bodies, to change them; none
    /// for any other.
    pub fn oop_parts_mut(&mut self) -> (Vec<&mut Expr>, Vec<&mut Vec<Stmt>>) {
        match self {
            Stmt::SetMember { object, index, value, .. } => (std::iter::once(object).chain(index.as_mut()).chain(std::iter::once(value)).collect(), Vec::new()),
            Stmt::CallMethod { call, .. } => (std::iter::once(&mut call.object).chain(call.args.iter_mut()).collect(), Vec::new()),
            Stmt::StaticIf { cond, then, otherwise, .. } => (vec![cond], std::iter::once(then).chain(otherwise.as_mut()).collect()),
            _ => (Vec::new(), Vec::new()),
        }
    }
}

/// The functions an expression calls or names as code, statements calling
/// too: every name, once per use.
pub fn called(body: &[Stmt]) -> Vec<String> {
    let mut out = Vec::new();
    fn statements(body: &[Stmt], out: &mut Vec<String>) {
        for s in body {
            match s {
                Stmt::Call { call, .. } => out.push(call.name.clone()),
                Stmt::If { branches, otherwise, .. } => {
                    branches.iter().for_each(|(_, b)| statements(b, out));
                    if let Some(b) = otherwise {
                        statements(b, out);
                    }
                }
                Stmt::Loop { body, .. } => statements(body, out),
                Stmt::Debug { stmt, .. } => statements(std::slice::from_ref(stmt), out),
                Stmt::StaticIf { then, otherwise, .. } => {
                    statements(then, out);
                    if let Some(b) = otherwise {
                        statements(b, out);
                    }
                }
                _ => {}
            }
        }
    }
    statements(body, &mut out);
    each_expr(body, &mut |e| match e {
        Expr::Call(c) => out.push(c.name.clone()),
        Expr::Code(n) => out.push(n.clone()),
        _ => {}
    });
    out
}

/// The functions an expression calls or names as code.
pub fn called_in(e: &Expr) -> Vec<String> {
    called(&[Stmt::Return { value: Some(e.clone()), line: 0 }])
}

// ------------------------------------------------------- making nodes
//
// For a source that isn't text (the map's data): its statements and
// values made as they are. A number is written as it's meant to read.

impl Expr {
    /// A literal, as written (`'hfoo'`, `true`, `null`).
    pub fn lit(text: impl Into<String>) -> Expr {
        Expr::Literal(text.into())
    }

    pub fn int(v: impl std::fmt::Display) -> Expr {
        Expr::Literal(v.to_string())
    }

    /// A real with `decimals` decimals (World Editor's: one for a
    /// coordinate, three for a fraction).
    pub fn real(v: f32, decimals: usize) -> Expr {
        let text = format!("{v:.decimals$}");
        // A sign is an operator in JASS.
        match text.strip_prefix('-') {
            Some(rest) => Expr::Unary(UnaryOp::Neg, Box::new(Expr::Literal(rest.to_string()))),
            None => Expr::Literal(text),
        }
    }

    pub fn bool(v: bool) -> Expr {
        Expr::Literal(v.to_string())
    }

    /// A string, as JASS writes it.
    pub fn string(s: &str) -> Expr {
        let mut out = String::from("\"");
        for c in s.chars() {
            match c {
                '"' => out += "\\\"",
                '\\' => out += "\\\\",
                '\n' => out += "\\n",
                '\r' => out += "\\r",
                c => out.push(c),
            }
        }
        out.push('"');
        Expr::Literal(out)
    }

    /// An object's id in single quotes (`'hfoo'`).
    pub fn rawcode(id: &str) -> Expr {
        Expr::Literal(format!("'{id}'"))
    }

    pub fn var(name: impl Into<String>) -> Expr {
        Expr::Var(name.into())
    }

    pub fn call(name: impl Into<String>, args: Vec<Expr>) -> Expr {
        Expr::Call(Call { name: name.into(), args })
    }

    pub fn code(name: impl Into<String>) -> Expr {
        Expr::Code(name.into())
    }

    pub fn binary(a: Expr, op: BinOp, b: Expr) -> Expr {
        Expr::Binary(Box::new(a), op, Box::new(b))
    }
}

impl Stmt {
    pub fn call(name: impl Into<String>, args: Vec<Expr>) -> Stmt {
        Stmt::Call { call: Call { name: name.into(), args }, line: 0 }
    }

    pub fn set(name: impl Into<String>, value: Expr) -> Stmt {
        Stmt::Set { name: name.into(), index: None, value, line: 0 }
    }

    pub fn set_at(name: impl Into<String>, index: Expr, value: Expr) -> Stmt {
        Stmt::Set { name: name.into(), index: Some(index), value, line: 0 }
    }

    pub fn local(ty: impl Into<String>, name: impl Into<String>, init: Option<Expr>) -> Stmt {
        Stmt::Local { ty: ty.into(), name: name.into(), array: false, init, line: 0 }
    }
}

impl Function {
    /// A function taking and returning nothing, made where `origin` says.
    pub fn new(name: impl Into<String>, body: Vec<Stmt>, origin: Origin) -> Function {
        Function { sig: Signature { name: name.into(), params: Vec::new(), returns: None, constant: false }, body, owner: Owner::default(), origin }
    }
}

/// The integer `e` is before the game runs: a literal (`12`, `$FF`,
/// `0x1F`, `017`, `'A'`), a constant of `constants` (by name, its value),
/// `+ - * /` of them, `-` before one; `None` otherwise.
pub fn const_int(e: &Expr, constants: &dyn Fn(&str) -> Option<Expr>) -> Option<i64> {
    fn go(e: &Expr, constants: &dyn Fn(&str) -> Option<Expr>, depth: usize) -> Option<i64> {
        if depth > 32 {
            return None;
        }
        match e {
            Expr::Literal(t) => {
                if let Some(code) = t.strip_prefix('\'').and_then(|r| r.strip_suffix('\'')) {
                    return Some(code.bytes().fold(0i64, |a, b| a * 256 + b as i64));
                }
                if let Some(h) = t.strip_prefix('$').or_else(|| t.strip_prefix("0x")).or_else(|| t.strip_prefix("0X")) {
                    return i64::from_str_radix(h, 16).ok();
                }
                if t.len() > 1 && t.starts_with('0') {
                    return i64::from_str_radix(&t[1..], 8).ok();
                }
                t.parse().ok()
            }
            Expr::Var(n) => go(&constants(n)?, constants, depth + 1),
            Expr::Unary(UnaryOp::Neg, x) => go(x, constants, depth + 1).map(|v| -v),
            Expr::Unary(UnaryOp::Plus, x) => go(x, constants, depth + 1),
            Expr::Binary(a, op, b) => {
                let (a, b) = (go(a, constants, depth + 1)?, go(b, constants, depth + 1)?);
                match op {
                    BinOp::Add => a.checked_add(b),
                    BinOp::Sub => a.checked_sub(b),
                    BinOp::Mul => a.checked_mul(b),
                    BinOp::Div if b != 0 => Some(a / b),
                    _ => None,
                }
            }
            _ => None,
        }
    }
    go(e, constants, 0)
}

impl Module {
    /// [`const_int`] over this module's constant globals.
    pub fn const_int(&self, e: &Expr) -> Option<i64> {
        let constants = |n: &str| self.globals.iter().find(|g| g.name == n && g.constant && !g.array).and_then(|g| g.init.clone());
        const_int(e, &constants)
    }
}

impl Global {
    /// A variable, of `ty`, set to `init` where the game reads globals.
    pub fn new(name: impl Into<String>, ty: impl Into<String>, init: Option<Expr>, origin: Origin) -> Global {
        Global { name: name.into(), ty: ty.into(), array: false, size: None, constant: false, init, owner: Owner::default(), origin }
    }
}

impl Origin {
    /// What a build made of a file: the file, no line.
    pub fn file(name: impl Into<String>) -> Origin {
        Origin { file: name.into(), line: 0 }
    }
}

impl Module {
    /// Adds `other`'s items after this one's.
    /// Every line it says its items and statements are at, as `to` gives
    /// it (each file's line from the one it was read at).
    pub fn map_lines(&mut self, to: &dyn Fn(usize) -> usize) {
        fn body(b: &mut [Stmt], to: &dyn Fn(usize) -> usize) {
            for s in b {
                match s {
                    Stmt::If { branches, otherwise, line } => {
                        *line = to(*line);
                        branches.iter_mut().for_each(|(_, b)| body(b, to));
                        if let Some(b) = otherwise {
                            body(b, to);
                        }
                    }
                    Stmt::StaticIf { then, otherwise, line, .. } => {
                        *line = to(*line);
                        body(then, to);
                        if let Some(b) = otherwise {
                            body(b, to);
                        }
                    }
                    Stmt::Loop { body: b, line } => {
                        *line = to(*line);
                        body(b, to);
                    }
                    Stmt::Debug { stmt, line } => {
                        *line = to(*line);
                        body(std::slice::from_mut(&mut **stmt), to);
                    }
                    Stmt::Local { line, .. }
                    | Stmt::Set { line, .. }
                    | Stmt::Call { line, .. }
                    | Stmt::ExitWhen { line, .. }
                    | Stmt::Return { line, .. }
                    | Stmt::SetMember { line, .. }
                    | Stmt::CallMethod { line, .. } => *line = to(*line),
                }
            }
        }
        let function = |f: &mut Function| {
            f.origin.line = to(f.origin.line);
            body(&mut f.body, to);
        };
        let methods = |ms: &mut Vec<Method>| ms.iter_mut().for_each(|m| function(&mut m.function));
        let fields = |fs: &mut Vec<Field>| fs.iter_mut().for_each(|f| f.origin.line = to(f.origin.line));
        self.types.iter_mut().for_each(|t| t.origin.line = to(t.origin.line));
        self.natives.iter_mut().for_each(|n| n.origin.line = to(n.origin.line));
        self.globals.iter_mut().for_each(|g| g.origin.line = to(g.origin.line));
        self.functions.iter_mut().for_each(function);
        self.libraries.iter_mut().for_each(|l| l.origin.line = to(l.origin.line));
        for s in &mut self.structs {
            s.origin.line = to(s.origin.line);
            fields(&mut s.fields);
            methods(&mut s.methods);
        }
        for i in &mut self.interfaces {
            i.origin.line = to(i.origin.line);
            methods(&mut i.methods);
        }
        for x in &mut self.mixins {
            x.origin.line = to(x.origin.line);
            fields(&mut x.fields);
            methods(&mut x.methods);
        }
        self.function_types.iter_mut().for_each(|t| t.origin.line = to(t.origin.line));
        self.hooks.iter_mut().for_each(|h| h.origin.line = to(h.origin.line));
    }

    pub fn append(&mut self, other: Module) {
        self.types.extend(other.types);
        self.natives.extend(other.natives);
        self.globals.extend(other.globals);
        self.functions.extend(other.functions);
        self.libraries.extend(other.libraries);
        self.structs.extend(other.structs);
        self.interfaces.extend(other.interfaces);
        self.mixins.extend(other.mixins);
        self.function_types.extend(other.function_types);
        self.hooks.extend(other.hooks);
    }

    /// What of vJASS and Zinc the module still has, which only lowering
    /// (the build's stage 4) turns into the game's JASS: an error each.
    pub fn not_lowered(&self) -> Vec<Error> {
        self.not_lowered_with(false)
    }

    /// [`Module::not_lowered`]; with `classes` (AngelScript's) structs,
    /// interfaces and their members are kept, not errors.
    pub fn not_lowered_with(&self, classes: bool) -> Vec<Error> {
        let mut out = Vec::new();
        let mut push = |origin: &Origin, what: String| out.push(Error { origin: origin.clone(), message: format!("{what} isn't built into JASS yet") });
        for l in &self.libraries {
            push(&l.origin, format!("{} {}", if l.scope { "scope" } else { "library" }, l.name));
        }
        for s in self.structs.iter().filter(|_| !classes) {
            push(&s.origin, format!("struct {}", s.name));
        }
        for i in self.interfaces.iter().filter(|_| !classes) {
            push(&i.origin, format!("interface {}", i.name));
        }
        for x in &self.mixins {
            push(&x.origin, format!("module {}", x.name));
        }
        for t in &self.function_types {
            push(&t.origin, format!("function interface {}", t.sig.name));
        }
        for h in &self.hooks {
            push(&h.origin, format!("hook {} {}", h.name, h.target));
        }
        // A global's initial value: never a member (lowering refuses one).
        for g in self.globals.iter().filter(|_| !classes) {
            let mut member = false;
            if let Some(e) = &g.init {
                each_expr(&[Stmt::Return { value: Some(e.clone()), line: 0 }], &mut |x| member |= matches!(x, Expr::Member(..) | Expr::Method(_) | Expr::Element(..)));
            }
            if member {
                push(&g.origin, format!("a member (`a.b`) in the initial value of {}", g.name));
            }
        }
        for t in self.types.iter().filter(|t| t.size.is_some()) {
            push(&t.origin, format!("dynamic array type {}", t.name));
        }
        for f in &self.functions {
            let mut what = None;
            each_expr(&f.body, &mut |e| match e {
                Expr::Member(..) | Expr::Method(_) | Expr::Element(..) if !classes => what = what.take().or(Some("a member (`a.b`)")),
                Expr::Binary(_, BinOp::Mod, _) => what = what.take().or(Some("`%`")),
                _ => {}
            });
            fn statements(body: &[Stmt], what: &mut Option<&'static str>, classes: bool) {
                for s in body {
                    match s {
                        Stmt::SetMember { .. } | Stmt::CallMethod { .. } if !classes => *what = what.take().or(Some("a member (`a.b`)")),
                        Stmt::StaticIf { .. } => *what = what.take().or(Some("`static if`")),
                        Stmt::If { branches, otherwise, .. } => {
                            branches.iter().for_each(|(_, b)| statements(b, what, classes));
                            if let Some(b) = otherwise {
                                statements(b, what, classes);
                            }
                        }
                        Stmt::Loop { body, .. } => statements(body, what, classes),
                        Stmt::Debug { stmt, .. } => statements(std::slice::from_ref(stmt), what, classes),
                        _ => {}
                    }
                }
            }
            statements(&f.body, &mut what, classes);
            if let Some(w) = what {
                push(&f.origin, format!("{w} in {}", f.sig.name));
            }
        }
        out
    }
}
