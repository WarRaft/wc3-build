//! The map's own part of its script, out of its files, as World Editor
//! makes it (see the README, "The map's script"), made as the IR's nodes:
//! every output converts the same tree. It comes in two parts around the
//! rest of the code, whose `main`s merge with them in order:
//!
//! - [`start`]: `config` (the map's name, players, forces, start
//!   locations: `InitCustomPlayerSlots`, `InitCustomTeams`,
//!   `InitAllyPriorities`), and a `main` that sets the map up as World
//!   Editor's does (camera bounds, day and night, fog, water, sound,
//!   music), then what makes the map's things (`InitSounds`,
//!   `CreateRegions` with their weather and sounds, `CreateCameras`,
//!   upgrades, tech tree, random groups and item tables:
//!   [`crate::tables`]; what's placed: [`crate::placed`]), `InitBlizzard` and
//!   `InitGlobals` (the triggers' variables). The triggers' `main`s come
//!   after it (World Editor's `InitCustomTriggers`).
//! - [`end`]: a `main` that runs the triggers of map initialization
//!   (`RunInitializationTriggers`) once every other `main` has run.
//!
//! What the game's files decide comes from them, through the project's
//! lookup ([`GameData`]): the camera's margins (`UI\MiscData.txt`), the
//! day and night models, the ambient sounds and the sound environment of
//! each tileset (`UI\WorldEditData.txt`), the labels of the game's sounds
//! (`UI\SoundInfo\*.slk`). How long a sound plays is its file's
//! ([`crate::audio`]), which the caller reads.

use std::collections::HashMap;
use w3i_rs::Info;

use crate::ir::*;

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
    /// The game's sounds' labels (`UI\SoundInfo\*.slk`), by their files
    /// (`DirectoryBase` and each of `FileNames`), lowercase.
    pub sound_labels: HashMap<String, String>,
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
            sound_labels: HashMap::new(),
        }
    }

    /// Adds the labels of a table of the game's sounds (an `.slk` of
    /// `UI\SoundInfo`: `SoundName`, `FileNames`, `DirectoryBase`); a file
    /// keeps the first label found for it.
    pub fn add_sound_info(&mut self, slk: &str) {
        let doc = slk_rs::parse(slk);
        let table = slk_rs::Table::new(&doc);
        let column = |name: &str| (1..=table.cols()).find(|&c| table.text(1, c) == name);
        let (Some(label), Some(files), Some(dir)) = (column("SoundName"), column("FileNames"), column("DirectoryBase")) else { return };
        for row in 2..=table.rows() {
            let name = table.text(row, label);
            if name.is_empty() {
                continue;
            }
            // A folder with or without its `\` (both in the game's tables).
            let mut base = table.text(row, dir);
            if !base.is_empty() && !base.ends_with('\\') {
                base.push('\\');
            }
            for file in table.text(row, files).split(',').map(str::trim).filter(|f| !f.is_empty()) {
                self.sound_labels.entry(format!("{base}{file}").to_lowercase()).or_insert_with(|| name.clone());
            }
        }
    }
}

/// What the map's part of its script is made of: its files, read.
pub struct Map<'a> {
    pub info: &'a Info,
    /// The triggers' variables (`wtg_rs::jass::map_variable`).
    pub variables: &'a [wtg_rs::jass::MapVariable],
    /// The triggers run at map initialization, by their names in the
    /// script (`wtg_rs::jass::initialization_triggers`).
    pub initialization: &'a [String],
    pub regions: Option<&'a w3r_rs::Regions>,
    pub cameras: Option<&'a w3c_rs::Cameras>,
    pub sounds: Option<&'a w3s_rs::Sounds>,
    /// How long a sound file plays, in milliseconds (`audio::duration_ms`
    /// of it, wherever it's found: the map, the game): `None` when it isn't.
    pub duration: &'a dyn Fn(&str) -> Option<u32>,
    /// What's placed (`war3map.doo`, `war3mapUnits.doo`).
    pub doodads: Option<&'a doo_rs::Doodads>,
    pub units: Option<&'a doo_rs::Units>,
    /// What the placed things' types are (the game's and the map's object
    /// data).
    pub objects: &'a crate::objects::Objects,
    /// The names the rest of the code uses ([`used_names`]): a placed
    /// thing is named (`gg_unit_hfoo_0012`) only when it's one.
    pub used: &'a std::collections::HashSet<String>,
}

/// The names World Editor's script declares for what the map's files
/// place and name, each with its JASS type, as a build names them:
/// regions (`gg_rct_`), cameras (`gg_cam_`), sounds (as the file names
/// them: `gg_snd_`, a string for music), destructables (`gg_dest_`) and
/// what `war3mapUnits.doo` places (`gg_unit_`, `gg_item_`: which it is
/// `objects` say, the game's tables and the map's object data, as a build
/// tells them; without them, both names for each). For what the map's
/// code may name (a custom script checked before a build).
pub fn declared_names(
    regions: Option<&w3r_rs::Regions>,
    cameras: Option<&w3c_rs::Cameras>,
    sounds: Option<&w3s_rs::Sounds>,
    doodads: Option<&doo_rs::Doodads>,
    units: Option<&doo_rs::Units>,
    objects: Option<&crate::objects::Objects>,
) -> Vec<(String, &'static str)> {
    let ident = wtg_rs::jass::ident;
    let mut out = Vec::new();
    out.extend(regions.into_iter().flat_map(|r| &r.regions).map(|g| (format!("gg_rct_{}", ident(&g.name)), "rect")));
    out.extend(cameras.into_iter().flat_map(|c| &c.cameras).map(|c| (format!("gg_cam_{}", ident(&c.name)), "camerasetup")));
    out.extend(sounds.into_iter().flat_map(|s| &s.sounds).map(|s| (s.name.clone(), if s.flags & w3s_rs::MUSIC != 0 { "string" } else { "sound" })));
    out.extend(doodads.into_iter().flat_map(|d| &d.doodads).map(|d| (crate::placed::name("dest", d.placement.id, d.number), "destructable")));
    // Start locations are no units: `config`'s.
    for u in units.into_iter().flat_map(|u| &u.units).filter(|u| u.placement.id != parser_rs::Rawcode(*b"sloc")) {
        let item = objects.map(|o| o.placed(u.placement.id) == crate::objects::Placed::Item);
        if item != Some(true) {
            out.push((crate::placed::name("unit", u.placement.id, u.number), "unit"));
        }
        if item != Some(false) {
            out.push((crate::placed::name("item", u.placement.id, u.number), "item"));
        }
    }
    out
}

/// Every name a module's functions and globals use.
pub fn used_names(m: &Module) -> std::collections::HashSet<String> {
    let mut out = std::collections::HashSet::new();
    let mut name = |e: &Expr| {
        if let Expr::Var(n) | Expr::Code(n) | Expr::Index(n, _) = e {
            out.insert(n.clone());
        }
    };
    for f in &m.functions {
        each_expr(&f.body, &mut name);
    }
    let inits: Vec<Stmt> = m.globals.iter().filter_map(|g| g.init.clone()).map(|e| Stmt::Return { value: Some(e), line: 0 }).collect();
    each_expr(&inits, &mut name);
    for f in &m.functions {
        collect_sets(&f.body, &mut out);
    }
    out
}

/// The names statements set.
fn collect_sets(body: &[Stmt], out: &mut std::collections::HashSet<String>) {
    for s in body {
        match s {
            Stmt::Set { name, .. } => {
                out.insert(name.clone());
            }
            Stmt::If { branches, otherwise, .. } => {
                branches.iter().for_each(|(_, b)| collect_sets(b, out));
                if let Some(b) = otherwise {
                    collect_sets(b, out);
                }
            }
            Stmt::Loop { body, .. } => collect_sets(body, out),
            Stmt::Debug { stmt, .. } => collect_sets(std::slice::from_ref(&**stmt), out),
            _ => {}
        }
    }
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

/// Where what the map's part makes comes from, for errors.
const ORIGIN: &str = "the map";

/// A coordinate as World Editor writes it: one decimal.
fn coord(v: f32) -> Expr {
    Expr::real(v, 1)
}

fn player(p: &w3i_rs::Player) -> Expr {
    Expr::call("Player", vec![Expr::int(p.number)])
}

/// An object's id: its four characters, or its number.
fn rawcode(r: w3i_rs::Rawcode) -> Expr {
    if r.is_printable() { Expr::rawcode(&r.to_string()) } else { Expr::int(r.to_int()) }
}

fn function(name: &str, body: Vec<Stmt>) -> Function {
    Function::new(name, body, Origin::file(ORIGIN))
}

fn global(name: String, ty: &str) -> Global {
    Global::new(name, ty, Some(Expr::lit("null")), Origin::file(ORIGIN))
}

/// The map's part that comes before the rest of the code: `config` and
/// the functions it calls, the start of `main` and the functions it calls,
/// and the globals they set; or what in the map's data doesn't read.
pub fn start(map: &Map, game: &GameData) -> Result<Module, Vec<Error>> {
    let mut m = Module::default();
    let info = map.info;
    m.functions.push(config(info));
    m.functions.push(player_slots(info));
    m.functions.push(teams(info));
    if has_ally_priorities(info) {
        m.functions.push(ally_priorities(info));
    }

    let mut parts = Vec::new();
    if let Some(s) = map.sounds.filter(|s| !s.sounds.is_empty()) {
        let (f, globals) = sounds(s, game, map.duration);
        m.functions.push(f);
        m.globals.extend(globals);
        parts.push("InitSounds");
    }
    if let Some(r) = map.regions.filter(|r| !r.regions.is_empty()) {
        let (f, globals) = regions(r);
        m.functions.push(f);
        m.globals.extend(globals);
        parts.push("CreateRegions");
    }
    if let Some(c) = map.cameras.filter(|c| !c.cameras.is_empty()) {
        let (f, globals) = cameras(c);
        m.functions.push(f);
        m.globals.extend(globals);
        parts.push("CreateCameras");
    }
    for (name, functions) in [("InitUpgrades", crate::tables::upgrades(info)), ("InitTechTree", crate::tables::tech_tree(info))] {
        if !functions.is_empty() {
            m.functions.extend(functions);
            parts.push(name);
        }
    }
    if let Some((f, globals)) = crate::tables::random_groups(info) {
        m.functions.push(f);
        m.globals.extend(globals);
        parts.push("InitRandomGroups");
    }
    m.functions.extend(crate::tables::item_tables(info));
    let made = crate::placed::make(&crate::placed::Placing {
        info,
        doodads: map.doodads,
        units: map.units,
        objects: map.objects,
        used: map.used,
        regions: map.regions,
    });
    if !made.errors.is_empty() {
        return Err(made.errors);
    }
    m.functions.extend(made.functions);
    m.globals.extend(made.globals);
    parts.extend(made.calls);

    let mut main = setting_up(info, game);
    for p in parts {
        main.push(Stmt::call(p, Vec::new()));
    }
    main.push(Stmt::call("InitBlizzard", Vec::new()));
    if !map.variables.is_empty() {
        let (f, globals) = init_globals(map.variables)?;
        m.globals.splice(0..0, globals);
        m.functions.push(f);
        main.push(Stmt::call("InitGlobals", Vec::new()));
    }
    m.functions.push(function("main", main));
    Ok(m)
}

fn config(info: &Info) -> Function {
    let players = &info.players;
    let mut b = vec![
        Stmt::call("SetMapName", vec![Expr::string(&info.name)]),
        Stmt::call("SetMapDescription", vec![Expr::string(&info.description)]),
        Stmt::call("SetPlayers", vec![Expr::int(players.len())]),
        Stmt::call("SetTeams", vec![Expr::int(players.len())]),
    ];
    // The map's own placement only when its forces are its own and none is
    // allied (Blizzard's 108 maps: one is so).
    let own = info.flags & CUSTOM_FORCES != 0 && info.forces.iter().all(|f| f.flags & ALLIED == 0);
    let placement = if own { "MAP_PLACEMENT_USE_MAP_SETTINGS" } else { "MAP_PLACEMENT_TEAMS_TOGETHER" };
    b.push(Stmt::call("SetGamePlacement", vec![Expr::var(placement)]));
    for (i, p) in players.iter().enumerate() {
        b.push(Stmt::call("DefineStartLocation", vec![Expr::int(i), coord(p.start_x), coord(p.start_y)]));
    }
    b.push(Stmt::call("InitCustomPlayerSlots", Vec::new()));
    if info.flags & CUSTOM_FORCES != 0 {
        b.push(Stmt::call("InitCustomTeams", Vec::new()));
    } else {
        // The game's teams: every slot a user's (Blizzard's maps without
        // forces of their own have only users), then the game's setting.
        for p in players {
            b.push(Stmt::call("SetPlayerSlotAvailable", vec![player(p), Expr::var("MAP_CONTROL_USER")]));
        }
        b.push(Stmt::call("InitGenericPlayerSlots", Vec::new()));
    }
    if has_ally_priorities(info) {
        b.push(Stmt::call("InitAllyPriorities", Vec::new()));
    }
    function("config", b)
}

fn player_slots(info: &Info) -> Function {
    let mut b = Vec::new();
    for (i, p) in info.players.iter().enumerate() {
        b.push(Stmt::call("SetPlayerStartLocation", vec![player(p), Expr::int(i)]));
        // Fixed (1), not "random but fixed" (2): Blizzard's maps, whatever
        // the map's flag.
        if p.fixed_start == 1 {
            b.push(Stmt::call("ForcePlayerStartLocation", vec![player(p), Expr::int(i)]));
        }
        b.push(Stmt::call("SetPlayerColor", vec![player(p), Expr::call("ConvertPlayerColor", vec![Expr::int(p.number)])]));
        let race = RACES.get(p.race as usize).copied().unwrap_or(RACES[0]);
        b.push(Stmt::call("SetPlayerRacePreference", vec![player(p), Expr::var(race)]));
        // Reign of Chaos's World Editor (`war3map.w3i` 18) lets every race
        // be chosen.
        let selectable = info.version < 25 || p.race == 0 || info.flags & FIXED_PLAYER_SETTINGS == 0;
        b.push(Stmt::call("SetPlayerRaceSelectable", vec![player(p), Expr::bool(selectable)]));
        let controller = CONTROLLERS.get(p.kind as usize).copied().unwrap_or(CONTROLLERS[0]);
        b.push(Stmt::call("SetPlayerController", vec![player(p), Expr::var(controller)]));
    }
    function("InitCustomPlayerSlots", b)
}

/// The forces: teams, and what their players share.
fn teams(info: &Info) -> Function {
    let mut b = Vec::new();
    for (f, force) in info.forces.iter().enumerate() {
        let members: Vec<&w3i_rs::Player> =
            info.players.iter().filter(|p| (0..32).contains(&p.number) && force.players & (1 << p.number) != 0).collect();
        for p in &members {
            b.push(Stmt::call("SetPlayerTeam", vec![player(p), Expr::int(f)]));
            if force.flags & ALLIED_VICTORY != 0 {
                b.push(Stmt::call("SetPlayerState", vec![player(p), Expr::var("PLAYER_STATE_ALLIED_VICTORY"), Expr::int(1)]));
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
                for o in members.iter().filter(|o| o.number != a.number) {
                    b.push(Stmt::call(call, vec![player(a), player(o), Expr::bool(true)]));
                }
            }
        }
    }
    function("InitCustomTeams", b)
}

/// Whether any start location prefers another near it: else World Editor
/// writes no `InitAllyPriorities`.
fn has_ally_priorities(info: &Info) -> bool {
    info.players.iter().any(|p| info.players.iter().any(|q| q.number != p.number && (0..32).contains(&q.number) && (p.ally_high | p.ally_low) & (1 << q.number) != 0))
}

/// Whom each start location prefers near it (whatever the flag "modify
/// ally priorities" says: World Editor writes them all the same).
fn ally_priorities(info: &Info) -> Function {
    let players = &info.players;
    let bit = |mask: u32, q: &w3i_rs::Player| (0..32).contains(&q.number) && mask & (1 << q.number) != 0;
    let mut b = Vec::new();
    for (i, p) in players.iter().enumerate() {
        let near: Vec<(usize, &str)> = players
            .iter()
            .enumerate()
            .filter(|(j, _)| *j != i)
            .filter_map(|(j, q)| {
                // Both set: low (Blizzard's Monolith).
                if bit(p.ally_low, q) {
                    Some((j, "MAP_LOC_PRIO_LOW"))
                } else if bit(p.ally_high, q) {
                    Some((j, "MAP_LOC_PRIO_HIGH"))
                } else {
                    None
                }
            })
            .collect();
        if near.is_empty() {
            continue;
        }
        b.push(Stmt::call("SetStartLocPrioCount", vec![Expr::int(i), Expr::int(near.len())]));
        for (k, (j, prio)) in near.iter().enumerate() {
            b.push(Stmt::call("SetStartLocPrio", vec![Expr::int(i), Expr::int(k), Expr::int(j), Expr::var(*prio)]));
        }
    }
    function("InitAllyPriorities", b)
}

/// `main`'s start: the map's camera bounds, light, fog, water and sound.
fn setting_up(info: &Info, game: &GameData) -> Vec<Stmt> {
    let mut b = Vec::new();
    let bounds = info.camera_bounds;
    let [left, right, top, bottom] = game.margins.map(|m| m * CELL);
    let edge = |v: f32, margin: f32, side: &str| {
        let op = if side == "LEFT" || side == "BOTTOM" { BinOp::Add } else { BinOp::Sub };
        Expr::binary(coord(v + margin), op, Expr::call("GetCameraMargin", vec![Expr::var(format!("CAMERA_MARGIN_{side}"))]))
    };
    b.push(Stmt::call(
        "SetCameraBounds",
        vec![
            edge(bounds[0], -left, "LEFT"),
            edge(bounds[1], -bottom, "BOTTOM"),
            edge(bounds[2], right, "RIGHT"),
            edge(bounds[3], top, "TOP"),
            edge(bounds[4], -left, "LEFT"),
            edge(bounds[5], top, "TOP"),
            edge(bounds[6], right, "RIGHT"),
            edge(bounds[7], -bottom, "BOTTOM"),
        ],
    ));
    let light = if info.light_tileset != 0 { info.light_tileset } else { info.tileset } as char;
    if let (Some(t), Some(u)) = (game.terrain_lights.get(&light), game.unit_lights.get(&light)) {
        b.push(Stmt::call("SetDayNightModels", vec![Expr::string(t), Expr::string(u)]));
    }
    if info.flags & TERRAIN_FOG != 0 {
        let f = &info.fog;
        // The file's color is blue, green, red.
        let c = |v: u8| Expr::real(f32::from(v) / 255.0, 3);
        b.push(Stmt::call(
            "SetTerrainFogEx",
            vec![Expr::int(f.style), coord(f.start_z), coord(f.end_z), Expr::real(f.density, 3), c(f.color[2]), c(f.color[1]), c(f.color[0])],
        ));
    }
    // The map's weather, over the whole map (the camera's bounds, its
    // margins and the bounds' complements: left, right, bottom, top).
    if info.weather != w3i_rs::Rawcode::NONE && info.weather.to_int() != 0 {
        b.insert(0, Stmt::local("weathereffect", "we", None));
        let [ml, mr, mt, mb] = [left, right, top, bottom];
        let c = info.camera_complements.map(|v| v as f32 * CELL);
        let rect = Expr::call(
            "Rect",
            vec![coord(bounds[0] - ml - c[0]), coord(bounds[1] - mb - c[2]), coord(bounds[2] + mr + c[1]), coord(bounds[3] + mt + c[3])],
        );
        b.push(Stmt::set("we", Expr::call("AddWeatherEffect", vec![rect, rawcode(info.weather)])));
        b.push(Stmt::call("EnableWeatherEffect", vec![Expr::var("we"), Expr::bool(true)]));
    }
    if info.flags & WATER_TINT != 0 {
        let w = info.water_color;
        b.push(Stmt::call("SetWaterBaseColor", w.iter().map(|&v| Expr::int(v)).collect()));
    }
    let tileset = info.tileset as char;
    // None named: the tileset's.
    let environment = Some(info.sound_environment.as_str()).filter(|e| !e.is_empty()).or_else(|| game.sound_environments.get(&tileset).map(String::as_str));
    if let Some(e) = environment {
        b.push(Stmt::call("NewSoundEnvironment", vec![Expr::string(e)]));
    }
    if let Some(d) = game.day.get(&tileset) {
        b.push(Stmt::call("SetAmbientDaySound", vec![Expr::string(d)]));
    }
    if let Some(n) = game.night.get(&tileset) {
        b.push(Stmt::call("SetAmbientNightSound", vec![Expr::string(n)]));
    }
    b.push(Stmt::call("SetMapMusic", vec![Expr::string("Music"), Expr::bool(true), Expr::int(0)]));
    b
}

/// The triggers' variables, declared with their types' empty values, and
/// `InitGlobals` setting them (an array each cell up to its size, as
/// World Editor's loop).
fn init_globals(vars: &[wtg_rs::jass::MapVariable]) -> Result<(Function, Vec<Global>), Vec<Error>> {
    let mut globals = Vec::new();
    let mut body = Vec::new();
    let mut errors = Vec::new();
    let mut looped = false;
    for v in vars {
        let mut g = Global::new(v.name.clone(), v.ty.clone(), v.empty.map(Expr::lit), Origin::file(ORIGIN));
        g.array = v.array.is_some();
        globals.push(g);
        let Some(text) = &v.value else { continue };
        let value = match crate::from_jass::expr(text) {
            Ok(e) => e,
            Err(e) => {
                errors.push(Error { origin: Origin::file(ORIGIN), message: format!("the initial value of {}: {e}", v.name) });
                continue;
            }
        };
        match v.array {
            Some(size) => {
                looped = true;
                let i = || Expr::var("i");
                body.push(Stmt::set("i", Expr::int(0)));
                body.push(Stmt::Loop {
                    body: vec![
                        Stmt::ExitWhen { cond: Expr::binary(i(), BinOp::Gt, Expr::int(size)), line: 0 },
                        Stmt::set_at(v.name.clone(), i(), value),
                        Stmt::set("i", Expr::binary(i(), BinOp::Add, Expr::int(1))),
                    ],
                    line: 0,
                });
            }
            None => body.push(Stmt::set(v.name.clone(), value)),
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    if looped {
        body.insert(0, Stmt::local("integer", "i", Some(Expr::int(0))));
    }
    Ok((function("InitGlobals", body), globals))
}

/// The regions (`gg_rct_<name>`), each with its weather and its sound.
fn regions(r: &w3r_rs::Regions) -> (Function, Vec<Global>) {
    let mut b = Vec::new();
    let mut globals = Vec::new();
    if r.regions.iter().any(|g| g.weather != w3i_rs::Rawcode::NONE) {
        b.push(Stmt::local("weathereffect", "we", None));
    }
    for g in &r.regions {
        let name = format!("gg_rct_{}", wtg_rs::jass::ident(&g.name));
        // Two of a name: declared once, each set (World Editor's, Diablo III
        // beta's).
        if !globals.iter().any(|x: &Global| x.name == name) {
            globals.push(global(name.clone(), "rect"));
        }
        b.push(Stmt::set(name.clone(), Expr::call("Rect", vec![coord(g.left), coord(g.bottom), coord(g.right), coord(g.top)])));
        if g.weather != w3i_rs::Rawcode::NONE {
            b.push(Stmt::set("we", Expr::call("AddWeatherEffect", vec![Expr::var(name.clone()), rawcode(g.weather)])));
            b.push(Stmt::call("EnableWeatherEffect", vec![Expr::var("we"), Expr::bool(true)]));
        }
        if !g.sound.is_empty() {
            let (w, h) = (g.right - g.left, g.top - g.bottom);
            // At its center; World Editor's height is half its width.
            b.push(Stmt::call("SetSoundPosition", vec![Expr::var(g.sound.clone()), coord(g.left + w / 2.0), coord(g.bottom + h / 2.0), coord(w / 2.0)]));
            b.push(Stmt::call("RegisterStackedSound", vec![Expr::var(g.sound.clone()), Expr::bool(true), coord(w), coord(h)]));
        }
    }
    (function("CreateRegions", b), globals)
}

/// The sounds (`gg_snd_<name>`, as the file names them): music a string,
/// its file; any other made (`CreateSound`), with its label's settings
/// when it's one of the game's (`SetSoundParamsFromLabel`), how long it
/// plays, and each setting the file gives (not World Editor's "default").
fn sounds(s: &w3s_rs::Sounds, game: &GameData, duration: &dyn Fn(&str) -> Option<u32>) -> (Function, Vec<Global>) {
    use w3s_rs::{DEFAULT_INT, DEFAULT_REAL, LOOPING, MUSIC, SOUND_3D, STOP_OUT_OF_RANGE};
    let mut b = Vec::new();
    let mut globals = Vec::new();
    let real = |v: f32| v != DEFAULT_REAL;
    for snd in &s.sounds {
        let name = snd.name.clone();
        let me = || Expr::var(name.clone());
        if snd.flags & MUSIC != 0 {
            globals.push(Global::new(name.clone(), "string", None, Origin::file(ORIGIN)));
            b.push(Stmt::set(name.clone(), Expr::string(&snd.file)));
            continue;
        }
        globals.push(global(name.clone(), "sound"));
        let flag = |f: u32| Expr::bool(snd.flags & f != 0);
        b.push(Stmt::set(
            name.clone(),
            Expr::call(
                "CreateSound",
                vec![
                    Expr::string(&snd.file),
                    flag(LOOPING),
                    flag(SOUND_3D),
                    flag(STOP_OUT_OF_RANGE),
                    Expr::int(snd.fade_in),
                    Expr::int(snd.fade_out),
                    Expr::string(&snd.eax),
                ],
            ),
        ));
        let label = game.sound_labels.get(&snd.file.to_lowercase());
        if let Some(label) = label {
            b.push(Stmt::call("SetSoundParamsFromLabel", vec![me(), Expr::string(label)]));
        }
        if let Some(ms) = duration(&snd.file) {
            b.push(Stmt::call("SetSoundDuration", vec![me(), Expr::int(ms)]));
        }
        // Without a label, channel, volume and pitch are written whatever
        // they are, "default" too (-1, 4294967296.0).
        let own = label.is_none();
        if own || snd.channel != DEFAULT_INT {
            b.push(Stmt::call("SetSoundChannel", vec![me(), Expr::int(snd.channel)]));
        }
        if own || snd.volume != DEFAULT_INT {
            b.push(Stmt::call("SetSoundVolume", vec![me(), Expr::int(snd.volume)]));
        }
        if own || real(snd.pitch) {
            b.push(Stmt::call("SetSoundPitch", vec![me(), coord(snd.pitch)]));
        }
        if snd.flags & SOUND_3D != 0 {
            if real(snd.min_distance) && real(snd.max_distance) {
                b.push(Stmt::call("SetSoundDistances", vec![me(), coord(snd.min_distance), coord(snd.max_distance)]));
            }
            if real(snd.distance_cutoff) {
                b.push(Stmt::call("SetSoundDistanceCutoff", vec![me(), coord(snd.distance_cutoff)]));
            }
            if real(snd.cone_inside) && real(snd.cone_outside) && snd.cone_outside_volume != DEFAULT_INT {
                b.push(Stmt::call("SetSoundConeAngles", vec![me(), coord(snd.cone_inside), coord(snd.cone_outside), Expr::int(snd.cone_outside_volume)]));
            }
            if real(snd.cone_x) && real(snd.cone_y) && real(snd.cone_z) {
                b.push(Stmt::call("SetSoundConeOrientation", vec![me(), coord(snd.cone_x), coord(snd.cone_y), coord(snd.cone_z)]));
            }
        }
    }
    (function("InitSounds", b), globals)
}

/// The cameras (`gg_cam_<name>`): each field, and where it looks.
fn cameras(c: &w3c_rs::Cameras) -> (Function, Vec<Global>) {
    let mut b = Vec::new();
    let mut globals = Vec::new();
    for cam in &c.cameras {
        let name = format!("gg_cam_{}", wtg_rs::jass::ident(&cam.name));
        globals.push(global(name.clone(), "camerasetup"));
        b.push(Stmt::set(name.clone(), Expr::call("CreateCameraSetup", Vec::new())));
        let mut fields = vec![
            ("CAMERA_FIELD_ZOFFSET", cam.z_offset),
            ("CAMERA_FIELD_ROTATION", cam.rotation),
            ("CAMERA_FIELD_ANGLE_OF_ATTACK", cam.angle_of_attack),
            ("CAMERA_FIELD_TARGET_DISTANCE", cam.distance),
            ("CAMERA_FIELD_ROLL", cam.roll),
            ("CAMERA_FIELD_FIELD_OF_VIEW", cam.field_of_view),
            ("CAMERA_FIELD_FARZ", cam.far_z),
        ];
        if c.layout == w3c_rs::Layout::Local {
            fields.extend([
                ("CAMERA_FIELD_NEARZ", cam.near_z),
                ("CAMERA_FIELD_LOCAL_PITCH", cam.local_pitch),
                ("CAMERA_FIELD_LOCAL_YAW", cam.local_yaw),
                ("CAMERA_FIELD_LOCAL_ROLL", cam.local_roll),
            ]);
        }
        for (field, v) in fields {
            b.push(Stmt::call("CameraSetupSetField", vec![Expr::var(name.clone()), Expr::var(field), coord(v), coord(0.0)]));
        }
        b.push(Stmt::call("CameraSetupSetDestPosition", vec![Expr::var(name.clone()), coord(cam.target_x), coord(cam.target_y), coord(0.0)]));
    }
    (function("CreateCameras", b), globals)
}

/// The map's part that comes after the rest of the code: the triggers of
/// map initialization run.
pub fn end(map: &Map) -> Module {
    let run = map.initialization.iter().map(|t| Stmt::call("ConditionalTriggerExecute", vec![Expr::var(t.clone())])).collect();
    let mut m = Module::default();
    m.functions.push(function("RunInitializationTriggers", run));
    m.functions.push(function("main", vec![Stmt::call("RunInitializationTriggers", Vec::new())]));
    m
}

#[cfg(test)]
#[path = "map.test.rs"]
mod tests;
