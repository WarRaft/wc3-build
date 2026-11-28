use super::*;

fn all(_: &str) -> bool {
    true
}

/// `files` (name, text): the first preprocessed, the others what it may
/// include; its text and its errors' messages.
fn run_with(files: &[(&str, &str)], feature: &dyn Fn(&str) -> bool) -> (String, Vec<String>, Made) {
    let include = |_: &str, path: &str| files.iter().find(|(n, _)| *n == path).map(|(n, t)| (n.to_string(), t.to_string()));
    let o = Options { feature, include: &include, debug: true, date: "2026.11.28".into(), time: "12:00:00".into() };
    let made = preprocess(files[0].0, files[0].1, &o);
    (made.map.text.clone(), made.errors.iter().map(|(_, m)| m.clone()).collect(), made)
}

fn run(text: &str) -> (String, Vec<String>) {
    let (t, e, _) = run_with(&[("a.j", text)], &all);
    (t, e)
}

#[test]
fn macros_with_and_without_arguments() {
    let (t, e) = run("define FOOTMAN = 'hfoo'\ndefine {\n    msg(text) = DisplayTextToPlayer(GetLocalPlayer(), text, 0, 0)\n    msg(text, p) = DisplayTextToPlayer(Player(p), text, 0, 0)\n}\ncall CreateUnit(p, FOOTMAN, 0, 0, 0)\nmsg(\"one\")\nmsg(\"two\", 1)\nset s = \"FOOTMAN\"\n");
    assert!(e.is_empty(), "{e:?}");
    assert_eq!(t, "call CreateUnit(p, 'hfoo', 0, 0, 0)\nDisplayTextToPlayer(GetLocalPlayer(), \"one\", 0, 0)\nDisplayTextToPlayer(Player(1), \"two\", 0, 0)\nset s = \"FOOTMAN\"\n");
}

#[test]
fn a_value_of_several_lines_stringize_and_paste() {
    let (t, e) = run("define register_func(type) = {\n    nothing func_##type (type t) {\n        BJDebugMsg(`type`)\n    }\n}\nregister_func(real)\n");
    assert!(e.is_empty(), "{e:?}");
    assert_eq!(t, "nothing func_real (real t) {\n    BJDebugMsg(\"real\")\n}\n");
}

#[test]
fn a_hook_and_its_paste() {
    // `Remove##Unit`: pasted after the expansion, not expanded again.
    let (t, e) = run("define RemoveUnit(u) = {\n    BJDebugMsg(\"removed\")\n    Remove##Unit(u)\n}\nRemoveUnit(x)\n");
    assert!(e.is_empty(), "{e:?}");
    assert_eq!(t, "BJDebugMsg(\"removed\")\nRemoveUnit(x)\n");
}

#[test]
fn setdef_undef_and_a_second_define() {
    let (t, e) = run("define msg = \"text\"\nBJDebugMsg(msg)\nsetdef msg = \"other\"\nBJDebugMsg(msg)\nundef msg\nBJDebugMsg(msg)\ndefine X = 1\ndefine X = 2\n");
    assert_eq!(t, "BJDebugMsg(\"text\")\nBJDebugMsg(\"other\")\nBJDebugMsg(msg)\n");
    assert_eq!(e, ["macro X is defined already (a.j:7); setdef changes it"]);
}

#[test]
fn a_special_name() {
    let (t, _) = run("define <GetPlayableMapRect()> = bj_mapInitialPlayableArea\nset r = GetPlayableMapRect()\n");
    assert_eq!(t, "set r = bj_mapInitialPlayableArea\n");
}

#[test]
fn predefined_macros() {
    let (t, _) = run("function F takes nothing returns nothing\ncall X(COUNTER, COUNTER, DEBUG, FUNCNAME, WAR3VER, DATE)\n");
    assert_eq!(t, "function F takes nothing returns nothing\ncall X(0, 1, 1, F, WAR3VER_24, 2026.11.28)\n");
}

#[test]
fn conditional_translation_and_error() {
    let (t, e) = run("define CONTROL = 3\n#if CONTROL == 1\none\n#elseif CONTROL == 3\nthree\n#else\nother\n#endif\ndefine FLAG = true\n#if FLAG\n#error \"stop\"\n#endif\n#if 0\n#error \"not this\"\n#endif\n");
    assert_eq!(t, "three\n");
    assert_eq!(e, ["stop"]);
}

#[test]
fn enums() {
    let (t, _) = run("enum { FOOTMAN, MAGE }\nenum (color) { RED, GREEN }\nenum { WARDEN }\nenum (moves)\n    WALK\n    STOP\nendenum\nX(FOOTMAN, MAGE, RED, GREEN, WARDEN, WALK, STOP)\n");
    assert_eq!(t, "X(0, 1, 0, 1, 2, 0, 1)\n");
}

#[test]
fn includes_in_place_guarded_and_never_round() {
    let files = [
        ("a.j", "first\ninclude \"b.j\"\ninclude \"b.j\"\ninclude \"c.j\"\nlast\n"),
        ("b.j", "#guard b_j\nfrom b\n"),
        // c includes d, which includes c: a cycle, said and broken.
        ("c.j", "from c\ninclude \"d.j\"\n"),
        ("d.j", "from d\ninclude \"c.j\"\n"),
    ];
    let (t, e, made) = run_with(&files, &all);
    assert_eq!(t, "first\nfrom b\nfrom c\nfrom d\nlast\n");
    assert_eq!(e, ["include cycle: c.j → d.j → c.j; not included again"]);
    // The cycle is said where the include that closes it is written.
    assert_eq!((made.errors[0].0.file.as_str(), made.errors[0].0.line), ("d.j", 2));
    // A file that includes itself.
    let (_, e, _) = run_with(&[("self.j", "include \"self.j\"\n")], &all);
    assert_eq!(e, ["include cycle: self.j → self.j; not included again"]);
}

#[test]
fn each_piece_knows_where_it_s_written() {
    let (_, _, made) = run_with(&[("a.j", "define msg(text) = BJDebugMsg(text)\n\nmsg(\"hi\")\n")], &all);
    let m = &made.map;
    assert_eq!(m.text, "\nBJDebugMsg(\"hi\")\n");
    // `BJDebugMsg(` from the define (line 1), `"hi"` from the use (line 3).
    assert_eq!(m.origin(2, 1).map(|(s, _)| (s.line, s.col)), Some((1, 20)));
    assert_eq!(m.origin(2, 12).map(|(s, _)| (s.line, s.col)), Some((3, 5)));
    let (_, e) = m.origin(2, 1).unwrap();
    assert_eq!(m.chain(e).iter().map(|x| x.name.as_str()).collect::<Vec<_>>(), ["macro msg"]);
}

#[test]
fn a_feature_off_leaves_its_lines() {
    let off = |f: &str| f != "macros";
    let (t, _, _) = run_with(&[("a.j", "define X = 1\ncall F(X)\n")], &off);
    assert_eq!(t, "define X = 1\ncall F(X)\n");
}
