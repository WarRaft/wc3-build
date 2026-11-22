//! The model of a map's code that every source goes into and every output
//! is made from (see the README). For now it holds what JASS has: types,
//! natives, globals and functions; libraries, structs and methods come
//! with vJASS and Zinc, as items of their own, so an output that has
//! classes (AngelScript) gets them as they are.
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
}

#[derive(Debug, Clone, PartialEq)]
pub struct TypeDef {
    pub name: String,
    pub extends: String,
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
    pub constant: bool,
    pub init: Option<Expr>,
    pub origin: Origin,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Function {
    pub sig: Signature,
    pub body: Vec<Stmt>,
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
}

#[derive(Debug, Clone, PartialEq)]
pub struct Call {
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
