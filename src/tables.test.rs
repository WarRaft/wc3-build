use super::*;
use w3i_rs::{ItemTable, Player, Tech, UnitTable, Upgrade};

/// The JASS built, each line trimmed.
fn built(m: &Module) -> String {
    crate::jass::build(m).unwrap().lines().map(str::trim).collect::<Vec<_>>().join("\n")
}

fn text(fs: &[Function]) -> String {
    built(&Module { functions: fs.to_vec(), ..Default::default() })
}

fn info() -> Info {
    let r = |s: &[u8; 4]| Rawcode(*s);
    let mut info = Info { version: 25, ..Default::default() };
    for n in [0, 3] {
        info.players.push(Player { number: n, ..Default::default() });
    }
    let up = |players, id, level, availability| Upgrade { players, id: r(id), level, availability };
    info.upgrades = vec![
        up(0b1001, b"Rhst", 0, 2),
        up(0b1001, b"Rhst", 1, 2),
        up(0b0001, b"Rhpm", 1, 0),
        up(0b0001, b"Rhpm", 2, 0),
        up(0b0001, b"Rhar", 0, 1),
        // Player 1 isn't the map's.
        up(0b0010, b"Remg", 0, 2),
    ];
    info.techs = vec![Tech { players: 0b1000, id: r(b"hkni") }];
    info.unit_tables = vec![UnitTable {
        number: 2,
        name: "Creeps".into(),
        columns: vec![0, 0],
        rows: vec![(60, vec![r(b"nfrl"), Rawcode::NONE]), (40, vec![r(b"nwlt")])],
    }];
    info.item_tables = vec![ItemTable { number: 7, name: "Drop".into(), sets: vec![vec![(50, r(b"YYI3"))], vec![]] }];
    info
}

#[test]
fn upgrades_and_tech_tree() {
    let info = info();
    let t = text(&upgrades(&info));
    assert!(t.contains("function InitUpgrades_Player0 takes nothing returns nothing\ncall SetPlayerTechResearched(Player(0), 'Rhst', 2)\ncall SetPlayerTechMaxAllowed(Player(0), 'Rhpm', 1)\nendfunction"), "{t}");
    assert!(t.contains("function InitUpgrades_Player3 takes nothing returns nothing\ncall SetPlayerTechResearched(Player(3), 'Rhst', 2)\nendfunction"), "{t}");
    assert!(!t.contains("Remg") && !t.contains("Rhar"), "{t}");
    assert!(t.contains("function InitUpgrades takes nothing returns nothing\ncall InitUpgrades_Player0()\ncall InitUpgrades_Player3()\nendfunction"), "{t}");
    let t = text(&tech_tree(&info));
    assert!(t.contains("call SetPlayerTechMaxAllowed(Player(3), 'hkni', 0)") && !t.contains("Player0"), "{t}");
    assert!(upgrades(&Info::default()).is_empty());
}

#[test]
fn random_groups_and_item_tables() {
    let info = info();
    let (f, globals) = random_groups(&info).unwrap();
    assert_eq!(globals[0].name, "gg_rg_000");
    assert!(globals[0].array);
    let m = Module { functions: vec![f], globals, ..Default::default() };
    let t = built(&m);
    assert!(t.contains("integer array gg_rg_000"), "{t}");
    assert!(t.contains("call RandomDistAddItem(0, 60)\ncall RandomDistAddItem(1, 40)\nset curset = RandomDistChoose()"), "{t}");
    assert!(t.contains("if curset == 0 then\nset gg_rg_000[0] = 'nfrl'\nset gg_rg_000[1] = -1\nelseif curset == 1 then\nset gg_rg_000[0] = 'nwlt'\nset gg_rg_000[1] = -1\nelse\nset gg_rg_000[0] = -1\nset gg_rg_000[1] = -1\nendif"), "{t}");
    let t = text(&item_tables(&info, &Objects::default()));
    assert!(t.contains("function ItemTable000007_DropItems"), "{t}");
    assert!(t.contains("call RandomDistAddItem(ChooseRandomItemEx(ITEM_TYPE_ANY, 3), 50)\ncall RandomDistAddItem(-1, 50)"), "{t}");
}
