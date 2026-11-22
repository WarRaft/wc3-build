//! The map's part of the script made of a real map's files, against what
//! World Editor made of them: `MAP_W3I` (its `war3map.w3i`), `MAP_SCRIPT`
//! (its `war3map.j`), `GAME_UI` (a path list of folders with the game's
//! `MiscData.txt` and `WorldEditData.txt`, the last that has one winning).
//! Each function made is compared with World Editor's by its calls, numbers
//! by their values.

use std::path::{Path, PathBuf};

use wc3_build::from_jass::{Source, module};
use wc3_build::ir::{Expr, Function, Stmt};

fn read(p: &Path) -> String {
    String::from_utf8_lossy(&std::fs::read(p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))).into_owned()
}

/// A function's calls, as text, numbers by their values.
fn calls(f: &Function) -> Vec<String> {
    fn expr(e: &Expr) -> String {
        match e {
            Expr::Literal(t) => t.parse::<f64>().map_or(t.clone(), |v| format!("{v}")),
            Expr::Var(n) => n.clone(),
            Expr::Index(n, i) => format!("{n}[{}]", expr(i)),
            Expr::Call(c) => format!("{}({})", c.name, c.args.iter().map(expr).collect::<Vec<_>>().join(", ")),
            Expr::Code(n) => format!("function {n}"),
            Expr::Unary(op, x) => match (op, &**x) {
                (wc3_build::ir::UnaryOp::Neg, Expr::Literal(t)) => t.parse::<f64>().map_or(format!("-{t}"), |v| format!("{}", -v)),
                _ => format!("{op:?} {}", expr(x)),
            },
            Expr::Binary(a, op, b) => format!("{} {op:?} {}", expr(a), expr(b)),
        }
    }
    let mut out = Vec::new();
    fn walk(body: &[Stmt], out: &mut Vec<String>, expr: &dyn Fn(&Expr) -> String) {
        for s in body {
            match s {
                Stmt::Call { call, .. } => out.push(expr(&Expr::Call(call.clone()))),
                Stmt::If { branches, otherwise, .. } => {
                    branches.iter().for_each(|(_, b)| walk(b, out, expr));
                    if let Some(b) = otherwise {
                        walk(b, out, expr);
                    }
                }
                Stmt::Loop { body, .. } => walk(body, out, expr),
                _ => {}
            }
        }
    }
    walk(&f.body, &mut out, &expr);
    out
}

#[test]
#[ignore]
fn the_maps_part_as_world_editors() {
    let (Some(w3i), Some(script)) = (std::env::var_os("MAP_W3I"), std::env::var_os("MAP_SCRIPT")) else { return };
    let ui: Vec<PathBuf> = std::env::split_paths(&std::env::var_os("GAME_UI").expect("GAME_UI")).collect();
    let last = |name: &str| ui.iter().rev().map(|d| d.join(name)).find(|p| p.is_file()).map(|p| read(&p)).unwrap_or_default();
    let game = wc3_build::map::GameData::read(&last("MiscData.txt"), &last("WorldEditData.txt"));
    let info = w3i_rs::parse(&std::fs::read(&w3i).unwrap()).unwrap();
    let map = wc3_build::map::Map { info: &info, initialization: &[], globals: true };
    let ours_text = wc3_build::map::start(&map, &game);
    let ours = module(&[Source { name: "map", text: &ours_text }]).unwrap_or_else(|e| panic!("{e:?}\n{ours_text}"));
    let theirs_text = read(Path::new(&script));
    let theirs = module(&[Source { name: "war3map.j", text: &theirs_text }]).unwrap();
    let find = |m: &wc3_build::ir::Module, name: &str| m.functions.iter().find(|f| f.sig.name == name).map(calls);
    // main: the map's setting up, before what makes its parts.
    let parts = ["InitSounds", "CreateRegions", "CreateCameras", "CreateAllDestructables", "CreateAllItems", "CreateAllUnits", "InitCustomTriggers", "RunInitializationTriggers", "InitUpgrades", "InitTechTree", "InitRandomGroups", "InitMapItemTables"];
    let keep = |c: &String| !parts.iter().any(|p| c.starts_with(&format!("{p}(")));
    for name in ["config", "InitCustomPlayerSlots", "InitCustomTeams", "InitAllyPriorities", "main"] {
        let (a, b) = (find(&ours, name), find(&theirs, name));
        let (a, b) = (a.map(|v| v.into_iter().filter(keep).collect::<Vec<_>>()), b.map(|v| v.into_iter().filter(keep).collect::<Vec<_>>()));
        println!("{name}: {} calls of ours, {} of World Editor's", a.as_ref().map_or(0, Vec::len), b.as_ref().map_or(0, Vec::len));
        assert_eq!(a, b, "{name}");
    }
}

/// A function's assignments, as text, numbers by their values.
fn sets(f: &Function) -> Vec<String> {
    fn walk(body: &[Stmt], out: &mut Vec<String>) {
        for s in body {
            match s {
                Stmt::Set { name, index, value, .. } => out.push(format!("{name}{} = {value:?}", if index.is_some() { "[]" } else { "" })),
                Stmt::Loop { body, .. } => walk(body, out),
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    walk(&f.body, &mut out);
    // The loop counter is World Editor's and ours alike, set where it is.
    out.retain(|s| !s.starts_with("i = "));
    out
}

/// The triggers' variables (`MAP_VARIABLES`: wtg_rs::jass::variables of
/// the map's) set as World Editor's `InitGlobals` sets them.
#[test]
#[ignore]
fn init_globals_as_world_editors() {
    let (Some(vars), Some(script)) = (std::env::var_os("MAP_VARIABLES"), std::env::var_os("MAP_SCRIPT")) else { return };
    let ours_text = read(Path::new(&vars));
    let ours = module(&[Source { name: "variables", text: &ours_text }]).unwrap();
    let theirs_text = read(Path::new(&script));
    let theirs = module(&[Source { name: "war3map.j", text: &theirs_text }]).unwrap();
    let find = |m: &wc3_build::ir::Module| m.functions.iter().find(|f| f.sig.name == "InitGlobals").map(sets).unwrap_or_default();
    let (a, b) = (find(&ours), find(&theirs));
    println!("InitGlobals: {} of ours, {} of World Editor's", a.len(), b.len());
    assert_eq!(a, b);
}

/// The whole map built (`MAP_W3I` with `GAME_UI` for the map's part,
/// `MAP_DIR` for the rest: its `.j` files, `initialization.txt` the
/// triggers of map initialization), and its `main` against World Editor's
/// (`MAP_SCRIPT`), the functions World Editor's calls to set the triggers
/// up (`InitCustomTriggers`, `InitTrig_...`, `RunInitializationTriggers`)
/// read where they're called, what the map's part doesn't make yet
/// (regions, sounds, cameras, what's placed) left out.
#[test]
#[ignore]
fn the_whole_main_as_world_editors() {
    let (Some(w3i), Some(script), Some(dir)) = (std::env::var_os("MAP_W3I"), std::env::var_os("MAP_SCRIPT"), std::env::var_os("MAP_DIR")) else { return };
    let ui: Vec<PathBuf> = std::env::split_paths(&std::env::var_os("GAME_UI").expect("GAME_UI")).collect();
    let last = |name: &str| ui.iter().rev().map(|d| d.join(name)).find(|p| p.is_file()).map(|p| read(&p)).unwrap_or_default();
    let game = wc3_build::map::GameData::read(&last("MiscData.txt"), &last("WorldEditData.txt"));
    let info = w3i_rs::parse(&std::fs::read(&w3i).unwrap()).unwrap();
    let dir = PathBuf::from(dir);
    let init: Vec<String> = read(&dir.join("initialization.txt")).lines().map(String::from).collect();
    let map = wc3_build::map::Map { info: &info, initialization: &init, globals: true };
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "j")).collect();
    files.sort();
    let mut texts = vec![("map start".to_string(), wc3_build::map::start(&map, &game))];
    texts.extend(files.iter().map(|p| (p.to_string_lossy().into_owned(), read(p))));
    texts.push(("map end".to_string(), wc3_build::map::end(&map)));
    let sources: Vec<Source> = texts.iter().map(|(n, t)| Source { name: n, text: t }).collect();
    let built = wc3_build::jass_of(&sources).unwrap_or_else(|e| panic!("{:?}", &e[..e.len().min(5)]));
    let ours = module(&[Source { name: "built", text: &built }]).unwrap();
    let theirs_text = read(Path::new(&script));
    let theirs = module(&[Source { name: "war3map.j", text: &theirs_text }]).unwrap();
    // A function's calls, those of the functions `inline` names read where
    // they're called.
    fn flat(m: &wc3_build::ir::Module, name: &str, inline: &dyn Fn(&str) -> bool, out: &mut Vec<String>) {
        let Some(f) = m.functions.iter().find(|f| f.sig.name == name) else { return };
        for c in calls(f) {
            let callee = c.split('(').next().unwrap_or("").to_string();
            if inline(&callee) && m.functions.iter().any(|g| g.sig.name == callee) {
                flat(m, &callee, inline, out);
            } else {
                out.push(c);
            }
        }
    }
    let inline = |n: &str| n == "InitCustomTriggers" || n.starts_with("InitTrig_") || n == "RunInitializationTriggers";
    let later = ["InitSounds(", "CreateRegions(", "CreateCameras(", "CreateAllDestructables(", "CreateAllItems(", "CreateAllUnits("];
    let (mut a, mut b) = (Vec::new(), Vec::new());
    flat(&ours, "main", &inline, &mut a);
    flat(&theirs, "main", &inline, &mut b);
    b.retain(|c| !later.iter().any(|l| c.starts_with(l)));
    println!("main: {} calls of ours, {} of World Editor's ({} lines built)", a.len(), b.len(), built.lines().count());
    // The map's setting up, before the first trigger: as it is.
    let trigger = |c: &String| c.contains("gg_trg_");
    let (pa, pb) = (a.iter().position(trigger).unwrap_or(a.len()), b.iter().position(trigger).unwrap_or(b.len()));
    assert_eq!(a[..pa], b[..pb], "the map's setting up");
    // Each trigger's setting up (the calls naming it), in its order: a
    // map's war3map.j may be older than its triggers' order.
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
    let mut differ = Vec::new();
    for (name, theirs) in &tb {
        match ta.get(name) {
            Some(ours) if ours == theirs => {}
            ours => differ.push(format!("{name}:\n  ours:   {ours:?}\n  theirs: {theirs:?}")),
        }
    }
    let extra: Vec<&String> = ta.keys().filter(|k| !tb.contains_key(*k)).collect();
    println!("{} triggers set up alike, {} differ, {} only ours", tb.len() - differ.len(), differ.len(), extra.len());
    for d in differ.iter().take(8) {
        println!("{d}");
    }
    for e in extra.iter().take(8) {
        println!("only ours: {e}: {:?}", ta[*e]);
    }
    assert!(differ.is_empty() && extra.is_empty());
}
