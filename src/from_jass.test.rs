use super::*;

#[test]
fn literals_as_written_and_errors_with_their_file() {
    let m = module(&[Source { name: "a.j", text: "integer x = 'hfoo' + $FF\nreal r = .5\nstring s = \"a\\\"b\"\n" }]).unwrap();
    let inits: Vec<String> = m.globals.iter().map(|g| format!("{:?}", g.init.as_ref().unwrap())).collect();
    assert_eq!(inits[0], "Binary(Literal(\"'hfoo'\"), Add, Literal(\"$FF\"))");
    assert_eq!(inits[1], "Literal(\".5\")");
    assert_eq!(inits[2], "Literal(\"\\\"a\\\\\\\"b\\\"\")");
    let e = module(&[Source { name: "ok.j", text: "function F\nendfunction\n" }, Source { name: "bad.j", text: "\n\nfunction" }]).unwrap_err();
    assert_eq!(e.len(), 1);
    assert_eq!((e[0].origin.file.as_str(), e[0].origin.line), ("bad.j", 3));
}

#[test]
fn vjass_goes_in_as_it_is() {
    let text = include_str!("../../jass-rs/tests/fixtures/vjass.j");
    let m = module(&[Source { name: "v.j", text }]).unwrap_or_else(|e| panic!("{e:?}"));
    let libs: Vec<(&str, bool, Option<&str>, Option<&str>)> = m.libraries.iter().map(|l| (l.name.as_str(), l.scope, l.parent.as_deref(), l.initializer.as_deref())).collect();
    assert_eq!(libs, [("Counters", false, None, Some("Init")), ("Counters_Inner", true, Some("Counters"), Some("Start"))]);
    let owned = |scope: &str, access| Owner { scope: Some(scope.into()), access };
    let globals: Vec<(&str, Owner)> = m.globals.iter().map(|g| (g.name.as_str(), g.owner.clone())).collect();
    assert_eq!(globals, [("count", owned("Counters", Access::Private)), ("MAX", owned("Counters", Access::Public)), ("sized", owned("Counters", Access::Everyone))]);
    let functions: Vec<(&str, Owner)> = m.functions.iter().map(|f| (f.sig.name.as_str(), f.owner.clone())).collect();
    assert_eq!(functions, [("Start", owned("Counters_Inner", Access::Private)), ("Init", owned("Counters", Access::Private))]);
    assert_eq!(m.function_types[0].sig.name, "Callback");
    assert_eq!(m.types[0].name, "IntList");
    assert!(m.types[0].size.is_some());
    assert_eq!(m.interfaces[0].methods[1].defaults, Some(Expr::lit("\"shape\"")));
    assert_eq!(m.mixins[0].name, "Pooled");

    let s = &m.structs[0];
    assert_eq!((s.name.as_str(), s.extends.as_deref(), s.size.clone()), ("Counter", Some("Shape"), Some(Expr::lit("20000"))));
    assert_eq!(s.mixins, [("Pooled".to_string(), false), ("Extra".to_string(), true)]);
    let list = s.fields.iter().find(|f| f.name == "list").unwrap();
    assert_eq!(list.array, Some(Some(Expr::lit("4"))));
    let create = &s.methods[0].function.body;
    let this_type = || Expr::var("thistype");
    assert_eq!(
        create[4],
        Stmt::SetMember { object: Expr::var("this"), name: "id".into(), index: None, value: Expr::Member(Box::new(this_type()), "made".into()), line: 38 }
    );
    // A real as written (`2.`).
    assert_eq!(
        s.methods[1].function.body[0],
        Stmt::Return { value: Some(Expr::binary(Expr::Member(Box::new(Expr::var("this")), "value".into()), BinOp::Mul, Expr::lit("2."))), line: 43 }
    );
    let start = &m.functions[0].body;
    assert!(matches!(&start[1], Stmt::CallMethod { call, .. } if call.name == "destroy"));
    assert!(matches!(&start[3], Stmt::StaticIf { cond, otherwise: Some(_), .. } if *cond == Expr::var("LIBRARY_TimerUtils")));
    // Not lowered yet.
    assert!(crate::jass::build(&m).is_err());
}

#[test]
fn jasshelpers_preprocessor_run_first() {
    // A textmacro of one file run in another; what's said of the code is
    // said of the line that ran it.
    let a = "// @dialect vjass\n//! textmacro T takes X\nfunction $X$ takes nothing returns nothing\n    call Nope$X$()\nendfunction\n//! endtextmacro\n";
    let b = "// @dialect vjass\n\n//! runtextmacro T(\"Made\")\n";
    let m = module(&[Source { name: "a.j", text: a }, Source { name: "b.j", text: b }]).unwrap_or_else(|e| panic!("{e:?}"));
    let f = m.functions.iter().find(|f| f.sig.name == "Made").expect("made");
    assert_eq!((f.origin.file.as_str(), f.origin.line), ("b.j", 3));
    assert!(matches!(&f.body[0], Stmt::Call { call, line: 3 } if call.name == "NopeMade"), "{:?}", f.body);
    // `//! inject main`: a function of its own, line for line.
    let m = module(&[Source { name: "t.j", text: "// @dialect vjass\n//! inject main\n//! dovjassinit\n//! endinject\n" }]).unwrap();
    assert_eq!((m.functions[0].sig.name.as_str(), m.functions[0].origin.line), ("wcb__inject_main", 2));
    // A table that isn't there: an error of the preprocessor's.
    let errors = module(&[Source { name: "t.j", text: "// @dialect vjass\n//! loaddata \"x.slk\"\n" }]).unwrap_err();
    assert_eq!((errors[0].origin.line, errors[0].message.as_str()), (2, "`//! loaddata`: no file \"x.slk\""));
    // Tools to run on the map: `external`, and a block with its input
    // (the `i` and a space off each `//! i` line; the rest left out).
    let text = concat!(
        "// @dialect vjass\n\n//! external ObjectMerger w3u hfoo h000 unam \"Captain\"\n",
        "//! externalblock extension=lua LuaMerger $FILENAME$ -v\n",
        "//! i print(1)\n    // not sent\n//! i\n//!   i   two\n//! endexternalblock\n",
        "function F takes nothing returns nothing\nendfunction\n",
    );
    let m = module(&[Source { name: "t.j", text }]).unwrap();
    let got: Vec<_> = m.externals.iter().map(|e| (e.tool.as_str(), e.args.as_str(), e.input.as_deref(), e.extension.as_deref(), e.origin.line)).collect();
    assert_eq!(got, [
        ("ObjectMerger", "w3u hfoo h000 unam \"Captain\"", None, None, 3),
        ("LuaMerger", "$FILENAME$ -v", Some("print(1)\n\n  two\n"), Some("lua"), 4),
    ]);
    // The code after the block is read, at its lines.
    assert_eq!(m.functions[0].origin.line, 10);
    let errors = module(&[Source { name: "t.j", text: "// @dialect vjass\n//! externalblock X\n//! i a\n" }]).unwrap_err();
    assert_eq!((errors[0].origin.line, errors[0].message.as_str()), (2, "`//! externalblock` has no `//! endexternalblock`"));
    let errors = module(&[Source { name: "t.j", text: "// @dialect vjass\n//! external\n" }]).unwrap_err();
    assert_eq!(errors[0].message, "`//! external` needs a tool's name");
}
