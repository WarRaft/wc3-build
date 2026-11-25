//! A map's whole script: the map's part ([`crate::map`], [`crate::placed`],
//! [`crate::tables`]) made of its files, the triggers and the code between
//! its start and its end, built into the game's JASS ([`crate::jass`]).
//!
//! It knows no folders and no projects: every file comes from `read`, the
//! project's lookup (the game's folders, then the map's roots, the last
//! that has a file winning), and the code comes as texts, already made
//! (the triggers' JASS) or as written (the map's custom script, the
//! project's code). The caller (`wc3-server`'s `build`) knows where each
//! is.

use std::collections::HashSet;

use crate::from_jass::{Source, module_with};
use crate::ir::{Error, Module, Origin};
use crate::map::{GameData, Map};
use crate::objects::Objects;

/// The game's tables of its sounds (`UI\SoundInfo`): what `GameData`
/// reads a sound's label from. Their names are the game's files', not
/// data in them.
const SOUND_TABLES: &[&str] = &[
    "AbilitySounds.slk",
    "AmbienceSounds.slk",
    "AnimSounds.slk",
    "DialogSounds.slk",
    "EnvironmentSounds.slk",
    "MIDISounds.slk",
    "UISounds.slk",
    "UnitAckSounds.slk",
    "UnitCombatSounds.slk",
];

/// The game's tables of the types `Objects` reads (`Units\`).
const TYPE_TABLES: &[&str] = &["UnitBalance.slk", "UnitUI.slk", "UnitAbilities.slk", "ItemData.slk", "AbilityData.slk"];

/// The game's abilities' texts `Objects` reads (`Units\`): their orders.
const ABILITY_FUNCS: &[&str] =
    &["HumanAbilityFunc.txt", "OrcAbilityFunc.txt", "UndeadAbilityFunc.txt", "NightElfAbilityFunc.txt", "NeutralAbilityFunc.txt", "CommonAbilityFunc.txt", "ItemAbilityFunc.txt", "CampaignAbilityFunc.txt"];

/// What a whole script is made of.
pub struct Input<'a> {
    /// A file by its name in the map or the game (`war3map.w3i`,
    /// `Units\UnitBalance.slk`), as the lookup finds it.
    pub read: &'a dyn Fn(&str) -> Option<Vec<u8>>,
    /// An editor's file of text with the project's patches on it
    /// (`UI\WorldEditData.txt`).
    pub read_patched: &'a dyn Fn(&str) -> Option<String>,
    /// The triggers' variables (`wtg_rs::jass::map_variable`).
    pub variables: Vec<wtg_rs::jass::MapVariable>,
    /// The triggers run at map initialization, by their names in the
    /// script (`wtg_rs::jass::initialization_triggers`).
    pub initialization: Vec<String>,
    /// The code, in its order: the triggers' JASS, the map's custom
    /// script, the project's code.
    pub code: Vec<Code>,
    /// Shorter names ([`crate::uglify`]).
    pub uglify: bool,
    /// The language written ([`crate::jass::Script`]).
    pub script: crate::jass::Script,
    /// Where JassHelper's `//! import "path"` in a file of the code leads
    /// ([`crate::preprocess::Import`]).
    pub import: &'a crate::preprocess::Import<'a>,
}

/// A piece of code: its name (a `.zn` one is Zinc, a `.vj` one vJASS),
/// its text, and its dialect when its header names none.
#[derive(Debug, Clone, PartialEq)]
pub struct Code {
    pub name: String,
    pub text: String,
    pub dialect: jass_rs::Dialect,
}

/// What a whole build made.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Script {
    /// `war3map.j`.
    pub text: String,
    /// What it says about the build (names left long, and why).
    pub notes: Vec<String>,
    /// What's wrong with the script, which is made all the same
    /// ([`crate::jass::Built::problems`]).
    pub problems: Vec<Error>,
}

/// The map's whole script, or why it can't be made.
pub fn script(input: &Input) -> Result<Script, Vec<Error>> {
    let read = input.read;
    let error = |file: &str, message: String| vec![Error { origin: Origin::file(file), message }];
    let text = |name: &str| read(name).map(|b| String::from_utf8_lossy(&b).into_owned());

    let Some(info_bytes) = read("war3map.w3i") else {
        return Err(error("war3map.w3i", "the map has no war3map.w3i".into()));
    };
    let info = w3i_rs::parse(&info_bytes).map_err(|e| error("war3map.w3i", format!("{e}")))?;

    // What the game's files say: its margins, lights, sounds' labels.
    let mut game = GameData::read(&text("UI\\MiscData.txt").unwrap_or_default(), &(input.read_patched)("UI\\WorldEditData.txt").unwrap_or_default());
    for t in SOUND_TABLES {
        if let Some(s) = text(&format!("UI\\SoundInfo\\{t}")) {
            game.add_sound_info(&s);
        }
    }

    // What the types are: the game's tables, then the map's object data.
    let mut objects = Objects::default();
    for t in TYPE_TABLES {
        if let Some(s) = text(&format!("Units\\{t}")) {
            objects.add_table(&s);
        }
    }
    for t in ABILITY_FUNCS {
        if let Some(s) = text(&format!("Units\\{t}")) {
            objects.add_ability_func(&s);
        }
    }
    for (name, kind) in [("war3map.w3u", w3abdhqtu_rs::Kind::Unit), ("war3map.w3t", w3abdhqtu_rs::Kind::Item), ("war3map.w3a", w3abdhqtu_rs::Kind::Ability)] {
        if let Some(b) = read(name) {
            objects.add_object_data(&w3abdhqtu_rs::parse(&b, kind).map_err(|e| error(name, format!("{e}")))?);
        }
    }

    let regions = read("war3map.w3r").map(|b| w3r_rs::parse(&b).map_err(|e| error("war3map.w3r", format!("{e}")))).transpose()?;
    let cameras = read("war3map.w3c").map(|b| w3c_rs::parse(&b, w3c_rs::Layout::from_w3i_version(info.version)).map_err(|e| error("war3map.w3c", format!("{e}")))).transpose()?;
    let sounds = read("war3map.w3s").map(|b| w3s_rs::parse(&b).map_err(|e| error("war3map.w3s", format!("{e}")))).transpose()?;
    let doodads = read("war3map.doo").map(|b| doo_rs::parse_doodads_any(&b).map_err(|e| error("war3map.doo", format!("{e}")))).transpose()?;
    let units = read("war3mapUnits.doo").map(|b| doo_rs::parse_units_any(&b).map_err(|e| error("war3mapUnits.doo", format!("{e}")))).transpose()?;

    // The code first: what it uses names the placed things it needs.
    // All of it at once: JassHelper's textmacros run across files.
    let sources: Vec<(Source, jass_rs::Dialect)> = input.code.iter().map(|c| (Source { name: &c.name, text: &c.text }, c.dialect)).collect();
    let code = module_with(&sources, input.import)?;
    let used: HashSet<String> = crate::map::used_names(&code);
    let duration = |f: &str| read(f).and_then(|b| crate::audio::duration_ms(&b));

    let map = Map {
        info: &info,
        variables: &input.variables,
        initialization: &input.initialization,
        regions: regions.as_ref(),
        cameras: cameras.as_ref(),
        sounds: sounds.as_ref(),
        duration: &duration,
        doodads: doodads.as_ref(),
        units: units.as_ref(),
        objects: &objects,
        used: &used,
    };
    let mut whole: Module = crate::map::start(&map, &game)?;
    whole.append(code);
    whole.append(crate::map::end(&map));
    // The game's natives and functions, for hooks of them.
    let (common_j, blizzard_j) = (text("Scripts\\common.j").unwrap_or_default(), text("Scripts\\Blizzard.j").unwrap_or_default());
    let game = [&common_j, &blizzard_j].iter().flat_map(|t| crate::from_jass::signatures(t)).collect();
    // AngelScript's: common.j's names in Jass::, what's used of Blizzard.j.
    let scripts = input.script == crate::jass::Script::AngelScript;
    let (common_j, blizzard_j) = if scripts { (common_j, blizzard_j) } else { Default::default() };
    let built = crate::jass::build_with(&whole, &crate::jass::Options { uglify: input.uglify, game, script: input.script, common_j, blizzard_j })?;
    Ok(Script { text: built.script, notes: built.notes, problems: built.problems })
}

#[cfg(test)]
#[path = "whole.test.rs"]
mod tests;
