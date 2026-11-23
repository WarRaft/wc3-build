use crate::from_jass::{Source, module};
use crate::ir::*;

fn read(name: &str, text: &str) -> Module {
    module(&[Source { name, text }]).unwrap_or_else(|e| panic!("{e:?}"))
}

/// Statements as JASS writes them, each line trimmed (built alone, with
/// no library around them).
fn jass(m: &Module) -> String {
    crate::jass::build(m).unwrap_or_else(|e| panic!("{e:?}")).lines().map(str::trim).collect::<Vec<_>>().join("\n")
}

#[test]
fn outside_a_library_zinc_is_jass() {
    let m = read(
        "a.zn",
        r#"
integer total = 0;
constant real HALF = 0.5;

function Sum(integer n) -> integer {
    integer i, s = 0;
    for (0 <= i < n) { s += i; }
    for (n > i >= 0) { if (i == 3) { break; } else if (i > 5) { s -= 1; } else { s *= 2; } }
    while (s > 100 && i != 0) s /= 2;
    do { s = s + 'hfoo'; } while (s < 0);
    { debug BJDebugMsg("s: " + I2S(s)); }
    return s;
}

function Start() {
    TimerStart(CreateTimer(), 1.0, false, function() { total = Sum(4); });
}
"#,
    );
    let out = jass(&m);
    for part in [
        "integer total = 0",
        "constant real HALF = 0.5",
        // `for` up: from 0, out at n; down: from n - 1, out below 0.
        "set i = 0\nloop\nexitwhen i >= n\nset s = s + i\nset i = i + 1\nendloop",
        "set i = n - 1\nloop\nexitwhen i < 0\nif i == 3 then\nexitwhen true\nelseif i > 5 then\nset s = s - 1\nelse\nset s = s * 2\nendif\nset i = i - 1\nendloop",
        "loop\nexitwhen not (s > 100 and i != 0)\nset s = s / 2\nendloop",
        "loop\nset s = s + 'hfoo'\nexitwhen not (s < 0)\nendloop",
        "debug call BJDebugMsg(\"s: \" + I2S(s))",
        // The anonymous function, a function of its own above its user.
        "function wcb__anon_0 takes nothing returns nothing\nset total = Sum(4)\nendfunction",
        "call TimerStart(CreateTimer(), 1.0, false, function wcb__anon_0)",
    ] {
        assert!(out.contains(part), "{part}\nnot in:\n{out}");
    }
}

#[test]
fn libraries_and_structs_go_in_as_they_are() {
    let m = read(
        "b.zn",
        r#"
library Counters requires Utils, optional TimerUtils {
    integer counts[8];
    public constant integer MAX = 8;

    public struct Counter[100] extends array {
        integer value;
        static integer made = 0;
        private real ratio;
        module Pooled;

        static method create(integer v) -> thistype {
            thistype c = thistype.allocate();
            c.value = v;
            c.list[1] += 2;
            thistype.made += 1;
            return c;
        }
        method twice() -> integer { return this.value * 2 % 7; }
    }

    interface Shape { method area() -> real; }
    module Pooled { static method get() -> thistype { return 0; } }

    function onInit() {
        static if (LIBRARY_TimerUtils) { Counter.create(1).twice(); }
    }
}
"#,
    );
    let lib = &m.libraries[0];
    assert_eq!((lib.name.as_str(), lib.scope, lib.initializer.as_deref()), ("Counters", false, Some("onInit")));
    assert_eq!(lib.requires, vec![("Utils".to_string(), false), ("TimerUtils".to_string(), true)]);
    let owned = |access| Owner { scope: Some("Counters".into()), access };
    assert_eq!(m.globals.iter().map(|g| (g.name.as_str(), g.array, g.owner.clone())).collect::<Vec<_>>(), vec![("counts", true, owned(Access::Private)), ("MAX", false, owned(Access::Everyone))]);
    assert_eq!(m.functions[0].owner, owned(Access::Private));

    let s = &m.structs[0];
    assert_eq!((s.name.as_str(), s.extends.as_deref(), s.owner.clone()), ("Counter", Some("array"), owned(Access::Everyone)));
    assert_eq!(s.size, Some(Expr::lit("100")));
    assert_eq!(s.fields.iter().map(|f| (f.name.as_str(), f.is_static, f.private)).collect::<Vec<_>>(), vec![("value", false, false), ("made", true, false), ("ratio", false, true)]);
    assert_eq!(s.mixins, vec![("Pooled".to_string(), false)]);
    let create = &s.methods[0];
    assert!(create.is_static && !create.abstract_);
    let this = || Expr::var("thistype");
    assert_eq!(
        create.function.body[0],
        Stmt::Local { ty: "thistype".into(), name: "c".into(), array: false, init: Some(Expr::Method(Box::new(MethodCall { object: this(), name: "allocate".into(), args: vec![] }))), line: 13 }
    );
    let member = |o: Expr, n: &str| Expr::Member(Box::new(o), n.into());
    assert_eq!(create.function.body[1], Stmt::SetMember { object: Expr::var("c"), name: "value".into(), index: None, value: Expr::var("v"), line: 14 });
    assert_eq!(
        create.function.body[2],
        Stmt::SetMember {
            object: Expr::var("c"),
            name: "list".into(),
            index: Some(Expr::lit("1")),
            value: Expr::binary(Expr::Element(Box::new(member(Expr::var("c"), "list")), Box::new(Expr::lit("1"))), BinOp::Add, Expr::lit("2")),
            line: 15
        }
    );
    assert_eq!(
        create.function.body[3],
        Stmt::SetMember { object: this(), name: "made".into(), index: None, value: Expr::binary(member(this(), "made"), BinOp::Add, Expr::lit("1")), line: 16 }
    );
    let twice = &s.methods[1].function;
    assert_eq!(
        twice.body[0],
        Stmt::Return { value: Some(Expr::binary(Expr::binary(member(Expr::var("this"), "value"), BinOp::Mul, Expr::lit("2")), BinOp::Mod, Expr::lit("7"))), line: 19 }
    );
    assert!(m.interfaces[0].methods[0].abstract_);
    assert_eq!(m.mixins[0].name, "Pooled");
    let init = m.functions.iter().find(|f| f.sig.name == "onInit").unwrap();
    assert!(matches!(&init.body[0], Stmt::StaticIf { then, .. } if matches!(&then[0], Stmt::CallMethod { .. })));

    // Built: the library it requires isn't there.
    let errors = crate::jass::build(&m).unwrap_err();
    assert_eq!(errors.iter().map(|e| e.message.as_str()).collect::<Vec<_>>(), ["Counters requires Utils, which isn't there"]);
}

#[test]
fn zinc_in_vjass_too() {
    let m = read("c.j", "// @dialect vjass\nglobals\n    integer x = 1\nendglobals\n//! zinc\nfunction Twice(integer a) -> integer { return a * 2; }\n//! endzinc\n");
    assert_eq!(m.globals[0].name, "x");
    assert_eq!(m.functions[0].sig.name, "Twice");
    assert_eq!(m.functions[0].origin.line, 6);
}

#[test]
fn what_isnt_read_is_an_error() {
    let errors = module(&[Source { name: "d.zn", text: "integer grid[2][2];\nfunction F() { integer a[2][3]; }\n" }]).unwrap_err();
    let messages: Vec<(usize, &str)> = errors.iter().map(|e| (e.origin.line, e.message.as_str())).collect();
    assert_eq!(messages, [(1, "grid: an array of more than one dimension isn't read yet"), (2, "a: an array of more than one dimension isn't read yet")]);
}
