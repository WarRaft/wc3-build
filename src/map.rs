//! The map's own part of its script, out of its files, as World Editor
//! makes it (see the README, "The map's script"): JASS of this toolchain,
//! read into the IR like any other source. It comes in two parts around
//! the rest of the code, whose `main`s merge with them in order:
//!
//! - [`start`]: `config` (the map's name, players, forces, start
//!   locations: `InitCustomPlayerSlots`, `InitCustomTeams`,
//!   `InitAllyPriorities`), and a `main` that sets the map up as World
//!   Editor's does (camera bounds, day and night, fog, water, sound,
//!   music), then `InitBlizzard` and `InitGlobals` (the triggers'
//!   variables, made of them). The triggers' `main`s come after it (World
//!   Editor's `InitCustomTriggers`).
//! - [`end`]: a `main` that runs the triggers of map initialization
//!   (`RunInitializationTriggers`) once every other `main` has run.
//!
//! What the game's files decide comes from them, through the project's
//! lookup ([`GameData`]): the camera's margins (`UI\MiscData.txt`), the
//! day and night models, the ambient sounds and the sound environment of
//! each tileset (`UI\WorldEditData.txt`).

use std::collections::HashMap;
use std::fmt::Write as _;

use w3i_rs::Info;

/// What the game's files say about building a map's script.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GameData {
    /// The camera's margins, in cells: left, right, top, bottom
    /// (`UI\MiscData.txt`, `[CameraMargins]`).
    pub margins: [f32; 4],
    /// By tileset (`UI\WorldEditData.txt`): the day and night models of
    /// the terrain (`[TerrainLights]`) and of units (`[UnitLights]`), the
    /// ambient sounds of the day (`[DayAmbience]`) and the night
    /// (`[NightAmbience]`).
    pub terrain_lights: HashMap<char, String>,
    pub unit_lights: HashMap<char, String>,
    pub day: HashMap<char, String>,
    pub night: HashMap<char, String>,
    /// The sound environment of a map that names none, by tileset
    /// (`[SoundEnvironment]`).
    pub sound_environments: HashMap<char, String>,
}

/// The size of a cell, in the game's units.
const CELL: f32 = 128.0;

/// The sections of an INI-like file of the game's: `[name]`, `key=value`,
/// `//` comments.
fn sections(text: &str) -> HashMap<String, Vec<(String, String)>> {
    let mut out: HashMap<String, Vec<(String, String)>> = HashMap::new();
    let mut current = String::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("//") {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            current = name.to_string();
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            out.entry(current.clone()).or_default().push((k.trim().to_string(), v.trim().to_string()));
        }
    }
    out
}

impl GameData {
    /// From the texts of `UI\MiscData.txt` and `UI\WorldEditData.txt` (the
    /// files the project's lookup finds).
    pub fn read(misc_data: &str, world_edit_data: &str) -> GameData {
        let misc = sections(misc_data);
        let margin = |key: &str| {
            misc.get("CameraMargins").and_then(|s| s.iter().find(|(k, _)| k.eq_ignore_ascii_case(key))).and_then(|(_, v)| v.parse().ok()).unwrap_or(0.0)
        };
        let edit = sections(world_edit_data);
        let by_tileset = |name: &str| -> HashMap<char, String> {
            edit.get(name)
                .into_iter()
                .flatten()
                .filter_map(|(k, v)| {
                    let mut c = k.chars();
                    match (c.next(), c.next()) {
                        (Some(t), None) => Some((t, v.clone())),
                        _ => None,
                    }
                })
                .collect()
        };
        GameData {
            margins: [margin("left"), margin("right"), margin("top"), margin("bottom")],
            terrain_lights: by_tileset("TerrainLights"),
            unit_lights: by_tileset("UnitLights"),
            day: by_tileset("DayAmbience"),
            night: by_tileset("NightAmbience"),
            sound_environments: by_tileset("SoundEnvironment"),
        }
    }
}

/// What the map's part of its script is made of.
pub struct Map<'a> {
    pub info: &'a Info,
    /// The triggers run at map initialization, by their names in the
    /// script (`wtg_rs::jass::initialization_triggers`).
    pub initialization: &'a [String],
    /// Are there the triggers' variables, and their `InitGlobals`?
    pub globals: bool,
}

/// A string as JASS writes it.
fn string(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out += "\\\"",
            '\\' => out += "\\\\",
            '\n' => out += "\\n",
            '\r' => out += "\\r",
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A real as World Editor writes a coordinate: one decimal.
fn coord(v: f32) -> String {
    format!("{v:.1}")
}

const RACES: [&str; 5] = ["RACE_PREF_RANDOM", "RACE_PREF_HUMAN", "RACE_PREF_ORC", "RACE_PREF_UNDEAD", "RACE_PREF_NIGHTELF"];
const CONTROLLERS: [&str; 5] = ["MAP_CONTROL_USER", "MAP_CONTROL_USER", "MAP_CONTROL_COMPUTER", "MAP_CONTROL_NEUTRAL", "MAP_CONTROL_RESCUABLE"];

/// Flags of `war3map.w3i` (`w3i_rs::FLAGS`).
const FIXED_PLAYER_SETTINGS: u32 = 0x20;
const CUSTOM_FORCES: u32 = 0x40;
const TERRAIN_FOG: u32 = 0x2000;
const WATER_TINT: u32 = 0x1_0000;

/// A force's flags (`w3i_rs::FORCE_FLAGS`).
const ALLIED: u32 = 0x1;
const ALLIED_VICTORY: u32 = 0x2;
const SHARED_VISION: u32 = 0x8;
const SHARED_CONTROL: u32 = 0x10;
const SHARED_ADVANCED_CONTROL: u32 = 0x20;

/// The map's part that comes before the rest of the code: `config` and
/// the functions it calls, and the start of `main`.
pub fn start(map: &Map, game: &GameData) -> String {
    let info = map.info;
    let mut out = String::new();
    let players = &info.players;
    let player = |p: &w3i_rs::Player| format!("Player({})", p.number);

    // config
    out += "function config\n";
    let _ = writeln!(out, "    SetMapName({})", string(&info.name));
    let _ = writeln!(out, "    SetMapDescription({})", string(&info.description));
    let _ = writeln!(out, "    SetPlayers({})", players.len());
    let _ = writeln!(out, "    SetTeams({})", players.len());
    let placement = if info.flags & CUSTOM_FORCES != 0 { "MAP_PLACEMENT_TEAMS_TOGETHER" } else { "MAP_PLACEMENT_USE_MAP_SETTINGS" };
    let _ = writeln!(out, "    SetGamePlacement({placement})");
    for (i, p) in players.iter().enumerate() {
        let _ = writeln!(out, "    DefineStartLocation({i}, {}, {})", coord(p.start_x), coord(p.start_y));
    }
    out += "    InitCustomPlayerSlots()\n    InitCustomTeams()\n    InitAllyPriorities()\n";
    out += "endfunction\n\n";

    // The players.
    out += "function InitCustomPlayerSlots\n";
    for (i, p) in players.iter().enumerate() {
        let me = player(p);
        let _ = writeln!(out, "    SetPlayerStartLocation({me}, {i})");
        if p.fixed_start != 0 || info.flags & FIXED_PLAYER_SETTINGS != 0 {
            let _ = writeln!(out, "    ForcePlayerStartLocation({me}, {i})");
        }
        let _ = writeln!(out, "    SetPlayerColor({me}, ConvertPlayerColor({}))", p.number);
        let race = RACES.get(p.race as usize).copied().unwrap_or(RACES[0]);
        let _ = writeln!(out, "    SetPlayerRacePreference({me}, {race})");
        let selectable = p.race == 0 || info.flags & FIXED_PLAYER_SETTINGS == 0;
        let _ = writeln!(out, "    SetPlayerRaceSelectable({me}, {selectable})");
        let controller = CONTROLLERS.get(p.kind as usize).copied().unwrap_or(CONTROLLERS[0]);
        let _ = writeln!(out, "    SetPlayerController({me}, {controller})");
    }
    out += "endfunction\n\n";

    // The forces: teams, and what their players share.
    out += "function InitCustomTeams\n";
    for (f, force) in info.forces.iter().enumerate() {
        let members: Vec<&w3i_rs::Player> =
            players.iter().filter(|p| (0..32).contains(&p.number) && force.players & (1 << p.number) != 0).collect();
        for p in &members {
            let _ = writeln!(out, "    SetPlayerTeam({}, {f})", player(p));
            if force.flags & ALLIED_VICTORY != 0 {
                let _ = writeln!(out, "    SetPlayerState({}, PLAYER_STATE_ALLIED_VICTORY, 1)", player(p));
            }
        }
        for (flag, call) in [
            (ALLIED, "SetPlayerAllianceStateAllyBJ"),
            (SHARED_VISION, "SetPlayerAllianceStateVisionBJ"),
            (SHARED_CONTROL, "SetPlayerAllianceStateControlBJ"),
            (SHARED_ADVANCED_CONTROL, "SetPlayerAllianceStateFullControlBJ"),
        ] {
            if force.flags & flag == 0 {
                continue;
            }
            for a in &members {
                for b in members.iter().filter(|b| b.number != a.number) {
                    let _ = writeln!(out, "    {call}({}, {}, true)", player(a), player(b));
                }
            }
        }
    }
    out += "endfunction\n\n";

    // Whom each start location prefers near it (whatever the flag "modify
    // ally priorities" says: World Editor writes them all the same).
    {
        out += "function InitAllyPriorities\n";
        for (i, p) in players.iter().enumerate() {
            let bit = |mask: u32, q: &w3i_rs::Player| (0..32).contains(&q.number) && mask & (1 << q.number) != 0;
            let near: Vec<(usize, &str)> = players
                .iter()
                .enumerate()
                .filter(|(j, _)| *j != i)
                .filter_map(|(j, q)| {
                    if bit(p.ally_high, q) {
                        Some((j, "MAP_LOC_PRIO_HIGH"))
                    } else if bit(p.ally_low, q) {
                        Some((j, "MAP_LOC_PRIO_LOW"))
                    } else {
                        None
                    }
                })
                .collect();
            if near.is_empty() {
                continue;
            }
            let _ = writeln!(out, "    SetStartLocPrioCount({i}, {})", near.len());
            for (k, (j, prio)) in near.iter().enumerate() {
                let _ = writeln!(out, "    SetStartLocPrio({i}, {k}, {j}, {prio})");
            }
        }
        out += "endfunction\n\n";
    }

    // main: the map set up.
    out += "function main\n";
    let b = info.camera_bounds;
    let [left, right, top, bottom] = game.margins.map(|m| m * CELL);
    let x = |v: f32, margin: f32, side: &str| {
        let sign = if side == "LEFT" || side == "BOTTOM" { "+" } else { "-" };
        format!("{} {sign} GetCameraMargin(CAMERA_MARGIN_{side})", coord(v + margin))
    };
    let _ = writeln!(
        out,
        "    SetCameraBounds({}, {}, {}, {}, {}, {}, {}, {})",
        x(b[0], -left, "LEFT"),
        x(b[1], -bottom, "BOTTOM"),
        x(b[2], right, "RIGHT"),
        x(b[3], top, "TOP"),
        x(b[4], -left, "LEFT"),
        x(b[5], top, "TOP"),
        x(b[6], right, "RIGHT"),
        x(b[7], -bottom, "BOTTOM"),
    );
    let light = if info.light_tileset != 0 { info.light_tileset } else { info.tileset } as char;
    if let (Some(t), Some(u)) = (game.terrain_lights.get(&light), game.unit_lights.get(&light)) {
        let _ = writeln!(out, "    SetDayNightModels({}, {})", string(t), string(u));
    }
    if info.flags & TERRAIN_FOG != 0 {
        let f = &info.fog;
        let c = |v: u8| format!("{:.3}", f32::from(v) / 255.0);
        let _ = writeln!(
            out,
            "    SetTerrainFogEx({}, {}, {}, {:.3}, {}, {}, {})",
            f.style,
            coord(f.start_z),
            coord(f.end_z),
            f.density,
            c(f.color[0]),
            c(f.color[1]),
            c(f.color[2])
        );
    }
    if info.flags & WATER_TINT != 0 {
        let w = info.water_color;
        let _ = writeln!(out, "    SetWaterBaseColor({}, {}, {}, {})", w[0], w[1], w[2], w[3]);
    }
    let tileset = info.tileset as char;
    // None named: the tileset's.
    let environment = Some(info.sound_environment.as_str()).filter(|e| !e.is_empty()).or_else(|| game.sound_environments.get(&tileset).map(String::as_str));
    if let Some(e) = environment {
        let _ = writeln!(out, "    NewSoundEnvironment({})", string(e));
    }
    if let Some(d) = game.day.get(&tileset) {
        let _ = writeln!(out, "    SetAmbientDaySound({})", string(d));
    }
    if let Some(n) = game.night.get(&tileset) {
        let _ = writeln!(out, "    SetAmbientNightSound({})", string(n));
    }
    out += "    SetMapMusic(\"Music\", true, 0)\n";
    out += "    InitBlizzard()\n";
    if map.globals {
        out += "    InitGlobals()\n";
    }
    out += "endfunction\n";
    out
}

/// The map's part that comes after the rest of the code: the triggers of
/// map initialization run.
pub fn end(map: &Map) -> String {
    let mut out = String::from("function RunInitializationTriggers\n");
    for t in map.initialization {
        let _ = writeln!(out, "    ConditionalTriggerExecute({t})");
    }
    out += "endfunction\n\nfunction main\n    RunInitializationTriggers()\nendfunction\n";
    out
}

#[cfg(test)]
#[path = "map.test.rs"]
mod tests;
