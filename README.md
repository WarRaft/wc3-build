# wc3-build

Builds a Warcraft III map's script. Every source goes into one model of
the code (the IR, `ir`), and each output is made from it. The plan, and
what's decided about it, is in the [`wc3-rs` README](../wc3-rs/README.md#building-a-map):
the IR keeps what's object-oriented (libraries, structs, methods) for an
output that has classes (AngelScript for UjAPI), and the map's data goes
into it as code too.

## What's there

- **`ir`**: the model: types, natives, globals, functions, statements,
  expressions. Each item knows its file and line (`Origin`) for errors; a
  literal is kept as it was written (`'hfoo'`, `$FF`, `.5`).
- **`from_jass`**: JASS files (this toolchain's or the game's, parsed by
  `jass-rs`) into one module, in their order. Zinc isn't built yet; a
  statement outside a function is an error.
- **`jass`**: the game's JASS out of a module:
  1. Every `main` and every `config` (taking and returning nothing)
     merged into one, their statements in the order of the files; a local
     of a later one named as an earlier one's is renamed (`i__2`).
  2. Globals into `globals`, each after those its initial value reads
     (a cycle is an error). An initial value that calls a function of the
     module, which the game hasn't read at `globals`, is set at the start
     of `main` instead; a constant can't wait, and is an error.
  3. Functions sorted so that each comes after those it calls or names as
     code (strongly connected components, in their order otherwise),
     `config` and `main` last. A call below the caller (a cycle) goes
     through a trigger, as JassHelper's `.evaluate`: `wcb__call_F`, above
     every function, puts the arguments into `F`'s own globals and
     evaluates `wcb__trig_F`, whose condition `wcb__thunk_F` (right after
     `F`) calls `F` and keeps what it returns. JASS runs one thing at a
     time and an evaluated function takes no arguments, so no two calls
     race for those globals. The triggers are made at the start of
     `config` and `main` (a `main` is made if there's none).
  4. Locals to the top of their function: those before its first other
     statement stay; any other is declared at the top and set where it
     was.
  5. Printed with `set`, `call`, `local` and `takes nothing returns
     nothing`, in parentheses where the game's precedence needs them
     (`and` looser than `or`).

`jass_of(files)` is the two together. `wc3-mcp`'s `jass_build` runs it on
files or a folder.

- **`map`**: the map's own part of its script, out of its files, as World
  Editor makes it, made as the IR's nodes (`ir`'s `Expr::call`,
  `Stmt::set` ...: every output converts the same tree). Two parts around
  the rest of the code, whose `main`s merge with them in order:
  - `start`: `config` (`SetMapName`, players, teams, placement, start
    locations; `InitCustomPlayerSlots`, `InitCustomTeams` with what each
    force's players share and allied victory, `InitAllyPriorities`, which
    World Editor writes whatever its flag says), and a `main` setting the
    map up: camera bounds (`war3map.w3i`'s, less the camera's margins),
    day and night models, terrain fog and water tint (each when its flag
    is set), sound environment (the tileset's when none), ambient sounds,
    music, then `InitSounds` (`war3map.w3s`), `CreateRegions`
    (`war3map.w3r`, each with its weather and its sound), `CreateCameras`
    (`war3map.w3c`), what's placed (`CreateAllDestructables`,
    `CreateAllItems`, `CreateAllUnits`: `placed`), `InitBlizzard` and
    `InitGlobals`. The triggers' `main`s follow (World Editor's
    `InitCustomTriggers`).
  - `end`: `RunInitializationTriggers`, the triggers of map
    initialization run once every `main` has.

  What the game's files decide is read from them (`GameData`, through the
  project's lookup): the camera's margins (`UI\MiscData.txt`), and by
  tileset the day and night models, the ambient sounds and the sound
  environment (`UI\WorldEditData.txt`), the labels of the game's sounds
  (`UI\SoundInfo\*.slk`, by their files). The triggers' variables and
  which triggers run at initialization are `wtg-rs`'s
  (`jass::map_variable`, `jass::initialization_triggers`). Random groups
  and item tables, upgrades and the tech tree come next.
  - Sounds: music is a string, its file; any other is made
    (`CreateSound`), with its label's settings when it's one of the
    game's (`SetSoundParamsFromLabel`), how long it plays, and each
    setting the file gives rather than World Editor's "default".
- **`placed`**: what `war3map.doo` and `war3mapUnits.doo` place, as
  World Editor's script makes it. A thing is named (`gg_dest_B000_0261`,
  `gg_unit_hfoo_0012`, `gg_item_ratc_0003`: type and number) only when
  the rest of the code uses that name (`map::used_names`); else it's a
  local, or isn't kept.
  - Destructables: those the game doesn't make (the file's flag 2 clear:
    World Editor clears it for those its script makes) and those the
    code names; the named first, by type. `CreateDestructableZ` with the
    flag "fixed height" (4), `CreateDeadDestructable` with no life, part
    of its life set (`0.10 * life`).
  - Items: `CreateItem`, by type.
  - Units: a function each for each owner's buildings and units
    (`CreateBuildingsForPlayer7`, `CreateUnitsForPlayer11`,
    `CreateNeutralHostile(Buildings)`, `CreateNeutralPassive(Buildings)`;
    the neutral owners are 12 and 15, or 24 and 27 from 1.29), by their
    numbers; `CreatePlayerBuildings` and `CreatePlayerUnits` even when
    empty. On each: a hero's level and attributes, life, mana, gold, its
    team color (the type's, unless the file gives one), acquisition
    range (200 for "camp"), each hero ability chosen as many times as its
    level, the order that turns each ability's autocast on or off,
    inventory, a waygate's destination.
  - What something drops when it dies: a trigger on its death (a unit's
    change of owner too) and `<Doodad|Unit><its place in the file>_DropItems`,
    one item of each set that has any (a set short of 100% may drop none,
    `-1`; a random item `YlI1` is `ChooseRandomItemEx(ITEM_TYPE_ARTIFACT, 1)`).

  Within a type, World Editor's order of destructables and items is its
  own (neither their numbers nor their places in the file); ours is the
  file's. What hasn't been seen in a map yet, and is made as World Editor
  is known to: a hero's attributes, life, gold, a placed color,
  inventory, waygates. Random units and items, and drops from the map's
  item tables, are errors until random tables are built.
- **`objects`**: what `placed` needs of the types, read as World Editor
  reads it: the game's tables (`UnitBalance.slk` `isbldg`, `unitUI.slk`
  `teamColor`, `ItemData.slk`, `AbilityData.slk` `hero`,
  `*AbilityFunc.txt` `Orderon`/`Orderoff`), then the map's object data,
  a custom object taking its standard one's. World Editor's quirks: a
  custom ability's order is always written, `""` when its standard one
  has none (a standard ability with none gets no call); an item above
  level 10 in a set of drops is written as none (`-1`; seen once, an
  item of level 50).
- **`uglify`**: shorter names, on the finished tree (`jass::Options`'s
  `uglify`, `jass::build_with`): every name the module declares that the
  game doesn't need as it is (its functions but `main` and `config`,
  globals, types; each function's parameters and locals) renamed to the
  shortest free one (`a` ... `Z`, `aa` ...), the names used most getting
  the shortest. Natives and whatever the module uses without declaring
  (the game's names) are kept and never given, nor are JASS's words. A
  function's locals and parameters take the shortest names again in each
  function, clear of what it uses (they're given names nothing else can
  have first, so a top-level name given later is never taken for one). A
  literal naming a function in `ExecuteFunc("F")` or a global in
  `TriggerRegisterVariableEvent(t, "udg_x", ...)` is renamed with it;
  where `ExecuteFunc` is given a name that isn't a literal, every function
  taking nothing keeps its name, and the build says so (`Built::notes`).
  Three maps' scripts come out with the same problems, rule by rule, at
  64% to 76% of their size.
- **`audio`**: how long a sound file plays, as World Editor works it out:
  a WAV's data over its bytes a second; an MP3's size from its first frame
  over that frame's bit rate (so a variable bit rate file with a Xing
  frame comes out longer than it plays), an ID3v1 tag at its end taken for
  128 bytes more; rounded to the millisecond. The caller reads the file.

## Tests

`cargo test`: each output is read back by `jass-rs` and must break no
rule of the group `pure`, have no duplicate, and check clean for names and
types; calls only go up.

`tests/real.rs` (ignored) builds real scripts: `JASS_GAME` (folders with
the game's `common.j` and `Blizzard.j`) and `JASS_FILES` (each built
alone) or `JASS_DIR` (its `.j` files built together, the script written
next to them as `built.j.out`). The `war3map.j` of three maps build with
the problems of names and types they had and no more; the 752 triggers of
The Castle (the server's ignored test `wtg_jass_files` writes their JASS),
its variables and the header of its `war3map.wct` build into one script
whose only problems are the `gg_` names of the map's data, which the map's
script will make (the build's stage 2).

`tests/real_map.rs` (ignored) compares the map's part with World Editor's
`war3map.j` by calls, numbers by their values: `MAP_W3I`, `MAP_SCRIPT`,
`GAME_UI` (folders with `MiscData.txt` and `WorldEditData.txt`);
`MAP_VARIABLES` for `InitGlobals`; `MAP_DIR` (the triggers' JASS and
`initialization.txt`, which the server's `wtg_jass_files` writes) for the
whole `main`, World Editor's `InitCustomTriggers` and `InitTrig_` read
where they're called. The Castle and CruiserCommand: `config`, the
players, the forces, the priorities and the setting up in `main` call
what World Editor's do; The Castle's `InitGlobals` sets its 163 values as
World Editor's does, and its whole `main` is World Editor's
(every trigger set up alike; their order is the triggers' as they are,
newer than its `war3map.j`). Its sounds (422 calls, 78 MP3 and 8 WAV
durations), regions (338) and cameras (675), and CruiserCommand's, call
and set what World Editor's do, none left out; so do the placed things
of both, every function (The Castle: 108, with 96 that drop items;
CruiserCommand: 40), the destructables and items as sets, and The
Castle's whole `main` is 1475 calls. The test takes `MAP` (the map's
files, with its object data), `WTG_TEXT`, `MAP_PATCH` (a project's
`TriggerData.patch.txt`), `GAME_UI` (the layers' `UI` folders, lowest
first, with `SoundInfo`; the `Units` folder next to each gives the
tables of `objects`), `GAME_FILES` (the game's sound files). The names
the code uses are World Editor's script's, but for what the map's part
makes.
