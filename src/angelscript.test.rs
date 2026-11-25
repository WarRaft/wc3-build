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

/// What a hashtable takes, for structs kept as numbers.
const HASHTABLE: &str = "\
type hashtable extends agent
native InitHashtable takes nothing returns hashtable
native SaveInteger takes hashtable table, integer parentKey, integer childKey, integer value returns nothing
native LoadInteger takes hashtable table, integer parentKey, integer childKey returns integer
";

fn angelscript_vjass(text: &str) -> Result<String, Vec<String>> {
    let m = crate::from_jass::module_as(&[Source { name: "t.j", text }], jass_rs::Dialect::VJass).unwrap_or_else(|e| panic!("{e:?}"));
    let common = format!("{}{HASHTABLE}", crate::structs::tests::COMMON);
    let game = crate::from_jass::signatures(&common);
    let options = Options { script: Script::AngelScript, game, common_j: common, ..Default::default() };
    let out = build_with(&m, &options).map_err(|e| e.into_iter().map(|e| e.message).collect::<Vec<_>>())?.script;
    if let Err(e) = as_rs::parse(&out) {
        panic!("line {}: {}\n{out}", e.line, e.message);
    }
    Ok(out)
}

#[test]
fn structs_as_classes() {
    let out = angelscript_vjass(r#"
interface Shape
    method area takes nothing returns real
    method name takes nothing returns string defaults "shape"
endinterface
struct Square extends Shape
    real side = 1.
    method area takes nothing returns real
        return this.side * this.side
    endmethod
endstruct
struct Node
    static integer count = 0
    Node next
    integer value
    integer array marks[4]
    static method create takes integer v returns thistype
        local thistype c = thistype.allocate()
        set c.value = v
        set c.next = 0
        set thistype.count = thistype.count + 1
        set c.marks[2] = v
        return c
    endmethod
    method onDestroy takes nothing returns nothing
        set thistype.count = thistype.count - 1
    endmethod
    stub method describe takes nothing returns string
        return "node"
    endmethod
endstruct
struct Leaf extends Node
    method describe takes nothing returns string
        return "leaf " + super.describe()
    endmethod
endstruct
globals
    hashtable ht = InitHashtable()
endglobals
function Use takes nothing returns nothing
    local Node n = Node.create(5)
    local Node m
    local Shape s = Square.create()
    local real a = s.area()
    set n.next = Leaf.allocate()
    call SaveInteger(ht, 0, 0, n)
    set m = Node(LoadInteger(ht, 0, 0))
    if m == n and m.next != 0 then
        call BJDebugMsg(m.next.describe() + I2S(m.getType()) + I2S(Leaf.typeid))
    endif
    call BJDebugMsg(s.name())
    call m.destroy()
endfunction
"#)
    .unwrap_or_else(|e| panic!("{e:?}"));
    for part in [
        // An interface: a class with what a struct without a method gives.
        "class Shape {\n    int wcb__id = 0;\n    int wcb__type() { return 0; }\n    void wcb__onDestroy() {}\n    float area() { return 0.0f; }\n    string name() { return \"shape\"; }\n}",
        "class Square : Shape {\n    float side = 1.0f;\n    int wcb__type() { return 1; }\n    float area() {\n        return this.side * this.side;\n    }\n}",
        "class Node {\n    int wcb__id = 0;\n    Node@ next = null;\n    int value = 0;\n    WC3::int_array marks;\n    int wcb__type() { return 1; }\n    void wcb__onDestroy() {\n        onDestroy__Node();\n    }\n",
        "    void onDestroy__Node() {\n        s__Node_count = s__Node_count - 1;\n    }\n",
        "class Leaf : Node {\n    int wcb__type() { return 2; }\n    string describe() {\n        return \"leaf \" + Node::describe();\n    }\n}",
        // Static members: globals and functions.
        "int s__Node_count = 0;",
        "Node@ s__Node_create(int v) {\n    Node@ c = s__Node__allocate();\n    c.value = v;\n    @c.next = null;\n    s__Node_count = s__Node_count + 1;\n    c.marks[2] = v;\n    return c;\n}",
        "Node@ s__Node__allocate() {\n    Node@ o = Node();\n    s__Node__add(o);\n    return o;\n}",
        "Leaf@ s__Leaf__allocate() {\n    Leaf@ o = Leaf();\n    s__Node__add(o);\n    return o;\n}",
        "array<Shape@> si__Shape_all = {null};",
        // Objects and their numbers.
        "    Node@ n = s__Node_create(5);\n    Node@ m = null;\n    Shape@ s = s__Square__allocate();\n    float a = s.area();\n",
        "    @n.next = s__Leaf__allocate();\n    Jass::SaveInteger(ht, 0, 0, s__Node__id(n));\n    @m = cast<Node>(s__Node__at(Jass::LoadInteger(ht, 0, 0)));\n    if (m is n && !(m.next is null)) {\n        Jass::BJDebugMsg(m.next.describe() + Jass::I2S(m.wcb__type()) + Jass::I2S(2));\n    }\n    Jass::BJDebugMsg(s.name());\n    s__Node__destroy(m);\n",
    ] {
        assert!(out.contains(part), "{part}\nnot in:\n{out}");
    }
}

#[test]
fn instance_zero_is_no_object() {
    let e = angelscript_vjass("struct L\n    L next\n    static method f takes nothing returns L\n        return thistype(0).next\n    endmethod\nendstruct\n").unwrap_err();
    assert!(e.iter().any(|e| e.contains("instance 0")), "{e:?}");
}

#[test]
fn the_rest_of_vjass_as_classes() {
    let out = angelscript_vjass(r#"
function interface Op takes integer a returns integer
function Twice takes integer a returns integer
    return a * 2
endfunction
module Counted
    static integer made = 0
    static method onInit takes nothing returns nothing
        set thistype.made = 0
    endmethod
endmodule
struct Vec
    implement Counted
    real x
    method operator length takes nothing returns real
        return this.x
    endmethod
    method operator length= takes real v returns nothing
        set this.x = v
    endmethod
    method operator [] takes integer i returns real
        return this.x * i
    endmethod
    method operator < takes Vec other returns boolean
        return this.x < other.x
    endmethod
    method scale takes real k returns nothing
        set this.x = this.x * k
    endmethod
    static method tick takes nothing returns nothing
    endmethod
endstruct
struct Holder
    delegate Vec v
endstruct
function Use takes nothing returns nothing
    local Vec a = Vec.create()
    local Vec b = Vec.create()
    local Holder h = Holder.create()
    local Op f = Op.Twice
    set h.v = a
    set a.length = 2.
    call h.scale(3.)
    if a < b and a[2] > 1. then
        call BJDebugMsg(I2S(f.evaluate(4)))
    endif
    call a.scale.evaluate(2.)
    call Vec.tick.execute()
endfunction
"#)
    .unwrap_or_else(|e| panic!("{e:?}"));
    for part in [
        "    float wcb__get_length() {\n        return this.x;\n    }",
        "    void wcb__set_length(float v) {\n        this.x = v;\n    }",
        "    a.wcb__set_length(2.0f);\n    h.v.scale(3.0f);\n    if (a.wcb__lt(b) && a.wcb__getindex(2) > 1.0f) {\n        Jass::BJDebugMsg(Jass::I2S(wcb__fi_Op(f, 4)));\n    }\n    a.scale(2.0f);\n    wcb__exec_s__Vec_tick();\n",
        "@h.v = a;",
        "int s__Vec_made = 0;",
        // No ExecuteFunc in AngelScript: called straight.
        "    s__Vec_onInit__Counted();\n",
        "void wcb__exec_s__Vec_tick() {",
    ] {
        assert!(out.contains(part), "{part}\nnot in:\n{out}");
    }
}

#[test]
fn execute_func_called_straight() {
    let m = module(&[Source {
        name: "t.j",
        text: r#"globals
    constant string PREFIX = "Do"
    string chosen = "DoA"
endglobals
function DoA takes nothing returns nothing
endfunction
function DoB takes integer x returns nothing
endfunction
function main takes nothing returns nothing
    call ExecuteFunc("DoA")
    call ExecuteFunc(PREFIX + "A")
    call ExecuteFunc(chosen)
    call ExecuteFunc("DoB")
    call ExecuteFunc("Nowhere")
    call ExecuteFunc("StartTimerBJ")
endfunction
"#,
    }])
    .unwrap();
    let common = crate::structs::tests::COMMON.to_string();
    let game = crate::from_jass::signatures(&common).into_iter().chain(crate::from_jass::signatures(BLIZZARD)).collect();
    let built = build_with(&m, &Options { script: Script::AngelScript, game, common_j: common, blizzard_j: BLIZZARD.into(), ..Default::default() }).unwrap();
    let out = &built.script;
    // What's told before the game runs is a call; the rest is left, a problem each.
    assert!(out.contains("void main() {\n    DoA();\n    DoA();\n    Jass::ExecuteFunc(chosen);\n    Jass::ExecuteFunc(\"DoB\");\n    Jass::ExecuteFunc(\"Nowhere\");\n    Jass::ExecuteFunc(\"StartTimerBJ\");\n}"), "{out}");
    let lines: Vec<(usize, &str)> = built.problems.iter().map(|p| (p.origin.line, p.message.as_str())).collect();
    assert_eq!(lines.len(), 4, "{lines:?}");
    assert_eq!(lines[0].0, 12, "{lines:?}");
    assert!(lines[0].1.contains("told only as the game runs"), "{lines:?}");
    assert!(lines[1].1.contains("DoB takes arguments"), "{lines:?}");
    assert!(lines[2].1.contains("no function Nowhere"), "{lines:?}");
    assert!(lines[3].1.contains("StartTimerBJ takes arguments"), "{lines:?}");
}

#[test]
fn array_structs_execute_and_names() {
    let out = angelscript_vjass(r#"
struct playerdata extends array
    integer a
    integer b
    method sum takes nothing returns integer
        return this.a + this.b
    endmethod
endstruct
struct Fx
    real t
    method wait takes real extra returns nothing
        call TriggerSleepAction(this.t + extra)
    endmethod
endstruct
struct mystruct
    static method mymethod takes nothing returns nothing
        call BJDebugMsg("this works")
    endmethod
endstruct
scope test
    public function xxx takes nothing returns nothing
        call BJDebugMsg(xxx.name)
    endfunction
endscope
function init takes nothing returns nothing
    local playerdata pd
    local Fx f = Fx.create()
    set playerdata[3].a = 12
    set pd = playerdata[4]
    set pd.b = 111
    set playerdata[3].a = playerdata[3].b + pd.sum()
    call f.wait.execute(1.)
    call ExecuteFunc(mystruct.mymethod.name)
endfunction
"#)
    .unwrap_or_else(|e| panic!("{e:?}"));
    for part in [
        // An array struct: its fields arrays, its instances integers.
        "WC3::int_array s__playerdata_a;\nWC3::int_array s__playerdata_b;\n",
        // `this` is AngelScript's word: a function's parameter is prefixed.
        "int s__playerdata_sum(int wcb__this) {\n    return s__playerdata_a[wcb__this] + s__playerdata_b[wcb__this];\n}",
        "    int pd = 0;\n    Fx@ f = s__Fx__allocate();\n    s__playerdata_a[3] = 12;\n    pd = 4;\n    s__playerdata_b[pd] = 111;\n    s__playerdata_a[3] = s__playerdata_b[3] + s__playerdata_sum(pd);\n",
        // An object's method in a thread of its own: through a function taking it.
        "    wcb__exec_wcb__x_Fx_wait(f, 1.0f);\n",
        "void wcb__x_Fx_wait(Fx@ wcb__o, float extra) {\n    wcb__o.wait(extra);\n}",
        "Fx@ wcb__arg_wcb__x_Fx_wait_0 = null;",
        "void wcb__exec_wcb__x_Fx_wait(Fx@ wcb__o, float extra) {\n    @wcb__arg_wcb__x_Fx_wait_0 = wcb__o;\n    wcb__arg_wcb__x_Fx_wait_1 = extra;\n    Jass::TriggerExecute(wcb__trig_wcb__x_Fx_wait);\n}",
        // `.name`: the name built; ExecuteFunc of it a call.
        "    s__mystruct_mymethod();\n",
        "void test_xxx() {\n    Jass::BJDebugMsg(\"test_xxx\");\n}",
    ] {
        assert!(out.contains(part), "{part}\nnot in:\n{out}");
    }
    assert!(!out.contains("class playerdata"), "{out}");
}

#[test]
fn an_objects_method_has_no_name() {
    let e = angelscript_vjass("struct V\n    method m takes nothing returns nothing\n    endmethod\nendstruct\nfunction F takes nothing returns nothing\n    call ExecuteFunc(V.m.name)\nendfunction\n").unwrap_err();
    assert!(e.iter().any(|m| m.contains("V.m.name") && m.contains("static method instead")), "{e:?}");
}
