use crate::from_jass::{Source, module};
use crate::jass::{Options, build_with};

#[test]
fn what_nothing_uses_left_out_alone() {
    let text = "globals\n    integer used = 1\n    integer unused = 2\nendglobals\nfunction Small takes nothing returns integer\n    return used\nendfunction\nfunction Never takes nothing returns nothing\nendfunction\nfunction main takes nothing returns nothing\n    call BJDebugMsg(I2S(Small()))\nendfunction\n";
    let m = module(&[Source { name: "t.j", text }]).unwrap();
    let b = build_with(&m, &Options { dead_code: true, ..Default::default() }).unwrap();
    // Without inlining, the one-liner stays; what nothing reaches goes.
    assert!(b.script.contains("function Small") && !b.script.contains("function Never") && !b.script.contains("unused"), "{}", b.script);
    assert!(b.notes.iter().any(|n| n == "dead_code: 1 function(s) and 1 global(s) nothing uses left out"), "{:?}", b.notes);
}
