use super::*;
use crate::from_jass::{Source, module};

/// `m` with every name `to` gives put back: a module renamed and put back
/// is the module it was.
fn put_back(m: &mut Module, to: &HashMap<String, String>) {
    let back: HashMap<String, String> = to.iter().map(|(a, b)| (b.clone(), a.clone())).collect();
    apply_top(m, &back);
}

#[test]
fn names_by_use_and_the_games_kept() {
    let src = "\
native KillUnit takes unit u returns nothing
integer counter = 0
unit array heroes
function Helper takes integer count returns integer
    integer total = count + counter
    return total
endfunction
function Tick
    counter = Helper(counter) + Helper(1) + counter
    KillUnit(heroes[counter])
    ExecuteFunc(\"Tick\")
    TriggerRegisterVariableEvent(null, \"counter\", EQUAL, 1)
    Condition(function Helper2)
endfunction
function Helper2 returns boolean
    return bj_MAX > 0
endfunction
function main
    Tick()
endfunction
";
    let mut m = module(&[Source { name: "a.j", text: src }]).unwrap();
    let r = uglify(&mut m);
    // counter is used most: `a`; the game's names (KillUnit, bj_MAX,
    // ExecuteFunc ...) and main are as they were.
    assert_eq!(r.names.get("counter").map(String::as_str), Some("a"));
    assert!(!r.names.contains_key("main") && !r.names.contains_key("KillUnit"));
    let out = crate::jass::build(&m).unwrap();
    for part in [
        "native KillUnit takes unit u returns nothing",
        "    integer a = 0\n",
        "call ExecuteFunc(\"",
        "call TriggerRegisterVariableEvent(null, \"a\", EQUAL, 1)",
        "function main takes nothing returns nothing",
        "bj_MAX > 0",
    ] {
        assert!(out.contains(part), "{part}\n---\n{out}");
    }
    // A function's locals: the shortest names clear of what it uses (the
    // global `a`, its own name `b`), the most used first.
    assert_eq!(r.names["Helper"], "b");
    assert!(out.contains("function b takes integer d returns integer\n    local integer c = d + a\n    return c\n"), "{out}");
    // Put back, it's what it was (its locals aside).
    let mut again = module(&[Source { name: "a.j", text: src }]).unwrap();
    let mut back = m.clone();
    put_back(&mut back, &r.names);
    for f in again.functions.iter_mut().chain(back.functions.iter_mut()) {
        rename_locals(f);
    }
    assert_eq!(back, again);
}

#[test]
fn a_name_not_known_keeps_what_it_may_name() {
    let src = "function A\nendfunction\nfunction B takes integer i\nendfunction\nfunction main\n    string s = \"A\"\n    ExecuteFunc(s)\n    B(1)\nendfunction\n";
    let mut m = module(&[Source { name: "a.j", text: src }]).unwrap();
    let r = uglify(&mut m);
    assert!(!r.names.contains_key("A") && r.names.contains_key("B"), "{r:?}");
    assert_eq!(r.kept.len(), 1);
}

#[test]
fn the_shortest_names_in_order() {
    let names: Vec<String> = Names { n: 0 }.take(60).collect();
    assert_eq!(&names[..3], ["a", "b", "c"]);
    assert_eq!(names[51], "Z");
    assert_eq!(&names[52..55], ["aa", "ab", "ac"]);
    let n = Names { n: 52 + 52 * 62 }.next().unwrap();
    assert_eq!(n, "aaa");
}

#[test]
fn a_local_renamed_once() {
    // `distance` used most takes `a`, `d` then takes `b`: neither twice.
    let src = "function F takes integer x\n    real distance = 200\n    integer d = x\n    distance = distance + distance + d\nendfunction\nfunction main\n    F(1)\nendfunction\n";
    let mut m = module(&[Source { name: "a.j", text: src }]).unwrap();
    uglify(&mut m);
    let out = crate::jass::build(&m).unwrap();
    let program = jass_rs::parse(&out).unwrap();
    assert!(!jass_rs::lint(&program).iter().any(|d| d.rule == parser_rs::rule::DUPLICATE_DECLARATION), "{out}");
    assert!(out.contains("    local real b = 200\n    local integer c = d\n    set b = b + b + c\n"), "{out}");
}

#[test]
fn a_global_never_meets_a_local_of_its_new_name() {
    // The array is used most: it's `a`... and the loop's `a` isn't it.
    let src = "integer array a\nfunction F\n    integer i = 0\n    loop\n        exitwhen i > 1\n        a[i] = a[i] + a[0] + a[1]\n        i = i + 1\n    endloop\nendfunction\nfunction main\n    F()\nendfunction\n";
    let mut m = module(&[Source { name: "a.j", text: src }]).unwrap();
    uglify(&mut m);
    let out = crate::jass::build(&m).unwrap();
    let program = jass_rs::parse(&out).unwrap();
    let common = jass_rs::names::Declarations::default();
    assert!(jass_rs::names::check(&program, &[&common]).iter().all(|d| d.rule != parser_rs::rule::ARRAY_ACCESS), "{out}");
    assert!(out.contains("set a[c] = a[c] + a[0] + a[1]"), "{out}");
}
