//! JassHelper's preprocessor, on the text of all the code at once (a
//! textmacro declared in one file is run in another), before any of it is
//! read:
//!
//! - `//! import [vjass|zinc|comment] "path"`: the file named, once
//!   however many times it's imported, as a file of its own (its order
//!   doesn't matter: the build sorts what's declared); `zinc` read as Zinc
//!   (a `.zn` name), `comment` not read at all. Where the path leads is
//!   the caller's ([`Import`]).
//! - `//! textmacro NAME [takes a, b]` ... `//! endtextmacro`: a macro, its
//!   lines left out where they're written; `textmacro_once`: one that may
//!   be declared again (the first is kept). Two of a name are an error.
//! - `//! runtextmacro [optional] NAME("x", "y")`: the macro's lines in
//!   its place, each `$a$` the argument given for `a`, its own
//!   `runtextmacro`s run too; `optional`: nothing when there's no such
//!   macro.
//! - `//! novjass` ... `//! endnovjass`: left out.
//!
//! Each line made keeps where it comes from: a macro's, the line of the
//! `runtextmacro` (the outermost) that put it there; so what's said of
//! the code is said of the lines written.

use std::collections::HashMap;

use crate::ir::{Error, Origin};

/// Where `//! import "path"` in the file `from` leads: the file's name
/// (what its code is said to come from) and text; `None` when there's
/// no such file.
pub type Import<'a> = dyn Fn(&str, &str) -> Option<(String, String)> + 'a;

/// A file of code, its macros run.
#[derive(Debug, Clone, PartialEq)]
pub struct Expanded {
    pub name: String,
    pub text: String,
    /// Each line's line in the file as written (from 1).
    pub lines: Vec<usize>,
    /// The index of the file it was imported by (`None`: given, not
    /// imported), for its dialect.
    pub imported_by: Option<usize>,
}

/// A macro.
struct Macro {
    params: Vec<String>,
    /// Its lines, without their line breaks.
    body: Vec<String>,
    once: bool,
    origin: Origin,
}

/// How deep `runtextmacro`s may run in each other.
const DEPTH: usize = 64;

/// What follows `//!` on a directive line, trimmed; `None` for any other
/// line.
fn directive(line: &str) -> Option<&str> {
    line.trim_start().strip_prefix("//!").map(str::trim)
}

/// The quoted strings of an argument list `"a", "b"`, as written between
/// their quotes (JassHelper takes them as they are).
fn strings(text: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    let mut rest = text.trim();
    while !rest.is_empty() {
        let Some(r) = rest.strip_prefix('"') else { return Err(format!("expected a quoted argument at `{rest}`")) };
        let Some(end) = r.find('"') else { return Err("an argument's quote isn't closed".into()) };
        out.push(r[..end].to_string());
        rest = r[end + 1..].trim_start();
        if let Some(r) = rest.strip_prefix(',') {
            rest = r.trim_start();
            if rest.is_empty() {
                return Err("an argument expected after `,`".into());
            }
        } else if !rest.is_empty() {
            return Err(format!("expected `,` at `{rest}`"));
        }
    }
    Ok(out)
}

/// The function an `//! inject main` (`config`) is made: `wcb__inject_main`.
pub const INJECT: &str = "wcb__inject_";

/// What an `//! dovjassinit` is made: a call of it, which lowering puts
/// vJASS's initialization in place of.
pub const DOVJASSINIT: &str = "wcb__dovjassinit";

/// The files `files` (each a name and its text), their imports with them
/// (after the file importing them), their macros run.
pub fn expand(files: &[(&str, &str)], import: &Import) -> Result<Vec<Expanded>, Vec<Error>> {
    let mut errors = Vec::new();
    let error = |file: &str, line: usize, message: String| Error { origin: Origin { file: file.into(), line }, message };

    // The files and what they import, each once.
    let mut all: Vec<(String, String, Option<usize>)> = files.iter().map(|(n, t)| (n.to_string(), t.to_string(), None)).collect();
    let mut seen: std::collections::HashSet<String> = all.iter().map(|(n, _, _)| n.clone()).collect();
    let mut k = 0;
    while k < all.len() {
        let (name, text) = (all[k].0.clone(), all[k].1.clone());
        for (i, line) in text.lines().enumerate() {
            let Some(rest) = directive(line).and_then(|d| d.strip_prefix("import")).filter(|r| r.is_empty() || r.starts_with(char::is_whitespace)) else { continue };
            let rest = rest.trim();
            let (kind, path) = match rest.split_once(char::is_whitespace) {
                Some((k, p)) if ["vjass", "zinc", "comment"].contains(&k) => (k, p.trim()),
                _ => ("vjass", rest),
            };
            let Some(path) = path.strip_prefix('"').and_then(|p| p.strip_suffix('"')) else {
                errors.push(error(&name, i + 1, format!("`//! import` needs a quoted path: {rest}")));
                continue;
            };
            if kind == "comment" {
                continue;
            }
            match import(&name, path) {
                Some((found, text)) => {
                    // Zinc's by its name: a file read as Zinc ends `.zn`.
                    let found = if kind == "zinc" && !found.to_ascii_lowercase().ends_with(".zn") { format!("{found}.zn") } else { found };
                    if seen.insert(found.clone()) {
                        all.push((found, text, Some(k)));
                    }
                }
                None => errors.push(error(&name, i + 1, format!("`//! import`: no file \"{path}\""))),
            }
        }
        k += 1;
    }

    // The macros, and each file without them (and without novjass).
    let mut macros: HashMap<String, Macro> = HashMap::new();
    let mut loaded = 0;
    let mut kept: Vec<Vec<(String, usize)>> = Vec::new();
    for (name, text, _) in &all {
        let mut own = Vec::new();
        // An `//! inject` open: what it replaces, and its line.
        let mut injecting: Option<(String, usize)> = None;
        let mut lines = text.lines().enumerate();
        while let Some((i, line)) = lines.next() {
            let Some(d) = directive(line) else {
                own.push((line.to_string(), i + 1));
                continue;
            };
            let word = d.split_whitespace().next().unwrap_or("");
            match word {
                "textmacro" | "textmacro_once" => {
                    let head = d[word.len()..].trim();
                    let (mname, params) = match head.split_once(char::is_whitespace) {
                        Some((n, rest)) => {
                            let rest = rest.trim();
                            match rest.strip_prefix("takes") {
                                Some(p) => (n, p.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect::<Vec<_>>()),
                                None => {
                                    errors.push(error(name, i + 1, format!("`//! textmacro {n}`: `takes` expected before `{rest}`")));
                                    (n, Vec::new())
                                }
                            }
                        }
                        None => (head, Vec::new()),
                    };
                    let mut body = Vec::new();
                    let mut closed = false;
                    for (_, l) in lines.by_ref() {
                        if directive(l).is_some_and(|d| d == "endtextmacro") {
                            closed = true;
                            break;
                        }
                        body.push(l.to_string());
                    }
                    if !closed {
                        errors.push(error(name, i + 1, format!("`//! textmacro {mname}` has no `//! endtextmacro`")));
                    }
                    if mname.is_empty() {
                        errors.push(error(name, i + 1, "`//! textmacro` needs a name".into()));
                        continue;
                    }
                    let once = word == "textmacro_once";
                    match macros.get(mname) {
                        Some(m) if once && m.once => {}
                        Some(m) => errors.push(error(name, i + 1, format!("textmacro {mname} is declared already ({}:{})", m.origin.file, m.origin.line))),
                        None => {
                            macros.insert(mname.to_string(), Macro { params, body, once, origin: Origin { file: name.clone(), line: i + 1 } });
                        }
                    }
                }
                "endtextmacro" => errors.push(error(name, i + 1, "`//! endtextmacro` without its `//! textmacro`".into())),
                "novjass" => {
                    let mut closed = false;
                    for (_, l) in lines.by_ref() {
                        if directive(l).is_some_and(|d| d == "endnovjass") {
                            closed = true;
                            break;
                        }
                    }
                    if !closed {
                        errors.push(error(name, i + 1, "`//! novjass` has no `//! endnovjass`".into()));
                    }
                }
                // `//! inject main` ... `//! endinject`: a function of its own,
                // line for line, which replaces the map's `main` (`config`)
                // when the code is lowered (`lower`); `//! dovjassinit` in
                // it, where vJASS's initialization goes.
                "inject" => {
                    let what = d[word.len()..].trim();
                    if !matches!(what, "main" | "config") {
                        errors.push(error(name, i + 1, format!("`//! inject {what}`: main or config")));
                    } else if let Some((_, at)) = &injecting {
                        errors.push(error(name, i + 1, format!("`//! inject` in the `//! inject` of line {at}")));
                    } else {
                        own.push((format!("function {INJECT}{what} takes nothing returns nothing"), i + 1));
                        injecting = Some((what.to_string(), i + 1));
                    }
                }
                "endinject" => match injecting.take() {
                    Some(_) => own.push(("endfunction".to_string(), i + 1)),
                    None => errors.push(error(name, i + 1, "`//! endinject` without its `//! inject`".into())),
                },
                "dovjassinit" => match &injecting {
                    Some((what, _)) if what == "main" => own.push((format!("call {DOVJASSINIT}()"), i + 1)),
                    _ => errors.push(error(name, i + 1, "`//! dovjassinit` is for an `//! inject main` only".into())),
                },
                // `//! loaddata "path.slk"`: the table made a function setting
                // its structs (`wcb__loaddata_<n>`), all on this line, which
                // lowering runs after vJASS's initialization.
                "loaddata" => {
                    let rest = d[word.len()..].trim();
                    let Some(path) = rest.strip_prefix('"').and_then(|p| p.strip_suffix('"')) else {
                        errors.push(error(name, i + 1, format!("`//! loaddata` needs a quoted path: {rest}")));
                        continue;
                    };
                    let Some((_, slk)) = import(name, path) else {
                        errors.push(error(name, i + 1, format!("`//! loaddata`: no file \"{path}\"")));
                        continue;
                    };
                    match load_data(&slk, loaded) {
                        Ok(lines) => {
                            loaded += 1;
                            own.extend(lines.into_iter().map(|l| (l, i + 1)));
                        }
                        Err(e) => errors.push(error(name, i + 1, format!("`//! loaddata \"{path}\"`: {e}"))),
                    }
                }
                // Read already.
                "import" => {}
                _ => own.push((line.to_string(), i + 1)),
            }
        }
        if let Some((what, at)) = injecting {
            errors.push(error(name, at, format!("`//! inject {what}` has no `//! endinject`")));
        }
        kept.push(own);
    }

    // The macros run, in every file.
    let mut out = Vec::new();
    for ((name, _, by), own) in all.iter().zip(kept) {
        let mut text = String::new();
        let mut lines = Vec::new();
        for (line, at) in own {
            run(&line, at, name, &macros, 0, &mut text, &mut lines, &mut errors);
        }
        out.push(Expanded { name: name.clone(), text, lines, imported_by: *by });
    }
    if errors.is_empty() { Ok(out) } else { Err(errors) }
}

/// What a `//! loaddata` is made: `wcb__loaddata_<n>`, run after vJASS's
/// initialization.
pub const LOADDATA: &str = "wcb__loaddata_";

/// The function JassHelper makes of an SLK table (`//! loaddata`, its
/// `n`th): the struct named at (1, 1), the fields its first row names, and
/// for each row after, `set s = S.getFromKey(<key>)` (the key in its first
/// column) and `set s.<field> = <value>` for each value it has. Numbers
/// and `true`/`false` as written, text as a JASS string.
fn load_data(slk: &str, n: usize) -> Result<Vec<String>, String> {
    let doc = slk_rs::parse(slk);
    let table = slk_rs::Table::new(&doc);
    let literal = |row: u32, col: u32| -> Option<String> {
        match &table.get(row, col)?.value {
            slk_rs::ast::Value::Number(v) => Some(v.clone()),
            slk_rs::ast::Value::Bool(b) => Some(b.to_string()),
            slk_rs::ast::Value::Str(t) | slk_rs::ast::Value::Text(t) => Some(format!("\"{}\"", t.replace('\\', "\\\\").replace('"', "\\\""))),
            _ => None,
        }
    };
    let ident = |t: &str| !t.is_empty() && t.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) && t.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    let st = table.text(1, 1);
    if !ident(&st) {
        return Err(format!("its cell (1, 1) names no struct: {st:?}"));
    }
    let fields: Vec<(u32, String)> = (2..=table.cols()).map(|c| (c, table.text(1, c))).filter(|(_, f)| !f.is_empty()).collect();
    if let Some((c, f)) = fields.iter().find(|(_, f)| !ident(f)) {
        return Err(format!("its column {c} names no field: {f:?}"));
    }
    let s = format!("{LOADDATA}s");
    let mut out = vec![format!("function {LOADDATA}{n} takes nothing returns nothing"), format!("local {st} {s}")];
    for row in 2..=table.rows() {
        let Some(key) = literal(row, 1) else { continue };
        out.push(format!("set {s} = {st}.getFromKey({key})"));
        for (col, field) in &fields {
            if let Some(v) = literal(row, *col) {
                out.push(format!("set {s}.{field} = {v}"));
            }
        }
    }
    out.push("endfunction".into());
    Ok(out)
}

/// `line` (the file's `at`), its `runtextmacro` run, into `text`.
#[allow(clippy::too_many_arguments)]
fn run(line: &str, at: usize, file: &str, macros: &HashMap<String, Macro>, depth: usize, text: &mut String, lines: &mut Vec<usize>, errors: &mut Vec<Error>) {
    let error = |message: String| Error { origin: Origin { file: file.into(), line: at }, message };
    let Some(rest) = directive(line).and_then(|d| d.strip_prefix("runtextmacro")).filter(|r| r.is_empty() || r.starts_with(char::is_whitespace)) else {
        text.push_str(line);
        text.push('\n');
        lines.push(at);
        return;
    };
    let rest = rest.trim();
    let (optional, rest) = match rest.strip_prefix("optional") {
        Some(r) if r.starts_with(char::is_whitespace) => (true, r.trim()),
        _ => (false, rest),
    };
    let Some((name, args)) = rest.split_once('(') else {
        errors.push(error(format!("`//! runtextmacro {rest}`: `(` expected")));
        return;
    };
    let name = name.trim();
    let Some(args) = args.trim_end().strip_suffix(')') else {
        errors.push(error(format!("`//! runtextmacro {name}`: `)` expected at the end")));
        return;
    };
    let Some(m) = macros.get(name) else {
        if !optional {
            errors.push(error(format!("no textmacro {name}")));
        }
        return;
    };
    let args = match strings(args) {
        Ok(a) => a,
        Err(e) => {
            errors.push(error(format!("`//! runtextmacro {name}`: {e}")));
            return;
        }
    };
    if args.len() != m.params.len() {
        errors.push(error(format!("textmacro {name} takes {} argument(s), given {}", m.params.len(), args.len())));
        return;
    }
    if depth >= DEPTH {
        errors.push(error(format!("textmacro {name} runs itself (or textmacros run each other) past {DEPTH} deep")));
        return;
    }
    for body in &m.body {
        let mut l = body.clone();
        for (p, a) in m.params.iter().zip(&args) {
            l = l.replace(&format!("${p}$"), a);
        }
        run(&l, at, file, macros, depth + 1, text, lines, errors);
    }
}

#[cfg(test)]
#[path = "preprocess.test.rs"]
mod tests;
