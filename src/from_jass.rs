//! JASS (this toolchain's, or the game's) into the [`ir`](crate::ir).
//! Each file is parsed by `jass-rs` and its items go into one module, in
//! the files' order. A literal is taken from the source as it's written.

use jass_rs::ast as j;

use crate::ir::*;

/// A source file: its name (for errors) and its text.
pub struct Source<'a> {
    pub name: &'a str,
    pub text: &'a str,
}

/// The module of `files`, in their order; every file that doesn't parse,
/// or has what isn't built yet (Zinc), is an error.
pub fn module(files: &[Source]) -> Result<Module, Vec<Error>> {
    let mut m = Module::default();
    let mut errors = Vec::new();
    for f in files {
        match jass_rs::parse_file(f.text) {
            Ok(p) => add(&mut m, f, &p, &mut errors),
            Err(e) => errors.push(Error { origin: Origin { file: f.name.into(), line: e.line }, message: e.message }),
        }
    }
    if errors.is_empty() { Ok(m) } else { Err(errors) }
}

fn add(m: &mut Module, f: &Source, p: &j::Program, errors: &mut Vec<Error>) {
    let lines: Vec<&str> = f.text.lines().collect();
    let r = Reader { lines: &lines };
    let origin = |line: usize| Origin { file: f.name.to_string(), line };
    for item in &p.items {
        match item {
            j::TopLevel::TypeDef { name, extends, line } => {
                m.types.push(TypeDef { name: name.clone(), extends: extends.clone(), origin: origin(*line) })
            }
            j::TopLevel::Native(sig) => m.natives.push(Native { sig: signature(sig), origin: origin(sig.line) }),
            j::TopLevel::Globals(decls) => m.globals.extend(decls.iter().map(|g| r.global(g, origin(g.line)))),
            j::TopLevel::Global(g) => m.globals.push(r.global(g, origin(g.line))),
            j::TopLevel::Function(func) => m.functions.push(Function {
                sig: signature(&func.sig),
                body: r.stmts(&func.body),
                origin: origin(func.sig.line),
            }),
            j::TopLevel::Stmt(s) => errors.push(Error {
                origin: origin(stmt_line(s)),
                message: "a statement outside a function: move it into one (a `main` of its own, say)".into(),
            }),
            j::TopLevel::Zinc(z) => errors.push(Error {
                origin: origin(z.open.start_line),
                message: "Zinc isn't built yet".into(),
            }),
        }
    }
}

fn stmt_line(s: &j::Stmt) -> usize {
    match s {
        j::Stmt::Local { line, .. }
        | j::Stmt::Set { line, .. }
        | j::Stmt::Call { line, .. }
        | j::Stmt::If { line, .. }
        | j::Stmt::Loop { line, .. }
        | j::Stmt::ExitWhen { line, .. }
        | j::Stmt::Return { line, .. }
        | j::Stmt::Debug { line, .. } => *line,
    }
}

fn signature(s: &j::FunctionSig) -> Signature {
    Signature {
        name: s.name.clone(),
        params: s.params.iter().map(|p| Param { ty: p.type_name.clone(), name: p.name.clone() }).collect(),
        returns: s.returns.clone(),
        constant: s.is_constant,
    }
}

/// Reads the AST with its source at hand, for literals as written.
struct Reader<'a> {
    lines: &'a [&'a str],
}

impl Reader<'_> {
    fn global(&self, g: &j::GlobalDecl, origin: Origin) -> Global {
        Global {
            name: g.name.clone(),
            ty: g.type_name.clone(),
            array: g.is_array,
            constant: g.is_constant,
            init: g.initializer.as_ref().map(|e| self.expr(e)),
            origin,
        }
    }

    fn stmts(&self, body: &[j::Stmt]) -> Vec<Stmt> {
        body.iter().map(|s| self.stmt(s)).collect()
    }

    fn stmt(&self, s: &j::Stmt) -> Stmt {
        match s {
            j::Stmt::Local { type_name, is_array, name, initializer, line, .. } => Stmt::Local {
                ty: type_name.clone(),
                name: name.clone(),
                array: *is_array,
                init: initializer.as_ref().map(|e| self.expr(e)),
                line: *line,
            },
            j::Stmt::Set { name, index, value, line, .. } => Stmt::Set {
                name: name.clone(),
                index: index.as_ref().map(|e| self.expr(e)),
                value: self.expr(value),
                line: *line,
            },
            j::Stmt::Call { name, args, line, .. } => {
                Stmt::Call { call: Call { name: name.clone(), args: args.iter().map(|a| self.expr(a)).collect() }, line: *line }
            }
            j::Stmt::If { branches, else_branch, line, .. } => Stmt::If {
                branches: branches.iter().map(|(c, b)| (self.expr(c), self.stmts(b))).collect(),
                otherwise: else_branch.as_ref().map(|b| self.stmts(b)),
                line: *line,
            },
            j::Stmt::Loop { body, line } => Stmt::Loop { body: self.stmts(body), line: *line },
            j::Stmt::ExitWhen { cond, line } => Stmt::ExitWhen { cond: self.expr(cond), line: *line },
            j::Stmt::Return { value, line } => Stmt::Return { value: value.as_ref().map(|e| self.expr(e)), line: *line },
            j::Stmt::Debug { stmt, line } => Stmt::Debug { stmt: Box::new(self.stmt(stmt)), line: *line },
        }
    }

    /// A literal's text as written (its span), else as the AST has it.
    fn text(&self, span: j::Span) -> Option<String> {
        if span.start_line != span.end_line {
            return None;
        }
        let line = self.lines.get(span.start_line.checked_sub(1)?)?;
        let chars: Vec<char> = line.chars().collect();
        let (a, b) = (span.start_col.checked_sub(1)?, span.end_col.checked_sub(1)?);
        chars.get(a..b).map(|c| c.iter().collect())
    }

    fn expr(&self, e: &j::Expr) -> Expr {
        use j::ExprKind as K;
        match &e.kind {
            K::IntLiteral(v) => Expr::Literal(self.text(e.span).unwrap_or_else(|| v.to_string())),
            K::RealLiteral(v) => Expr::Literal(self.text(e.span).unwrap_or_else(|| format!("{v:?}"))),
            K::StringLiteral(s) => Expr::Literal(self.text(e.span).unwrap_or_else(|| format!("{s:?}"))),
            K::BoolLiteral(b) => Expr::Literal(b.to_string()),
            K::Null => Expr::Literal("null".into()),
            K::Var(n) => Expr::Var(n.clone()),
            K::ArrayAccess(n, i) => Expr::Index(n.clone(), Box::new(self.expr(i))),
            K::Call(n, args) => Expr::Call(Call { name: n.clone(), args: args.iter().map(|a| self.expr(a)).collect() }),
            K::FuncRef(n) => Expr::Code(n.clone()),
            K::Unary(op, x) => Expr::Unary(
                match op {
                    j::UnaryOp::Neg => UnaryOp::Neg,
                    j::UnaryOp::Plus => UnaryOp::Plus,
                    j::UnaryOp::Not => UnaryOp::Not,
                },
                Box::new(self.expr(x)),
            ),
            K::Binary(a, op, b) => Expr::Binary(
                Box::new(self.expr(a)),
                match op {
                    j::BinOp::Add => BinOp::Add,
                    j::BinOp::Sub => BinOp::Sub,
                    j::BinOp::Mul => BinOp::Mul,
                    j::BinOp::Div => BinOp::Div,
                    j::BinOp::Eq => BinOp::Eq,
                    j::BinOp::NotEq => BinOp::NotEq,
                    j::BinOp::Gt => BinOp::Gt,
                    j::BinOp::Lt => BinOp::Lt,
                    j::BinOp::GtEq => BinOp::GtEq,
                    j::BinOp::LtEq => BinOp::LtEq,
                    j::BinOp::And => BinOp::And,
                    j::BinOp::Or => BinOp::Or,
                },
                Box::new(self.expr(b)),
            ),
        }
    }
}

#[cfg(test)]
#[path = "from_jass.test.rs"]
mod tests;
