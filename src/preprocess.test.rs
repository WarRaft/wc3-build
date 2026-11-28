use super::*;

fn none(_: &str, _: &str) -> Option<(String, String)> {
    None
}

#[test]
fn textmacros_run_where_named_across_files() {
    let a = "//! textmacro Getter takes NAME, TYPE\nfunction Get$NAME$ takes nothing returns $TYPE$\n    return $NAME$\nendfunction\n//! endtextmacro\nglobals\nendglobals\n";
    let b = "//! runtextmacro Getter(\"Gold\", \"integer\")\n//! runtextmacro optional Missing()\n//! novjass\nnot code\n//! endnovjass\n//!runtextmacro Twice(\"x\")\n//! textmacro Twice takes V\n//! runtextmacro Getter(\"$V$1\", \"real\")\n//! runtextmacro Getter(\"$V$2\", \"real\")\n//! endtextmacro\n";
    let out = expand(&[("a.j", a), ("b.j", b)], &none).unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(out[0].text, "globals\nendglobals\n");
    assert_eq!(out[0].lines, vec![6, 7]);
    assert_eq!(
        out[1].text,
        "function GetGold takes nothing returns integer\n    return Gold\nendfunction\nfunction Getx1 takes nothing returns real\n    return x1\nendfunction\nfunction Getx2 takes nothing returns real\n    return x2\nendfunction\n"
    );
    // Each line made: the line of the runtextmacro that put it there.
    assert_eq!(out[1].lines, vec![1, 1, 1, 6, 6, 6, 6, 6, 6]);
}

#[test]
fn what_a_textmacro_cant_be() {
    let e = expand(
        &[("a.j", "//! textmacro A\n//! endtextmacro\n//! textmacro A\n//! endtextmacro\n//! textmacro_once B\n//! endtextmacro\n//! textmacro_once B\n//! endtextmacro\n//! runtextmacro A(\"x\")\n//! runtextmacro C()\n//! textmacro R\n//! runtextmacro R()\n//! endtextmacro\n//! runtextmacro R()\n//! textmacro Open\n")],
        &none,
    )
    .unwrap_err();
    let got: Vec<(usize, &str)> = e.iter().map(|e| (e.origin.line, e.message.as_str())).collect();
    assert_eq!(
        got,
        vec![
            (3, "textmacro A is declared already (a.j:1)"),
            (15, "`//! textmacro Open` has no `//! endtextmacro`"),
            (9, "textmacro A takes 0 argument(s), given 1"),
            (10, "no textmacro C"),
            (14, "textmacro R runs itself (or textmacros run each other) past 64 deep"),
        ]
    );
}

#[test]
fn imports_once_each_as_files_of_their_own() {
    let import = |from: &str, path: &str| -> Option<(String, String)> {
        match path {
            "lib.j" => Some(("lib.j".into(), "//! import \"a.j\"\nfunction L takes nothing returns nothing\nendfunction\n".into())),
            "z" => Some(("z".into(), "library Z {}\n".into())),
            // Imported back: a file given already, not again.
            "a.j" => Some(("a.j".into(), String::new())),
            _ => {
                assert_eq!(from, "a.j");
                None
            }
        }
    };
    let out = expand(&[("a.j", "//! import \"lib.j\"\n//! import zinc \"z\"\n//! import comment \"docs\"\n//! import \"lib.j\"\n")], &import).unwrap_or_else(|e| panic!("{e:?}"));
    let names: Vec<(&str, Option<usize>)> = out.iter().map(|e| (e.name.as_str(), e.imported_by)).collect();
    assert_eq!(names, vec![("a.j", None), ("lib.j", Some(0)), ("z.zn", Some(0))]);
    assert_eq!(out[0].text, "");
    assert_eq!(out[1].text, "function L takes nothing returns nothing\nendfunction\n");
    let e = expand(&[("a.j", "//! import \"nope\"\n")], &import).unwrap_err();
    assert_eq!(e[0].message, "`//! import`: no file \"nope\"");
}

#[test]
fn every_piece_knows_where_it_s_written() {
    let a = "//! textmacro Init takes NAME\nfunction $NAME$_init takes nothing returns nothing\nendfunction\n//! endtextmacro\n";
    let b = "\n//! runtextmacro Init(\"Hero\")\n";
    let out = expand(&[("a.j", a), ("b.j", b)], &|_, _| None).unwrap();
    let m = &out[1].map;
    assert_eq!(m.text, "\nfunction Hero_init takes nothing returns nothing\nendfunction\n");
    // `function ` is the macro's body (a.j:2), `Hero` the argument where
    // it's given (b.j:2, its column), `_init` the body again.
    assert_eq!(m.origin(2, 1).map(|(s, _)| (s.file, s.line, s.col)), Some(("a.j".into(), 2, 1)));
    assert_eq!(m.origin(2, 10).map(|(s, _)| (s.file, s.line, s.col)), Some(("b.j".into(), 2, 24)));
    assert_eq!(m.origin(2, 14).map(|(s, _)| (s.file, s.line, s.col)), Some(("a.j".into(), 2, 16)));
    // The expansion: Init, defined at a.j:1, used at b.j:2.
    let (_, e) = m.origin(2, 1).unwrap();
    let chain = m.chain(e);
    assert_eq!(chain.iter().map(|e| (e.name.as_str(), e.defined.line, e.used.file.as_str(), e.used.line)).collect::<Vec<_>>(), [("Init", 1, "b.j", 2)]);
    // And each line is said where the macro is used, as before.
    assert_eq!(out[1].lines, [1, 2, 2]);
}
