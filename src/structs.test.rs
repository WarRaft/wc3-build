use crate::from_jass::{Source, module};

fn build(files: &[(&str, &str)]) -> Result<String, Vec<String>> {
    let sources: Vec<Source> = files.iter().map(|(name, text)| Source { name, text }).collect();
    let m = module(&sources).unwrap_or_else(|e| panic!("{e:?}"));
    let out = crate::jass::build(&m).map_err(|e| e.into_iter().map(|e| e.message).collect::<Vec<_>>())?;
    as_the_games(&out);
    Ok(out.lines().map(str::trim).collect::<Vec<_>>().join("\n"))
}

/// The game's declarations the tests' scripts use.
pub(crate) const COMMON: &str = "\
type agent extends handle
type timer extends agent
native CreateTimer takes nothing returns timer
native TimerStart takes timer whichTimer, real timeout, boolean periodic, code handlerFunc returns nothing
native ExecuteFunc takes string funcName returns nothing
native I2S takes integer i returns string
native DoNothing takes nothing returns nothing
native BJDebugMsg takes string msg returns nothing
native ModuloInteger takes integer dividend, integer divisor returns integer
native InitBlizzard takes nothing returns nothing
type trigger extends agent
type boolexpr extends agent
type conditionfunc extends boolexpr
type triggercondition extends agent
type triggeraction extends handle
native CreateTrigger takes nothing returns trigger
native TriggerEvaluate takes trigger whichTrigger returns boolean
native TriggerExecute takes trigger whichTrigger returns nothing
native Condition takes code func returns conditionfunc
native TriggerAddCondition takes trigger whichTrigger, boolexpr condition returns triggercondition
native TriggerAddAction takes trigger whichTrigger, code actionFunc returns triggeraction
native TriggerSleepAction takes real timeout returns nothing
";

/// `out`, as the game reads it: no rule of `pure`, names and types right
/// over [`COMMON`].
pub(crate) fn as_the_games(out: &str) {
    let program = jass_rs::parse(out).unwrap_or_else(|e| panic!("line {}: {}\n{out}", e.line, e.message));
    let pure: Vec<_> = jass_rs::lint(&program)
        .into_iter()
        .filter(|d| parser_rs::find_rule(d.rule).is_some_and(|r| r.groups.contains(&parser_rs::group::PURE)) || d.rule == parser_rs::rule::DUPLICATE_DECLARATION)
        .collect();
    assert!(pure.is_empty(), "{pure:?}\n{out}");
    let common = jass_rs::names::Declarations::of(&jass_rs::parse(COMMON).unwrap());
    let problems = jass_rs::names::check(&program, &[&common]);
    assert!(problems.is_empty(), "{problems:?}\n{out}");
}

const COUNTERS: &str = r#"// @dialect vjass
library Counters
    module Counted
        static integer made = 0
        static method onInit takes nothing returns nothing
            set thistype.made = 0
        endmethod
    endmodule

    struct Counter[100]
        integer value = 5
        real ratio
        integer array marks[4]
        static Counter last
        implement Counted

        static method create takes integer v returns thistype
            local thistype c = thistype.allocate()
            set c.value = v
            set c.marks[2] = c.value + c.marks[1]
            set thistype.made = thistype.made + 1
            set Counter.last = c
            return c
        endmethod

        method twice takes nothing returns integer
            return .value * 2
        endmethod

        method onDestroy takes nothing returns nothing
            set .value = 0
        endmethod

        static method onInit takes nothing returns nothing
            call TimerStart(CreateTimer(), 1., true, function thistype.tick)
        endmethod

        static method tick takes nothing returns nothing
            call Counter.last.destroy()
        endmethod
    endstruct

    struct Plain extends array
        integer x
        static method get takes integer i returns thistype
            return thistype(i)
        endmethod
    endstruct

    function Use takes nothing returns integer
        local Counter c = Counter.create(3)
        local integer n = c.twice() + Counter(7).value + Plain.get(2).x
        call c.destroy()
        return n
    endfunction
endlibrary
"#;

#[test]
fn structs_into_jass() {
    let out = build(&[("c.j", COUNTERS)]).unwrap_or_else(|e| panic!("{e:?}"));
    for part in [
        // Fields as arrays, static ones as globals; the allocator's.
        "integer array s__Counter_value",
        "real array s__Counter_ratio",
        "integer array s__Counter_marks",
        "integer s__Counter_last",
        "integer s__Counter_made = 0",
        "integer si__Counter_F = 0",
        "integer array si__Counter_V",
        "integer array s__Plain_x",
        // A field's value set when allocated; past its size, none.
        "function s__Counter__allocate takes nothing returns integer\nlocal integer this = si__Counter_F\nif this != 0 then\nset si__Counter_F = si__Counter_V[this]\nelse\nset si__Counter_I = si__Counter_I + 1\nset this = si__Counter_I\nendif\nif this >= 100 then\nreturn 0\nendif\nset si__Counter_V[this] = -1\nset s__Counter_value[this] = 5\nreturn this\nendfunction",
        "function s__Counter_destroy takes integer this returns nothing\nif this != 0 and si__Counter_V[this] == -1 then\ncall s__Counter_onDestroy(this)\nset si__Counter_V[this] = si__Counter_F\nset si__Counter_F = this\nendif\nendfunction",
        // Methods, `this` first; a field array of 4 by `this * 4 + i`.
        "function s__Counter_create takes integer v returns integer\nlocal integer c = s__Counter__allocate()\nset s__Counter_value[c] = v\nset s__Counter_marks[c * 4 + 2] = s__Counter_value[c] + s__Counter_marks[c * 4 + 1]",
        "set s__Counter_made = s__Counter_made + 1\nset s__Counter_last = c\nreturn c",
        "function s__Counter_twice takes integer this returns integer\nreturn s__Counter_value[this] * 2",
        "call TimerStart(CreateTimer(), 1., true, function s__Counter_tick)",
        "call s__Counter_destroy(s__Counter_last)",
        // A cast is its value.
        "function s__Plain_get takes integer i returns integer\nreturn i",
        "local integer c = s__Counter_create(3)\nlocal integer n = s__Counter_twice(c) + s__Counter_value[7] + s__Plain_x[s__Plain_get(2)]\ncall s__Counter_destroy(c)",
        // The module's `onInit` and the struct's both run.
        "call ExecuteFunc(\"s__Counter_onInit__Counted\")\ncall ExecuteFunc(\"s__Counter_onInit\")",
    ] {
        assert!(out.contains(part), "{part}\nnot in:\n{out}");
    }
}

#[test]
fn what_structs_cant_be() {
    let src = "// @dialect vjass\ninterface I\n    method f takes nothing returns nothing\nendinterface\nstruct S extends I\n    T d\n    static delegate T e\nendstruct\nstruct T\nendstruct\nfunction F takes nothing returns nothing\n    local integer i = 0\n    call i.g()\nendfunction\n";
    let found = build(&[("e.j", src)]).unwrap_err();
    assert_eq!(found, ["struct S: `delegate` e, static or an array isn't built into JASS yet", "struct S has no method f of interface I", ".g() of what isn't a struct (or whose type can't be told)"], "{found:#?}");
}

#[test]
fn interfaces_children_and_stubs_by_type() {
    let src = r#"// @dialect vjass
library Shapes
    interface Shape
        method area takes nothing returns real
        method name takes nothing returns string defaults "shape"
    endinterface

    struct Square extends Shape
        real side = 1.
        method area takes nothing returns real
            return .side * .side
        endmethod
        method name takes nothing returns string
            return "square"
        endmethod
    endstruct

    struct Circle extends Shape
        real r
        method area takes nothing returns real
            return 3.14 * .r * .r
        endmethod
        method onDestroy takes nothing returns nothing
            set .r = 0.
        endmethod
    endstruct

    struct Animal
        integer legs = 4
        stub method sound takes integer times returns string
            return "..."
        endmethod
        method walk takes nothing returns integer
            return .legs
        endmethod
        method onDestroy takes nothing returns nothing
            set .legs = 0
        endmethod
    endstruct

    struct Bird extends Animal
        integer wings = 2
        method sound takes integer times returns string
            return super.sound(times) + "tweet"
        endmethod
        method onDestroy takes nothing returns nothing
            set .wings = 0
        endmethod
    endstruct

    function Use takes nothing returns real
        local Shape s = Square.create()
        local Animal a = Bird.create()
        local real x = s.area()
        call BJDebugMsg(s.name() + a.sound(2) + I2S(a.walk() + a.getType() + Bird.typeid))
        call s.destroy()
        call a.destroy()
        return x
    endfunction
endlibrary
"#;
    let out = build(&[("s.j", src)]).unwrap_or_else(|e| panic!("{e:?}"));
    for part in [
        // One free list a family, the instance's type kept.
        "integer si__Shape_F = 0",
        "integer array si__Shape_type",
        "integer array si__Animal_type",
        "function s__Square__allocate takes nothing returns integer\nlocal integer this = si__Shape_F",
        "set si__Shape_type[this] = 1\nset s__Square_side[this] = 1.\nreturn this",
        // A child's allocate: its type, its parent's fields and its own.
        "set si__Animal_type[this] = 2\nset s__Animal_legs[this] = 4\nset s__Bird_wings[this] = 2\nreturn this",
        // By the type: the arguments into globals, the type's trigger.
        "function wcb__d_Shape_area takes integer this returns real\nset wcb__da_Shape_area_0 = this\ncall TriggerEvaluate(wcb__dt_Shape_area[si__Shape_type[this]])\nreturn wcb__dr_Shape_area\nendfunction",
        // No method of its own: the interface's `defaults`.
        "if wcb__dt_Shape_name[si__Shape_type[this]] == null then\nreturn \"shape\"\nendif",
        "function wcb__dthunk_Circle_area takes nothing returns boolean\nset wcb__dr_Shape_area = s__Circle_area(wcb__da_Shape_area_0)\nreturn true",
        "function wcb__d_Animal_sound takes integer this, integer times returns string\nset wcb__da_Animal_sound_0 = this\nset wcb__da_Animal_sound_1 = times",
        // `super`: the parent's, straight.
        "return s__Animal_sound(this, times) + \"tweet\"",
        // Not a stub: the parent's, straight; the type; a struct's type.
        "s__Animal_walk(a) + si__Animal_type[a] + 2",
        "local real x = wcb__d_Shape_area(s)",
        // Destroyed by the type: the child's `onDestroy`, then the parent's.
        "function wcb__dthunk_Bird__destroy takes nothing returns boolean\ncall s__Bird_onDestroy(wcb__da_Animal__destroy)\ncall s__Animal_onDestroy(wcb__da_Animal__destroy)\nreturn true",
        "call s__Shape__destroy(s)\ncall s__Animal__destroy(a)",
        // The triggers made before anything runs.
        "set wcb__dt_Shape_area[1] = CreateTrigger()\ncall TriggerAddCondition(wcb__dt_Shape_area[1], Condition(function wcb__dthunk_Square_area))",
    ] {
        assert!(out.contains(part), "{part}\nnot in:\n{out}");
    }
    assert!(out.contains("wcb__dthunk_Square_name") && !out.contains("wcb__dthunk_Circle_name"), "{out}");
}

#[test]
fn evaluate_and_execute_through_triggers() {
    let src = r#"// @dialect vjass
library L
    function Twice takes integer x returns integer
        return x * 2
    endfunction
    function Late takes integer x, real r returns nothing
        call TriggerSleepAction(r)
        call BJDebugMsg(I2S(x))
    endfunction
    struct S
        integer v
        method add takes integer d returns integer
            set .v = .v + d
            return .v
        endmethod
    endstruct
    function Go takes nothing returns nothing
        local S s = S.create()
        local integer n = Twice.evaluate(3) + s.add.evaluate(1)
        call Late.execute(n, 2.)
    endfunction
endlibrary
"#;
    let out = build(&[("e.j", src)]).unwrap_or_else(|e| panic!("{e:?}"));
    for part in [
        // Above every function: the arguments into globals, the trigger.
        "function wcb__call_Twice takes integer x returns integer\nset wcb__arg_Twice_0 = x\ncall TriggerEvaluate(wcb__trig_Twice)\nreturn wcb__ret_Twice\nendfunction",
        "function wcb__call_s__S_add takes integer this, integer d returns integer\nset wcb__arg_s__S_add_0 = this\nset wcb__arg_s__S_add_1 = d",
        "function wcb__exec_Late takes integer x, real r returns nothing\nset wcb__arg_Late_0 = x\nset wcb__arg_Late_1 = r\ncall TriggerExecute(wcb__trig_Late)\nendfunction",
        // After each: what the trigger runs.
        "function wcb__thunk_Twice takes nothing returns boolean\nset wcb__ret_Twice = Twice(wcb__arg_Twice_0)\nreturn true",
        "function wcb__action_Late takes nothing returns nothing\ncall Late(wcb__arg_Late_0, wcb__arg_Late_1)\nendfunction",
        "local integer n = wcb__call_Twice(3) + wcb__call_s__S_add(s, 1)\ncall wcb__exec_Late(n, 2.)",
        // The triggers made in `main`.
        "call TriggerAddAction(wcb__trig_Late, function wcb__action_Late)",
    ] {
        assert!(out.contains(part), "{part}\nnot in:\n{out}");
    }
}

#[test]
fn function_interfaces_by_number() {
    let src = r#"// @dialect vjass
library F
    function interface Op takes integer x, S s returns integer
    struct S
        integer v
    endstruct
    function Twice takes integer x, S s returns integer
        return x * 2 + s.v
    endfunction
    function Thrice takes integer x, S s returns integer
        return x * 3
    endfunction
    function Go takes nothing returns nothing
        local Op o = Op.Twice
        local S s = S.create()
        local integer n = o.evaluate(1, s)
        set o = Op.Thrice
        call o.execute(n, s)
        set o = Op.Twice
    endfunction
endlibrary
"#;
    let out = build(&[("f.j", src)]).unwrap_or_else(|e| panic!("{e:?}"));
    for part in [
        "trigger array wcb__fit_Op",
        "integer wcb__fia_Op_1",
        "function wcb__fi_Op takes integer wcb__f, integer x, integer s returns integer\nset wcb__fia_Op_0 = x\nset wcb__fia_Op_1 = s\ncall TriggerEvaluate(wcb__fit_Op[wcb__f])\nreturn wcb__fir_Op\nendfunction",
        "function wcb__fx_Op takes integer wcb__f, integer x, integer s returns nothing",
        "function wcb__fithunk_Op_1 takes nothing returns boolean\nset wcb__fir_Op = Twice(wcb__fia_Op_0, wcb__fia_Op_1)\nreturn true",
        "function wcb__fiaction_Op_2 takes nothing returns nothing\ncall Thrice(wcb__fia_Op_0, wcb__fia_Op_1)",
        "local integer o = 1",
        "local integer n = wcb__fi_Op(o, 1, s)",
        "set o = 2",
        "call wcb__fx_Op(o, n, s)",
        "set wcb__fit_Op[2] = CreateTrigger()",
    ] {
        assert!(out.contains(part), "{part}\nnot in:\n{out}");
    }
    assert!(!out.contains("wcb__fit_Op[3]"), "{out}");
}

#[test]
fn a_function_interface_takes_only_its_like() {
    let src = r#"// @dialect vjass
library F
    function interface Op takes integer x returns integer
    function Real takes real x returns integer
        return 0
    endfunction
    function Go takes nothing returns nothing
        local Op o = Op.Real
        set o = Op.Nothing
    endfunction
endlibrary
"#;
    let e = build(&[("f.j", src)]).unwrap_err();
    assert!(e.iter().any(|m| m.contains("Op.Real: Real doesn't take and return what Op does")), "{e:?}");
    assert!(e.iter().any(|m| m.contains("Op.Nothing: no function Nothing")), "{e:?}");
}

#[test]
fn operators_as_jasshelpers() {
    let src = r#"// @dialect vjass
library O
    struct V
        integer raw
        integer array cell[4]
        static integer made = 0
        method operator x takes nothing returns integer
            return .raw
        endmethod
        method operator x= takes integer v returns nothing
            set .raw = v
        endmethod
        method operator [] takes integer i returns integer
            return .cell[i]
        endmethod
        method operator []= takes integer i, integer v returns nothing
            set .cell[i] = v
        endmethod
        method operator < takes V other returns boolean
            return .raw < other.raw
        endmethod
        static method operator count takes nothing returns integer
            return thistype.made
        endmethod
    endstruct
    struct ById extends array
        static method operator [] takes integer id returns thistype
            return id - 100
        endmethod
    endstruct
    struct Holder
        V v
    endstruct
    function Go takes nothing returns nothing
        local V a = V.create()
        local V b = V.create()
        local V array list
        local Holder h = Holder.create()
        local ById k = ById[105]
        set a.x = 3
        set b.x = a.x + 1
        set a[2] = a[1]
        set h.v[0] = h.v[1]
        set list[0] = a
        if a < b or a > b or a <= b or a >= b then
            call BJDebugMsg(I2S(V.count))
        endif
    endfunction
endlibrary
"#;
    let out = build(&[("o.j", src)]).unwrap_or_else(|e| panic!("{e:?}"));
    for part in [
        "function s__V__get_x takes integer this returns integer",
        "function s__V__set_x takes integer this, integer v returns nothing",
        "function s__V__getindex takes integer this, integer i returns integer",
        "function s__V__setindex takes integer this, integer i, integer v returns nothing",
        "function s__V__lt takes integer this, integer other returns boolean\nreturn s__V_raw[this] < s__V_raw[other]",
        "function s__V__get_count takes nothing returns integer",
        "local integer k = s__ById__getindex(105)",
        "call s__V__set_x(a, 3)",
        "call s__V__set_x(b, s__V__get_x(a) + 1)",
        "call s__V__setindex(a, 2, s__V__getindex(a, 1))",
        "call s__V__setindex(s__Holder_v[h], 0, s__V__getindex(s__Holder_v[h], 1))",
        "set list[0] = a",
        "if s__V__lt(a, b) or s__V__lt(b, a) or not s__V__lt(b, a) or not s__V__lt(a, b) then",
        "call BJDebugMsg(I2S(s__V__get_count()))",
    ] {
        assert!(out.contains(part), "{part}\nnot in:\n{out}");
    }
}

#[test]
fn what_a_delegate_has() {
    let src = r#"// @dialect vjass
library D
    struct Inner
        integer hp
        method operator mana takes nothing returns integer
            return 5
        endmethod
        method operator mana= takes integer v returns nothing
        endmethod
        method heal takes integer by returns nothing
            set .hp = .hp + by
        endmethod
    endstruct
    struct Middle
        delegate Inner inner
    endstruct
    struct Outer
        integer own
        delegate Middle middle
        method heal2 takes nothing returns nothing
            call .heal(2)
        endmethod
    endstruct
    function Go takes nothing returns nothing
        local Outer o = Outer.create()
        set o.hp = o.hp + o.own
        set o.mana = o.mana
        call o.heal(1)
    endfunction
endlibrary
"#;
    let out = build(&[("d.j", src)]).unwrap_or_else(|e| panic!("{e:?}"));
    for part in [
        "set s__Inner_hp[s__Middle_inner[s__Outer_middle[o]]] = s__Inner_hp[s__Middle_inner[s__Outer_middle[o]]] + s__Outer_own[o]",
        "call s__Inner__set_mana(s__Middle_inner[s__Outer_middle[o]], s__Inner__get_mana(s__Middle_inner[s__Outer_middle[o]]))",
        "call s__Inner_heal(s__Middle_inner[s__Outer_middle[o]], 1)",
        "call s__Inner_heal(s__Middle_inner[s__Outer_middle[this]], 2)",
    ] {
        assert!(out.contains(part), "{part}\nnot in:\n{out}");
    }
}

#[test]
fn hooks_run_before_what_they_hook() {
    let src = r#"// @dialect vjass
library H
    private function Log takes integer i returns nothing
        call BJDebugMsg(I2S(i))
    endfunction
    struct Watch
        static method seen takes integer i returns nothing
            call BJDebugMsg(I2S(i + 1))
        endmethod
    endstruct
    hook BJDebugMsgInt Log
    hook BJDebugMsgInt Watch.seen
    function Go takes nothing returns nothing
        call BJDebugMsgInt(5)
    endfunction
endlibrary
function BJDebugMsgInt takes integer i returns nothing
    call BJDebugMsg(I2S(i))
endfunction
"#;
    let out = build(&[("h.j", src)]).unwrap_or_else(|e| panic!("{e:?}"));
    for part in [
        "function h__BJDebugMsgInt takes integer i returns nothing\ncall H__Log(i)\ncall s__Watch_seen(i)\ncall BJDebugMsgInt(i)\nendfunction",
        "function Go takes nothing returns nothing\ncall h__BJDebugMsgInt(5)",
    ] {
        assert!(out.contains(part), "{part}\nnot in:\n{out}");
    }
    // A hook taking something else: an error.
    let bad = "// @dialect vjass\nfunction W takes real r returns nothing\nendfunction\nfunction V takes integer i returns nothing\nendfunction\nhook V W\nhook Nope W\n";
    let e = build(&[("b.j", bad)]).unwrap_err();
    assert_eq!(e, ["hook V W: W doesn't take what V takes", "hook Nope: no native or function Nope"]);
    // A native of the game's (given as the game's): its wrapper takes and gives back what it does.
    let m = crate::from_jass::module(&[crate::from_jass::Source { name: "n.j", text: "// @dialect vjass\nfunction Seen takes integer i returns nothing\nendfunction\nhook I2S Seen\nfunction F takes nothing returns string\n    return I2S(1)\nendfunction\n" }]).unwrap();
    let game = crate::from_jass::signatures(COMMON);
    let out = crate::jass::build_with(&m, &crate::jass::Options { game, ..Default::default() }).unwrap_or_else(|e| panic!("{e:?}")).script;
    assert!(out.contains("function h__I2S takes integer i returns string\n    call Seen(i)\n    return I2S(i)\nendfunction"), "{out}");
    assert!(out.contains("return h__I2S(1)"), "{out}");
}

#[test]
fn a_library_made_of_textmacros_as_jasshelper_reads_it() {
    let lib = r#"// @dialect vjass
library Store
//! novjass
    Store API:
        Ints[key] -> integer
//! endnovjass
    function LoadI takes integer p, integer k returns integer
        return p + k
    endfunction
    function SaveI takes integer p, integer k, integer v returns nothing
        call BJDebugMsg(I2S(p + k + v))
    endfunction
//! textmacro_once STORE takes NAME, TYPE, FUNC
    struct $NAME$ extends array
        method operator [] takes integer key returns $TYPE$
            return Load$FUNC$(this, key)
        endmethod
        method operator []= takes integer key, $TYPE$ value returns nothing
            call Save$FUNC$(this, key, value)
        endmethod
    endstruct
//! endtextmacro
//! runtextmacro STORE("Ints", "integer", "I")
//! runtextmacro optional STORE_REALS()
endlibrary
"#;
    let user = r#"// @dialect vjass
//! textmacro_once STORE takes NAME, TYPE, FUNC
    // A second declaration of a macro declared once: the first is kept.
//! endtextmacro
scope Use initializer Init
    private function Removed takes integer i returns nothing
        call BJDebugMsg("removing")
    endfunction
    hook SaveI Removed2
    private function Removed2 takes integer p, integer k, integer v returns nothing
    endfunction
    private function Init takes nothing returns nothing
        local Ints t = 5
        set t[3] = t[2] + 1
    endfunction
endscope
"#;
    let out = build(&[("store.j", lib), ("use.j", user)]).unwrap_or_else(|e| panic!("{e:?}"));
    for part in [
        "function s__Ints__getindex takes integer this, integer key returns integer\nreturn LoadI(this, key)",
        "function s__Ints__setindex takes integer this, integer key, integer value returns nothing\ncall h__SaveI(this, key, value)",
        "function h__SaveI takes integer p, integer k, integer v returns nothing\ncall Use__Removed2(p, k, v)\ncall SaveI(p, k, v)",
        "call s__Ints__setindex(t, 3, s__Ints__getindex(t, 2) + 1)",
    ] {
        assert!(out.contains(part), "{part}\nnot in:\n{out}");
    }
    assert!(!out.contains("Store API"), "{out}");
}

#[test]
fn array_struct_indexed_and_names() {
    let out = build(&[(
        "t.j",
        r#"// @dialect vjass
struct playerdata extends array
    integer a
endstruct
struct mystruct
    static method mymethod takes nothing returns nothing
    endmethod
endstruct
scope test
    public function xxx takes nothing returns nothing
        call BJDebugMsg(xxx.name)
    endfunction
endscope
function init takes nothing returns nothing
    set playerdata[3].a = playerdata[2].a
    call ExecuteFunc(mystruct.mymethod.name)
endfunction
"#,
    )])
    .unwrap();
    // `S[i]` of an array struct: instance i; `.name`: the name built.
    assert!(out.contains("set s__playerdata_a[3] = s__playerdata_a[2]"), "{out}");
    assert!(out.contains("call ExecuteFunc(\"s__mystruct_mymethod\")"), "{out}");
    assert!(out.contains("call BJDebugMsg(\"test_xxx\")"), "{out}");
}
