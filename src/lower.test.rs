use crate::from_jass::{Source, module};

/// The JASS built of `files`, each line trimmed.
fn built(files: &[(&str, &str)]) -> String {
    let sources: Vec<Source> = files.iter().map(|(name, text)| Source { name, text }).collect();
    let m = module(&sources).unwrap_or_else(|e| panic!("{e:?}"));
    let out = crate::jass::build(&m).unwrap_or_else(|e| panic!("{e:?}"));
    crate::structs::tests::as_the_games(&out);
    out.lines().map(str::trim).collect::<Vec<_>>().join("\n")
}

fn errors(files: &[(&str, &str)]) -> Vec<String> {
    let sources: Vec<Source> = files.iter().map(|(name, text)| Source { name, text }).collect();
    let m = module(&sources).unwrap_or_else(|e| panic!("{e:?}"));
    crate::jass::build(&m).unwrap_err().into_iter().map(|e| e.message).collect()
}

const VJASS: &str = r#"// @dialect vjass
library B initializer Init requires A
    globals
        private integer count = 0
        public constant boolean LOUD = true
    endglobals
    private function Init takes nothing returns nothing
        set count = A_Twice(count) + Helper()
        static if LIBRARY_A and B_LOUD then
            call BJDebugMsg(I2S(count))
        elseif LIBRARY_Missing then
            call BJDebugMsg("never")
        else
            call BJDebugMsg("quiet")
        endif
    endfunction
    scope Inner initializer Start
        private function Start takes nothing returns nothing
            set count = count + 1
        endfunction
    endscope
endlibrary

library A requires optional Missing
    public function Twice takes integer x returns integer
        return x * 2
    endfunction
    function Helper takes nothing returns integer
        local integer count = 3
        return count
    endfunction
endlibrary
"#;

#[test]
fn libraries_and_scopes_into_jass() {
    let out = built(&[("v.j", VJASS), ("m.j", "function main takes nothing returns nothing\n    call InitBlizzard()\nendfunction\n")]);
    for part in [
        // Private and public names; a local keeps its own.
        "integer B__count = 0",
        "constant boolean B_LOUD = true",
        "function A_Twice takes integer x returns integer",
        "function Helper takes nothing returns integer\nlocal integer count = 3\nreturn count",
        "function B__Init takes nothing returns nothing\nset B__count = A_Twice(B__count) + Helper()\ncall BJDebugMsg(I2S(B__count))\nendfunction",
        "function B_Inner__Start takes nothing returns nothing\nset B__count = B__count + 1",
        // A before B (B requires it), the scope's after (called straight);
        // right after `InitBlizzard()`.
        "call InitBlizzard()\ncall ExecuteFunc(\"B__Init\")\ncall B_Inner__Start()",
    ] {
        assert!(out.contains(part), "{part}\nnot in:\n{out}");
    }
    assert!(!out.contains("never") && !out.contains("quiet"), "{out}");
}

#[test]
fn zinc_libraries_into_jass() {
    let zinc = "library Z requires A {\n    integer n = 7;\n    public function Rest(integer a) -> integer { return a % n; }\n    function onInit() { n = Rest(10) + A_Twice(1); }\n}\n";
    let a = "// @dialect vjass\nlibrary A\n    public function Twice takes integer x returns integer\n        return x * 2\n    endfunction\nendlibrary\n";
    let out = built(&[("z.zn", zinc), ("a.j", a)]);
    for part in [
        "integer Z__n = 7",
        // Zinc's `public` is everyone's, as written.
        "function Rest takes integer a returns integer\nreturn ModuloInteger(a, Z__n)",
        "function Z__onInit takes nothing returns nothing\nset Z__n = Rest(10) + A_Twice(1)",
        "function main takes nothing returns nothing\ncall ExecuteFunc(\"Z__onInit\")",
    ] {
        assert!(out.contains(part), "{part}\nnot in:\n{out}");
    }
}

#[test]
fn what_libraries_cant_be() {
    let found = errors(&[(
        "e.j",
        "// @dialect vjass\nlibrary L initializer Go requires Nowhere\n    function Go takes integer x returns nothing\n    endfunction\nendlibrary\nlibrary P requires Q\nendlibrary\nlibrary Q requires P\nendlibrary\n",
    )]);
    assert_eq!(found, ["L requires Nowhere, which isn't there", "P requires itself, through the libraries it requires"], "{found:?}");
    let found = errors(&[("i.j", "// @dialect vjass\nlibrary L initializer Go\n    function Go takes integer x returns nothing\n    endfunction\nendlibrary\n")]);
    assert_eq!(found, ["L's initializer Go takes arguments: it can't run at initialization"]);
    let found = errors(&[("s.j", "// @dialect vjass\nfunction F takes integer x returns nothing\n    static if x > 1 then\n    endif\nendfunction\n")]);
    assert_eq!(found, ["a `static if` of what isn't a constant boolean"]);
}

#[test]
fn a_triggers_init_its_code_hasnt_isnt_called() {
    let code = "// @dialect vjass\nscope Lock initializer Start\n    private function Start takes nothing returns nothing\n    endfunction\nendscope\nfunction InitTrig_Other takes nothing returns nothing\nendfunction\n";
    let triggers = "trigger gg_trg_Lock = null\ntrigger gg_trg_Other = null\nfunction main takes nothing returns nothing\n    call InitTrig_Lock()\n    call InitTrig_Other()\nendfunction\n";
    let out = built(&[("t.j", triggers), ("c.j", code)]);
    assert!(out.contains("call InitTrig_Other()") && !out.contains("InitTrig_Lock"), "{out}");
}

/// vJASS as real libraries write it (three-corridors).
#[test]
fn real_libraries_vjass_built() {
    let text = concat!(
        "// @dialect vjass\n",
        "library Tools requires /* */ Base /* */ optional Missing\n",
        "    globals\n        private constant boolean VERBOSE = true\n    endglobals\n",
        "    static if DEBUG_MODE then\n",
        "        private function Say takes string s returns nothing\n            call BJDebugMsg(\"debug \" + s)\n        endfunction\n",
        "    elseif LIBRARY_Missing then\n",
        "        private function Say takes string s returns nothing\n            call Missing(s)\n        endfunction\n",
        "    elseif VERBOSE then\n",
        "        private function Say takes string s returns nothing\n            call BJDebugMsg(s)\n        endfunction\n",
        "    endif\n",
        "    function Hello takes nothing returns nothing\n        call Say(\"hi\")\n    endfunction\n",
        "endlibrary\n",
        "library Base\n    globals\n        constant boolean VERBOSE = true\n    endglobals\n",
        "    struct Node\n        debug boolean checked\n        integer v\n        static if VERBOSE then\n            static integer made = 0\n        else\n            static integer never = 0\n        endif\n",
        "        static method create takes nothing returns Node\n            debug local integer probe = 1\n            local Node this = Node.allocate()\n            set this.v = 1\n            return this\n        endmethod\n",
        "        method operator [] takes integer i returns integer\n            return this.v + i\n        endmethod\n",
        "        method operator []= takes integer i, integer x returns nothing\n            set this.v = i + x\n        endmethod\n",
        "        method twice takes nothing returns nothing\n            set Node(this)[1] = Node(this)[2]\n        endmethod\n",
        "    endstruct\n",
        "endlibrary\n",
        "function main takes nothing returns nothing\n    call Hello()\n    call Node.create()\nendfunction\n",
        "function config takes nothing returns nothing\nendfunction\n",
    );
    let m = crate::from_jass::module(&[crate::from_jass::Source { name: "t.j", text }]).unwrap_or_else(|e| panic!("{e:?}"));
    let out = crate::jass::build(&m).unwrap_or_else(|e| panic!("{e:?}"));
    crate::structs::tests::as_the_games(&out);
    // VERBOSE's branch: neither debug mode's nor the missing library's.
    assert!(out.contains("call BJDebugMsg(s)") && !out.contains("\"debug \"") && !out.contains("Missing("), "{out}");
    // No debug member: one array a field.
    assert!(!out.contains("checked") && !out.contains("probe"), "{out}");
    // A member's static if: its library's constant chose.
    assert!(out.contains("made") && !out.contains("never"), "{out}");
    // A cast's element set: its `[]=`, of its `[]`.
    assert!(out.contains("call s__Node__setindex(this, 1, s__Node__getindex(this, 2))"), "{out}");
}
