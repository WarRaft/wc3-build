//! JASS (this toolchain's, or the game's) and vJASS into the
//! [`ir`](crate::ir). Each file is parsed by `jass-rs` and its items go
//! into one module, in the files' order. A literal is taken from the
//! source as it's written.
//!
//! vJASS: a library or scope is a [`Library`] and what's in it its own
//! ([`Owner`]: `private`, `public`, else everyone's); structs, interfaces,
//! modules and function interfaces go in as they are; `a.b`, `a.b()`,
//! `this`, `thistype` and `super` as the IR's members and names;
//! `static if ... elseif` as `StaticIf`s, each in the one before's
//! `else`. A `keyword` declares a name ahead, and is left out. JassHelper's
//! preprocessor (`//! textmacro`, `//! runtextmacro`, `//! inject` ...)
//! isn't read: each is an error, not code lost.

use jass_rs::ast as j;

use crate::ir::*;

/// A source file: its name (for errors) and its text.
pub struct Source<'a> {
    pub name: &'a str,
    pub text: &'a str,
}

/// The module of `files`, in their order: a `.zn` file is Zinc
/// ([`crate::from_zinc`]), any other JASS or vJASS (its `//! zinc` blocks
/// Zinc); every file that doesn't parse, or has what isn't read into the
/// IR, is an error.
pub fn module(files: &[Source]) -> Result<Module, Vec<Error>> {
    module_as(files, jass_rs::Dialect::Jass)
}

/// [`module`], a file without a `// @dialect` header read as `fallback`
/// (a `.vj` one as vJASS): the project's dialect.
pub fn module_as(files: &[Source], fallback: jass_rs::Dialect) -> Result<Module, Vec<Error>> {
    let given: Vec<(Source, jass_rs::Dialect)> = files.iter().map(|f| (Source { name: f.name, text: f.text }, fallback)).collect();
    module_with(&given, &|_, _| None)
}

/// The module of `files` (each with the dialect it's read in without a
/// header), in their order, JassHelper's preprocessor run over them all
/// first ([`crate::preprocess`]: macros, `novjass`, imports found by
/// `import`); what's said of the code is said of the lines written.
pub fn module_with(files: &[(Source, jass_rs::Dialect)], import: &crate::preprocess::Import) -> Result<Module, Vec<Error>> {
    let given: Vec<(&str, &str)> = files.iter().map(|(f, _)| (f.name, f.text)).collect();
    let expanded = crate::preprocess::expand(&given, import)?;
    let mut m = Module::default();
    let mut errors = Vec::new();
    for (k, e) in expanded.iter().enumerate() {
        // An imported file: the dialect of the file it came from at first.
        let mut root = k;
        while let Some(by) = expanded[root].imported_by {
            root = by;
        }
        let fallback = files.get(root).map_or(jass_rs::Dialect::Jass, |(_, d)| *d);
        let f = Source { name: &e.name, text: &e.text };
        let mut one = Module::default();
        let mut errs = Vec::new();
        one_file(&mut one, &f, fallback, &mut errs);
        // Back to the lines written.
        // (Past the last, as an error at the end of a file can be: the last.)
        let line = |l: usize| if l == 0 { 0 } else { e.lines.get(l - 1).or(e.lines.last()).copied().unwrap_or(l) };
        one.map_lines(&line);
        errs.iter_mut().for_each(|x| x.origin.line = line(x.origin.line));
        errors.extend(errs);
        m.append(one);
    }
    if errors.is_empty() { Ok(m) } else { Err(errors) }
}

/// One file into `m`: Zinc (a `.zn`), else JASS or vJASS.
fn one_file(m: &mut Module, f: &Source, fallback: jass_rs::Dialect, errors: &mut Vec<Error>) {
    if f.name.to_ascii_lowercase().ends_with(".zn") {
        match zinc_rs::parse(f.text) {
            Ok(p) => crate::from_zinc::add(m, f, &p, errors),
            Err(e) => errors.push(Error { origin: Origin { file: f.name.into(), line: e.line }, message: e.message }),
        }
        return;
    }
    // What JassHelper's preprocessor has that isn't run yet.
    let directives = directives(f);
    if !directives.is_empty() {
        errors.extend(directives);
        return;
    }
    let dialect = jass_rs::lexer::Dialect::declared(f.text).unwrap_or(if f.name.to_ascii_lowercase().ends_with(".vj") { jass_rs::Dialect::VJass } else { fallback });
    match jass_rs::parse_with(f.text, dialect) {
        Ok(p) => add(m, f, &p, errors),
        Err(e) => errors.push(Error { origin: Origin { file: f.name.into(), line: e.line }, message: e.message }),
    }
}

/// One JASS expression (`CreateGroup()`, `'hfoo'`, `bj_X + 1`), as the
/// game's data gives values: what it is, or why it doesn't read.
/// The natives and functions `text` (a script of the game's: `common.j`,
/// `Blizzard.j`) declares; none when it doesn't read.
pub fn signatures(text: &str) -> Vec<Signature> {
    let Ok(p) = jass_rs::parse(text) else { return Vec::new() };
    p.items
        .iter()
        .filter_map(|i| match i {
            j::TopLevel::Native(s) => Some(signature(s)),
            j::TopLevel::Function(f) => Some(signature(&f.sig)),
            _ => None,
        })
        .collect()
}

pub fn expr(text: &str) -> Result<Expr, String> {
    let src = format!("function F takes nothing returns nothing\n    return {text}\nendfunction\n");
    let p = jass_rs::parse(&src).map_err(|e| format!("{text:?}: {}", e.message))?;
    let lines: Vec<&str> = src.lines().collect();
    let r = Reader { lines: &lines, file: "" };
    match p.items.first() {
        Some(j::TopLevel::Function(f)) => match f.body.as_slice() {
            [j::Stmt::Return { value: Some(v), .. }] => Ok(r.expr(v)),
            _ => Err(format!("{text:?} isn't one expression")),
        },
        _ => Err(format!("{text:?} isn't one expression")),
    }
}

/// JassHelper's preprocessor directives, which aren't read.
/// JassHelper's directives not run yet (the rest are [`crate::preprocess`]'s).
const DIRECTIVES: [&str; 5] = ["inject", "endinject", "dovjassinit", "loaddata", "external"];

/// Each of JassHelper's preprocessor directives in a file: an error.
fn directives(f: &Source) -> Vec<Error> {
    let mut out = Vec::new();
    for (i, l) in f.text.lines().enumerate() {
        let Some(rest) = l.trim_start().strip_prefix("//!") else { continue };
        let word = rest.split_whitespace().next().unwrap_or("");
        if DIRECTIVES.contains(&word) {
            out.push(Error { origin: Origin { file: f.name.to_string(), line: i + 1 }, message: format!("JassHelper's `//! {word}` isn't read yet") });
        }
    }
    out
}

fn add(m: &mut Module, f: &Source, p: &j::Program, errors: &mut Vec<Error>) {
    let lines: Vec<&str> = f.text.lines().collect();
    let r = Reader { lines: &lines, file: f.name };
    r.items(m, f, &p.items, None, errors);
}

/// Who an item of vJASS's library or scope `scope` is.
fn owner(scope: Option<&str>, m: &j::Modifiers) -> Owner {
    let access = match (scope, m.access) {
        (None, _) | (_, j::Access::Default) => Access::Everyone,
        (Some(_), j::Access::Public) => Access::Public,
        (Some(_), j::Access::Private) => Access::Private,
    };
    Owner { scope: scope.map(String::from), access }
}

pub(crate) fn signature(s: &j::FunctionSig) -> Signature {
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
    file: &'a str,
}

impl Reader<'_> {
    fn origin(&self, line: usize) -> Origin {
        Origin { file: self.file.to_string(), line }
    }

    /// `items`, in vJASS's library or scope `scope`.
    fn items(&self, m: &mut Module, f: &Source, items: &[j::TopLevel], scope: Option<&str>, errors: &mut Vec<Error>) {
        for item in items {
            match item {
                j::TopLevel::TypeDef { name, extends, line, size, space, modifiers } => m.types.push(TypeDef {
                    name: name.clone(),
                    extends: extends.clone(),
                    size: size.as_ref().map(|e| self.expr(e)),
                    space: space.as_ref().map(|e| self.expr(e)),
                    owner: owner(scope, modifiers),
                    origin: self.origin(*line),
                }),
                j::TopLevel::Native(sig) => m.natives.push(Native { sig: signature(sig), origin: self.origin(sig.line) }),
                j::TopLevel::Globals(decls) => m.globals.extend(decls.iter().map(|g| self.global(g, scope))),
                j::TopLevel::Global(g) => m.globals.push(self.global(g, scope)),
                j::TopLevel::Function(func) => m.functions.push(Function {
                    sig: signature(&func.sig),
                    body: self.stmts(&func.body),
                    owner: owner(scope, &func.sig.modifiers),
                    origin: self.origin(func.sig.line),
                }),
                j::TopLevel::Stmt(s) => errors.push(Error {
                    origin: self.origin(s.line()),
                    message: "a statement outside a function: move it into one (a `main` of its own, say)".into(),
                }),
                j::TopLevel::Zinc(z) => crate::from_zinc::add(m, f, &z.program, errors),
                j::TopLevel::Library(l) => {
                    // A scope in a library or scope is named by its path
                    // (`Outer_Inner`), as JassHelper names it: two of a name
                    // in different parents are two.
                    let name = scope.map_or_else(|| l.name.clone(), |p| format!("{p}_{}", l.name));
                    m.libraries.push(Library {
                        name: name.clone(),
                        scope: l.scope,
                        parent: scope.map(String::from),
                        requires: l.requires.clone(),
                        initializer: l.initializer.clone(),
                        origin: self.origin(l.line),
                    });
                    self.items(m, f, &l.items, Some(&name), errors);
                }
                j::TopLevel::Struct(st) => {
                    let (fields, methods, mixins) = self.members(&st.members, scope);
                    m.structs.push(Struct {
                        name: st.name.clone(),
                        owner: owner(scope, &st.modifiers),
                        extends: st.extends.clone(),
                        size: st.size.as_ref().map(|e| self.expr(e)),
                        fields,
                        methods,
                        mixins,
                        origin: self.origin(st.line),
                    });
                }
                j::TopLevel::Interface(t) => {
                    let (fields, methods, _) = self.members(&t.members, scope);
                    m.interfaces.push(Interface { name: t.name.clone(), owner: owner(scope, &t.modifiers), fields, size: t.size.as_ref().map(|e| self.expr(e)), methods, origin: self.origin(t.line) });
                }
                j::TopLevel::Module(t) => {
                    let (fields, methods, mixins) = self.members(&t.members, scope);
                    m.mixins.push(Mixin { name: t.name.clone(), owner: owner(scope, &t.modifiers), fields, methods, mixins, origin: self.origin(t.line) });
                }
                j::TopLevel::FunctionInterface(sig) => {
                    m.function_types.push(FunctionType { sig: signature(sig), owner: owner(scope, &sig.modifiers), origin: self.origin(sig.line) })
                }
                j::TopLevel::Keyword { .. } => {}
                j::TopLevel::Hook { name, target, line } => {
                    m.hooks.push(Hook { name: name.clone(), target: target.clone(), owner: owner(scope, &j::Modifiers::default()), origin: self.origin(*line) })
                }
            }
        }
    }

    /// A struct's, interface's or module's members.
    fn members(&self, members: &[j::StructMember], scope: Option<&str>) -> (Vec<Field>, Vec<Method>, Vec<(String, bool)>) {
        let (mut fields, mut methods, mut mixins) = (Vec::new(), Vec::new(), Vec::new());
        for member in members {
            match member {
                j::StructMember::Field(g) => fields.push(Field {
                    name: g.name.clone(),
                    ty: g.type_name.clone(),
                    array: g.is_array.then(|| g.size.as_ref().map(|e| self.expr(e))),
                    is_static: g.modifiers.is_static,
                    constant: g.is_constant,
                    readonly: g.modifiers.is_readonly,
                    delegate: g.modifiers.is_delegate,
                    private: g.modifiers.access == j::Access::Private,
                    init: g.initializer.as_ref().map(|e| self.expr(e)),
                    origin: self.origin(g.line),
                }),
                j::StructMember::Method(md) => {
                    let m = &md.sig.modifiers;
                    methods.push(Method {
                        function: Function {
                            sig: signature(&md.sig),
                            body: md.body.as_ref().map(|b| self.stmts(b)).unwrap_or_default(),
                            owner: Owner { scope: scope.map(String::from), access: Access::Everyone },
                            origin: self.origin(md.sig.line),
                        },
                        is_static: m.is_static,
                        stub: m.is_stub,
                        private: m.access == j::Access::Private,
                        abstract_: md.body.is_none(),
                        defaults: md.defaults.as_ref().map(|e| self.expr(e)),
                    });
                }
                j::StructMember::Implement { name, optional, .. } => mixins.push((name.clone(), *optional)),
            }
        }
        (fields, methods, mixins)
    }

    fn global(&self, g: &j::GlobalDecl, scope: Option<&str>) -> Global {
        Global {
            name: g.name.clone(),
            ty: g.type_name.clone(),
            array: g.is_array,
            // A 2D array: of its width times its height.
            size: g.size.as_ref().map(|w| match &g.height {
                Some(h) => Expr::binary(self.expr(w), BinOp::Mul, self.expr(h)),
                None => self.expr(w),
            }),
            dims: g.size.as_ref().zip(g.height.as_ref()).map(|(w, h)| (self.expr(w), self.expr(h))),
            constant: g.is_constant,
            init: g.initializer.as_ref().map(|e| self.expr(e)),
            owner: owner(scope, &g.modifiers),
            origin: self.origin(g.line),
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
            // `a[i][j] = v`: an element of `a[i]`.
            j::Stmt::SetMember { target, index: Some(j), value, line, .. } if matches!(target.kind, j::ExprKind::ArrayAccess(..)) => {
                Stmt::SetElement { target: self.expr(target), index: self.expr(j), value: self.expr(value), line: *line }
            }
            j::Stmt::SetMember { target, index, value, line, .. } => {
                let (object, name) = match &target.kind {
                    j::ExprKind::Member(o, n) => (self.expr(o), n.clone()),
                    _ => unreachable!("the parser sets a member or an element only"),
                };
                Stmt::SetMember { object, name, index: index.as_ref().map(|e| self.expr(e)), value: self.expr(value), line: *line }
            }
            j::Stmt::CallMethod { call, line, .. } => match self.expr(call) {
                Expr::Method(call) => Stmt::CallMethod { call: *call, line: *line },
                _ => unreachable!("the parser calls a method only"),
            },
            j::Stmt::StaticIf { branches, else_branch, line, .. } => {
                // `elseif`s: each in the one before's `else`.
                let mut otherwise = else_branch.as_ref().map(|b| self.stmts(b));
                for (cond, body) in branches.iter().skip(1).rev() {
                    otherwise = Some(vec![Stmt::StaticIf { cond: self.expr(cond), then: self.stmts(body), otherwise, line: *line }]);
                }
                let (cond, body) = &branches[0];
                Stmt::StaticIf { cond: self.expr(cond), then: self.stmts(body), otherwise, line: *line }
            }
        }
    }

    fn text(&self, span: j::Span) -> Option<String> {
        written(self.lines, span)
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
            K::This => Expr::var("this"),
            K::ThisType => Expr::var("thistype"),
            K::Super => Expr::var("super"),
            K::Member(o, n) => Expr::Member(Box::new(self.expr(o)), n.clone()),
            K::MethodCall(o, n, args) => Expr::Method(Box::new(MethodCall { object: self.expr(o), name: n.clone(), args: args.iter().map(|a| self.expr(a)).collect() })),
            K::Index(o, i) => Expr::Element(Box::new(self.expr(o)), Box::new(self.expr(i))),
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

/// What's written at `span` (one line) of a file's `lines`: a literal as
/// it was written; `None` when the span isn't one line of them.
pub(crate) fn written(lines: &[&str], span: j::Span) -> Option<String> {
    if span.start_line != span.end_line {
        return None;
    }
    let line = lines.get(span.start_line.checked_sub(1)?)?;
    let chars: Vec<char> = line.chars().collect();
    let (a, b) = (span.start_col.checked_sub(1)?, span.end_col.checked_sub(1)?);
    chars.get(a..b).map(|c| c.iter().collect())
}

#[cfg(test)]
#[path = "from_jass.test.rs"]
mod tests;
