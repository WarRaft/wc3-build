//! What `war3map.w3i` says of the game beyond its players, as World
//! Editor's script makes it: the upgrades each player starts with
//! (`InitUpgrades`), what each can't make (`InitTechTree`), the random
//! groups (`InitRandomGroups`, a group chosen for each and its units put in
//! `gg_rg_<number>`, which a random unit takes: `placed`) and the item
//! tables (`ItemTable<number>_DropItems`, which a thing that drops from one
//! runs when it dies). Called by [`crate::map::start`].

use w3i_rs::{Info, Rawcode};

use crate::ir::*;
use crate::objects::Objects;

const ORIGIN: &str = "the map";

fn function(name: impl Into<String>, body: Vec<Stmt>) -> Function {
    Function::new(name, body, Origin::file(ORIGIN))
}

/// A type in a random group: its id, none (`-1`) for an empty cell.
fn id(r: Rawcode) -> Expr {
    if r.is_none() || r.to_int() == u32::MAX {
        Expr::int(-1)
    } else if r.is_printable() {
        Expr::rawcode(&r.to_string())
    } else {
        Expr::int(r.to_int() as i32)
    }
}

fn player(number: i32) -> Expr {
    Expr::call("Player", vec![Expr::int(number)])
}

/// The players a mask has, of the map's, in its order.
fn players(info: &Info, mask: u32) -> impl Iterator<Item = i32> + '_ {
    info.players.iter().map(|p| p.number).filter(move |&n| (0..32).contains(&n) && mask & (1 << n) != 0)
}

/// A function for each player with any of `calls` (theirs, in order), and
/// the one calling them (`name`): none when no player has any.
fn per_player(info: &Info, name: &str, calls: impl Fn(i32) -> Vec<Stmt>) -> Vec<Function> {
    let mut out = Vec::new();
    let mut all = Vec::new();
    for p in &info.players {
        let b = calls(p.number);
        if b.is_empty() {
            continue;
        }
        let f = format!("{name}_Player{}", p.number);
        all.push(Stmt::call(f.clone(), Vec::new()));
        out.push(function(f, b));
    }
    if !all.is_empty() {
        out.push(function(name, all));
    }
    out
}

/// `InitUpgrades`: for each upgrade, by the order of its first entry, a
/// player's researched levels (`SetPlayerTechResearched`, the highest
/// researched level counting from 1) and the first level made
/// unavailable (`SetPlayerTechMaxAllowed`, counting from 0).
pub fn upgrades(info: &Info) -> Vec<Function> {
    per_player(info, "InitUpgrades", |n| {
        // (id, researched?) → the level written, in the order first met.
        let mut set: Vec<((Rawcode, bool), i32)> = Vec::new();
        for u in info.upgrades.iter().filter(|u| players(info, u.players).any(|p| p == n)) {
            let (researched, level) = match u.availability {
                2 => (true, u.level + 1),
                0 => (false, u.level),
                _ => continue,
            };
            match set.iter_mut().find(|(k, _)| *k == (u.id, researched)) {
                Some((_, l)) => *l = if researched { (*l).max(level) } else { (*l).min(level) },
                None => set.push(((u.id, researched), level)),
            }
        }
        set.into_iter()
            .map(|((r, researched), level)| {
                let call = if researched { "SetPlayerTechResearched" } else { "SetPlayerTechMaxAllowed" };
                Stmt::call(call, vec![player(n), id(r), Expr::int(level)])
            })
            .collect()
    })
}

/// `InitTechTree`: what each player can't make (`SetPlayerTechMaxAllowed`
/// 0), in the file's order.
pub fn tech_tree(info: &Info) -> Vec<Function> {
    per_player(info, "InitTechTree", |n| {
        info.techs
            .iter()
            .filter(|t| players(info, t.players).any(|p| p == n))
            .map(|t| Stmt::call("SetPlayerTechMaxAllowed", vec![player(n), id(t.id), Expr::int(0)]))
            .collect()
    })
}

/// The global of a random group, by its place among the map's (not its
/// number): an id per column.
pub fn group_name(index: usize) -> String {
    format!("gg_rg_{index:03}")
}

/// `InitRandomGroups`: for each group a row chosen by its chances
/// (`RandomDistChoose`, the row's index), and its ids put in the group's
/// global (`-1` for an empty cell, all `-1` for no row); with the globals.
pub fn random_groups(info: &Info) -> Option<(Function, Vec<Global>)> {
    if info.unit_tables.is_empty() {
        return None;
    }
    let mut b = vec![Stmt::local("integer", "curset", None)];
    let mut globals = Vec::new();
    for (index, t) in info.unit_tables.iter().enumerate() {
        let name = group_name(index);
        let mut g = Global::new(name.clone(), "integer", None, Origin::file(ORIGIN));
        g.array = true;
        globals.push(g);
        b.push(Stmt::call("RandomDistReset", Vec::new()));
        for (i, (chance, _)) in t.rows.iter().enumerate() {
            b.push(Stmt::call("RandomDistAddItem", vec![Expr::int(i), Expr::int(*chance)]));
        }
        b.push(Stmt::set("curset", Expr::call("RandomDistChoose", Vec::new())));
        let columns = t.columns.len();
        let fill = |ids: &mut dyn Iterator<Item = Expr>| -> Vec<Stmt> {
            ids.take(columns).enumerate().map(|(c, v)| Stmt::set_at(name.clone(), Expr::int(c), v)).collect()
        };
        let branches: Vec<(Expr, Vec<Stmt>)> = t
            .rows
            .iter()
            .enumerate()
            .map(|(i, (_, ids))| {
                let cond = Expr::binary(Expr::var("curset"), BinOp::Eq, Expr::int(i));
                (cond, fill(&mut ids.iter().map(|&r| id(r)).chain(std::iter::repeat_with(|| Expr::int(-1)))))
            })
            .collect();
        let none = fill(&mut std::iter::repeat_with(|| Expr::int(-1)));
        if branches.is_empty() {
            b.extend(none);
        } else {
            b.push(Stmt::If { branches, otherwise: Some(none), line: 0 });
        }
    }
    Some((function("InitRandomGroups", b), globals))
}

/// `ItemTable<number>_DropItems` for each of the map's item tables, as a
/// thing's own drops (`placed`).
pub fn item_tables(info: &Info, objects: &Objects) -> Vec<Function> {
    info.item_tables
        .iter()
        .map(|t| {
            let sets: Vec<doo_rs::ItemSet> = t.sets.iter().map(|s| s.iter().map(|&(chance, r)| (r, chance.max(0) as u32)).collect()).collect();
            let sets = crate::placed::drop_sets(&sets).unwrap_or_default();
            crate::placed::drop_items(format!("ItemTable{:06}_DropItems", t.number), &sets, objects)
        })
        .collect()
}

#[cfg(test)]
#[path = "tables.test.rs"]
mod tests;
