//! The map's part of the script made of a real map's files, against what
//! World Editor made of them. `MAP` is a folder with the map's files
//! (`war3map.w3i`, `.w3r`, `.w3c`; `war3map.j`, World Editor's script),
//! `WTG_TEXT` the text of its `war3map.wtg` (for its variables), `GAME_UI`
//! a path list of folders with the game's `MiscData.txt`,
//! `WorldEditData.txt` and `TriggerData.txt` (the last that has one
//! wins). Each function made is compared with World Editor's by its calls
//! and what it sets, numbers by their values. With `MAP_TRIGGERS` (the
//! triggers' JASS and `initialization.txt`, which the server's ignored test
//! `wtg_jass_files` writes), the whole `main` too.

use std::path::{Path, PathBuf};

use wc3_build::from_jass::{Source, module};
use wc3_build::ir::{Expr, Function, Module, Stmt, UnaryOp};
use wc3_build::map::{GameData, Map};

fn read(p: &Path) -> String {
    String::from_utf8_lossy(&std::fs::read(p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))).into_owned()
}

fn expr(e: &Expr) -> String {
    match e {
        Expr::Literal(t) => t.parse::<f64>().map_or(t.clone(), |v| format!("{v}")),
        Expr::Var(n) => n.clone(),
        Expr::Index(n, i) => format!("{n}[{}]", expr(i)),
        Expr::Call(c) => format!("{}({})", c.name, c.args.iter().map(expr).collect::<Vec<_>>().join(", ")),
        Expr::Code(n) => format!("function {n}"),
        Expr::Unary(UnaryOp::Neg, x) if matches!(&**x, Expr::Literal(_)) => match &**x {
            Expr::Literal(t) => t.parse::<f64>().map_or(format!("-{t}"), |v| format!("{}", -v)),
            _ => unreachable!(),
        },
        Expr::Unary(op, x) => format!("{op:?} {}", expr(x)),
        Expr::Binary(a, op, b) => format!("{} {op:?} {}", expr(a), expr(b)),
    }
}

/// A function's calls and what it sets, in order, as text; the counter of
/// a loop left out.
fn statements(f: &Function) -> Vec<String> {
    fn walk(body: &[Stmt], out: &mut Vec<String>) {
        for s in body {
            match s {
                Stmt::Call { call, .. } => out.push(expr(&Expr::Call(call.clone()))),
                Stmt::Set { name, index, value, .. } if name != "i" => {
                    out.push(format!("{name}{} = {}", if index.is_some() { "[]" } else { "" }, expr(value)))
                }
                Stmt::If { branches, otherwise, .. } => {
                    branches.iter().for_each(|(_, b)| walk(b, out));
                    if let Some(b) = otherwise {
                        walk(b, out);
                    }
                }
                Stmt::Loop { body, .. } => walk(body, out),
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    walk(&f.body, &mut out);
    out
}

/// The map's part, made of the files at `MAP`, and World Editor's script.
fn made() -> Option<(Module, Module, Map<'static>, GameData)> {
    let dir = PathBuf::from(std::env::var_os("MAP")?);
    let ui: Vec<PathBuf> = std::env::split_paths(&std::env::var_os("GAME_UI").expect("GAME_UI")).collect();
    let last = |name: &str| ui.iter().rev().map(|d| d.join(name)).find(|p| p.is_file()).map(|p| read(&p)).unwrap_or_default();
    let mut game = GameData::read(&last("MiscData.txt"), &last("WorldEditData.txt"));
    // The tables of the game's sounds, each the last layer's.
    let mut tables: std::collections::BTreeMap<String, PathBuf> = Default::default();
    for d in &ui {
        for e in std::fs::read_dir(d.join("SoundInfo")).into_iter().flatten().flatten() {
            if e.path().extension().is_some_and(|x| x.eq_ignore_ascii_case("slk")) {
                tables.insert(e.file_name().to_string_lossy().to_lowercase(), e.path());
            }
        }
    }
    for p in tables.values() {
        game.add_sound_info(&read(p));
    }
    // A project's patch of the editor's data (`MAP_PATCH`, a
    // TriggerData.patch.txt): its type defaults, which the base hasn't.
    let patch = std::env::var_os("MAP_PATCH").map(|p| read(Path::new(&p))).unwrap_or_default();
    let defaults = patch.split_once("[TriggerTypeDefaults]").map_or("", |(_, r)| r.split("\n[").next().unwrap_or(""));
    let trigger_ui = wtg_rs::ui::Ui::parse(&format!("{}\n[TriggerTypeDefaults]\n{defaults}", last("TriggerData.txt")), "", &|k| k.to_string());
    let info: &'static w3i_rs::Info = Box::leak(Box::new(w3i_rs::parse(&std::fs::read(dir.join("war3map.w3i")).unwrap()).unwrap()));
    let file = |name: &str| std::fs::read(dir.join(name)).ok();
    let regions = file("war3map.w3r").map(|b| &*Box::leak(Box::new(w3r_rs::parse(&b).unwrap())));
    let layout = w3c_rs::Layout::from_w3i_version(info.version);
    let cameras = file("war3map.w3c").map(|b| &*Box::leak(Box::new(w3c_rs::parse(&b, layout).unwrap())));
    let variables: Vec<wtg_rs::jass::MapVariable> = match std::env::var_os("WTG_TEXT") {
        Some(p) => wtg_rs::text::from_text(&read(Path::new(&p)), None)
            .file
            .expect("the triggers' text reads")
            .variables
            .into_iter()
            .map(|v| wtg_rs::jass::map_variable(&wtg_rs::text::Declared { variable: v, scope: wtg_rs::text::Scope::Global }, &trigger_ui))
            .collect(),
        None => Vec::new(),
    };
    let init: Vec<String> = std::env::var_os("MAP_TRIGGERS")
        .map(|d| read(&PathBuf::from(d).join("initialization.txt")).lines().map(String::from).collect())
        .unwrap_or_default();
    let sounds = file("war3map.w3s").map(|b| &*Box::leak(Box::new(w3s_rs::parse(&b).unwrap())));
    // A sound file: the map's, else the game's (`GAME_FILES`, folders of
    // its files, the last that has one winning).
    let roots: Vec<PathBuf> = std::iter::once(dir.clone())
        .chain(std::env::var_os("GAME_FILES").map(|g| std::env::split_paths(&g).collect::<Vec<_>>()).unwrap_or_default().into_iter().rev())
        .collect();
    let duration: &'static dyn Fn(&str) -> Option<u32> = Box::leak(Box::new(move |f: &str| {
        let rel = f.replace('\\', "/");
        roots.iter().map(|r| r.join(&rel)).find(|p| p.is_file()).and_then(|p| wc3_build::audio::duration_ms(&std::fs::read(p).ok()?))
    }));
    let map = Map { info, variables: Box::leak(variables.into_boxed_slice()), initialization: Box::leak(init.into_boxed_slice()), regions, cameras, sounds, duration };
    let ours = wc3_build::map::start(&map, &game).unwrap_or_else(|e| panic!("{e:?}"));
    let theirs_text = read(&dir.join("war3map.j"));
    let theirs = module(&[Source { name: "war3map.j", text: &theirs_text }]).unwrap();
    Some((ours, theirs, map, game))
}

#[test]
#[ignore]
fn the_maps_part_as_world_editors() {
    let Some((ours, theirs, map, _)) = made() else { return };
    let find = |m: &Module, name: &str| m.functions.iter().find(|f| f.sig.name == name).map(statements);
    // main: the map's setting up; what makes things not made yet left out.
    let later = ["CreateAllDestructables(", "CreateAllItems(", "CreateAllUnits(", "InitCustomTriggers(", "RunInitializationTriggers(", "InitUpgrades(", "InitTechTree(", "InitRandomGroups(", "InitMapItemTables("];
    let keep = |c: &String| !later.iter().any(|p| c.starts_with(p));
    let mut names = vec!["config", "InitCustomPlayerSlots", "InitCustomTeams", "InitAllyPriorities", "InitSounds", "CreateRegions", "CreateCameras", "main"];
    if !map.variables.is_empty() {
        names.push("InitGlobals");
    }
    for name in names {
        let (a, b) = (find(&ours, name).map(|v| v.into_iter().filter(keep).collect::<Vec<_>>()), find(&theirs, name).map(|v| v.into_iter().filter(keep).collect::<Vec<_>>()));
        println!("{name}: {} of ours, {} of World Editor's", a.as_ref().map_or(0, Vec::len), b.as_ref().map_or(0, Vec::len));
        let near = |x: &String, y: &String| x == y;
        let alike = a == b;
        if !alike {
            let (a, b) = (a.unwrap_or_default(), b.unwrap_or_default());
            let first = a.iter().zip(&b).position(|(x, y)| !near(x, y)).unwrap_or(a.len().min(b.len()));
            panic!("{name}: first difference at {first}:\n ours:   {:?}\n theirs: {:?}", a.get(first..(first + 3).min(a.len())), b.get(first..(first + 3).min(b.len())));
        }
    }
}

/// The whole map built (the map's part, the triggers, the map's end), and
/// its `main` against World Editor's, the functions World Editor's calls
/// to set the triggers up (`InitCustomTriggers`, `InitTrig_...`,
/// `RunInitializationTriggers`) read where they're called; what the
/// map's part doesn't make yet left out. Each trigger is compared on its
/// own: a map's war3map.j may be older than its triggers' order.
#[test]
#[ignore]
fn the_whole_main_as_world_editors() {
    let Some(triggers) = std::env::var_os("MAP_TRIGGERS") else { return };
    let Some((start, theirs, map, _)) = made() else { return };
    let dir = PathBuf::from(triggers);
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "j")).collect();
    files.sort();
    let texts: Vec<(String, String)> = files.iter().map(|p| (p.to_string_lossy().into_owned(), read(p))).collect();
    let sources: Vec<Source> = texts.iter().map(|(n, t)| Source { name: n, text: t }).collect();
    let mut whole = start;
    whole.append(module(&sources).unwrap_or_else(|e| panic!("{:?}", &e[..e.len().min(5)])));
    whole.append(wc3_build::map::end(&map));
    let built = wc3_build::jass::build(&whole).unwrap_or_else(|e| panic!("{:?}", &e[..e.len().min(5)]));
    let ours = module(&[Source { name: "built", text: &built }]).unwrap();
    fn flat(m: &Module, name: &str, inline: &dyn Fn(&str) -> bool, out: &mut Vec<String>) {
        let Some(f) = m.functions.iter().find(|f| f.sig.name == name) else { return };
        for c in statements(f) {
            let callee = c.split('(').next().unwrap_or("").to_string();
            if inline(&callee) && m.functions.iter().any(|g| g.sig.name == callee) {
                flat(m, &callee, inline, out);
            } else {
                out.push(c);
            }
        }
    }
    let inline = |n: &str| n == "InitCustomTriggers" || n.starts_with("InitTrig_") || n == "RunInitializationTriggers";
    let later = ["CreateAllDestructables(", "CreateAllItems(", "CreateAllUnits("];
    let (mut a, mut b) = (Vec::new(), Vec::new());
    flat(&ours, "main", &inline, &mut a);
    flat(&theirs, "main", &inline, &mut b);
    // What sets the triggers (`gg_trg_X = CreateTrigger()`) is World
    // Editor's only: ours are made where they're declared.
    a.retain(|c| !c.contains(" = CreateTrigger()"));
    b.retain(|c| !later.iter().any(|l| c.starts_with(l)) && !c.contains(" = CreateTrigger()"));
    println!("main: {} of ours, {} of World Editor's ({} lines built)", a.len(), b.len(), built.lines().count());
    let trigger = |c: &String| c.contains("gg_trg_");
    let (pa, pb) = (a.iter().position(trigger).unwrap_or(a.len()), b.iter().position(trigger).unwrap_or(b.len()));
    assert_eq!(a[..pa], b[..pb], "the map's setting up");
    let by_trigger = |calls: &[String]| {
        let mut m: std::collections::BTreeMap<String, Vec<String>> = Default::default();
        for c in calls {
            if let Some(i) = c.find("gg_trg_") {
                let name: String = c[i..].chars().take_while(|ch| ch.is_alphanumeric() || *ch == '_').collect();
                m.entry(name).or_default().push(c.clone());
            }
        }
        m
    };
    let (ta, tb) = (by_trigger(&a[pa..]), by_trigger(&b[pb..]));
    let differ: Vec<String> = tb.iter().filter(|(k, v)| ta.get(*k) != Some(v)).map(|(k, v)| format!("{k}:\n  ours:   {:?}\n  theirs: {v:?}", ta.get(k))).collect();
    let extra: Vec<&String> = ta.keys().filter(|k| !tb.contains_key(*k)).collect();
    println!("{} triggers set up alike, {} differ, {} only ours", tb.len() - differ.len(), differ.len(), extra.len());
    for d in differ.iter().take(8) {
        println!("{d}");
    }
    assert!(differ.is_empty() && extra.is_empty());
}
