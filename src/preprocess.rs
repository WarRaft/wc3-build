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
//! - `//! external NAME args`, `//! externalblock [extension=x] NAME args`
//!   ... `//! i line` ... `//! endexternalblock`: a tool to run on the map
//!   built ([`crate::ir::External`]), the block's `//! i` lines its input
//!   (the rest of the block left out); the caller runs them.
//!
//! Each line made keeps where it comes from: a macro's, the line of the
//! `runtextmacro` (the outermost) that put it there; so what's said of
//! the code is said of the lines written.

use std::collections::HashMap;

use parser_rs::expand::{Expansion, Line, Mapped, Spelling};

use crate::ir::{Error, External, Origin};

/// Where `//! import "path"` in the file `from` leads: the file's name
/// (what its code is said to come from) and text; `None` when there's
/// no such file.
pub type Import<'a> = dyn Fn(&str, &str) -> Option<(String, String)> + 'a;

/// A file of code, its macros run.
#[derive(Debug, Clone, PartialEq)]
pub struct Expanded {
    pub name: String,
    pub text: String,
    /// Each line's line in the file as written (from 1): a macro's, the
    /// line of the outermost `runtextmacro` that put it there.
    pub lines: Vec<usize>,
    /// Where each piece of `text` is written and the expansions that made
    /// it (`parser_rs::expand`).
    pub map: Mapped,
    /// The index of the file it was imported by (`None`: given, not
    /// imported), for its dialect.
    pub imported_by: Option<usize>,
    /// Its `//! external`s, in order.
    pub externals: Vec<External>,
}

/// An `//! external`'s or `//! externalblock`'s head (`[extension=x] NAME
/// args`): the extension, the tool, its arguments.
fn external_head(head: &str) -> Option<(Option<String>, String, String)> {
    let mut rest = head.trim();
    let mut extension = None;
    if let Some(r) = rest.strip_prefix("extension=") {
        let (ext, after) = r.split_once(char::is_whitespace).unwrap_or((r, ""));
        extension = Some(ext.trim_start_matches('.').to_string());
        rest = after.trim();
    }
    let (tool, args) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
    (!tool.is_empty()).then(|| (extension, tool.to_string(), args.trim().to_string()))
}

/// A macro.
struct Macro {
    params: Vec<String>,
    /// Its lines, without their line breaks, each where it's written.
    body: Vec<Line>,
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
/// their quotes (JassHelper takes them as they are), each with the byte it
/// starts at in `text`.
fn strings(text: &str) -> Result<Vec<(String, usize)>, String> {
    let mut out = Vec::new();
    let mut rest = text.trim();
    while !rest.is_empty() {
        let Some(r) = rest.strip_prefix('"') else { return Err(format!("expected a quoted argument at `{rest}`")) };
        let Some(end) = r.find('"') else { return Err("an argument's quote isn't closed".into()) };
        out.push((r[..end].to_string(), text.len() - r.len()));
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
    let (out, errors) = expand_lenient(files, import);
    if errors.is_empty() { Ok(out) } else { Err(errors) }
}

/// [`expand`] that goes on past what's wrong: what it could make, and
/// what's wrong (the editor reads code being written through it).
pub fn expand_lenient(files: &[(&str, &str)], import: &Import) -> (Vec<Expanded>, Vec<Error>) {
    let mut errors = Vec::new();
    let error = |file: &str, line: usize, message: String| Error { origin: Origin { file: file.into(), line }, message };

    // The files and what they import, each once.
    let mut all: Vec<(String, String, Option<usize>)> = files.iter().map(|(n, t)| (n.to_string(), t.to_string(), None)).collect();
    let mut seen: std::collections::HashSet<String> = all.iter().map(|(n, _, _)| n.clone()).collect();
    let mut k = 0;
    while k < all.len() {
        let (name, text) = (all[k].0.clone(), all[k].1.clone());
        for (i, line) in parser_rs::lines::lines(&text).enumerate() {
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
    let mut kept: Vec<Vec<Line>> = Vec::new();
    let mut externals: Vec<Vec<External>> = Vec::new();
    for (name, text, _) in &all {
        let mut own = Vec::new();
        let mut tools = Vec::new();
        // An `//! inject` open: what it replaces, and its line.
        let mut injecting: Option<(String, usize)> = None;
        // A line of the file as written, or one made at the directive of
        // line `at` (said to be written there).
        let written = |text: &str, at: usize| Line::written(text, Spelling::new(name, at, 1), None);
        let mut lines = parser_rs::lines::lines(&text).enumerate();
        while let Some((i, line)) = lines.next() {
            let Some(d) = directive(line) else {
                own.push(written(line, i + 1));
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
                    for (j, l) in lines.by_ref() {
                        if directive(l).is_some_and(|d| d == "endtextmacro") {
                            closed = true;
                            break;
                        }
                        body.push(written(l, j + 1));
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
                        own.push(written(&format!("function {INJECT}{what} takes nothing returns nothing"), i + 1));
                        injecting = Some((what.to_string(), i + 1));
                    }
                }
                "endinject" => match injecting.take() {
                    Some(_) => own.push(written("endfunction", i + 1)),
                    None => errors.push(error(name, i + 1, "`//! endinject` without its `//! inject`".into())),
                },
                "dovjassinit" => match &injecting {
                    Some((what, _)) if what == "main" => own.push(written(&format!("call {DOVJASSINIT}()"), i + 1)),
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
                            own.extend(lines.into_iter().map(|l| written(&l, i + 1)));
                        }
                        Err(e) => errors.push(error(name, i + 1, format!("`//! loaddata \"{path}\"`: {e}"))),
                    }
                }
                // A tool run on the map built: the caller's.
                "external" => match external_head(&d[word.len()..]) {
                    Some((None, tool, args)) => tools.push(External { tool, args, input: None, extension: None, origin: Origin { file: name.clone(), line: i + 1 } }),
                    Some((Some(_), _, _)) => errors.push(error(name, i + 1, "`extension=` is an `//! externalblock`'s".into())),
                    None => errors.push(error(name, i + 1, "`//! external` needs a tool's name".into())),
                },
                "externalblock" => {
                    let head = external_head(&d[word.len()..]);
                    let mut input = String::new();
                    let mut closed = false;
                    for (_, l) in lines.by_ref() {
                        match directive(l) {
                            Some("endexternalblock") => {
                                closed = true;
                                break;
                            }
                            // `//! i text`: the `i` and one space after it off.
                            Some(d) if d == "i" || d.starts_with("i ") => {
                                let after = l.trim_start().strip_prefix("//!").unwrap_or("").trim_start();
                                let text = after.strip_prefix('i').unwrap_or("");
                                input.push_str(text.strip_prefix(' ').unwrap_or(text));
                                input.push('\n');
                            }
                            _ => {}
                        }
                    }
                    if !closed {
                        errors.push(error(name, i + 1, "`//! externalblock` has no `//! endexternalblock`".into()));
                    }
                    match head {
                        Some((extension, tool, args)) => tools.push(External { tool, args, input: Some(input), extension, origin: Origin { file: name.clone(), line: i + 1 } }),
                        None => errors.push(error(name, i + 1, "`//! externalblock` needs a tool's name".into())),
                    }
                }
                "endexternalblock" => errors.push(error(name, i + 1, "`//! endexternalblock` without its `//! externalblock`".into())),
                // Read already.
                "import" => {}
                _ => own.push(written(line, i + 1)),
            }
        }
        if let Some((what, at)) = injecting {
            errors.push(error(name, at, format!("`//! inject {what}` has no `//! endinject`")));
        }
        kept.push(own);
        externals.push(tools);
    }

    // The macros run, in every file.
    let mut out = Vec::new();
    for (((name, _, by), own), externals) in all.iter().zip(kept).zip(externals) {
        let mut map = Mapped::default();
        for line in own {
            let at = line.pieces.first().map_or(0, |(_, _, s, _)| s.line);
            run(&line, at, name, &macros, None, &mut map, &mut errors);
        }
        let lines = (1..=map.lines).map(|l| map.written_line(l).map_or(0, |s| s.line)).collect();
        out.push(Expanded { name: name.clone(), text: map.text.clone(), lines, map, imported_by: *by, externals });
    }
    (out, errors)
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

/// `line` (the file's `at`: what errors are said at), its `runtextmacro`
/// run, into `map`; `parent` the expansion it's in.
fn run(line: &Line, at: usize, file: &str, macros: &HashMap<String, Macro>, parent: Option<usize>, map: &mut Mapped, errors: &mut Vec<Error>) {
    let error = |message: String| Error { origin: Origin { file: file.into(), line: at }, message };
    let text = line.text.as_str();
    let Some(rest) = directive(text).and_then(|d| d.strip_prefix("runtextmacro")).filter(|r| r.is_empty() || r.starts_with(char::is_whitespace)) else {
        map.push_line(line);
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
    let given = match strings(args) {
        Ok(a) => a,
        Err(e) => {
            errors.push(error(format!("`//! runtextmacro {name}`: {e}")));
            return;
        }
    };
    if given.len() != m.params.len() {
        errors.push(error(format!("textmacro {name} takes {} argument(s), given {}", m.params.len(), given.len())));
        return;
    }
    if map.chain(parent).len() >= DEPTH {
        errors.push(error(format!("textmacro {name} runs itself (or textmacros run each other) past {DEPTH} deep")));
        return;
    }
    // Columns on `line`: the byte `b` of `text` is its character `col(b)`.
    let col = |b: usize| text[..b].chars().count() + 1;
    let args_at = text.len() - text[text.find('(').map_or(0, |p| p + 1)..].len();
    let name_at = text.find(name).unwrap_or(0);
    let used = line.origin(col(name_at)).map_or(Spelling::new(file, at, 1), |(s, _)| s);
    let defined = Spelling::new(&m.origin.file, m.origin.line, 1);
    let me = map.expansion(Expansion { name: name.to_string(), defined, used, parent });
    // Each argument as written where it's given.
    let args: Vec<Line> = given
        .iter()
        .map(|(a, start)| {
            let from = col(args_at + start);
            let mut l = Line::default();
            l.push_part(line, from, from + a.chars().count());
            l
        })
        .collect();
    for body in &m.body {
        let mut own = body.clone();
        for p in &mut own.pieces {
            p.3 = Some(me);
        }
        run(&own.substitute(&m.params, &args), at, file, macros, Some(me), map, errors);
    }
}

#[cfg(test)]
#[path = "preprocess.test.rs"]
mod tests;
