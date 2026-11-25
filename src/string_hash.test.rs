use super::*;
use crate::from_jass::{Source, module};
use crate::jass::{Options, build_with};

/// Bob Jenkins' lookup2.c (as published) with SStrHash2's normalization,
/// compiled and run: what it gives (each length of the last 11 bytes).
const C: [(&str, i32); 11] = [
    ("", -1119235827),
    ("a", -1587459251),
    ("A", -1587459251),
    ("ab", -1331083366),
    ("abc", 1043745117),
    ("hello", -1801350911),
    ("Hello World", -1563733934),
    ("units\\human/footman", 736755776),
    ("abcdefghijk", -1733605089),
    ("abcdefghijkl", -358854790),
    ("abcdefghijklm", -911692467),
];

#[test]
fn lookup2_as_published() {
    for (text, want) in C {
        assert_eq!(string_hash(text.as_bytes()), want, "{text:?}");
    }
    // Case and slashes don't count.
    assert_eq!(string_hash(b"Units/Human"), string_hash(b"UNITS\\HUMAN"));
}

/// The game's `SStrHash2` (pjass's `sstrhash.c`, compiled and run):
/// lines (`LOOKUP2_IN`) and its numbers (`LOOKUP2_OUT`), each the same
/// here. 10013 lines of 0 to 40 ASCII characters were.
#[test]
#[ignore]
fn lookup2_against_c() {
    let input = std::fs::read_to_string(std::env::var("LOOKUP2_IN").expect("LOOKUP2_IN")).unwrap();
    let output = std::fs::read_to_string(std::env::var("LOOKUP2_OUT").expect("LOOKUP2_OUT")).unwrap();
    let mut n = 0;
    for (text, want) in input.lines().zip(output.lines()) {
        assert_eq!(string_hash(text.as_bytes()), want.parse::<i32>().unwrap(), "{text:?}");
        n += 1;
    }
    assert!(n > 1000);
    println!("{n} the same");
}

fn built(text: &str, string_hash: bool) -> String {
    let m = module(&[Source { name: "t.j", text }]).unwrap_or_else(|e| panic!("{e:?}"));
    build_with(&m, &Options { string_hash, ..Default::default() }).unwrap_or_else(|e| panic!("{e:?}")).script
}

#[test]
fn literals_hashed_as_built() {
    let text = concat!(
        "// @dialect vjass\n",
        "globals\n    integer K = StringHash(\"hello\")\nendglobals\n",
        "function F takes string s returns integer\n",
        "    local integer a = StringHash(\"a\") + StringHash(s)\n",
        "    local integer b = StringHash(\"\\\"q\\\"\")\n",
        "    local integer c = StringHash(\"д\")\n",
        "    return StringHash(\"Hello World\")\n",
        "endfunction\n",
    );
    let out = built(text, true);
    // Literals: numbers; what isn't one, or isn't ASCII, the game's to work out.
    let quoted = string_hash(b"\"q\"");
    for part in [
        "    integer K = -1801350911\n".to_string(),
        "    local integer a = -1587459251 + StringHash(s)\n".into(),
        format!("    local integer b = {quoted}\n"),
        "    local integer c = StringHash(\"д\")\n".into(),
        "    return -1563733934\n".into(),
    ] {
        assert!(out.contains(&part), "{part}---\n{out}");
    }
    // Past SStrHash2's buffer: the game's.
    let long = format!("    local integer d = StringHash(\"{}\")\n", "x".repeat(1024));
    assert!(built(&text.replace("    return", &format!("{long}    return")), true).contains(&long));
    // Not asked: as written.
    assert!(built(text, false).contains("StringHash(\"hello\")"));
}
