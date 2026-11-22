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
