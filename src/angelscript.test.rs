use crate::from_jass::{Source, module};
use crate::jass::{Options, Script, build_with};

/// A piece of `Blizzard.j`: one function the tests use, one they don't.
const BLIZZARD: &str = "\
globals
    constant real bj_PI = 3.14159
    timer bj_lastStartedTimer = null
    integer bj_unused = 0
endglobals
function StartTimerBJ takes timer t, boolean periodic, real timeout returns timer
    set bj_lastStartedTimer = t
    call TimerStart(t, timeout, periodic, null)
    return bj_lastStartedTimer
endfunction
function UnusedBJ takes nothing returns nothing
    set bj_unused = 1
endfunction
";

fn angelscript(text: &str) -> String {
    let m = module(&[Source { name: "t.j", text }]).unwrap_or_else(|e| panic!("{e:?}"));
    let common = format!("{}globals\n    constant integer PLAYER_NEUTRAL_AGGRESSIVE = 12\nendglobals\n", crate::structs::tests::COMMON);
    let game = crate::from_jass::signatures(&common).into_iter().chain(crate::from_jass::signatures(BLIZZARD)).collect();
    let options = Options { script: Script::AngelScript, game, common_j: common, blizzard_j: BLIZZARD.into(), ..Default::default() };
    let out = build_with(&m, &options).unwrap_or_else(|e| panic!("{e:?}")).script;
    // AngelScript as AngelScript reads it.
    if let Err(e) = as_rs::parse(&out) {
        panic!("line {}: {}\n{out}", e.line, e.message);
    }
    out
}

#[test]
fn jass_as_ujapis_angelscript() {
    let out = angelscript(r#"globals
    integer count = 7
    real half = .5
    string array names
    boolean done
    string none = null
    timer t
endglobals
function Name takes integer i returns string
    if names[i] == null then
        return null
    endif
    return names[i]
endfunction
function F takes integer a, integer b returns integer
    local integer i = 0
    local integer in
    local unit array units
    loop
        exitwhen i >= 10 and a != b or done
        set i = i + a / b
        set names[i] = "x" + Name(a) + "\n"
        if a == 'hfoo' then
            return $FF
        elseif b == 017 then
            call TimerStart(CreateTimer(), half, false, function Tick)
        else
            debug call BJDebugMsg("debugging")
        endif
    endloop
    set units[0] = null
    set in = PLAYER_NEUTRAL_AGGRESSIVE
    return i
endfunction
function Tick takes nothing returns nothing
    call StartTimerBJ(t, false, bj_PI)
endfunction
function main takes nothing returns nothing
    call TriggerAddCondition(CreateTrigger(), Condition(function Tick))
endfunction
"#);
    for part in [
        "const float bj_PI = 3.14159f;\ntimer bj_lastStartedTimer = nil;\n",
        "int count = 7;\nfloat half = 0.5f;\nWC3::string_array names;\nbool done = false;\nstring none = \"\";\ntimer t = nil;\n",
        "string Name(int i) {\n    if (names[i] == \"\") {\n        return \"\";\n    }\n    return names[i];\n}",
        "int F(int a, int b) {\n    int i = 0;\n    int wcb__in = 0;\n    WC3::unit_array units;\n    while (true) {\n        if (i >= 10 && (a != b || done)) break;\n        i = i + a / b;\n        names[i] = \"x\" + Name(a) + \"\\n\";\n        if (a == 'hfoo') {\n            return 0xFF;\n        } else if (b == 15) {\n            Jass::TimerStart(Jass::CreateTimer(), half, false, @Tick);\n        } else {\n        }\n    }\n    units[0] = nil;\n    wcb__in = Jass::PLAYER_NEUTRAL_AGGRESSIVE;\n    return i;\n}",
        "void Tick() {\n    StartTimerBJ(t, false, bj_PI);\n}",
        // Tick returns nothing: a BoolexprFunc of its own calls it.
        "void main() {\n    Jass::TriggerAddCondition(Jass::CreateTrigger(), Jass::Condition(@wcb__bool_Tick));\n}",
        "bool wcb__bool_Tick() {\n    Tick();\n    return false;\n}",
        "timer StartTimerBJ(timer t, bool periodic, float timeout) {\n    bj_lastStartedTimer = t;\n    Jass::TimerStart(t, timeout, periodic, null);\n    return bj_lastStartedTimer;\n}",
        "class string_array {",
        "class unit_array {",
    ] {
        assert!(out.contains(part), "{part}\nnot in:\n{out}");
    }
    // What isn't used of Blizzard.j isn't written.
    assert!(!out.contains("Unused") && !out.contains("bj_unused"), "{out}");
}

#[test]
fn code_as_ujapis_funcdefs() {
    let out = angelscript(r#"globals
    code array actions
    code check
endglobals
function Yes takes nothing returns boolean
    return true
endfunction
function Act takes nothing returns nothing
endfunction
function Hand takes code c returns conditionfunc
    return Condition(c)
endfunction
function main takes nothing returns nothing
    local code a = function Act
    set check = function Yes
    set actions[1] = function Yes
    call Hand(check)
    call Hand(function Act)
    call TimerStart(CreateTimer(), 1., false, actions[1])
    if a == null then
    endif
endfunction
"#);
    for part in [
        // Passed on to Condition: a BoolexprFunc, and what's given to it.
        "WC3::CallbackFunc_array actions;\nBoolexprFunc@ check = null;\n",
        "conditionfunc Hand(BoolexprFunc@ c) {\n    return Jass::Condition(c);\n}",
        "    CallbackFunc@ a = @Act;\n    @check = @Yes;\n    actions[1] = @wcb__void_Yes;\n    Hand(check);\n    Hand(@wcb__bool_Act);\n",
        "if (a is null) {",
        "void wcb__void_Yes() {\n    Yes();\n}",
        "class CallbackFunc_array {\n        private array<CallbackFunc@> v;",
        "@v[i] = @x;",
    ] {
        assert!(out.contains(part), "{part}\nnot in:\n{out}");
    }
}

#[test]
fn it_needs_common_j() {
    let m = module(&[Source { name: "t.j", text: "function main takes nothing returns nothing\nendfunction\n" }]).unwrap();
    let e = build_with(&m, &Options { script: Script::AngelScript, ..Default::default() }).unwrap_err();
    assert!(e[0].message.contains("common.j"), "{e:?}");
}
