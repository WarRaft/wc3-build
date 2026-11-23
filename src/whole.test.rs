use super::*;

fn files(read: &dyn Fn(&str) -> Option<Vec<u8>>, code: Vec<(&str, &str)>) -> Result<Script, Vec<Error>> {
    script(&Input { read, read_patched: &|_| None, variables: Vec::new(), initialization: vec!["gg_trg_Hello".into()], code: code.into_iter().map(|(n, t)| Code { name: n.into(), text: t.into(), dialect: jass_rs::Dialect::VJass }).collect(), uglify: false })
}

#[test]
fn a_whole_script_of_a_maps_files_and_its_code() {
    let w3i = w3i_rs::Info { version: 25, ..Default::default() }.to_bytes().unwrap();
    let read = move |name: &str| (name == "war3map.w3i").then(|| w3i.clone());
    let code = vec![
        ("Hello.j", "globals\n    trigger gg_trg_Hello = null\nendglobals\nfunction Trig_Hello takes nothing returns nothing\nendfunction\nfunction InitTrig_Hello takes nothing returns nothing\n    set gg_trg_Hello = CreateTrigger()\n    call TriggerAddAction(gg_trg_Hello, function Trig_Hello)\nendfunction\n"),
        ("code.j", "library L initializer I\n    private function I takes nothing returns nothing\n    endfunction\nendlibrary\n"),
    ];
    let s = files(&read, code).unwrap_or_else(|e| panic!("{e:?}"));
    for part in ["function config takes nothing returns nothing", "function main takes nothing returns nothing", "call ExecuteFunc(\"L__I\")", "call ConditionalTriggerExecute(gg_trg_Hello)"] {
        assert!(s.text.contains(part), "{part}\nnot in:\n{}", s.text);
    }
}

#[test]
fn no_map_no_script() {
    let e = files(&|_| None, Vec::new()).unwrap_err();
    assert_eq!(e[0].message, "the map has no war3map.w3i");
}
