//! The game's Lua of a module made the game's JASS ([`crate::jass`]'s
//! steps but the last: the same tree, written in Lua instead): what
//! `[build] script=lua` writes (`war3map.lua`).
//!
//! The natives and `Blizzard.j`'s functions are the game's in its Lua
//! too, so they're called as they are and never written again (their
//! `bj_` globals are the game's state). What JASS has that Lua writes
//! otherwise, by the types (of globals, locals, parameters, what
//! functions and the game's natives give back):
//!
//! - an integer divided by an integer is cut toward zero (`wcb__div`),
//!   as JASS does, not Lua's `//` (toward the lower);
//! - `+` of strings is `..`, a `null` string taken as empty;
//! - `!=` is `~=`; `and` binds looser than `or` in JASS, tighter in Lua:
//!   parentheses where the tree needs them;
//! - an array is a table giving its type's empty value for what's never
//!   set (`wcb__array`); a variable never set, its type's (0, false,
//!   nil);
//! - `'hfoo'` is the number it stands for, `$FF` `0xFF`, `017` (octal)
//!   `15`, `.5` `0.5`;
//! - `return` is `do return ... end` (JASS may have more after it);
//!   `loop` is `while true do`, `exitwhen c` `if c then break end`;
//! - `function F` is `F`; `debug` lines are left out, as the game does
//!   when it isn't debugging.

use std::collections::HashMap;
use std::fmt::Write as _;

use crate::ir::*;

/// What every script begins with: its helpers.
const PRELUDE: &str = "\
-- Made by wc3-build from JASS.
local function wcb__array(empty)
    return setmetatable({}, {__index = function() return empty end})
end
local function wcb__div(a, b)
    local q = a / b
    return math.tointeger(q >= 0 and math.floor(q) or math.ceil(q))
end
";

/// What a name's type is: globals', a function's locals' and parameters',
/// what functions give back.
pub(crate) struct Types<'a> {
    pub(crate) globals: HashMap<&'a str, &'a str>,
    pub(crate) returns: HashMap<&'a str, Option<&'a str>>,
    pub(crate) locals: HashMap<String, String>,
}

impl Types<'_> {
    /// The type of `e` when it can be told.
    pub(crate) fn of(&self, e: &Expr) -> Option<String> {
        match e {
            Expr::Literal(t) => literal_type(t).map(String::from),
            Expr::Var(n) | Expr::Index(n, _) => self.locals.get(n).cloned().or_else(|| self.globals.get(n.as_str()).map(|t| t.to_string())),
            Expr::Call(c) => self.returns.get(c.name.as_str()).copied().flatten().map(String::from),
            Expr::Code(_) => Some("code".into()),
            Expr::Unary(UnaryOp::Not, _) => Some("boolean".into()),
            Expr::Unary(_, x) => self.of(x),
            Expr::Binary(a, op, b) => match op {
                BinOp::Eq | BinOp::NotEq | BinOp::Gt | BinOp::Lt | BinOp::GtEq | BinOp::LtEq | BinOp::And | BinOp::Or => Some("boolean".into()),
                _ => {
                    let (x, y) = (self.of(a), self.of(b));
                    if x.as_deref() == Some("string") || y.as_deref() == Some("string") {
                        Some("string".into())
                    } else if x.as_deref() == Some("real") || y.as_deref() == Some("real") {
                        Some("real".into())
                    } else if x.as_deref() == Some("integer") && y.as_deref() == Some("integer") {
                        Some("integer".into())
                    } else {
                        x.or(y)
                    }
                }
            },
            Expr::Member(..) | Expr::Method(_) | Expr::Element(..) => None,
        }
    }
}

/// A literal's type, by how it's written.
pub(crate) fn literal_type(t: &str) -> Option<&'static str> {
    match t {
        "true" | "false" => Some("boolean"),
        "null" => None,
        _ if t.starts_with('"') => Some("string"),
        _ if t.starts_with('\'') || t.starts_with('$') || t.to_ascii_lowercase().starts_with("0x") => Some("integer"),
        _ if t.contains('.') => Some("real"),
        _ => Some("integer"),
    }
}

/// A type's empty value.
fn empty(ty: &str) -> &'static str {
    match ty {
        "integer" => "0",
        "real" => "0.0",
        "boolean" => "false",
        _ => "nil",
    }
}

/// The Lua of `m` (made the game's JASS: lowered, merged, sorted); `game`
/// the game's natives and functions, for what they give back.
pub fn print(m: &Module, game: &[Signature]) -> String {
    let mut types = Types { globals: HashMap::new(), returns: HashMap::new(), locals: HashMap::new() };
    for g in &m.globals {
        types.globals.insert(&g.name, &g.ty);
    }
    for s in game.iter().chain(m.natives.iter().map(|n| &n.sig)).chain(m.functions.iter().map(|f| &f.sig)) {
        types.returns.insert(&s.name, s.returns.as_deref());
    }
    let mut out = String::from(PRELUDE);
    out.push('\n');
    for g in &m.globals {
        let value = match (&g.init, g.array) {
            (_, true) => format!("wcb__array({})", empty(&g.ty)),
            (Some(e), false) => expr(e, 0, &types),
            (None, false) => empty(&g.ty).to_string(),
        };
        let _ = writeln!(out, "{} = {value}", g.name);
    }
    for f in &m.functions {
        types.locals = f.sig.params.iter().map(|p| (p.name.clone(), p.ty.clone())).collect();
        locals(&f.body, &mut types.locals);
        let params: Vec<&str> = f.sig.params.iter().map(|p| p.name.as_str()).collect();
        let _ = writeln!(out, "\nfunction {}({})", f.sig.name, params.join(", "));
        statements(&f.body, 1, &mut out, &types);
        out += "end\n";
    }
    out
}

pub(crate) fn locals(body: &[Stmt], out: &mut HashMap<String, String>) {
    for s in body {
        match s {
            Stmt::Local { ty, name, .. } => {
                out.insert(name.clone(), ty.clone());
            }
            Stmt::If { branches, otherwise, .. } => {
                branches.iter().for_each(|(_, b)| locals(b, out));
                if let Some(b) = otherwise {
                    locals(b, out);
                }
            }
            Stmt::Loop { body, .. } => locals(body, out),
            _ => {}
        }
    }
}

fn statements(body: &[Stmt], depth: usize, out: &mut String, t: &Types) {
    for s in body {
        statement(s, depth, out, t);
    }
}

fn call(c: &Call, t: &Types) -> String {
    format!("{}({})", c.name, c.args.iter().map(|a| expr(a, 0, t)).collect::<Vec<_>>().join(", "))
}

fn statement(s: &Stmt, depth: usize, out: &mut String, t: &Types) {
    let pad = "    ".repeat(depth);
    match s {
        Stmt::Local { ty, name, array, init, .. } => {
            let value = match (init, array) {
                (_, true) => Some(format!("wcb__array({})", empty(ty))),
                (Some(e), false) => Some(expr(e, 0, t)),
                (None, false) => None,
            };
            match value {
                Some(v) => writeln!(out, "{pad}local {name} = {v}"),
                None => writeln!(out, "{pad}local {name}"),
            }
            .ok();
        }
        Stmt::Set { name, index, value, .. } => {
            let index = index.as_ref().map_or(String::new(), |i| format!("[{}]", expr(i, 0, t)));
            let _ = writeln!(out, "{pad}{name}{index} = {}", expr(value, 0, t));
        }
        Stmt::Call { call: c, .. } => {
            let _ = writeln!(out, "{pad}{}", call(c, t));
        }
        Stmt::If { branches, otherwise, .. } => {
            for (i, (c, b)) in branches.iter().enumerate() {
                let word = if i == 0 { "if" } else { "elseif" };
                let _ = writeln!(out, "{pad}{word} {} then", expr(c, 0, t));
                statements(b, depth + 1, out, t);
            }
            if let Some(b) = otherwise {
                let _ = writeln!(out, "{pad}else");
                statements(b, depth + 1, out, t);
            }
            let _ = writeln!(out, "{pad}end");
        }
        Stmt::Loop { body, .. } => {
            let _ = writeln!(out, "{pad}while true do");
            statements(body, depth + 1, out, t);
            let _ = writeln!(out, "{pad}end");
        }
        Stmt::ExitWhen { cond, .. } => {
            let _ = writeln!(out, "{pad}if {} then break end", expr(cond, 0, t));
        }
        Stmt::Return { value, .. } => match value {
            Some(v) => {
                let _ = writeln!(out, "{pad}do return {} end", expr(v, 0, t));
            }
            None => {
                let _ = writeln!(out, "{pad}do return end");
            }
        },
        // As the game when it isn't debugging.
        Stmt::Debug { .. } => {}
        Stmt::SetMember { .. } | Stmt::SetElement { .. } | Stmt::CallMethod { .. } | Stmt::StaticIf { .. } => unreachable!("not lowered: build checks Module::not_lowered"),
    }
}

/// How tightly an operator binds in Lua.
fn precedence(op: BinOp, concat: bool) -> u8 {
    match op {
        BinOp::Or => 1,
        BinOp::And => 2,
        BinOp::Eq | BinOp::NotEq | BinOp::Gt | BinOp::Lt | BinOp::GtEq | BinOp::LtEq => 3,
        BinOp::Add if concat => 4,
        BinOp::Add | BinOp::Sub => 5,
        BinOp::Mul | BinOp::Div | BinOp::Mod => 6,
    }
}

const UNARY: u8 = 7;

/// A literal as Lua writes it.
fn literal(t: &str) -> String {
    if let Some(code) = t.strip_prefix('\'').and_then(|r| r.strip_suffix('\'')) {
        return code.bytes().fold(0i64, |acc, b| acc * 256 + b as i64).to_string();
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
        return s;
    }
    if t == "null" { "nil".into() } else { t.to_string() }
}

/// `e`, in parentheses when it binds looser than `min`.
fn expr(e: &Expr, min: u8, t: &Types) -> String {
    let (text, p) = match e {
        Expr::Literal(l) => (literal(l), 8),
        Expr::Var(n) => (n.clone(), 8),
        Expr::Index(n, i) => (format!("{n}[{}]", expr(i, 0, t)), 8),
        Expr::Call(c) => (call(c, t), 8),
        Expr::Code(n) => (n.clone(), 8),
        Expr::Member(..) | Expr::Method(_) | Expr::Element(..) => unreachable!("not lowered: build checks Module::not_lowered"),
        Expr::Unary(op, x) => {
            let inner = expr(x, UNARY, t);
            let text = match op {
                UnaryOp::Not => format!("not {inner}"),
                UnaryOp::Neg if inner.starts_with('-') => format!("- {inner}"),
                UnaryOp::Neg => format!("-{inner}"),
                UnaryOp::Plus => inner,
            };
            (text, UNARY)
        }
        Expr::Binary(a, op, b) => {
            let (ta, tb) = (t.of(a), t.of(b));
            let strings = *op == BinOp::Add && (ta.as_deref() == Some("string") || tb.as_deref() == Some("string"));
            let integers = ta.as_deref() == Some("integer") && tb.as_deref() == Some("integer");
            if *op == BinOp::Div && integers {
                (format!("wcb__div({}, {})", expr(a, 0, t), expr(b, 0, t)), 8)
            } else if strings {
                // A null string is an empty one, as JASS adds them.
                let side = |x: &Expr| match x {
                    Expr::Literal(l) if l.starts_with('"') => literal(l),
                    _ => format!("({} or \"\")", expr(x, 0, t)),
                };
                (format!("{} .. {}", side(a), side(b)), 4)
            } else {
                let p = precedence(*op, false);
                let sign = match op {
                    BinOp::Add => "+",
                    BinOp::Sub => "-",
                    BinOp::Mul => "*",
                    BinOp::Div => "/",
                    BinOp::Eq => "==",
                    BinOp::NotEq => "~=",
                    BinOp::Gt => ">",
                    BinOp::Lt => "<",
                    BinOp::GtEq => ">=",
                    BinOp::LtEq => "<=",
                    BinOp::And => "and",
                    BinOp::Or => "or",
                    BinOp::Mod => unreachable!("not lowered: build checks Module::not_lowered"),
                };
                (format!("{} {sign} {}", expr(a, p, t), expr(b, p + 1, t)), p)
            }
        }
    };
    if p < min { format!("({text})") } else { text }
}

#[cfg(test)]
#[path = "lua.test.rs"]
mod tests;
