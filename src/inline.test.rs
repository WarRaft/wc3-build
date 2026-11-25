use crate::from_jass::{Source, module};
use crate::jass::{Options, build_with};

/// `text` built optimized, each line trimmed; read back as the game's JASS.
fn optimized(text: &str) -> String {
    let m = module(&[Source { name: "t.j", text }]).unwrap_or_else(|e| panic!("{e:?}"));
    let out = build_with(&m, &Options { inline: true, dead_code: true, ..Default::default() }).unwrap_or_else(|e| panic!("{e:?}")).script;
    crate::structs::tests::as_the_games(&out);
    out.lines().map(str::trim).collect::<Vec<_>>().join("\n")
}

#[test]
fn the_manuals_inlining() {
    // JassHelper's manual's example, called from main.
    let out = optimized(
        "globals\n    integer array myarray\nendglobals\nfunction MyFunction takes integer a, integer b returns integer\n    return myarray[a] * b\nendfunction\nfunction MyOtherFunction takes nothing returns integer\n    return MyFunction(3, 4)\nendfunction\nfunction main takes nothing returns nothing\n    call BJDebugMsg(I2S(MyOtherFunction()))\nendfunction\n",
    );
    // Both one-liners: inlined into main, then left out as nothing calls them.
    assert!(out.contains("function main takes nothing returns nothing\ncall BJDebugMsg(I2S(myarray[3] * 4))\nendfunction"), "{out}");
    assert!(!out.contains("function MyFunction") && !out.contains("function MyOtherFunction"), "{out}");
}

#[test]
fn only_where_nothing_can_change() {
    let out = optimized(
        r#"globals
    integer g = 0
    integer unused = 5
endglobals
function Twice takes integer a returns integer
    return a + a
endfunction
function Swapped takes integer a, integer b returns integer
    return b - a
endfunction
function CallFirst takes integer a returns integer
    return ModuloInteger(0, 1) + a
endfunction
function SetG takes integer v returns nothing
    set g = v
endfunction
function SetParam takes integer v returns nothing
    set v = 1
endfunction
function Say takes string s returns nothing
    call BJDebugMsg(s)
endfunction
function Hidden takes nothing returns integer
    return g
endfunction
function Nobody takes nothing returns nothing
endfunction
function Named takes nothing returns nothing
endfunction
function main takes nothing returns nothing
    local integer g = 2
    call SetG(Twice(1) + Swapped(1, 2) + CallFirst(3))
    call SetParam(4)
    call Say("hi")
    call BJDebugMsg(I2S(Hidden()))
    call ExecuteFunc("Named")
endfunction
"#,
    );
    // Read twice, out of order, after a call, its own parameter set, a
    // global a local hides (SetG's g, Hidden's): not inlined; Say is.
    assert!(out.contains("call SetG(Twice(1) + Swapped(1, 2) + CallFirst(3))"), "{out}");
    assert!(out.contains("call SetParam(4)\ncall BJDebugMsg(\"hi\")\ncall BJDebugMsg(I2S(Hidden()))"), "{out}");
    // Dead: what nothing calls (but by ExecuteFunc's name), a global nothing reads.
    assert!(!out.contains("function Nobody") && out.contains("function Named") && !out.contains("unused") && !out.contains("function Say"), "{out}");
}

#[test]
fn an_execute_func_told_only_as_it_runs_keeps_every_function() {
    let out = optimized("globals\n    string n = \"Nobody\"\nendglobals\nfunction Nobody takes nothing returns nothing\nendfunction\nfunction main takes nothing returns nothing\n    call ExecuteFunc(n)\nendfunction\n");
    assert!(out.contains("function Nobody"), "{out}");
}
