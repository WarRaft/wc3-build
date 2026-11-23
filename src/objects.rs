//! What the map's part of the script needs to know of the types of what's
//! placed, as World Editor finds it: whether a unit type is a building
//! (they're made first), whether a type is an item's, a type's team
//! color, which abilities are a hero's, and the orders that turn an
//! ability's autocast on and off. The game's tables go in first
//! ([`Objects::add_table`], [`Objects::add_ability_func`], each layer's
//! over the one below), then the map's object data
//! ([`Objects::add_object_data`]), whose custom objects take what their
//! standard one has.

use std::collections::{HashMap, HashSet};

use parser_rs::Rawcode;
use w3abdhqtu_rs::{Kind, ObjectData, Value};

#[derive(Debug, Clone, Default)]
pub struct Objects {
    buildings: HashSet<Rawcode>,
    items: HashSet<Rawcode>,
    item_levels: HashMap<Rawcode, i32>,
    team_colors: HashMap<Rawcode, i32>,
    hero_abilities: HashSet<Rawcode>,
    /// Unit types with the gold mine's ability (`Agld`).
    gold_mines: HashSet<Rawcode>,
    /// The game's abilities' orders: on, off.
    orders: HashMap<Rawcode, (String, String)>,
    /// The map's custom abilities, by their standard ones: World Editor
    /// writes a custom one's order whatever it is, `""` when its standard
    /// one has none.
    custom_abilities: HashMap<Rawcode, Rawcode>,
}

/// What a placed thing's type is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placed {
    Unit,
    Building,
    Item,
}

/// Whether a list of abilities has the gold mine's.
fn mines(list: &str) -> bool {
    list.split(',').any(|a| a.trim() == "Agld")
}

impl Objects {
    /// A table of the game's (`.slk`): `Units\UnitBalance.slk` (`isbldg`),
    /// `Units\unitUI.slk` (`teamColor`), `Units\ItemData.slk` (its ids are
    /// items'), `Units\UnitAbilities.slk` (`abilList`: gold mines),
    /// `Units\AbilityData.slk` (`hero`); any other is left alone.
    /// Each row is its first column's id.
    pub fn add_table(&mut self, slk: &str) {
        let doc = slk_rs::parse(slk);
        let table = slk_rs::Table::new(&doc);
        let head = |c: u32| table.text(1, c);
        let column = |name: &str| (1..=table.cols()).find(|&c| head(c).eq_ignore_ascii_case(name));
        let first = head(1);
        let rows = || (2..=table.rows()).filter_map(|r| Rawcode::parse(&table.text(r, 1)).map(|id| (r, id)));
        let truth = |v: String| v.trim() == "1";
        if first.eq_ignore_ascii_case("itemID") {
            self.items.extend(rows().map(|(_, id)| id));
            if let Some(c) = column("Level") {
                for (r, id) in rows() {
                    if let Ok(v) = table.text(r, c).trim().parse() {
                        self.item_levels.insert(id, v);
                    }
                }
            }
            return;
        }
        if first.eq_ignore_ascii_case("unitAbilID") {
            if let Some(c) = column("abilList") {
                for (r, id) in rows() {
                    if mines(&table.text(r, c)) {
                        self.gold_mines.insert(id);
                    } else {
                        self.gold_mines.remove(&id);
                    }
                }
            }
            return;
        }
        if first.eq_ignore_ascii_case("alias") {
            if let Some(c) = column("hero") {
                for (r, id) in rows() {
                    if truth(table.text(r, c)) {
                        self.hero_abilities.insert(id);
                    } else {
                        self.hero_abilities.remove(&id);
                    }
                }
            }
            return;
        }
        if let Some(c) = column("isbldg") {
            for (r, id) in rows() {
                if truth(table.text(r, c)) {
                    self.buildings.insert(id);
                } else {
                    self.buildings.remove(&id);
                }
            }
        }
        if let Some(c) = column("teamColor") {
            for (r, id) in rows() {
                match table.text(r, c).trim().parse::<i32>() {
                    Ok(v) => self.team_colors.insert(id, v),
                    Err(_) => self.team_colors.remove(&id),
                };
            }
        }
    }

    /// A text of the game's abilities (`Units\*AbilityFunc.txt`): each
    /// section's `Orderon` and `Orderoff`.
    pub fn add_ability_func(&mut self, text: &str) {
        let mut current: Option<Rawcode> = None;
        for line in text.lines() {
            let line = line.trim();
            if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                current = Rawcode::parse(name);
                continue;
            }
            let (Some(id), Some((k, v))) = (current, line.split_once('=')) else { continue };
            let v = v.trim().trim_matches('"').to_string();
            let entry = self.orders.entry(id).or_default();
            if k.trim().eq_ignore_ascii_case("Orderon") {
                entry.0 = v;
            } else if k.trim().eq_ignore_ascii_case("Orderoff") {
                entry.1 = v;
            }
        }
    }

    /// A map's object data (`war3map.w3u`, `.w3t`, `.w3a`): a custom
    /// object is what its standard one is, but for the fields it changes
    /// (`ubdg` building, `utco` team color, `aher` hero ability). In a
    /// Reign of Chaos map, `war3map.w3u` has the items too: a custom
    /// object whose standard one is an item is one.
    pub fn add_object_data(&mut self, data: &ObjectData) {
        let field = |o: &w3abdhqtu_rs::Object, name: &[u8; 4]| {
            o.sets.first().and_then(|s| s.mods.iter().rev().find(|m| m.field == Rawcode(*name))).map(|m| m.value.clone())
        };
        let int = |v: Option<Value>| match v {
            Some(Value::Int(i)) => Some(i),
            _ => None,
        };
        for (o, custom) in data.originals.iter().map(|o| (o, false)).chain(data.customs.iter().map(|o| (o, true))) {
            let id = if custom { o.id } else { o.base };
            match data.kind {
                Kind::Unit | Kind::Item => {
                    if custom {
                        if self.buildings.contains(&o.base) {
                            self.buildings.insert(id);
                        }
                        if self.items.contains(&o.base) || data.kind == Kind::Item {
                            self.items.insert(id);
                        }
                        if let Some(&l) = self.item_levels.get(&o.base) {
                            self.item_levels.insert(id, l);
                        }
                        if let Some(&c) = self.team_colors.get(&o.base) {
                            self.team_colors.insert(id, c);
                        }
                        if self.gold_mines.contains(&o.base) {
                            self.gold_mines.insert(id);
                        }
                    }
                    if let Some(Value::Str(list)) = field(o, b"uabi") {
                        if mines(&list) {
                            self.gold_mines.insert(id);
                        } else {
                            self.gold_mines.remove(&id);
                        }
                    }
                    if let Some(b) = int(field(o, b"ubdg")) {
                        if b != 0 {
                            self.buildings.insert(id);
                        } else {
                            self.buildings.remove(&id);
                        }
                    }
                    if let Some(l) = int(field(o, b"ilev")) {
                        self.item_levels.insert(id, l);
                    }
                    if let Some(c) = int(field(o, b"utco")) {
                        self.team_colors.insert(id, c);
                    }
                }
                Kind::Ability => {
                    if custom {
                        self.custom_abilities.insert(id, o.base);
                        if self.hero_abilities.contains(&o.base) {
                            self.hero_abilities.insert(id);
                        }
                    }
                    if let Some(h) = int(field(o, b"aher")) {
                        if h != 0 {
                            self.hero_abilities.insert(id);
                        } else {
                            self.hero_abilities.remove(&id);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    /// What a type placed is: an item's, a building's, else a unit's.
    pub fn placed(&self, id: Rawcode) -> Placed {
        if self.items.contains(&id) {
            Placed::Item
        } else if self.buildings.contains(&id) {
            Placed::Building
        } else {
            Placed::Unit
        }
    }

    /// The team color a unit type has of its own: none when it's -1 or
    /// unknown.
    pub fn team_color(&self, unit: Rawcode) -> Option<i32> {
        self.team_colors.get(&unit).copied().filter(|&c| c >= 0)
    }

    /// Whether an item goes into a set of items dropped as itself: World
    /// Editor writes one above level 10 as none (`-1`). Seen once (an item
    /// of level 50 in CruiserCommand); the limit is the levels its random
    /// items have.
    pub fn droppable(&self, item: Rawcode) -> bool {
        self.item_levels.get(&item).is_none_or(|&l| l <= 10)
    }

    /// Whether a unit type is a gold mine: World Editor sets its gold.
    pub fn is_gold_mine(&self, unit: Rawcode) -> bool {
        self.gold_mines.contains(&unit)
    }

    pub fn is_hero_ability(&self, ability: Rawcode) -> bool {
        self.hero_abilities.contains(&ability)
    }

    /// The order that turns an ability's autocast on (`on`) or off, as
    /// World Editor writes it: a standard ability's own, `None` when it has
    /// none; a custom ability's standard one's, `""` when that has none.
    pub fn order(&self, ability: Rawcode, on: bool) -> Option<String> {
        let get = |id: &Rawcode| self.orders.get(id).map(|(a, b)| if on { a.clone() } else { b.clone() }).unwrap_or_default();
        match self.custom_abilities.get(&ability) {
            Some(base) => Some(get(base)),
            None => Some(get(&ability)).filter(|o| !o.is_empty()),
        }
    }
}
