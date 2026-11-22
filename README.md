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
  Editor makes it, written as this toolchain's JASS and read into the IR
  like any other source. Two parts around the rest of the code, whose
  `main`s merge with them in order:
  - `start`: `config` (`SetMapName`, players, teams, placement, start
    locations; `InitCustomPlayerSlots`, `InitCustomTeams` with what each
    force's players share and allied victory, `InitAllyPriorities`, which
    World Editor writes whatever its flag says), and a `main` setting the
    map up: camera bounds (`war3map.w3i`'s, less the camera's margins),
    day and night models, terrain fog and water tint (each when its flag
    is set), sound environment (the tileset's when none), ambient sounds,
    music, then `InitBlizzard` and `InitGlobals`. The triggers' `main`s
    follow (World Editor's `InitCustomTriggers`).
  - `end`: `RunInitializationTriggers`, the triggers of map
    initialization run once every `main` has.

  What the game's files decide is read from them (`GameData`, through the
  project's lookup): the camera's margins (`UI\MiscData.txt`), and by
  tileset the day and night models, the ambient sounds and the sound
  environment (`UI\WorldEditData.txt`). The triggers' variables and
  `InitGlobals`, and which triggers run at initialization, are
  `wtg-rs`'s (`jass::variables`, `jass::initialization_triggers`).
  Regions, cameras, sounds and what's placed come next.

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
World Editor's does, and its whole `main`, 1469 calls, is World Editor's
(every trigger set up alike; their order is the triggers' as they are,
newer than its `war3map.j`).
