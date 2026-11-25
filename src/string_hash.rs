//! `StringHash` worked out as the map is built (`[build] string_hash=1`,
//! [`crate::jass::Options::string_hash`]): `StringHash("text")` of a
//! string literal is the number the game gives, so the game hashes nothing
//! as it runs.
//!
//! The game's native is Storm's `SStrHash2` (as pjass has it from the
//! game, `../pjass-master/sstrhash.c`): Bob Jenkins' lookup2 hash (the
//! 1997 Dr Dobb's article), seed 0, of the text with `a`-`z` made `A`-`Z`
//! and `/` made `\` (`StringHash("ab") == StringHash("AB")`,
//! `StringHash("/") == StringHash("\\")`), into a buffer of 1024 bytes.
//! How the game hashes what isn't ASCII changed (1.30 and 1.31,
//! Reforged), so only a literal all ASCII and shorter than that buffer is
//! worked out; any other, and a `StringHash` of what isn't a literal, is
//! left to the game.

use crate::ir::*;

/// lookup2's mix of three words.
fn mix(a: &mut u32, b: &mut u32, c: &mut u32) {
    *a = a.wrapping_sub(*b).wrapping_sub(*c) ^ (*c >> 13);
    *b = b.wrapping_sub(*c).wrapping_sub(*a) ^ (*a << 8);
    *c = c.wrapping_sub(*a).wrapping_sub(*b) ^ (*b >> 13);
    *a = a.wrapping_sub(*b).wrapping_sub(*c) ^ (*c >> 12);
    *b = b.wrapping_sub(*c).wrapping_sub(*a) ^ (*a << 16);
    *c = c.wrapping_sub(*a).wrapping_sub(*b) ^ (*b >> 5);
    *a = a.wrapping_sub(*b).wrapping_sub(*c) ^ (*c >> 3);
    *b = b.wrapping_sub(*c).wrapping_sub(*a) ^ (*a << 10);
    *c = c.wrapping_sub(*a).wrapping_sub(*b) ^ (*b >> 15);
}

/// Bob Jenkins' lookup2 `hash(k, length, initval)`.
pub fn lookup2(k: &[u8], initval: u32) -> u32 {
    let word = |b: &[u8]| u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
    let (mut a, mut b, mut c) = (0x9e37_79b9u32, 0x9e37_79b9u32, initval);
    let mut rest = k;
    while rest.len() >= 12 {
        a = a.wrapping_add(word(&rest[0..4]));
        b = b.wrapping_add(word(&rest[4..8]));
        c = c.wrapping_add(word(&rest[8..12]));
        mix(&mut a, &mut b, &mut c);
        rest = &rest[12..];
    }
    // The last 11 bytes at most: c's lowest byte is the length's.
    c = c.wrapping_add(k.len() as u32);
    for (i, &byte) in rest.iter().enumerate() {
        let v = byte as u32;
        match i {
            0..=3 => a = a.wrapping_add(v << (8 * i)),
            4..=7 => b = b.wrapping_add(v << (8 * (i - 4))),
            _ => c = c.wrapping_add(v << (8 * (i - 7))),
        }
    }
    mix(&mut a, &mut b, &mut c);
    c
}

/// What the game's `StringHash` gives for `text` (ASCII).
pub fn string_hash(text: &[u8]) -> i32 {
    let normal: Vec<u8> = text
        .iter()
        .map(|&b| match b {
            b'a'..=b'z' => b - 0x20,
            b'/' => b'\\',
            b => b,
        })
        .collect();
    lookup2(&normal, 0) as i32
}

/// The text of a JASS string literal (`"a\"b"`, its quotes and escapes
/// read), when it's all ASCII.
fn ascii_text(literal: &str) -> Option<Vec<u8>> {
    let inner = literal.strip_prefix('"')?.strip_suffix('"')?;
    if !inner.is_ascii() {
        return None;
    }
    let mut out = Vec::new();
    let mut chars = inner.bytes();
    while let Some(c) = chars.next() {
        if c != b'\\' {
            out.push(c);
            continue;
        }
        out.push(match chars.next()? {
            b'n' => b'\n',
            b't' => b'\t',
            b'r' => b'\r',
            b'b' => 8,
            b'f' => 12,
            b'"' => b'"',
            b'\\' => b'\\',
            _ => return None,
        });
    }
    Some(out)
}

/// The value of `StringHash(args)`, when it's a literal's.
fn hashed(args: &[Expr]) -> Option<Expr> {
    let [Expr::Literal(l)] = args else { return None };
    let text = ascii_text(l)?;
    // SStrHash2's buffer: 0x400 bytes.
    if text.len() >= 0x400 {
        return None;
    }
    let n = string_hash(&text);
    Some(if n < 0 {
        // The smallest integer is written as JASS can: -2147483647 - 1.
        match n.checked_neg() {
            Some(p) => Expr::Unary(UnaryOp::Neg, Box::new(Expr::Literal(p.to_string()))),
            None => Expr::Binary(
                Box::new(Expr::Unary(UnaryOp::Neg, Box::new(Expr::Literal("2147483647".into())))),
                BinOp::Sub,
                Box::new(Expr::Literal("1".into())),
            ),
        }
    } else {
        Expr::Literal(n.to_string())
    })
}

/// Every `StringHash` of a literal in `e` its number: how many.
fn in_expr(e: &mut Expr) -> usize {
    let mut n = 0;
    match e {
        Expr::Call(c) => {
            n += c.args.iter_mut().map(in_expr).sum::<usize>();
            if c.name == "StringHash"
                && let Some(v) = hashed(&c.args)
            {
                *e = v;
                return n + 1;
            }
        }
        Expr::Index(_, i) | Expr::Unary(_, i) | Expr::Member(i, _) => n += in_expr(i),
        Expr::Binary(a, _, b) | Expr::Element(a, b) => n += in_expr(a) + in_expr(b),
        Expr::Method(m) => n += in_expr(&mut m.object) + m.args.iter_mut().map(in_expr).sum::<usize>(),
        Expr::Literal(_) | Expr::Var(_) | Expr::Code(_) => {}
    }
    n
}

fn in_body(body: &mut [Stmt]) -> usize {
    let mut n = 0;
    for s in body {
        n += match s {
            Stmt::Local { init, .. } => init.iter_mut().map(in_expr).sum(),
            Stmt::Set { index, value, .. } => index.iter_mut().map(in_expr).sum::<usize>() + in_expr(value),
            // A statement `call StringHash("x")` does nothing either way.
            Stmt::Call { call, .. } => call.args.iter_mut().map(in_expr).sum(),
            Stmt::If { branches, otherwise, .. } => {
                branches.iter_mut().map(|(c, b)| in_expr(c) + in_body(b)).sum::<usize>() + otherwise.as_deref_mut().map_or(0, in_body)
            }
            Stmt::Loop { body, .. } => in_body(body),
            Stmt::ExitWhen { cond, .. } => in_expr(cond),
            Stmt::Return { value, .. } => value.iter_mut().map(in_expr).sum(),
            Stmt::Debug { stmt, .. } => in_body(std::slice::from_mut(&mut **stmt)),
            Stmt::SetMember { object, index, value, .. } => in_expr(object) + index.iter_mut().map(in_expr).sum::<usize>() + in_expr(value),
            Stmt::SetElement { target, index, value, .. } => in_expr(target) + in_expr(index) + in_expr(value),
            Stmt::CallMethod { call, .. } => in_expr(&mut call.object) + call.args.iter_mut().map(in_expr).sum::<usize>(),
            Stmt::StaticIf { cond, then, otherwise, .. } => in_expr(cond) + in_body(then) + otherwise.as_deref_mut().map_or(0, in_body),
        };
    }
    n
}

/// `m`'s `StringHash`es of literals worked out ([`self`]): how many. None
/// when the code has a function of that name of its own (it isn't the
/// game's then).
pub fn hash_strings(m: &mut Module) -> usize {
    if m.functions.iter().any(|f| f.sig.name == "StringHash") || m.structs.iter().any(|s| s.name == "StringHash") {
        return 0;
    }
    let mut n = 0;
    for g in &mut m.globals {
        n += g.init.iter_mut().map(in_expr).sum::<usize>();
    }
    for f in &mut m.functions {
        n += in_body(&mut f.body);
    }
    // A class's, where structs are still classes (AngelScript).
    for st in &mut m.structs {
        for f in &mut st.fields {
            n += f.init.iter_mut().map(in_expr).sum::<usize>();
        }
        for me in &mut st.methods {
            n += in_body(&mut me.function.body);
        }
    }
    n
}

#[cfg(test)]
#[path = "string_hash.test.rs"]
mod tests;
