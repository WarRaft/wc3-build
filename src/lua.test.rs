use crate::from_jass::{Source, module};
use crate::jass::{Options, Script, build_with};

fn lua(text: &str) -> String {
    let m = module(&[Source { name: "t.j", text }]).unwrap_or_else(|e| panic!("{e:?}"));
    let game = crate::from_jass::signatures(crate::structs::tests::COMMON);
    let out = build_with(&m, &Options { script: Script::Lua, game, ..Default::default() }).unwrap_or_else(|e| panic!("{e:?}")).script;
    // Lua as Lua reads it.
    if let Err(e) = lua_rs::parse(&out) {
        panic!("line {}: {}\n{out}", e.line, e.message);
    }
    out
}

/// What `script` prints run by the `lua` on this machine (its natives
/// `BJDebugMsg` and `I2S` stand-ins), its `main` called; `None` without
/// one.
fn run(script: &str) -> Option<String> {
    let program = format!("function BJDebugMsg(s) print(s) end\nfunction I2S(i) return tostring(i) end\n{script}\nmain()\n");
    let dir = std::env::temp_dir().join(format!("wcb-lua-{}", std::process::id()));
    std::fs::create_dir_all(&dir).ok()?;
    let file = dir.join("t.lua");
    std::fs::write(&file, program).ok()?;
    let out = std::process::Command::new("lua").arg(&file).output().ok()?;
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

#[test]
fn it_runs_as_jass_would() {
    let out = lua(r#"globals
    string empty_s
endglobals
function main takes nothing returns nothing
    local integer a = -7
    call BJDebugMsg(I2S(a / 2))
    call BJDebugMsg(I2S(7 / 2))
    call BJDebugMsg("x" + empty_s + "y")
    if true and false or true then
        call BJDebugMsg("and looser")
    else
        call BJDebugMsg("or looser")
    endif
    call BJDebugMsg(I2S('A'))
endfunction
"#);
    let Some(printed) = run(&out) else { return };
    // JASS cuts toward zero, adds a null string as empty, binds `and` looser.
    assert_eq!(printed, "-3\n3\nxy\nand looser\n65\n", "{out}");
}

#[test]
fn jass_as_the_games_lua() {
    let out = lua(r#"globals
    integer count = 7
    real half = .5
    string array names
    boolean done
endglobals
function Name takes integer i returns string
    return names[i]
endfunction
function F takes integer a, integer b returns integer
    local integer i = 0
    local string s = "x" + Name(a)
    loop
        exitwhen i >= 10 and a != b or done
        set i = i + a / b
        if a == 'hfoo' then
            return $FF
        elseif b == 017 then
            call TimerStart(CreateTimer(), half, false, function Tick)
        else
            debug call BJDebugMsg("debugging")
        endif
    endloop
    return i
endfunction
function Tick takes nothing returns nothing
endfunction
"#);
    for part in [
        "count = 7\nhalf = 0.5\nnames = wcb__array(nil)\ndone = false\n",
        "function Name(i)\n    do return names[i] end\nend",
        "function F(a, b)\n    local i = 0\n    local s = \"x\" .. (Name(a) or \"\")\n    while true do\n        if i >= 10 and (a ~= b or done) then break end\n        i = i + wcb__div(a, b)\n        if a == 1751543663 then\n            do return 0xFF end\n        elseif b == 15 then\n            TimerStart(CreateTimer(), half, false, Tick)\n        else\n        end\n    end\n    do return i end\nend",
    ] {
        assert!(out.contains(part), "{part}\nnot in:\n{out}");
    }
}
