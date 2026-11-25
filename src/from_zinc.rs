//! Zinc (a `.zn` file, or a vJASS file's `//! zinc` block) into the
//! [`ir`](crate::ir), parsed by `zinc-rs`.
//!
//! - A library is a [`Library`]; its members are its own
//!   ([`Owner::scope`]), private unless `public` (Zinc's default), and its
//!   `onInit` is its initializer. What's outside any library (this
//!   toolchain's global scope) is everyone's.
//! - Structs, interfaces and modules go in as they are, with their fields
//!   and methods; a struct's members are public unless `private`.
//! - Zinc's own forms are made into JASS's, which the IR has: `while`,
//!   `do ... while` and `for` into `loop` and `exitwhen` (`for (0 <= i < n)`
//!   sets `i` to 0, leaves when `i >= n`, adds 1 at the end), `break` into
//!   `exitwhen true`, `+=` into `set x = x + v`, `else if` into `elseif`,
//!   a block into its statements.
//! - An anonymous function is made a function of its own
//!   (`wcb__anon_<n>`, private to its library), named where it was.
//! - A literal is taken as written.
//!
//! An array of more than one dimension isn't read (an error).

use zinc_rs::ast as z;

use crate::from_jass::{Source, written};
use crate::ir::*;

/// The prefix of an anonymous function's name.
const ANON: &str = "wcb__anon_";

/// Adds a Zinc program's items, from file `f`, to `m`.
pub fn add(m: &mut Module, f: &Source, p: &z::Program, errors: &mut Vec<Error>) {
    let lines: Vec<&str> = f.text.lines().collect();
    let next_anon = m.functions.iter().filter(|g| g.sig.name.starts_with(ANON)).count();
    let mut r = Reader { file: f.name, lines: &lines, m, errors, next_anon, scope: None, grids: Default::default() };
    // The arrays of several dimensions first: a function may use one
    // declared below it.
    let members = p.globals.iter().chain(p.libraries.iter().flat_map(|l| &l.members));
    for member in members {
        let vars: Vec<&z::VarDecl> = match member {
            z::Member::Var(v) => vec![v],
            z::Member::Struct(st) => st.members.iter().filter_map(|m| if let z::StructMember::Field(v) = m { Some(v) } else { None }).collect(),
            _ => Vec::new(),
        };
        for v in vars.into_iter().filter(|v| v.dims.len() > 1) {
            if let Some(rest) = v.dims[1..].iter().cloned().collect::<Option<Vec<_>>>() {
                r.grids.insert(v.name.clone(), rest);
            }
        }
    }
    for s in &p.stmts {
        r.error(s.line(), "a statement outside a function: move it into one");
    }
    for member in &p.globals {
        r.member(member);
    }
    for lib in &p.libraries {
        r.scope = Some(lib.name.clone());
        let initializer = lib
            .members
            .iter()
            .any(|x| matches!(x, z::Member::Function(f) if f.sig.name == "onInit" && f.sig.params.is_empty() && f.sig.returns.is_none()))
            .then(|| "onInit".to_string());
        let origin = r.origin(lib.line);
        r.m.libraries.push(Library {
            name: lib.name.clone(),
            scope: false,
            parent: None,
            requires: lib.requires.iter().map(|q| (q.name.clone(), q.optional)).collect(),
            initializer,
            origin,
        });
        for member in &lib.members {
            r.member(member);
        }
        r.scope = None;
    }
}

struct Reader<'a> {
    file: &'a str,
    lines: &'a [&'a str],
    m: &'a mut Module,
    errors: &'a mut Vec<Error>,
    next_anon: usize,
    /// The library being read.
    scope: Option<String>,
    /// Arrays of more than one dimension: each one's sizes past the first
    /// (`integer a[10][20]`: `[20]`), for `a[i][j]` as `a[i * 20 + j]`.
    grids: std::collections::HashMap<String, Vec<z::Expr>>,
}

impl Reader<'_> {
    fn origin(&self, line: usize) -> Origin {
        Origin { file: self.file.to_string(), line }
    }

    fn error(&mut self, line: usize, message: &str) {
        self.errors.push(Error { origin: self.origin(line), message: message.to_string() });
    }

    /// Who a library member is: in a library private unless `public`;
    /// outside one everyone's.
    fn owner(&self, modifiers: &z::Modifiers) -> Owner {
        let access = match (&self.scope, modifiers.access) {
            (None, _) | (Some(_), z::Access::Public) => Access::Everyone,
            (Some(_), _) => Access::Private,
        };
        Owner { scope: self.scope.clone(), access }
    }

    fn member(&mut self, member: &z::Member) {
        match member {
            z::Member::Var(v) => {
                if let Some(g) = self.global(v) {
                    self.m.globals.push(g);
                }
            }
            z::Member::Function(f) => {
                let owner = self.owner(&f.sig.modifiers);
                if let Some(func) = self.function(f, owner) {
                    self.m.functions.push(func);
                }
            }
            z::Member::Struct(s) => {
                let (fields, methods, mixins) = self.members(&s.members);
                let st = Struct {
                    name: s.name.clone(),
                    owner: self.owner(&s.modifiers),
                    extends: s.extends.clone(),
                    size: s.size.as_ref().map(|e| self.expr(e)),
                    fields,
                    methods,
                    mixins,
                    origin: self.origin(s.line),
                };
                self.m.structs.push(st);
            }
            z::Member::Interface(t) => {
                let (fields, methods, _) = self.members(&t.members);
                let i = Interface { name: t.name.clone(), owner: self.owner(&t.modifiers), fields, size: None, methods, origin: self.origin(t.line) };
                self.m.interfaces.push(i);
            }
            z::Member::Module(t) => {
                let (fields, methods, mixins) = self.members(&t.members);
                let x = Mixin { name: t.name.clone(), owner: self.owner(&t.modifiers), fields, methods, mixins, origin: self.origin(t.line) };
                self.m.mixins.push(x);
            }
            z::Member::TypeDef { modifiers, name, extends, size, line } => {
                let t = TypeDef {
                    name: name.clone(),
                    extends: extends.clone(),
                    size: size.as_ref().map(|e| self.expr(e)),
                    owner: self.owner(modifiers),
                    origin: self.origin(*line),
                };
                self.m.types.push(t);
            }
        }
    }

    /// A struct's, interface's or module's members.
    fn members(&mut self, members: &[z::StructMember]) -> (Vec<Field>, Vec<Method>, Vec<(String, bool)>) {
        let (mut fields, mut methods, mut mixins) = (Vec::new(), Vec::new(), Vec::new());
        for member in members {
            match member {
                z::StructMember::Field(v) => {
                    let Some(array) = self.dims(v) else { continue };
                    let field = Field {
                        name: v.name.clone(),
                        ty: v.type_name.clone(),
                        array,
                        is_static: v.modifiers.is_static,
                        constant: v.modifiers.is_constant,
                        readonly: v.modifiers.is_readonly,
                        delegate: v.modifiers.is_delegate,
                        private: v.modifiers.access == z::Access::Private,
                        init: v.initializer.as_ref().map(|e| self.expr(e)),
                        origin: self.origin(v.line),
                    };
                    fields.push(field);
                }
                z::StructMember::Method(f) => {
                    let m = &f.sig.modifiers;
                    let (is_static, stub, private) = (m.is_static, m.is_stub, m.access == z::Access::Private);
                    let function = Function {
                        sig: signature(&f.sig),
                        body: f.body.as_ref().map(|b| self.stmts(b)).unwrap_or_default(),
                        owner: Owner { scope: self.scope.clone(), access: Access::Everyone },
                        origin: self.origin(f.sig.line),
                    };
                    methods.push(Method { function, is_static, stub, private, abstract_: f.body.is_none(), defaults: None });
                }
                z::StructMember::Implement { name, optional, .. } => mixins.push((name.clone(), *optional)),
            }
        }
        (fields, methods, mixins)
    }

    /// An array's size: `None` not an array, `Some(None)` unsized. One
    /// of several dimensions (`a[10][20]`) is one array of them all (200;
    /// unsized when the first is), as JassHelper makes it: every size
    /// but the first must be given (an error, and `None` back, else).
    fn dims(&mut self, v: &z::VarDecl) -> Option<Option<Option<Expr>>> {
        match v.dims.as_slice() {
            [] => Some(None),
            [d] => Some(Some(d.as_ref().map(|e| self.expr(e)))),
            [first, rest @ ..] => {
                let Some(rest) = rest.iter().cloned().collect::<Option<Vec<z::Expr>>>() else {
                    self.error(v.line, &format!("{}: every size of an array of several dimensions but the first must be given", v.name));
                    return None;
                };
                self.grids.insert(v.name.clone(), rest.clone());
                // Of numbers: the number (a struct's field needs one).
                let number = |e: &z::Expr| if let z::ExprKind::IntLiteral(n) = e.kind { Some(n) } else { None };
                let size = first.as_ref().map(|f| match std::iter::once(f).chain(&rest).map(number).collect::<Option<Vec<i64>>>() {
                    Some(ns) => {
                        // Past a JASS array's places: several of them
                        // (`jass::split_big_arrays`).
                        Expr::int(ns.iter().product::<i64>())
                    }
                    None => rest.iter().fold(self.expr(f), |acc, d| Expr::binary(acc, BinOp::Mul, self.expr(d))),
                });
                Some(Some(size))
            }
        }
    }

    /// `a[i][j]` of an array of several dimensions: what's indexed (the
    /// name, or a member) and the one index it's at (`i * 20 + j`).
    /// `None` for any other.
    fn grid_index<'e>(&mut self, e: &'e z::Expr) -> Option<(&'e z::Expr, Expr)> {
        let mut indices = Vec::new();
        let mut base = e;
        while let z::ExprKind::Index(a, i) = &base.kind {
            indices.push(&**i);
            base = a;
        }
        let name = match &base.kind {
            z::ExprKind::Var(n) | z::ExprKind::Member(_, n) => n,
            _ => return None,
        };
        let sizes = self.grids.get(name)?.clone();
        indices.reverse();
        if indices.len() != sizes.len() + 1 {
            self.error(e.span.start_line, &format!("{name} has {} dimensions: as many indices", sizes.len() + 1));
            return None;
        }
        let mut at = self.expr(indices[0]);
        for (i, d) in indices[1..].iter().zip(&sizes) {
            at = Expr::binary(Expr::binary(at, BinOp::Mul, self.expr(d)), BinOp::Add, self.expr(i));
        }
        Some((base, at))
    }

    fn global(&mut self, v: &z::VarDecl) -> Option<Global> {
        let array = self.dims(v)?;
        Some(Global {
            name: v.name.clone(),
            ty: v.type_name.clone(),
            array: array.is_some(),
            size: array.clone().flatten(),
            dims: None,
            constant: v.modifiers.is_constant,
            init: v.initializer.as_ref().map(|e| self.expr(e)),
            owner: self.owner(&v.modifiers),
            origin: self.origin(v.line),
        })
    }

    fn function(&mut self, f: &z::FunctionDecl, owner: Owner) -> Option<Function> {
        let Some(body) = &f.body else {
            self.error(f.sig.line, &format!("{} has no body", f.sig.name));
            return None;
        };
        Some(Function { sig: signature(&f.sig), body: self.stmts(body), owner, origin: self.origin(f.sig.line) })
    }

    fn stmts(&mut self, body: &[z::Stmt]) -> Vec<Stmt> {
        let mut out = Vec::new();
        for s in body {
            self.stmt(s, &mut out);
        }
        out
    }

    fn stmt(&mut self, s: &z::Stmt, out: &mut Vec<Stmt>) {
        match s {
            z::Stmt::Local(v) => {
                let Some(array) = self.dims(v) else { return };
                let init = v.initializer.as_ref().map(|e| self.expr(e));
                out.push(Stmt::Local { ty: v.type_name.clone(), name: v.name.clone(), array: array.is_some(), init, line: v.line });
            }
            z::Stmt::Assign { target, op, value, line } => {
                let value = self.expr(value);
                let value = match op {
                    z::AssignOp::Assign => value,
                    op => {
                        let op = match op {
                            z::AssignOp::Add => BinOp::Add,
                            z::AssignOp::Sub => BinOp::Sub,
                            z::AssignOp::Mul => BinOp::Mul,
                            _ => BinOp::Div,
                        };
                        Expr::binary(self.expr(target), op, value)
                    }
                };
                if let Some(s) = self.set(target, value, *line) {
                    out.push(s);
                }
            }
            z::Stmt::Expr { expr, line } => match self.expr(expr) {
                Expr::Call(call) => out.push(Stmt::Call { call, line: *line }),
                Expr::Method(call) => out.push(Stmt::CallMethod { call: *call, line: *line }),
                _ => self.error(*line, "a statement of an expression that isn't a call"),
            },
            z::Stmt::If { is_static: true, cond, then_body, else_body, line, .. } => {
                let (cond, then) = (self.expr(cond), self.stmts(then_body));
                let otherwise = else_body.as_ref().map(|b| self.stmts(b));
                out.push(Stmt::StaticIf { cond, then, otherwise, line: *line });
            }
            z::Stmt::If { cond, then_body, else_body, line, .. } => {
                let mut branches = vec![(self.expr(cond), self.stmts(then_body))];
                let mut rest = else_body.as_deref();
                // `else if`: an `else` of one `if` (not a static one).
                let otherwise = loop {
                    match rest {
                        Some([z::Stmt::If { is_static: false, cond, then_body, else_body, .. }]) => {
                            branches.push((self.expr(cond), self.stmts(then_body)));
                            rest = else_body.as_deref();
                        }
                        Some(b) => break Some(self.stmts(b)),
                        None => break None,
                    }
                };
                out.push(Stmt::If { branches, otherwise, line: *line });
            }
            z::Stmt::While { cond, body, line } => {
                let mut b = vec![Stmt::ExitWhen { cond: not(self.expr(cond)), line: *line }];
                b.extend(self.stmts(body));
                out.push(Stmt::Loop { body: b, line: *line });
            }
            z::Stmt::DoWhile { body, cond, line } => {
                let mut b = self.stmts(body);
                b.push(Stmt::ExitWhen { cond: not(self.expr(cond)), line: *line });
                out.push(Stmt::Loop { body: b, line: *line });
            }
            z::Stmt::For { start, start_op, var, end_op, end, body, line, .. } => {
                // Up for `<`/`<=`, down for `>`/`>=`; a strict bound one
                // past it.
                let up = matches!(end_op, z::BinOp::Lt | z::BinOp::LtEq);
                let step = if up { BinOp::Add } else { BinOp::Sub };
                let start = self.expr(start);
                let first = match start_op {
                    z::BinOp::Lt | z::BinOp::Gt => Expr::binary(start, step, Expr::int(1)),
                    _ => start,
                };
                let past = match end_op {
                    z::BinOp::Lt => BinOp::GtEq,
                    z::BinOp::LtEq => BinOp::Gt,
                    z::BinOp::Gt => BinOp::LtEq,
                    _ => BinOp::Lt,
                };
                out.push(Stmt::Set { name: var.clone(), index: None, value: first, line: *line });
                let mut b = vec![Stmt::ExitWhen { cond: Expr::binary(Expr::var(var.clone()), past, self.expr(end)), line: *line }];
                b.extend(self.stmts(body));
                b.push(Stmt::Set { name: var.clone(), index: None, value: Expr::binary(Expr::var(var.clone()), step, Expr::int(1)), line: *line });
                out.push(Stmt::Loop { body: b, line: *line });
            }
            z::Stmt::Break { line } => out.push(Stmt::ExitWhen { cond: Expr::bool(true), line: *line }),
            z::Stmt::Return { value, line } => {
                let value = value.as_ref().map(|e| self.expr(e));
                out.push(Stmt::Return { value, line: *line });
            }
            z::Stmt::Debug { body, line } => {
                for s in self.stmts(body) {
                    if matches!(s, Stmt::Local { .. }) {
                        self.error(*line, "a local in `debug`");
                        continue;
                    }
                    out.push(Stmt::Debug { stmt: Box::new(s), line: *line });
                }
            }
            z::Stmt::Block { body, .. } => {
                for s in body {
                    self.stmt(s, out);
                }
            }
        }
    }

    /// `target = value`: a name, an element of one, a member, or an
    /// element of a member.
    fn set(&mut self, target: &z::Expr, value: Expr, line: usize) -> Option<Stmt> {
        let (base, index) = match self.grid_index(target) {
            Some((base, at)) => (base, Some(at)),
            None => match &target.kind {
                z::ExprKind::Index(a, i) => (&**a, Some(self.expr(i))),
                _ => (target, None),
            },
        };
        match &base.kind {
            z::ExprKind::Var(name) => Some(Stmt::Set { name: name.clone(), index, value, line }),
            z::ExprKind::Member(object, name) => Some(Stmt::SetMember { object: self.expr(object), name: name.clone(), index, value, line }),
            _ => {
                self.error(line, "this can't be set: only a name, a member or an element of one");
                None
            }
        }
    }

    fn expr(&mut self, e: &z::Expr) -> Expr {
        use z::ExprKind as K;
        let text = |r: &Self, fallback: String| Expr::Literal(written(r.lines, e.span).unwrap_or(fallback));
        match &e.kind {
            K::IntLiteral(v) => text(self, v.to_string()),
            K::RealLiteral(v) => text(self, format!("{v:?}")),
            K::StringLiteral(s) => text(self, format!("{s:?}")),
            K::BoolLiteral(b) => Expr::bool(*b),
            K::Null => Expr::lit("null"),
            K::This => Expr::var("this"),
            K::ThisType => Expr::var("thistype"),
            K::Var(n) => Expr::var(n.clone()),
            K::Member(object, name) => Expr::Member(Box::new(self.expr(object)), name.clone()),
            K::Index(a, i) => {
                // Of an array of several dimensions: one index.
                if let Some((base, at)) = self.grid_index(e) {
                    return match &base.kind {
                        K::Var(n) => Expr::Index(n.clone(), Box::new(at)),
                        _ => Expr::Element(Box::new(self.expr(base)), Box::new(at)),
                    };
                }
                let index = Box::new(self.expr(i));
                match &a.kind {
                    K::Var(n) => Expr::Index(n.clone(), index),
                    _ => Expr::Element(Box::new(self.expr(a)), index),
                }
            }
            K::Call(callee, args) => {
                let args: Vec<Expr> = args.iter().map(|a| self.expr(a)).collect();
                match &callee.kind {
                    K::Var(n) => Expr::call(n.clone(), args),
                    // `thistype(x)`: a cast.
                    K::ThisType => Expr::call("thistype", args),
                    K::Member(object, name) => Expr::Method(Box::new(MethodCall { object: self.expr(object), name: name.clone(), args })),
                    _ => {
                        self.error(e.span.start_line, "only a name or a member can be called");
                        Expr::lit("null")
                    }
                }
            }
            K::FuncRef(n) => Expr::code(n.clone()),
            K::AnonFunction(f) => {
                let name = format!("{ANON}{}", self.next_anon);
                self.next_anon += 1;
                let mut f = (**f).clone();
                f.sig.name = name.clone();
                let owner = Owner { scope: self.scope.clone(), access: Access::Private };
                if let Some(func) = self.function(&f, owner) {
                    self.m.functions.push(func);
                }
                Expr::code(name)
            }
            K::Unary(op, x) => {
                let op = match op {
                    z::UnaryOp::Neg => UnaryOp::Neg,
                    z::UnaryOp::Not => UnaryOp::Not,
                };
                Expr::Unary(op, Box::new(self.expr(x)))
            }
            K::Binary(a, op, b) => {
                let op = match op {
                    z::BinOp::Add => BinOp::Add,
                    z::BinOp::Sub => BinOp::Sub,
                    z::BinOp::Mul => BinOp::Mul,
                    z::BinOp::Div => BinOp::Div,
                    z::BinOp::Mod => BinOp::Mod,
                    z::BinOp::Eq => BinOp::Eq,
                    z::BinOp::NotEq => BinOp::NotEq,
                    z::BinOp::Gt => BinOp::Gt,
                    z::BinOp::Lt => BinOp::Lt,
                    z::BinOp::GtEq => BinOp::GtEq,
                    z::BinOp::LtEq => BinOp::LtEq,
                    z::BinOp::And => BinOp::And,
                    z::BinOp::Or => BinOp::Or,
                };
                Expr::binary(self.expr(a), op, self.expr(b))
            }
        }
    }
}

fn not(e: Expr) -> Expr {
    Expr::Unary(UnaryOp::Not, Box::new(e))
}

fn signature(s: &z::FunctionSig) -> Signature {
    Signature {
        name: s.name.clone(),
        params: s.params.iter().map(|p| Param { ty: p.type_name.clone(), name: p.name.clone() }).collect(),
        returns: s.returns.clone(),
        constant: s.modifiers.is_constant,
    }
}

#[cfg(test)]
#[path = "from_zinc.test.rs"]
mod tests;
