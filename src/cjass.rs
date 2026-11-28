//! cJass's preprocessor (AdicHelper's; `CJASS.md`, the manual, sections 3
//! to 5), on the one engine of macros and includes (`parser_rs::expand`):
//! every piece of the text made knows where it's written and the
//! expansion that made it, for the editor and the build alike.
//!
//! Each construct is one of cJass's features (`[cjass]` in `wc3.bni`, a
//! header's `@cjass.<feature>`), and runs only when it's on; off, its line
//! stays as it is (and the parser says what it makes of it):
//!
//! - `include` (`include "path"`, `#include`; `#guard name`): the file's
//!   text put in place, its own preprocessed; a file including itself,
//!   however far round, is an error naming the cycle, and isn't put in
//!   again; `#guard` puts a file in once.
//! - `macros`: `define NAME = value`, `define NAME(a, b) = value`
//!   (overloaded by the number of arguments), `define <any text> = value`,
//!   `define { ... }` and `define ... enddefine` blocks, a value `{ ... }`
//!   of several lines, `setdef`, `undef`, `define private` (a name
//!   defined once more is an error); in a value, `` `a` `` (an argument as
//!   a string) and `##` (two words pasted, anywhere in the code).
//! - `predefined`: `DATE`, `TIME`, `COUNTER`, `DEBUG`, `FUNCNAME`,
//!   `WAR3VER` (`WAR3VER_24`).
//! - `conditional`: `#if A == B`, `#if A != B`, `#if FLAG` (`true` or
//!   `1`), `#elseif`, `#else`, `#endif`.
//! - `error`: `#error "message"`.
//! - `enum`: `enum [(name)] { A, B }`, `enum [(name)]` ... `endenum`: the
//!   names defined as integers, each named enum counting from 0, the
//!   unnamed ones counting on together.

use std::collections::{HashMap, HashSet};

use parser_rs::expand::{Expansion, Line, Mapped, Spelling};

/// Where `include "path"` in the file `from` leads: the file's name and
/// text; `None` when there's no such file.
pub type Include<'a> = dyn Fn(&str, &str) -> Option<(String, String)> + 'a;

/// What the preprocessor is run with.
pub struct Options<'a> {
    /// Is cJass's feature on (`parser_rs::settings::Settings::cjass_feature`).
    pub feature: &'a dyn Fn(&str) -> bool,
    pub include: &'a Include<'a>,
    /// `DEBUG`'s value.
    pub debug: bool,
    /// `DATE`'s and `TIME`'s (`yyyy.mm.dd`, `hh:mm:ss`).
    pub date: String,
    pub time: String,
}

/// What a file made: its text and where each piece is written, and what's
/// wrong, each where.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Made {
    pub map: Mapped,
    pub errors: Vec<(Spelling, String)>,
}

/// A macro: its parameters (`None`: one without parentheses), its value's
/// lines, where it's defined.
#[derive(Debug, Clone)]
struct Macro {
    params: Option<Vec<String>>,
    body: Vec<Line>,
    defined: Spelling,
}

/// How deep macros may expand in each other.
const DEPTH: usize = 64;

/// The predefined macros.
const PREDEFINED: [&str; 6] = ["DATE", "TIME", "COUNTER", "DEBUG", "FUNCNAME", "WAR3VER"];

struct Run<'a> {
    o: &'a Options<'a>,
    macros: HashMap<String, Vec<Macro>>,
    /// `define <text> = value`: by their text.
    special: Vec<(String, Macro)>,
    guards: HashSet<String>,
    /// The files being put in, outermost first: a cycle's.
    stack: Vec<String>,
    counter: usize,
    enums: HashMap<Option<String>, i64>,
    funcname: String,
    made: Made,
}

/// Preprocesses the file `name` (its `text`).
pub fn preprocess(name: &str, text: &str, o: &Options) -> Made {
    let mut run = Run { o, macros: HashMap::new(), special: Vec::new(), guards: HashSet::new(), stack: Vec::new(), counter: 0, enums: HashMap::new(), funcname: String::new(), made: Made::default() };
    run.file(name, text, None);
    run.made
}

/// A token of a line: its kind and characters (columns, 1-based, end
/// exclusive).
#[derive(Debug, Clone, Copy, PartialEq)]
enum Tok {
    Word,
    Text,
    Comment,
    Other,
}

/// The tokens of `chars`: words, strings and rawcodes (a quote's to its
/// closing one), a `//` comment (to the end), and any other character.
fn tokens(chars: &[char]) -> Vec<(Tok, usize, usize)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let start = i;
        if c.is_ascii_alphabetic() || c == '_' {
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            out.push((Tok::Word, start + 1, i + 1));
        } else if c == '"' || c == '\'' {
            i += 1;
            while i < chars.len() && chars[i] != c {
                if chars[i] == '\\' && c == '"' {
                    i += 1;
                }
                i += 1;
            }
            i = (i + 1).min(chars.len());
            out.push((Tok::Text, start + 1, i + 1));
        } else if c == '/' && chars.get(i + 1) == Some(&'/') {
            out.push((Tok::Comment, start + 1, chars.len() + 1));
            break;
        } else {
            i += 1;
            out.push((Tok::Other, start + 1, i + 1));
        }
    }
    out
}

/// The characters `from..to` (columns) of `line`, as a line of their own.
fn part(line: &Line, from: usize, to: usize) -> Line {
    let mut l = Line::default();
    l.push_part(line, from, to);
    l
}

/// `line` with its leading and trailing spaces off.
fn trimmed(line: &Line) -> Line {
    let chars: Vec<char> = line.text.chars().collect();
    let a = chars.iter().take_while(|c| c.is_whitespace()).count();
    let b = chars.len() - chars.iter().rev().take_while(|c| c.is_whitespace()).count();
    part(line, a + 1, b.max(a) + 1)
}

impl Run<'_> {
    fn error(&mut self, at: Spelling, message: String) {
        self.made.errors.push((at, message));
    }

    fn on(&self, feature: &str) -> bool {
        (self.o.feature)(feature)
    }

    /// The file `name` put in, `by` the line including it (`None`: the
    /// file read).
    fn file(&mut self, name: &str, text: &str, by: Option<usize>) {
        if let Some(at) = self.stack.iter().position(|f| f == name) {
            let mut cycle: Vec<&str> = self.stack[at..].iter().map(String::as_str).collect();
            cycle.push(name);
            let from = self.made.expansions_used(by).unwrap_or_else(|| Spelling::new(name, 1, 1));
            self.error(from, format!("include cycle: {}; not included again", cycle.join(" → ")));
            return;
        }
        self.stack.push(name.to_string());
        let lines: Vec<Line> = parser_rs::lines::lines(text).enumerate().map(|(i, l)| Line::written(l, Spelling::new(name, i + 1, 1), by)).collect();
        self.lines(&lines, by);
        self.stack.pop();
    }

    /// Lines of a file (or a block), their directives run.
    fn lines(&mut self, lines: &[Line], parent: Option<usize>) {
        // Each `#if` open: is its branch taken, was one taken already.
        let mut conds: Vec<(bool, bool)> = Vec::new();
        let active = |conds: &Vec<(bool, bool)>| conds.iter().all(|(on, _)| *on);
        let mut i = 0;
        while i < lines.len() {
            let line = &lines[i];
            let at = line.origin(1).map_or_else(|| self.made.where_empty(line), |(s, _)| s);
            let text = line.text.trim();
            let word = |w: &str| text.strip_prefix(w).filter(|r| r.is_empty() || r.starts_with(char::is_whitespace)).map(str::trim);
            i += 1;
            // Conditional translation.
            if self.on("conditional") {
                if let Some(cond) = word("#if") {
                    let on = active(&conds) && self.condition(cond, &at);
                    conds.push((on, on));
                    continue;
                }
                // The branches outside the last `#if` taken: its own can be.
                let outer = |conds: &Vec<(bool, bool)>| conds.len() <= 1 || conds[..conds.len() - 1].iter().all(|(on, _)| *on);
                if let Some(cond) = word("#elseif") {
                    let Some(&(_, taken)) = conds.last() else {
                        self.error(at, "`#elseif` without its `#if`".into());
                        continue;
                    };
                    let now = !taken && outer(&conds) && self.condition(cond, &at);
                    let last = conds.last_mut().expect("one");
                    last.0 = now;
                    last.1 |= now;
                    continue;
                }
                if word("#else").is_some() {
                    let Some(&(_, taken)) = conds.last() else {
                        self.error(at, "`#else` without its `#if`".into());
                        continue;
                    };
                    let now = !taken && outer(&conds);
                    let last = conds.last_mut().expect("one");
                    last.0 = now;
                    last.1 = true;
                    continue;
                }
                if word("#endif").is_some() {
                    if conds.pop().is_none() {
                        self.error(at, "`#endif` without its `#if`".into());
                    }
                    continue;
                }
            }
            if !active(&conds) {
                continue;
            }
            if self.on("include") {
                if let Some(guard) = word("#guard") {
                    if !self.guards.insert(guard.to_string()) {
                        // Put in already: the rest of this file isn't.
                        return;
                    }
                    continue;
                }
                if let Some(path) = word("include").or_else(|| word("#include")) {
                    match path.strip_prefix('"').and_then(|p| p.strip_suffix('"')) {
                        Some(path) => {
                            let path = path.replace("\\\\", "\\");
                            let from = at.file.clone();
                            match (self.o.include)(&from, &path) {
                                Some((name, text)) => {
                                    let e = self.made.map.expansion(Expansion { name: format!("include \"{path}\""), defined: Spelling::new(&name, 1, 1), used: at.clone(), parent });
                                    self.file(&name, &text, Some(e));
                                }
                                None => self.error(at, format!("include: no file \"{path}\"")),
                            }
                        }
                        None => self.error(at, format!("include needs a quoted path: {path}")),
                    }
                    continue;
                }
            }
            if self.on("error")
                && let Some(message) = word("#error")
            {
                let message = message.trim_matches('"').to_string();
                self.error(at, message);
                continue;
            }
            if self.on("enum") && (word("enum").is_some()) {
                i = self.enumeration(lines, i - 1);
                continue;
            }
            if self.on("macros") {
                let define = word("define").or_else(|| word("#define"));
                if let Some(rest) = define {
                    i = self.define(lines, i - 1, rest, false);
                    continue;
                }
                if let Some(rest) = word("setdef") {
                    i = self.define(lines, i - 1, rest, true);
                    continue;
                }
                if let Some(name) = word("undef") {
                    if self.macros.remove(name).is_none() {
                        self.error(at, format!("undef: no macro {name}"));
                    }
                    continue;
                }
            }
            for made in self.expand(line, parent, &mut Vec::new(), 0) {
                self.note_function(&made.text);
                let made = self.paste(&made);
                self.made.map.push_line(&made);
            }
        }
        if !conds.is_empty() {
            let at = lines.last().and_then(|l| l.origin(1)).map_or_else(|| Spelling::new("", 0, 1), |(s, _)| s);
            self.error(at, "`#if` without its `#endif`".into());
        }
    }

    /// `#if`'s condition: `A == B`, `A != B`, or a flag (`true`, `1`),
    /// its sides with their macros expanded.
    fn condition(&mut self, cond: &str, at: &Spelling) -> bool {
        let value = |run: &mut Run, s: &str| -> String {
            let l = Line::written(s.trim(), at.clone(), None);
            run.expand(&l, None, &mut Vec::new(), 0).iter().map(|l| l.text.trim().to_string()).collect::<Vec<_>>().join(" ")
        };
        if let Some((a, b)) = cond.split_once("==") {
            return value(self, a) == value(self, b);
        }
        if let Some((a, b)) = cond.split_once("!=") {
            return value(self, a) != value(self, b);
        }
        matches!(value(self, cond).as_str(), "true" | "1")
    }

    /// A `define` (or `setdef`) at line `i` of `lines`, `rest` after its
    /// word: its macros defined; the line after it.
    fn define(&mut self, lines: &[Line], i: usize, rest: &str, set: bool) -> usize {
        let line = &lines[i];
        let word = if set { "setdef" } else { "define" };
        let after = line.text.find(word).map_or(0, |b| line.text[..b].chars().count() + word.len() + usize::from(line.text.trim_start().starts_with('#')));
        let head = trimmed(&part(line, after + 1, line.text.chars().count() + 1));
        // `define {` ... `}` and `define` ... `enddefine`: a macro a line.
        if rest.is_empty() || rest == "{" {
            let close = if rest.is_empty() { "enddefine" } else { "}" };
            let mut j = i + 1;
            while j < lines.len() && lines[j].text.trim() != close {
                if !lines[j].text.trim().is_empty() {
                    j = self.one(lines, j, &trimmed(&lines[j]), set);
                } else {
                    j += 1;
                }
            }
            if j >= lines.len() {
                let at = line.origin(1).map_or_else(|| Spelling::new("", 0, 1), |(s, _)| s);
                self.error(at, format!("`{word}` has no `{close}`"));
            }
            return j + 1;
        }
        self.one(lines, i, &head, set)
    }

    /// One macro, its head and value `head` (`[private] NAME(a, b) = v`)
    /// on line `i`, its value going on for lines when it opens a `{`: the
    /// line after it.
    fn one(&mut self, lines: &[Line], i: usize, head: &Line, set: bool) -> usize {
        let at = head.origin(1).unwrap_or((Spelling::new("", 0, 1), None)).0;
        let text = head.text.clone();
        let body_text = text.strip_prefix("private").filter(|r| r.starts_with(char::is_whitespace)).map_or(text.as_str(), str::trim_start);
        let skip = text.chars().count() - body_text.chars().count();
        // Its name, its parameters, where its value starts.
        let special = body_text.starts_with('<');
        let (name, params, value_from) = if let Some(r) = body_text.strip_prefix('<') {
            let Some(end) = r.find('>') else {
                self.error(at, "a macro's name `<...>` isn't closed".into());
                return i + 1;
            };
            (r[..end].to_string(), None, skip + end + 2)
        } else {
            let n: String = body_text.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
            if n.is_empty() {
                self.error(at, "a macro needs a name".into());
                return i + 1;
            }
            let after = &body_text[n.len()..];
            match after.strip_prefix('(') {
                Some(r) => match r.find(')') {
                    Some(end) => {
                        let ps: Vec<String> = r[..end].split(',').map(|p| p.trim().to_string()).filter(|p| !p.is_empty()).collect();
                        (n.clone(), Some(ps), skip + n.len() + end + 2)
                    }
                    None => {
                        self.error(at, format!("macro {n}: its parameters' `)` is missing"));
                        return i + 1;
                    }
                },
                None => (n.clone(), None, skip + n.len()),
            }
        };
        let rest = part(head, value_from + 1, text.chars().count() + 1);
        let value = trimmed(&rest);
        let mut next = i + 1;
        let body = match value.text.strip_prefix('=') {
            None if value.text.is_empty() => Vec::new(),
            None => {
                self.error(at, format!("macro {name}: `=` and its value expected"));
                return next;
            }
            Some(_) => {
                let v = trimmed(&part(&value, 2, value.text.chars().count() + 1));
                if v.text.starts_with('{') {
                    // `{ ... }`: its lines, to the `}` that closes it.
                    let inner = trimmed(&part(&v, 2, v.text.chars().count() + 1));
                    if let Some(one) = inner.text.strip_suffix('}') {
                        vec![trimmed(&part(&inner, 1, one.chars().count() + 1))]
                    } else {
                        let mut body = Vec::new();
                        if !inner.text.is_empty() {
                            body.push(inner);
                        }
                        let mut depth = 1;
                        while next < lines.len() {
                            let l = &lines[next];
                            next += 1;
                            depth += l.text.matches('{').count();
                            depth -= l.text.matches('}').count().min(depth);
                            if depth == 0 {
                                let t = l.text.trim_end();
                                let before = t.strip_suffix('}').unwrap_or(t);
                                if !before.trim().is_empty() {
                                    body.push(part(l, 1, before.chars().count() + 1));
                                }
                                break;
                            }
                            body.push(l.clone());
                        }
                        // Its lines as written, their common indent off.
                        let indent = body.iter().filter(|l| !l.text.trim().is_empty()).map(|l| l.text.chars().take_while(|c| c.is_whitespace()).count()).min().unwrap_or(0);
                        body.iter().map(|l| part(l, indent + 1, l.text.chars().count() + 1)).collect()
                    }
                } else {
                    vec![v]
                }
            }
        };
        let m = Macro { params: params.clone(), body, defined: at.clone() };
        if special {
            self.special.retain(|(t, _)| *t != name);
            self.special.push((name, m));
            return next;
        }
        let arity = |m: &Macro| m.params.as_ref().map(Vec::len);
        let same = self.macros.get(&name).and_then(|ms| ms.iter().position(|old| arity(old) == arity(&m)));
        match (same, set) {
            (Some(k), true) => self.macros.get_mut(&name).expect("found")[k] = m,
            (Some(k), false) => {
                let old = self.macros[&name][k].defined.clone();
                self.error(at, format!("macro {name} is defined already ({}:{}); setdef changes it", file_name(&old.file), old.line));
            }
            (None, _) => self.macros.entry(name).or_default().push(m),
        }
        next
    }

    /// An `enum` at line `i` of `lines`: its names defined; the line
    /// after it.
    fn enumeration(&mut self, lines: &[Line], i: usize) -> usize {
        let line = &lines[i];
        let at = line.origin(1).unwrap_or((Spelling::new("", 0, 1), None)).0;
        let text = line.text.trim();
        let rest = text["enum".len()..].trim();
        let (name, rest) = match rest.strip_prefix('(') {
            Some(r) => match r.split_once(')') {
                Some((n, r)) => (Some(n.trim().to_string()), r.trim()),
                None => {
                    self.error(at, "enum's `(name)` isn't closed".into());
                    return i + 1;
                }
            },
            None => (None, rest),
        };
        // Its members: `{ A, B }` (on a line or more), or a line each to
        // `endenum`.
        let mut members: Vec<(String, Spelling)> = Vec::new();
        let mut next = i + 1;
        let take = |s: &str, at: &Spelling, members: &mut Vec<(String, Spelling)>| {
            for m in s.split(',').map(str::trim).filter(|m| !m.is_empty()) {
                members.push((m.to_string(), at.clone()));
            }
        };
        if let Some(r) = rest.strip_prefix('{') {
            match r.split_once('}') {
                Some((inside, _)) => take(inside, &at, &mut members),
                None => {
                    take(r, &at, &mut members);
                    while next < lines.len() {
                        let l = &lines[next];
                        let a = l.origin(1).map_or(at.clone(), |(s, _)| s);
                        next += 1;
                        match l.text.split_once('}') {
                            Some((inside, _)) => {
                                take(inside, &a, &mut members);
                                break;
                            }
                            None => take(&l.text, &a, &mut members),
                        }
                    }
                }
            }
        } else {
            while next < lines.len() && lines[next].text.trim() != "endenum" {
                let a = lines[next].origin(1).map_or(at.clone(), |(s, _)| s);
                take(&lines[next].text, &a, &mut members);
                next += 1;
            }
            next += 1;
        }
        for (member, a) in members {
            let n = self.enums.entry(name.clone()).or_insert(0);
            let value = *n;
            *n += 1;
            let body = vec![Line::written(&value.to_string(), a.clone(), None)];
            self.macros.entry(member).or_default().push(Macro { params: None, body, defined: a });
        }
        next
    }

    /// `line` with its macros expanded: its lines (a macro's value may be
    /// several). `disabled`: the macros being expanded (not again inside
    /// themselves).
    fn expand(&mut self, line: &Line, parent: Option<usize>, disabled: &mut Vec<String>, depth: usize) -> Vec<Line> {
        let macros_on = self.on("macros");
        let predefined_on = self.on("predefined");
        if (!macros_on && !predefined_on) || depth > DEPTH {
            return vec![line.clone()];
        }
        let chars: Vec<char> = line.text.chars().collect();
        let toks = tokens(&chars);
        let mut out: Vec<Line> = vec![Line::default()];
        let mut from = 1;
        let mut t = 0;
        while t < toks.len() {
            let (kind, a, b) = toks[t];
            t += 1;
            // A special name, as written.
            if macros_on && kind != Tok::Text && kind != Tok::Comment {
                let rest: String = chars[a - 1..].iter().collect();
                if let Some((name, m)) = self.special.iter().find(|(s, _)| rest.starts_with(s.as_str()) && !disabled.contains(s)).cloned() {
                    let end = a + name.chars().count();
                    out.last_mut().expect("one").push_part(line, from, a);
                    self.put(&mut out, &m, &name, &[], line, a, parent, disabled, depth);
                    from = end;
                    while t < toks.len() && toks[t].1 < end {
                        t += 1;
                    }
                    continue;
                }
            }
            if kind != Tok::Word {
                continue;
            }
            let word: String = chars[a - 1..b - 1].iter().collect();
            if predefined_on && PREDEFINED.contains(&word.as_str()) && !self.macros.contains_key(&word) {
                let value = match word.as_str() {
                    "DATE" => self.o.date.clone(),
                    "TIME" => self.o.time.clone(),
                    "COUNTER" => {
                        self.counter += 1;
                        (self.counter - 1).to_string()
                    }
                    "DEBUG" => u8::from(self.o.debug).to_string(),
                    "FUNCNAME" => self.funcname.clone(),
                    _ => "WAR3VER_24".into(),
                };
                let at = line.origin(a).map_or_else(|| Spelling::new("", 0, a), |(s, _)| s);
                let m = Macro { params: None, body: vec![Line::written(&value, at.clone(), None)], defined: at };
                out.last_mut().expect("one").push_part(line, from, a);
                self.put(&mut out, &m, &word, &[], line, a, parent, disabled, depth);
                from = b;
                continue;
            }
            if !macros_on || disabled.contains(&word) {
                continue;
            }
            let Some(overloads) = self.macros.get(&word).cloned() else { continue };
            // `NAME(args)`: its arguments, the parentheses' columns.
            let mut k = b;
            while k <= chars.len() && chars.get(k - 1).is_some_and(|c| *c == ' ' || *c == '\t') {
                k += 1;
            }
            let args = if chars.get(k - 1) == Some(&'(') { arguments(&chars, k) } else { None };
            let chosen = match &args {
                Some((given, _)) => overloads.iter().find(|m| m.params.as_ref().is_some_and(|p| p.len() == given.len())).or_else(|| overloads.iter().find(|m| m.params.is_none())),
                None => overloads.iter().find(|m| m.params.is_none()),
            };
            let Some(m) = chosen.cloned() else { continue };
            out.last_mut().expect("one").push_part(line, from, a);
            match (&m.params, args) {
                (Some(_), Some((given, end))) => {
                    let given: Vec<Line> = given.iter().map(|(x, y)| trimmed(&part(line, *x, *y))).collect();
                    self.put(&mut out, &m, &word, &given, line, a, parent, disabled, depth);
                    from = end;
                    while t < toks.len() && toks[t].1 < end {
                        t += 1;
                    }
                }
                _ => {
                    self.put(&mut out, &m, &word, &[], line, a, parent, disabled, depth);
                    from = b;
                }
            }
        }
        out.last_mut().expect("one").push_part(line, from, chars.len() + 1);
        if out.len() == 1 && out[0].pieces.is_empty()
            && let Some((_, _, s, e)) = line.pieces.first()
        {
            out[0].pieces.push((1, 0, s.clone(), *e));
        }
        out
    }

    /// Macro `m` (named `name`), used at column `at` of `line` with the
    /// arguments `given`, put into `out` (its first line going on the last
    /// of `out`).
    #[allow(clippy::too_many_arguments)]
    fn put(&mut self, out: &mut Vec<Line>, m: &Macro, name: &str, given: &[Line], line: &Line, at: usize, parent: Option<usize>, disabled: &mut Vec<String>, depth: usize) {
        let used = line.origin(at).map_or_else(|| Spelling::new("", 0, at), |(s, _)| s);
        let me = self.made.map.expansion(Expansion { name: name.to_string(), defined: m.defined.clone(), used, parent });
        let params = m.params.clone().unwrap_or_default();
        // `` `a` ``: the argument as a string.
        let quoted: Vec<Line> = given.iter().map(|g| {
            let mut l = Line::default();
            let s = g.origin(1).map_or_else(|| Spelling::new("", 0, 1), |(s, _)| s);
            l.push("\"", s.clone(), None);
            l.push_part(g, 1, g.text.chars().count() + 1);
            l.push("\"", s, None);
            l
        }).collect();
        let ticked: Vec<String> = params.iter().map(|p| format!("`{p}`")).collect();
        disabled.push(name.to_string());
        for (k, body) in m.body.iter().enumerate() {
            let mut own = body.clone();
            for p in &mut own.pieces {
                p.3 = Some(me);
            }
            let made = substitute_words(&substitute_text(&own, &ticked, &quoted), &params, given);
            let lines = self.expand(&made, Some(me), disabled, depth + 1);
            for (j, l) in lines.into_iter().enumerate() {
                if k == 0 && j == 0 {
                    let last = out.last_mut().expect("one");
                    last.push_part(&l, 1, l.text.chars().count() + 1);
                } else {
                    out.push(l);
                }
            }
        }
        disabled.pop();
    }

    /// `FUNCNAME`: the function a line made starts, if any.
    fn note_function(&mut self, text: &str) {
        let t = text.trim_start();
        let t = t.strip_prefix("constant ").map_or(t, str::trim_start);
        if let Some(r) = t.strip_prefix("function ") {
            self.funcname = r.trim_start().chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
        } else if t.ends_with('{')
            && let Some((head, _)) = t.split_once('(')
            && let [_, name] = head.split_whitespace().collect::<Vec<_>>()[..]
        {
            self.funcname = name.to_string();
        }
    }

    /// `##` off a line made, and the spaces beside it: two words pasted.
    fn paste(&self, line: &Line) -> Line {
        if !self.on("macros") || !line.text.contains("##") {
            return line.clone();
        }
        let chars: Vec<char> = line.text.chars().collect();
        let mut out = Line::default();
        let mut from = 1;
        for (kind, a, b) in tokens(&chars) {
            if kind == Tok::Other && chars[a - 1] == '#' && chars.get(a) == Some(&'#') && b == a + 1 {
                // The spaces before it.
                let mut s = a;
                while s > from && chars[s - 2].is_whitespace() {
                    s -= 1;
                }
                out.push_part(line, from, s);
                let mut e = a + 2;
                while e <= chars.len() && chars[e - 1].is_whitespace() {
                    e += 1;
                }
                from = e;
            }
        }
        out.push_part(line, from, chars.len() + 1);
        out
    }
}

impl Made {
    /// The line that put in expansion `e`.
    fn expansions_used(&self, e: Option<usize>) -> Option<Spelling> {
        e.map(|k| self.map.expansions[k].used.clone())
    }

    /// Where an empty line is written (its one piece of no characters).
    fn where_empty(&self, line: &Line) -> Spelling {
        line.pieces.first().map_or_else(|| Spelling::new("", 0, 1), |(_, _, s, _)| s.clone())
    }
}

/// The arguments of a call whose `(` is at column `open` of `chars`: each
/// one's columns (from, to), and the column past its `)`; split at commas
/// outside parentheses, strings and rawcodes.
fn arguments(chars: &[char], open: usize) -> Option<(Vec<(usize, usize)>, usize)> {
    let mut depth = 0;
    let mut args = Vec::new();
    let mut start = open + 1;
    let mut i = open;
    let mut quote: Option<char> = None;
    while i <= chars.len() {
        let c = chars[i - 1];
        match quote {
            Some(q) => {
                if c == '\\' && q == '"' {
                    i += 1;
                } else if c == q {
                    quote = None;
                }
            }
            None => match c {
                '"' | '\'' => quote = Some(c),
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        if i > start || !args.is_empty() {
                            args.push((start, i));
                        }
                        return Some((args, i + 1));
                    }
                }
                ',' if depth == 1 => {
                    args.push((start, i));
                    start = i + 1;
                }
                _ => {}
            },
        }
        i += 1;
    }
    None
}

/// `line` with each of `texts` (written literally) replaced by its line of
/// `by`.
fn substitute_text(line: &Line, texts: &[String], by: &[Line]) -> Line {
    let mut l = line.clone();
    for (t, b) in texts.iter().zip(by) {
        let mut out = Line::default();
        let chars: Vec<char> = l.text.chars().collect();
        let tc: Vec<char> = t.chars().collect();
        let mut from = 1;
        let mut i = 0;
        while i + tc.len() <= chars.len() {
            if chars[i..i + tc.len()] == tc[..] {
                out.push_part(&l, from, i + 1);
                out.push_part(b, 1, b.text.chars().count() + 1);
                i += tc.len();
                from = i + 1;
            } else {
                i += 1;
            }
        }
        out.push_part(&l, from, chars.len() + 1);
        l = out;
    }
    l
}

/// `line` with each word that's one of `params` replaced by its argument
/// in `by`, outside strings and comments.
fn substitute_words(line: &Line, params: &[String], by: &[Line]) -> Line {
    if params.is_empty() {
        return line.clone();
    }
    let chars: Vec<char> = line.text.chars().collect();
    let mut out = Line::default();
    let mut from = 1;
    for (kind, a, b) in tokens(&chars) {
        if kind != Tok::Word {
            continue;
        }
        let word: String = chars[a - 1..b - 1].iter().collect();
        if let Some(k) = params.iter().position(|p| *p == word)
            && let Some(arg) = by.get(k)
        {
            out.push_part(line, from, a);
            out.push_part(arg, 1, arg.text.chars().count() + 1);
            from = b;
        }
    }
    out.push_part(line, from, chars.len() + 1);
    if out.pieces.is_empty()
        && let Some((_, _, s, e)) = line.pieces.first()
    {
        out.pieces.push((1, 0, s.clone(), *e));
    }
    out
}

fn file_name(path: &str) -> String {
    std::path::Path::new(path).file_name().map_or(path.to_string(), |n| n.to_string_lossy().into_owned())
}

#[cfg(test)]
#[path = "cjass.test.rs"]
mod tests;
