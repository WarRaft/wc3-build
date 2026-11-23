//! What a map places, as World Editor's script makes it (`war3map.doo`,
//! `war3mapUnits.doo`): `CreateAllDestructables`, `CreateAllItems`,
//! `CreateAllUnits` and the functions they call, and a `..._DropItems`
//! function for each thing that drops items when it dies. Called by
//! [`crate::map::start`].
//!
//! A thing is named (`gg_dest_B000_0261`, `gg_unit_hfoo_0012`,
//! `gg_item_ratc_0003`: its type and its number) when the rest of the code
//! uses that name; else it's made into a local, or not kept at all.

use std::collections::HashSet;

use doo_rs::{Doodad, ItemSet, Unit};
use parser_rs::Rawcode;

use crate::ir::*;
use crate::objects::{Objects, Placed};

/// A destructable's flag: the game makes it from `war3map.doo` (World
/// Editor clears it for those its script makes).
const MADE_BY_GAME: u8 = 0x2;
/// A destructable's flag: its height is its own, not the ground's.
const FIXED_Z: u8 = 0x4;

/// What World Editor leaves in a unit's gold that isn't a mine's.
const DEFAULT_GOLD: u32 = 12500;

/// Start locations: `config`'s, not made.
const START_LOCATION: Rawcode = Rawcode(*b"sloc");

/// How many players a map has before its neutral ones: 24 from 1.29.
pub fn max_players(info: &w3i_rs::Info) -> u32 {
    let [major, minor, ..] = info.game_version;
    if info.version >= 28 && (major, minor) >= (1, 29) { 24 } else { 12 }
}

/// What making the placed things needs: the map's files, what the types
/// are, the names the rest of the code uses, the regions (a waygate's
/// destination).
pub struct Placing<'a> {
    pub doodads: Option<&'a doo_rs::Doodads>,
    pub units: Option<&'a doo_rs::Units>,
    pub objects: &'a Objects,
    pub used: &'a HashSet<String>,
    pub regions: Option<&'a w3r_rs::Regions>,
    pub max_players: u32,
}

/// What's made: functions, the globals they set, the functions `main`
/// calls (in its order).
#[derive(Default)]
pub struct Made {
    pub functions: Vec<Function>,
    pub globals: Vec<Global>,
    pub calls: Vec<&'static str>,
    pub errors: Vec<Error>,
}

const ORIGIN: &str = "the map";

fn function(name: impl Into<String>, body: Vec<Stmt>) -> Function {
    Function::new(name, body, Origin::file(ORIGIN))
}

fn id(r: Rawcode) -> Expr {
    if r.is_printable() { Expr::rawcode(&r.to_string()) } else { Expr::int(r.to_int() as i32) }
}

fn coord(v: f32) -> Expr {
    Expr::real(v, 1)
}

/// An angle in radians as World Editor writes it: degrees, three
/// decimals.
fn degrees(radians: f32) -> Expr {
    Expr::real(radians.to_degrees(), 3)
}

/// A fraction of a percentage, two decimals (`0.10 * life`).
fn fraction(percent: i32) -> Expr {
    Expr::real(percent as f32 / 100.0, 2)
}

fn name(kind: &str, r: Rawcode, number: i32) -> String {
    format!("gg_{kind}_{r}_{number:04}")
}

fn error(message: String) -> Error {
    Error { origin: Origin::file(ORIGIN), message }
}

pub fn make(p: &Placing) -> Made {
    let mut made = Made::default();
    if let Some(d) = p.doodads {
        destructables(p, &d.doodads, &mut made);
    }
    if let Some(u) = p.units {
        items(p, &u.units, &mut made);
        units(p, &u.units, &mut made);
    }
    made
}

/// The trigger that drops a thing's items when it dies: `event` its
/// events, then `SaveDyingWidget` for a destructable, then `drops`.
fn drop_trigger(b: &mut Vec<Stmt>, me: &Expr, unit: bool, drops: String) {
    b.push(Stmt::set("t", Expr::call("CreateTrigger", Vec::new())));
    if unit {
        b.push(Stmt::call("TriggerRegisterUnitEvent", vec![Expr::var("t"), me.clone(), Expr::var("EVENT_UNIT_DEATH")]));
        b.push(Stmt::call("TriggerRegisterUnitEvent", vec![Expr::var("t"), me.clone(), Expr::var("EVENT_UNIT_CHANGE_OWNER")]));
    } else {
        b.push(Stmt::call("TriggerRegisterDeathEvent", vec![Expr::var("t"), me.clone()]));
        b.push(Stmt::call("TriggerAddAction", vec![Expr::var("t"), Expr::code("SaveDyingWidget")]));
    }
    b.push(Stmt::call("TriggerAddAction", vec![Expr::var("t"), Expr::code(drops)]));
}

/// An item of a set: its id, or a random item of a level and class
/// (`YlI1`: `Y`, the class (`Y` any, `i` permanent ...), `I`, the level);
/// none (`-1`) for one World Editor doesn't drop ([`Objects::droppable`]).
fn drop_item(r: Rawcode, objects: &Objects) -> Expr {
    const CLASSES: [&str; 8] = [
        "ITEM_TYPE_PERMANENT",
        "ITEM_TYPE_CHARGED",
        "ITEM_TYPE_POWERUP",
        "ITEM_TYPE_ARTIFACT",
        "ITEM_TYPE_PURCHASABLE",
        "ITEM_TYPE_CAMPAIGN",
        "ITEM_TYPE_MISCELLANEOUS",
        "ITEM_TYPE_UNKNOWN",
    ];
    if r.is_none() || r.to_int() == u32::MAX || !objects.droppable(r) {
        return Expr::int(-1);
    }
    let [a, class, i, level] = r.0;
    if a == b'Y' && i == b'I' && level.is_ascii_digit() {
        let level = Expr::int(level - b'0');
        let class = match class {
            b'Y' => Some("ITEM_TYPE_ANY"),
            b'i'..=b'p' => Some(CLASSES[(class - b'i') as usize]),
            _ => None,
        };
        if let Some(class) = class {
            return Expr::call("ChooseRandomItemEx", vec![Expr::var(class), level]);
        }
    }
    id(r)
}

/// The sets that drop anything: none when no set has an item.
fn drop_sets(sets: &[ItemSet]) -> Option<Vec<&ItemSet>> {
    let sets: Vec<&ItemSet> = sets.iter().filter(|s| !s.is_empty()).collect();
    (!sets.is_empty()).then_some(sets)
}

/// `<Kind><index>_DropItems`: one item of each set dropped where the thing
/// died, unless it's a hidden unit or one changing owner from any but
/// Neutral Hostile; a set short of 100 percent may drop none (`-1`).
fn drop_items(name: String, sets: &[&ItemSet], objects: &Objects) -> Function {
    let v = Expr::var;
    let null = || Expr::lit("null");
    let mut drops = Vec::new();
    for set in sets {
        drops.push(Stmt::call("RandomDistReset", Vec::new()));
        let mut total = 0;
        for &(item, chance) in set.iter() {
            total += chance;
            drops.push(Stmt::call("RandomDistAddItem", vec![drop_item(item, objects), Expr::int(chance)]));
        }
        if total < 100 {
            drops.push(Stmt::call("RandomDistAddItem", vec![Expr::int(-1), Expr::int(100 - total)]));
        }
        drops.push(Stmt::set("itemID", Expr::call("RandomDistChoose", Vec::new())));
        drops.push(Stmt::If {
            branches: vec![(
                Expr::binary(v("trigUnit"), BinOp::NotEq, null()),
                vec![Stmt::call("UnitDropItem", vec![v("trigUnit"), v("itemID")])],
            )],
            otherwise: Some(vec![Stmt::call("WidgetDropItem", vec![v("trigWidget"), v("itemID")])]),
            line: 0,
        });
    }
    let body = vec![
        Stmt::local("widget", "trigWidget", Some(null())),
        Stmt::local("unit", "trigUnit", Some(null())),
        Stmt::local("integer", "itemID", Some(Expr::int(0))),
        Stmt::local("boolean", "canDrop", Some(Expr::bool(true))),
        Stmt::set("trigWidget", v("bj_lastDyingWidget")),
        Stmt::If {
            branches: vec![(Expr::binary(v("trigWidget"), BinOp::Eq, null()), vec![Stmt::set("trigUnit", Expr::call("GetTriggerUnit", Vec::new()))])],
            otherwise: None,
            line: 0,
        },
        Stmt::If {
            branches: vec![(
                Expr::binary(v("trigUnit"), BinOp::NotEq, null()),
                vec![
                    Stmt::set("canDrop", Expr::Unary(UnaryOp::Not, Box::new(Expr::call("IsUnitHidden", vec![v("trigUnit")])))),
                    Stmt::If {
                        branches: vec![(
                            Expr::binary(v("canDrop"), BinOp::And, Expr::binary(Expr::call("GetChangingUnit", Vec::new()), BinOp::NotEq, null())),
                            vec![Stmt::set(
                                "canDrop",
                                Expr::binary(
                                    Expr::call("GetChangingUnitPrevOwner", Vec::new()),
                                    BinOp::Eq,
                                    Expr::call("Player", vec![v("PLAYER_NEUTRAL_AGGRESSIVE")]),
                                ),
                            )],
                        )],
                        otherwise: None,
                        line: 0,
                    },
                ],
            )],
            otherwise: None,
            line: 0,
        },
        Stmt::If { branches: vec![(v("canDrop"), drops)], otherwise: None, line: 0 },
        Stmt::set("bj_lastDyingWidget", null()),
        Stmt::call("DestroyTrigger", vec![Expr::call("GetTriggeringTrigger", Vec::new())]),
    ];
    function(name, body)
}

/// The destructables the script makes: those the game doesn't
/// (`war3map.doo`'s flag) and those the code names; the named first, each
/// by its type. Dead (no life) or with some of its life, its items
/// dropped by a trigger.
fn destructables(p: &Placing, doodads: &[Doodad], made: &mut Made) {
    let mut chosen: Vec<(bool, Rawcode, usize, &Doodad)> = Vec::new();
    for (i, d) in doodads.iter().enumerate() {
        let gg = name("dest", d.placement.id, d.number);
        let named = p.used.contains(&gg);
        if d.flags & MADE_BY_GAME == 0 || named {
            chosen.push((!named, d.placement.id, i, d));
        }
    }
    if chosen.is_empty() {
        return;
    }
    chosen.sort_by_key(|&(unnamed, r, i, _)| (unnamed, r, i));
    let mut b = vec![Stmt::local("destructable", "d", None), Stmt::local("trigger", "t", None), Stmt::local("real", "life", None)];
    for (unnamed, _, index, d) in chosen {
        let pl = &d.placement;
        let target = if unnamed {
            "d".to_string()
        } else {
            let gg = name("dest", pl.id, d.number);
            made.globals.push(Global::new(gg.clone(), "destructable", Some(Expr::lit("null")), Origin::file(ORIGIN)));
            gg
        };
        let me = Expr::var(target.clone());
        let dead = if d.life == 0 { "Dead" } else { "" };
        let mut args = vec![id(pl.id), coord(pl.x), coord(pl.y)];
        let call = if d.flags & FIXED_Z != 0 {
            args.push(coord(pl.z));
            format!("Create{dead}DestructableZ")
        } else {
            format!("Create{dead}Destructable")
        };
        args.extend([degrees(pl.angle), Expr::real(pl.scale[0], 3), Expr::int(pl.variation)]);
        b.push(Stmt::set(target, Expr::call(call, args)));
        if d.life > 0 && d.life < 100 {
            b.push(Stmt::set("life", Expr::call("GetDestructableLife", vec![me.clone()])));
            b.push(Stmt::call("SetDestructableLife", vec![me.clone(), Expr::binary(fraction(d.life.into()), BinOp::Mul, Expr::var("life"))]));
        }
        if d.item_table >= 0 {
            made.errors.push(error(format!("{}: drops from the map's item table {}, which isn't built yet", name("dest", pl.id, d.number), d.item_table)));
        } else if let Some(sets) = drop_sets(&d.item_sets) {
            let f = format!("Doodad{index:06}_DropItems");
            made.functions.push(drop_items(f.clone(), &sets, p.objects));
            drop_trigger(&mut b, &me, false, f);
        }
    }
    made.functions.push(function("CreateAllDestructables", b));
    made.calls.push("CreateAllDestructables");
}

/// The items placed: by type, then as placed.
fn items(p: &Placing, units: &[Unit], made: &mut Made) {
    let mut chosen: Vec<&Unit> = units.iter().filter(|u| u.placement.id != START_LOCATION && p.objects.placed(u.placement.id) == Placed::Item).collect();
    if chosen.is_empty() {
        return;
    }
    chosen.sort_by_key(|u| (u.placement.id, u.number));
    let mut b = vec![Stmt::local("integer", "itemID", None)];
    for u in chosen {
        let pl = &u.placement;
        let gg = name("item", pl.id, u.number);
        let create = Expr::call("CreateItem", vec![id(pl.id), coord(pl.x), coord(pl.y)]);
        if p.used.contains(&gg) {
            made.globals.push(Global::new(gg.clone(), "item", Some(Expr::lit("null")), Origin::file(ORIGIN)));
            b.push(Stmt::set(gg, create));
        } else {
            let Expr::Call(call) = create else { unreachable!() };
            b.push(Stmt::Call { call, line: 0 });
        }
    }
    made.functions.push(function("CreateAllItems", b));
    made.calls.push("CreateAllItems");
}

/// The units and buildings placed, a function for each owner's of each,
/// the buildings first: by their numbers.
fn units(p: &Placing, units: &[Unit], made: &mut Made) {
    let hostile = p.max_players;
    let passive = p.max_players + 3;
    // (owner, building) → units with their places in the file.
    let mut by: std::collections::BTreeMap<(u32, bool), Vec<(usize, &Unit)>> = Default::default();
    for (i, u) in units.iter().enumerate() {
        let kind = p.objects.placed(u.placement.id);
        if u.placement.id == START_LOCATION || kind == Placed::Item {
            continue;
        }
        by.entry((u.player, kind == Placed::Building)).or_default().push((i, u));
    }
    if by.is_empty() {
        return;
    }
    let fname = |owner: u32, building: bool| match (owner, building) {
        (o, true) if o == hostile => "CreateNeutralHostileBuildings".to_string(),
        (o, false) if o == hostile => "CreateNeutralHostile".to_string(),
        (o, true) if o == passive => "CreateNeutralPassiveBuildings".to_string(),
        (o, false) if o == passive => "CreateNeutralPassive".to_string(),
        (o, true) => format!("CreateBuildingsForPlayer{o}"),
        (o, false) => format!("CreateUnitsForPlayer{o}"),
    };
    let mut player_buildings = Vec::new();
    let mut player_units = Vec::new();
    for (&(owner, building), list) in &mut by {
        list.sort_by_key(|(_, u)| u.number);
        let owner_expr = if owner == hostile {
            Expr::var("PLAYER_NEUTRAL_AGGRESSIVE")
        } else if owner == passive {
            Expr::var("PLAYER_NEUTRAL_PASSIVE")
        } else {
            Expr::int(owner)
        };
        let mut b = vec![
            Stmt::local("player", "p", Some(Expr::call("Player", vec![owner_expr]))),
            Stmt::local("unit", "u", None),
            Stmt::local("integer", "unitID", None),
            Stmt::local("trigger", "t", None),
            Stmt::local("real", "life", None),
        ];
        for &(index, u) in list.iter() {
            unit(p, index, u, &mut b, made);
        }
        let f = fname(owner, building);
        if owner != hostile && owner != passive {
            if building { &mut player_buildings } else { &mut player_units }.push(Stmt::call(f.clone(), Vec::new()));
        }
        made.functions.push(function(f, b));
    }
    let mut all = Vec::new();
    let has = |owner: u32, building: bool| by.contains_key(&(owner, building));
    if has(hostile, true) {
        all.push(Stmt::call("CreateNeutralHostileBuildings", Vec::new()));
    }
    if has(passive, true) {
        all.push(Stmt::call("CreateNeutralPassiveBuildings", Vec::new()));
    }
    // The players' are made even when empty; the neutral ones' only when
    // they have any.
    made.functions.push(function("CreatePlayerBuildings", player_buildings));
    all.push(Stmt::call("CreatePlayerBuildings", Vec::new()));
    if has(hostile, false) {
        all.push(Stmt::call("CreateNeutralHostile", Vec::new()));
    }
    if has(passive, false) {
        all.push(Stmt::call("CreateNeutralPassive", Vec::new()));
    }
    made.functions.push(function("CreatePlayerUnits", player_units));
    all.push(Stmt::call("CreatePlayerUnits", Vec::new()));
    made.functions.push(function("CreateAllUnits", all));
    made.calls.push("CreateAllUnits");
}

/// One unit and what's set on it.
fn unit(p: &Placing, index: usize, u: &Unit, b: &mut Vec<Stmt>, made: &mut Made) {
    let pl = &u.placement;
    let gg = name("unit", pl.id, u.number);
    let target = if p.used.contains(&gg) {
        made.globals.push(Global::new(gg.clone(), "unit", Some(Expr::lit("null")), Origin::file(ORIGIN)));
        gg.clone()
    } else {
        "u".to_string()
    };
    let me = Expr::var(target.clone());
    let call = |name: &str, mut args: Vec<Expr>| {
        args.insert(0, me.clone());
        Stmt::call(name, args)
    };
    // Random ones are the map's random groups' and tables' (stage 2d).
    if [b"uDNR", b"bDNR", b"iDNR"].iter().any(|r| pl.id == Rawcode(**r)) {
        made.errors.push(error(format!("{gg}: a random unit, which isn't built yet")));
        return;
    }
    b.push(Stmt::set(target, Expr::call("CreateUnit", vec![Expr::var("p"), id(pl.id), coord(pl.x), coord(pl.y), degrees(pl.angle)])));
    let hero = pl.id.0[0].is_ascii_uppercase();
    if hero && u.level > 1 {
        b.push(call("SetHeroLevel", vec![Expr::int(u.level), Expr::bool(false)]));
    }
    if hero {
        for (f, v) in ["SetHeroStr", "SetHeroAgi", "SetHeroInt"].iter().zip(u.stats) {
            if v != 0 {
                b.push(call(f, vec![Expr::int(v), Expr::bool(true)]));
            }
        }
    }
    if u.hp >= 0 && u.hp != 100 {
        b.push(Stmt::set("life", Expr::call("GetUnitState", vec![me.clone(), Expr::var("UNIT_STATE_LIFE")])));
        b.push(call("SetUnitState", vec![Expr::var("UNIT_STATE_LIFE"), Expr::binary(fraction(u.hp), BinOp::Mul, Expr::var("life"))]));
    }
    if u.mana >= 0 {
        b.push(call("SetUnitState", vec![Expr::var("UNIT_STATE_MANA"), Expr::int(u.mana)]));
    }
    if u.gold != DEFAULT_GOLD {
        b.push(call("SetResourceAmount", vec![Expr::int(u.gold)]));
    }
    let color = if u.color >= 0 { Some(u.color) } else { p.objects.team_color(pl.id) };
    if let Some(c) = color {
        b.push(call("SetUnitColor", vec![Expr::call("ConvertPlayerColor", vec![Expr::int(c)])]));
    }
    if u.target_acquisition != -1.0 {
        let range = if u.target_acquisition == -2.0 { 200.0 } else { u.target_acquisition };
        b.push(call("SetUnitAcquireRange", vec![coord(range)]));
    }
    for &(ability, autocast, level) in &u.abilities {
        if hero && p.objects.is_hero_ability(ability) {
            for _ in 0..level {
                b.push(call("SelectHeroSkill", vec![id(ability)]));
            }
        }
        if let Some(order) = p.objects.order(ability, autocast != 0) {
            b.push(call("IssueImmediateOrder", vec![Expr::string(&order)]));
        }
    }
    for &(slot, item) in &u.items {
        b.push(call("UnitAddItemToSlotById", vec![id(item), Expr::int(slot)]));
    }
    if u.waygate >= 0 {
        match p.regions.and_then(|r| r.regions.iter().find(|g| g.index == u.waygate)) {
            Some(g) => {
                let rect = Expr::var(format!("gg_rct_{}", wtg_rs::jass::ident(&g.name)));
                b.push(call(
                    "WaygateSetDestination",
                    vec![Expr::call("GetRectCenterX", vec![rect.clone()]), Expr::call("GetRectCenterY", vec![rect])],
                ));
                b.push(call("WaygateActivate", vec![Expr::bool(true)]));
            }
            None => made.errors.push(error(format!("{gg}: a waygate to region {}, which the map hasn't", u.waygate))),
        }
    }
    if u.item_table >= 0 {
        made.errors.push(error(format!("{gg}: drops from the map's item table {}, which isn't built yet", u.item_table)));
    } else if let Some(sets) = drop_sets(&u.item_sets) {
        let f = format!("Unit{index:06}_DropItems");
        made.functions.push(drop_items(f.clone(), &sets, p.objects));
        drop_trigger(b, &me, true, f);
    }
}

#[cfg(test)]
#[path = "placed.test.rs"]
mod tests;
