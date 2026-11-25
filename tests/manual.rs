//! JassHelper's manual against this toolchain: `JASSHELPER_MANUAL` is the
//! manual's `topics` folder (a `.md` per topic). Every code block of a
//! topic (`sql`, `C++` or unmarked, and that reads as code) is read as the
//! editor reads it (`jass_rs`, vJASS; a Zinc topic's as Zinc) and built
//! (`wc3-build`, into the game's JASS); what either says is written to
//! `MANUAL_OUT` (a file), a block a paragraph, and counted. It fails
//! nothing by itself: an example may be a fragment, or wrong on purpose
//! (the manual shows errors too); it's read by a person.

use std::fmt::Write as _;
use std::path::PathBuf;

/// The topics under Zinc in the manual's tree (`JassHelper.tree`).
const ZINC: &[&str] = &[
    "Zinc.md",
    "Because-it-is-good-for-your-brains.md",
    "Enough-propaganda-how-do-I-use-it.md",
    "Syntax-basics.md",
    "library.md",
    "public-or-private.md",
    "Globals.md",
    "Functions.md",
    "assignment.md",
    "if.md",
    "Logical-operations.md",
    "Arithmetic-assignments.md",
    "static-if.md",
    "while.md",
    "for.md",
    "break.md",
    "debug.md",
    "local-variables.md",
    "Zinc-structs.md",
    "Zinc-Dynamic-arrays.md",
    "Function-pointers-interfaces.md",
    "Anonymous-functions.md",
    "Anonymous-methods.md",
    "The-small-print.md",
];

/// The code blocks of a topic: each one's first line (from 1) and text.
fn blocks(md: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let mut open: Option<(usize, String, String)> = None;
    for (i, line) in md.lines().enumerate() {
        let t = line.trim_start();
        match (&mut open, t.strip_prefix("```")) {
            (None, Some(lang)) => open = Some((i + 2, lang.trim().to_string(), String::new())),
            (Some((at, lang, text)), Some(_)) => {
                if matches!(lang.as_str(), "sql" | "C++" | "") {
                    out.push((*at, std::mem::take(text)));
                }
                open = None;
            }
            (Some((_, _, text)), None) => {
                text.push_str(line);
                text.push('\n');
            }
            (None, None) => {}
        }
    }
    out
}

/// Does `text` read as code at all (not a listing of a console or a list)?
fn code(text: &str) -> bool {
    ["function", "struct", "library", "scope", "globals", "interface", "module", "method", "//!", "type ", "native", "keyword", "hook", "set ", "call "].iter().any(|w| text.contains(w))
}

#[test]
#[ignore]
fn the_manuals_examples() {
    let Some(dir) = std::env::var_os("JASSHELPER_MANUAL").map(PathBuf::from) else { return };
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir).unwrap().filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.extension().is_some_and(|e| e == "md")).collect();
    files.sort();
    let mut report = String::new();
    let (mut total, mut parse_errors, mut build_errors, mut clean, mut panics) = (0, 0, 0, 0, 0);
    std::panic::set_hook(Box::new(|_| {}));
    for f in files {
        let topic = f.file_name().unwrap().to_string_lossy().into_owned();
        if topic == "Changelog.md" {
            continue;
        }
        let zinc = ZINC.contains(&topic.as_str());
        let md = std::fs::read_to_string(&f).unwrap();
        for (line, text) in blocks(&md).into_iter().filter(|(_, t)| code(t)) {
            total += 1;
            let mut said = Vec::new();
            // As the editor reads it.
            if zinc && !text.contains("//! zinc") {
                if let Err(e) = zinc_rs::parse(&text) {
                    said.push(format!("editor: line {}: {}", e.line, e.message));
                    parse_errors += 1;
                }
            } else {
                match jass_rs::parse_with(&text, jass_rs::Dialect::VJass) {
                    Err(e) => {
                        said.push(format!("editor: line {}: {}", e.line, e.message));
                        parse_errors += 1;
                    }
                    Ok(p) => {
                        for d in jass_rs::lint(&p).into_iter().filter(|d| d.level == parser_rs::Level::Error) {
                            said.push(format!("editor: line {}: {} [{}]", d.line, d.message, d.rule));
                        }
                    }
                }
            }
            // As the build makes it.
            let name = if zinc && !text.contains("//! zinc") { "t.zn" } else { "t.j" };
            let built = std::panic::catch_unwind(|| {
                wc3_build::from_jass::module_as(&[wc3_build::from_jass::Source { name, text: &text }], jass_rs::Dialect::VJass).and_then(|m| wc3_build::jass::build_with(&m, &Default::default()))
            });
            let built = match built {
                Ok(b) => b,
                Err(e) => {
                    panics += 1;
                    let what = e.downcast_ref::<String>().cloned().or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_default();
                    Err(vec![wc3_build::ir::Error { origin: wc3_build::ir::Origin::file("the build"), message: format!("PANIC: {what}") }])
                }
            };
            if let Err(errors) = built {
                build_errors += 1;
                for e in errors.iter().take(6) {
                    said.push(format!("build: line {}: {}", e.origin.line, e.message));
                }
            }
            if said.is_empty() {
                clean += 1;
                continue;
            }
            let first = text.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim();
            let _ = writeln!(report, "## {topic}:{line} ({}) {first}", if zinc { "Zinc" } else { "vJASS" });
            for s in said {
                let _ = writeln!(report, "  {s}");
            }
            report.push('\n');
        }
    }
    let summary = format!("{total} blocks: {clean} clean, {parse_errors} the editor can't read, {build_errors} the build refuses ({panics} of them a panic)\n\n");
    let out = std::env::var_os("MANUAL_OUT").map(PathBuf::from).unwrap_or_else(|| std::env::temp_dir().join("manual.txt"));
    std::fs::write(&out, summary.clone() + &report).unwrap();
    eprintln!("{summary}written to {}", out.display());
}
