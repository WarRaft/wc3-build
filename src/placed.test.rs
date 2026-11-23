use super::*;
use crate::map::{GameData, Map, start};

fn slk(rows: &[&[&str]]) -> String {
    let mut s = String::from("ID;P\n");
    for (y, row) in rows.iter().enumerate() {
        for (x, cell) in row.iter().enumerate() {
            let v = if cell.parse::<f64>().is_ok() { cell.to_string() } else { format!("\"{cell}\"") };
            s.push_str(&format!("C;Y{};X{};K{v}\n", y + 1, x + 1));
        }
    }
    s + "E\n"
}

#[test]
fn what_is_placed() {
    let mut objects = Objects::default();
    objects.add_table(&slk(&[&["unitBalanceID", "isbldg"], &["hbar", "1"], &["hfoo", "0"]]));
    objects.add_table(&slk(&[&["unitUIID", "teamColor"], &["hfoo", "-1"], &["Hpal", "3"]]));
    objects.add_table(&slk(&[&["itemID", "Level"], &["ratc", "2"], &["mcri", "50"]]));
    objects.add_table(&slk(&[&["alias", "hero"], &["AHhb", "1"], &["Ahea", "0"]]));
    objects.add_ability_func("[Ahea]\nOrderon=healon\nOrderoff=healoff\n[AHhb]\nOrder=holybolt\n");

    let r = Rawcode::from;
    let mut tree = Doodad { placement: doo_rs::Placement::new(r(b"LTlt"), 64.0, -32.0), flags: 0, life: 50, item_table: -1, item_sets: vec![vec![(r(b"ratc"), 60), (r(b"mcri"), 20)], vec![]], number: 7 };
    tree.placement.angle = std::f32::consts::PI;
    let wall = Doodad { placement: doo_rs::Placement::new(r(b"LTw0"), 0.0, 0.0), flags: 2, life: 100, item_table: -1, item_sets: vec![], number: 12 };
    let gate = Doodad { placement: doo_rs::Placement::new(r(b"LTg1"), 0.0, 0.0), flags: 4 | 2, life: 0, item_table: -1, item_sets: vec![], number: 3 };
    let doodads = doo_rs::Doodads { version: 8, subversion: 11, layout: doo_rs::Layout::Classic, doodads: vec![tree, wall, gate], special_version: 0, specials: vec![] };

    let mut paladin = Unit::new(r(b"Hpal"), 10.0, 20.0, 0);
    paladin.number = 2;
    paladin.level = 3;
    paladin.abilities = vec![(r(b"AHhb"), 0, 2), (r(b"Ahea"), 1, 1)];
    paladin.items = vec![(0, r(b"ratc"))];
    let mut footman = Unit::new(r(b"hfoo"), 0.0, 0.0, 12);
    footman.number = 1;
    footman.target_acquisition = -2.0;
    footman.hp = 50;
    let mut barracks = Unit::new(r(b"hbar"), 0.0, 0.0, 0);
    barracks.number = 4;
    let mut claws = Unit::new(r(b"ratc"), 5.0, 5.0, 15);
    claws.number = 9;
    let start_location = Unit::new(r(b"sloc"), 0.0, 0.0, 0);
    // From the map's group numbered 5 (its first), column 1; dropping from
    // item table 2.
    let mut creep = Unit::new(r(b"uDNR"), 1.0, 1.0, 13);
    creep.number = 6;
    creep.random = Random::Group { group: 5, position: 1 };
    creep.item_table = 2;
    let mut mine = Unit::new(r(b"bDNR"), 2.0, 2.0, 15);
    mine.number = 7;
    mine.random = Random::Table(vec![(r(b"ngol"), 50), (r(b"ngme"), 50)]);
    let units = doo_rs::Units {
        version: 8,
        subversion: 11,
        layout: doo_rs::Layout::Classic,
        units: vec![paladin, footman, barracks, claws, start_location, creep, mine],
    };

    let used: HashSet<String> = ["gg_dest_LTg1_0003", "gg_unit_Hpal_0002"].map(String::from).into();
    let mut info = w3i_rs::Info { version: 25, ..Default::default() };
    info.unit_tables.push(w3i_rs::UnitTable { number: 5, name: "Creeps".into(), columns: vec![0, 0], rows: vec![(100, vec![r(b"nfrl"), r(b"nfrs")])] });
    info.item_tables.push(w3i_rs::ItemTable { number: 2, name: "Drops".into(), sets: vec![vec![(100, r(b"YYI1"))]] });
    let map = Map {
        info: &info,
        variables: &[],
        initialization: &[],
        regions: None,
        cameras: None,
        sounds: None,
        duration: &|_| None,
        doodads: Some(&doodads),
        units: Some(&units),
        objects: &objects,
        used: &used,
    };
    let out = crate::jass::build(&start(&map, &GameData::default()).unwrap()).unwrap();
    for part in [
        "    destructable gg_dest_LTg1_0003 = null\n",
        "    unit gg_unit_Hpal_0002 = null\n",
        // The game makes the wall; the gate is named, dead and has its own
        // height; the tree is half alive and drops one of its items, the
        // claws, the crown (of any level) or none, its empty set none.
        "    set gg_dest_LTg1_0003 = CreateDeadDestructableZ('LTg1', 0.0, 0.0, 0.0, 0.000, 1.000, 0)\n    set d = CreateDestructable('LTlt', 64.0, -32.0, 180.000, 1.000, 0)\n    set life = GetDestructableLife(d)\n    call SetDestructableLife(d, 0.50 * life)\n    set t = CreateTrigger()\n    call TriggerRegisterDeathEvent(t, d)\n    call TriggerAddAction(t, function SaveDyingWidget)\n    call TriggerAddAction(t, function Doodad000000_DropItems)\n",
        "        call RandomDistReset()\n        call RandomDistAddItem('ratc', 60)\n        call RandomDistAddItem('mcri', 20)\n        call RandomDistAddItem(-1, 20)\n        set itemID = RandomDistChoose()\n",
        "function CreateAllItems takes nothing returns nothing\n    local integer itemID\n    call CreateItem('ratc', 5.0, 5.0)\nendfunction\n",
        "    set gg_unit_Hpal_0002 = CreateUnit(p, 'Hpal', 10.0, 20.0, 0.000)\n    call SetHeroLevel(gg_unit_Hpal_0002, 3, false)\n    call SetUnitColor(gg_unit_Hpal_0002, ConvertPlayerColor(3))\n    call SelectHeroSkill(gg_unit_Hpal_0002, 'AHhb')\n    call SelectHeroSkill(gg_unit_Hpal_0002, 'AHhb')\n    call IssueImmediateOrder(gg_unit_Hpal_0002, \"healon\")\n    call UnitAddItemToSlotById(gg_unit_Hpal_0002, 'ratc', 0)\n",
        "function CreateNeutralHostile takes nothing returns nothing\n    local player p = Player(PLAYER_NEUTRAL_AGGRESSIVE)\n    local unit u\n    local integer unitID\n    local trigger t\n    local real life\n    set u = CreateUnit(p, 'hfoo', 0.0, 0.0, 0.000)\n    set life = GetUnitState(u, UNIT_STATE_LIFE)\n    call SetUnitState(u, UNIT_STATE_LIFE, 0.50 * life)\n    call SetUnitAcquireRange(u, 200.0)\nendfunction\n",
        "function CreateBuildingsForPlayer0 takes nothing returns nothing\n",
        // Random: their colors red; the creep drops its table's.
        "    set unitID = gg_rg_000[1]\n    if unitID != -1 then\n        set u = CreateUnit(p, unitID, 1.0, 1.0, 0.000)\n        call SetUnitColor(u, ConvertPlayerColor(0))\n        set t = CreateTrigger()\n        call TriggerRegisterUnitEvent(t, u, EVENT_UNIT_DEATH)\n        call TriggerRegisterUnitEvent(t, u, EVENT_UNIT_CHANGE_OWNER)\n        call TriggerAddAction(t, function ItemTable000002_DropItems)\n    endif\n",
        "    call RandomDistReset()\n    call RandomDistAddItem('ngol', 50)\n    call RandomDistAddItem('ngme', 50)\n    set unitID = RandomDistChoose()\n    if unitID != -1 then\n        set u = CreateUnit(p, unitID, 2.0, 2.0, 0.000)\n        call SetUnitColor(u, ConvertPlayerColor(0))\n    endif\n",
        "function ItemTable000002_DropItems takes nothing returns nothing\n",
        "    call InitRandomGroups()\n    call CreateAllDestructables()\n",
        "function CreateAllUnits takes nothing returns nothing\n    call CreateNeutralPassiveBuildings()\n    call CreatePlayerBuildings()\n    call CreateNeutralHostile()\n    call CreatePlayerUnits()\nendfunction\n",
        "    call CreateAllDestructables()\n    call CreateAllItems()\n    call CreateAllUnits()\n    call InitBlizzard()\n",
    ] {
        assert!(out.contains(part), "{part}\nnot in:\n{out}");
    }
    assert!(!out.contains("LTw0"), "{out}");
}
