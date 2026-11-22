use super::*;
use crate::from_jass::{Source, module};

const COMMON: &str = "\
type agent extends handle
type trigger extends agent
type unit extends agent
type boolexpr extends agent
type conditionfunc extends boolexpr
type triggercondition extends agent
native CreateTrigger takes nothing returns trigger
native TriggerEvaluate takes trigger t returns boolean
native Condition takes code func returns conditionfunc
native TriggerAddCondition takes trigger t, boolexpr c returns triggercondition
native KillUnit takes unit u returns nothing
native I2S takes integer i returns string
";

/// What `files` build into, checked: it reads as the game's JASS (no rule
/// of `pure`), and its names and types are right over `COMMON`.
fn built(files: &[(&str, &str)]) -> String {
    let sources: Vec<Source> = files.iter().map(|(name, text)| Source { name, text }).collect();
    let out = build(&module(&sources).unwrap()).unwrap_or_else(|e| panic!("{e:?}"));
    let program = jass_rs::parse(&out).unwrap_or_else(|e| panic!("line {}: {}\n{out}", e.line, e.message));
    let pure: Vec<_> = jass_rs::lint(&program)
        .into_iter()
        .filter(|d| parser_rs::find_rule(d.rule).is_some_and(|r| r.groups.contains(&parser_rs::group::PURE)) || d.rule == parser_rs::rule::DUPLICATE_DECLARATION)
        .collect();
    assert!(pure.is_empty(), "{pure:?}\n{out}");
    let common = jass_rs::names::Declarations::of(&jass_rs::parse(COMMON).unwrap());
    let problems = jass_rs::names::check(&program, &[&common]);
    assert!(problems.is_empty(), "{problems:?}\n{out}");
    // Calls go only up.
    let mut seen: HashSet<String> = common.functions.keys().cloned().collect();
    for item in &program.items {
        if let jass_rs::ast::TopLevel::Function(f) = item {
            seen.insert(f.sig.name.clone());
            let ir = crate::from_jass::module(&[Source { name: "x", text: &out }]).unwrap();
            let me = ir.functions.iter().find(|g| g.sig.name == f.sig.name).unwrap();
            for c in called(&me.body) {
                assert!(seen.contains(&c), "{} calls {c}, which is below it\n{out}", f.sig.name);
            }
        }
    }
    out
}

#[test]
fn this_toolchains_jass_as_the_games() {
    let out = built(&[(
        "a.j",
        "\
function F returns integer
    integer i = 1
    i = i + 2 * (3 - 1)
    KillUnit(null)
    if i > 0 then
        string s = I2S(i)
        unit array us
        us[i] = null
    endif
    return -(-i)
endfunction
",
    )]);
    assert_eq!(
        out,
        "\
globals
endglobals

function F takes nothing returns integer
    local integer i = 1
    local string s
    local unit array us
    set i = i + 2 * (3 - 1)
    call KillUnit(null)
    if i > 0 then
        set s = I2S(i)
        set us[i] = null
    endif
    return - -i
endfunction
"
    );
}

#[test]
fn precedence_as_the_game_reads_it() {
    // `and` binds looser than `or`: what's written is kept.
    let out = built(&[(
        "a.j",
        "function F takes boolean a, boolean b, boolean c returns boolean\n    return (a and b) or c\nendfunction\nfunction G takes boolean a, boolean b, boolean c returns boolean\n    return a and (b or c)\nendfunction\nfunction H takes integer x returns boolean\n    return not (x == 1) and x - (1 - 2) > 0\nendfunction\n",
    )]);
    assert!(out.contains("return (a and b) or c\n"), "{out}");
    assert!(out.contains("return a and b or c\n"), "{out}");
    assert!(out.contains("return not (x == 1) and x - (1 - 2) > 0\n"), "{out}");
}

#[test]
fn functions_after_what_they_call() {
    let out = built(&[
        ("a.j", "function A\n    B()\nendfunction\nfunction main\n    A()\nendfunction\n"),
        ("b.j", "function B\n    C()\nendfunction\nfunction C\nendfunction\nfunction config\nendfunction\n"),
    ]);
    let order: Vec<&str> = out.lines().filter_map(|l| l.strip_prefix("function ")).map(|l| l.split(' ').next().unwrap()).collect();
    assert_eq!(order, ["C", "B", "A", "config", "main"]);
}

#[test]
fn calls_in_a_cycle_go_through_a_trigger() {
    let out = built(&[(
        "a.j",
        "\
function Even takes integer n returns boolean
    if n == 0 then
        return true
    endif
    return Odd(n - 1)
endfunction
function Odd takes integer n returns boolean
    if n == 0 then
        return false
    endif
    return Even(n - 1)
endfunction
function Start
    Condition(function Start)
endfunction
",
    )]);
    for part in [
        "    trigger wcb__trig_Odd\n    integer wcb__arg_Odd_0\n    boolean wcb__ret_Odd\n",
        "function wcb__call_Odd takes integer n returns boolean\n    set wcb__arg_Odd_0 = n\n    call TriggerEvaluate(wcb__trig_Odd)\n    return wcb__ret_Odd\nendfunction\n",
        "    return wcb__call_Odd(n - 1)\n",
        "function wcb__thunk_Odd takes nothing returns boolean\n    set wcb__ret_Odd = Odd(wcb__arg_Odd_0)\n    return true\nendfunction\n",
        // Made where the game starts.
        "function main takes nothing returns nothing\n    set wcb__trig_Odd = CreateTrigger()\n    call TriggerAddCondition(wcb__trig_Odd, Condition(function wcb__thunk_Odd))\nendfunction\n",
        // Itself: fine as it is.
        "    call Condition(function Start)\n",
    ] {
        assert!(out.contains(part), "{part}\n---\n{out}");
    }
}

#[test]
fn mains_and_configs_merged() {
    let out = built(&[
        ("a.j", "function main\n    integer i = 1\n    KillUnit(null)\nendfunction\nfunction config\n    integer i = 2\nendfunction\n"),
        ("b.j", "function main\n    integer i = 3\n    I2S(i)\nendfunction\n"),
    ]);
    assert!(out.contains("function main takes nothing returns nothing\n    local integer i = 1\n    local integer i__2\n    call KillUnit(null)\n    set i__2 = 3\n    call I2S(i__2)\nendfunction\n"), "{out}");
    assert_eq!(out.matches("function main").count(), 1);
}

#[test]
fn globals_after_what_they_read_and_calls_into_main() {
    let out = built(&[(
        "a.j",
        "integer b = a + 1\ninteger a = 1\ntrigger t = CreateTrigger()\nstring s = Name(a)\nfunction Name takes integer i returns string\n    return I2S(i)\nendfunction\n",
    )]);
    assert!(out.starts_with("globals\n    integer a = 1\n    integer b = a + 1\n    trigger t = CreateTrigger()\n    string s\nendglobals\n"), "{out}");
    assert!(out.contains("function main takes nothing returns nothing\n    set s = Name(a)\nendfunction\n"), "{out}");
    // A constant can't wait for main.
    let e = build(&module(&[Source { name: "c.j", text: "constant string S = N()\nfunction N returns string\n    return \"\"\nendfunction\n" }]).unwrap()).unwrap_err();
    assert!(e[0].message.contains("the constant 'S' is set by calling 'N'"), "{e:?}");
    let e = build(&module(&[Source { name: "c.j", text: "integer a = b\ninteger b = a\n" }]).unwrap()).unwrap_err();
    assert!(e[0].message.contains("reads itself"), "{e:?}");
}
