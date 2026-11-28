//! Real scripts built: `JASS_GAME` is a path list of folders with the
//! game's `common.j` and `Blizzard.j` (the last that has one wins);
//! `JASS_FILES` a path list of scripts, each built alone; `JASS_DIR` a
//! folder whose `.j` files are built together (a map's triggers, say).
//! What's built must read as the game's JASS (no rule of `pure`), and
//! have no problem of names or types the sources didn't have.

use std::path::{Path, PathBuf};

use jass_rs::names::Declarations;
use wc3_build::from_jass::Source;

fn read(p: &Path) -> String {
    String::from_utf8_lossy(&std::fs::read(p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))).into_owned()
}

fn game() -> Declarations {
    let folders: Vec<PathBuf> = std::env::split_paths(&std::env::var_os("JASS_GAME").expect("JASS_GAME")).collect();
    let mut env = Declarations::default();
    for name in ["common.j", "Blizzard.j"] {
        let p = folders.iter().rev().map(|g| g.join(name)).find(|p| p.is_file()).unwrap_or_else(|| panic!("no {name}"));
        env.extend(&Declarations::of(&jass_rs::parse(&read(&p)).unwrap()));
    }
    env
}

/// The problems of `text`: names and types over `env`, and `pure`'s.
fn problems(text: &str, env: &Declarations) -> Vec<String> {
    let program = jass_rs::parse(text).unwrap_or_else(|e| panic!("line {}: {}", e.line, e.message));
    let mut out: Vec<String> = jass_rs::names::check(&program, &[env]).into_iter().map(|d| d.message).collect();
    out.extend(
        jass_rs::lint(&program)
            .into_iter()
            .filter(|d| parser_rs::find_rule(d.rule).is_some_and(|r| r.groups.contains(&parser_rs::group::STRICT)) || d.rule == parser_rs::rule::DUPLICATE_DECLARATION)
            .map(|d| format!("{}: {}", d.rule, d.message)),
    );
    out.sort();
    out
}

#[test]
#[ignore]
fn real_scripts() {
    let env = game();
    for p in std::env::split_paths(&std::env::var_os("JASS_FILES").unwrap_or_default()) {
        let src = read(&p);
        let built = wc3_build::jass_of(&[Source { name: &p.to_string_lossy(), text: &src }]).unwrap_or_else(|e| panic!("{e:?}"));
        let before = problems(&src, &env);
        let after = problems(&built, &env);
        println!("{}: {} lines built, {} problems as it was", p.display(), built.lines().count(), before.len());
        assert_eq!(after, before, "{}", p.display());
        // Shorter names: the same problems, by rule (names in them differ).
        let module = wc3_build::from_jass::module(&[Source { name: &p.to_string_lossy(), text: &src }]).unwrap();
        let short = wc3_build::jass::build_with(&module, &wc3_build::jass::Options { uglify: true, ..Default::default() }).unwrap();
        let rules = |text: &str| {
            let program = jass_rs::parse(text).unwrap_or_else(|e| panic!("line {}: {}", e.line, e.message));
            let mut r: Vec<&'static str> = jass_rs::names::check(&program, &[&env]).into_iter().map(|d| d.rule).collect();
            r.extend(jass_rs::lint(&program).into_iter().map(|d| d.rule));
            r.sort();
            r
        };
        assert_eq!(rules(&short.script), rules(&built), "{} with shorter names", p.display());
        println!("    shorter names: {} bytes of {} ({}%){}", short.script.len(), built.len(), short.script.len() * 100 / built.len(), if short.notes.is_empty() { String::new() } else { format!("; {:?}", short.notes) });
    }
}

#[test]
#[ignore]
fn real_folder() {
    let env = game();
    let Some(dir) = std::env::var_os("JASS_DIR") else { return };
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "j")).collect();
    files.sort();
    let texts: Vec<(String, String)> = files.iter().map(|p| (p.to_string_lossy().into_owned(), read(p))).collect();
    let sources: Vec<Source> = texts.iter().map(|(n, t)| Source { name: n, text: t }).collect();
    let built = wc3_build::jass_of(&sources).unwrap_or_else(|e| panic!("{:?}", &e[..e.len().min(5)]));
    let out = Path::new(&dir).join("built.j.out");
    std::fs::write(&out, &built).unwrap();
    let after = problems(&built, &env);
    println!("{} files built into {} lines ({}), {} problems", files.len(), built.lines().count(), out.display(), after.len());
    for p in after.iter() {
        println!("    {p}");
    }
}

/// Built scripts (`BUILT_FILES`, a path list: a map's `war3map.j` as a
/// build wrote it, inlined, its dead code left out or not) read as the game reads them: no
/// problem of names, types or `pure`'s over `JASS_GAME`.
#[test]
#[ignore]
fn built_scripts_check_clean() {
    let env = game();
    for p in std::env::split_paths(&std::env::var_os("BUILT_FILES").unwrap_or_default()) {
        let found = problems(&read(&p), &env);
        println!("{}: {} problem(s)", p.display(), found.len());
        assert!(found.is_empty(), "{}:\n{}", p.display(), found.iter().take(20).cloned().collect::<Vec<_>>().join("\n"));
    }
}
