# wc3-build

Builds a Warcraft III map's script. Every source goes into one model of
the code (the IR, `ir`), and each output is made from it. The plan, and
what's decided about it, is in the [`wc3-rs` README](../wc3-rs/README.md#building-a-map):
the IR keeps what's object-oriented (libraries, structs, methods) for an
output that has classes (AngelScript for UjAPI), and the map's data goes
into it as code too.

## What's there

- **`ir`**: the model: types, natives, globals, functions, statements,
  expressions, and what vJASS and Zinc add as items of their own:
  libraries and scopes (`requires`, initializer, the scope they're in),
  structs (size, `extends`, fields, methods, the modules they take in),
  interfaces (with `defaults`), modules, function interfaces, dynamic
  array types. Every global, function, struct ... has its `Owner`: its
  library or scope, and who sees it (everyone; vJASS's `public`, as
  `Scope_name` outside; private). A name is kept as written there, and
  `a.b`, `a.b()`, `a.b[i]`, `set a.b = v`, `static if` and `%` as they
  are: what `a` is takes the types, which lowering has (the build's stage
  4). Each item knows its file and line (`Origin`) for errors; a literal
  is kept as it was written (`'hfoo'`, `$FF`, `.5`).
- **`preprocess`**: JassHelper's preprocessor, on all the code at once,
  before any of it is read (`from_jass::module_with`): `//! import
  [vjass|zinc|comment] "path"` (each file once, as a file of its own;
  where the path leads is the caller's: the project's build looks beside
  the importing file, then in the project's folder, then in its layers),
  `//! textmacro NAME [takes a, b]` ... `//! endtextmacro` (two of a
  name an error; `textmacro_once` may be declared again, the first
  kept), `//! runtextmacro [optional] NAME("x", ...)` (its lines in its
  place, `$a$` the argument, its own `runtextmacro`s run too, 64 deep at
  most; `optional`: nothing without the macro), `//! novjass` ...
  `//! endnovjass` (left out). Each line made keeps where it comes from:
  a macro's, the line of the `runtextmacro` that put it there, so what's
  said of the code is said of the lines written (`Module::map_lines`).
  `//! inject`, `//! dovjassinit`, `//! loaddata`, `//! external` aren't
  run yet: an error each, nothing lost quietly.
- **`from_jass`**: JASS and vJASS files (parsed by `jass-rs`) into one
  module, in their order; a `.zn` file and a `//! zinc` block go to
  `from_zinc`. A statement outside a function is an error. A `static
  if` with `elseif`s is one in each one's `else`; a `keyword` is left
  out; a `hook` goes in as it is (`lower` makes it).
- **`from_zinc`**: Zinc into the same IR. A library's members are
  private unless `public` and its `onInit` is its initializer; outside a
  library (this toolchain's global scope) everything is everyone's.
  Zinc's own forms become JASS's: `while` and `do ... while` a `loop`
  with an `exitwhen not (...)`; `for (0 <= i < n)` sets `i` to the first
  value (one past a strict bound), leaves when past the other
  (`exitwhen i >= n`), and steps by 1 at the end, up or down as the
  bounds say; `break` is `exitwhen true`; `x += v` is `set x = x + v`;
  `else if` an `elseif`; a block its statements. An anonymous function is
  a function of its own (`wcb__anon_<n>`, private to its library), named
  where it was. An array of more than one dimension isn't read yet.
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
     `config` and `main` (a `main` is made if there's none). Lowering's
     `F.evaluate(...)` (`wcb__call_F`) goes the same way wherever `F` is,
     and `F.execute(...)` (`wcb__exec_F`) too, but runs the trigger's
     action (`wcb__action_F`): a thread of its own, which reads the
     arguments before it can wait.
  4. Locals to the top of their function: those before its first other
     statement stay; any other is declared at the top and set where it
     was.
  5. Printed with `set`, `call`, `local` and `takes nothing returns
     nothing`, in parentheses where the game's precedence needs them
     (`and` looser than `or`).

`jass_of(files)` is the two together. `wc3-mcp`'s `jass_build` runs it on
files or a folder. Before step 1 the module is lowered (`lower`); what
isn't lowered yet is refused, each item with where it is ("interface
Shape isn't built into JASS yet").

- **`lua`** (`jass::Options::script` `Lua`): the same tree, after the same
  steps, written in the game's Lua (see the module's comment).
- **`angelscript`** (`Script::AngelScript`): UjAPI's AngelScript, as
  decided with its author: after the same steps but sorting (AngelScript
  calls a function wherever it is), `common.j`'s natives and globals in
  `Jass::` (`jass::Options::common_j`), every handle type a type of its
  own with `nil`, `int`/`float`/`bool`/`string`, `code` UjAPI's funcdef
  (`code@`, `function F` as `@F`), `'hfoo'` as in JASS, `void main()`
  and `void config()`. UjAPI ships no `Blizzard.j`: what the code uses
  of it (`Options::blizzard_j`), and what that uses, is taken in
  (`take_used`, tree-shaking) and written too. An array is a class of
  the namespace `WC3` for each type of element used (`WC3::int_array`):
  AngelScript's `array<T>` has a size, a JASS array has none, so its
  `get_opIndex`/`set_opIndex` read any index (what's never set, the
  type's empty value) and grow as it's set; `a[i] = x` reads as in JASS.
  A string is never null (`null` where one goes is `""`), a variable
  always has a value, a name that's one of AngelScript's words gets the
  build's prefix (`wcb__in`). Not yet: structs as classes (they're
  lowered into arrays, as for JASS).
- **`lower`** (the build's stage 4): what vJASS and Zinc add, made into
  JASS, for the JASS output and AngelScript's for now (the plan: classes
  for AngelScript).
  - Libraries in the order of `requires` (one required and not there is
    an error, unless `optional`; so is a cycle). A private name is
    `<scope>__<name>` (a scope in a scope `<outer>_<inner>__<name>`), a
    public one `<scope>_<name>`; a name used in a scope is its own, then
    its parent's ..., then everyone's, a function's parameters and locals
    first.
  - Initializers run at map initialization, a library's and a struct's
    `onInit` each by `ExecuteFunc` (a thread of its own), a scope's
    called straight (as JassHelper's in Diablo III beta; the others
    aren't seen in a map yet): first the `onInit`s of structs in no
    library, then for each library in order its structs' `onInit`s (a
    module's before its struct's) and its initializer, then the
    scopes'. They're called right after `InitBlizzard()`, as
    JassHelper does (before `InitGlobals` sets the triggers' variables);
    without it, a `main` of their own after the first.
  - A trigger's `InitTrig_<name>` that no code has (custom text that's
    a scope or a library with an initializer of its own) isn't called,
    as JassHelper does.
  - `static if` keeps the branch its constants choose: `LIBRARY_<name>`,
    `DEBUG_MODE` (false), constant boolean globals, `not`, `and`, `or`,
    `==`, `!=`; anything else is an error. `%` is `ModuloInteger`
    (`ModuloReal` by a real literal).
- **`structs`** (part of `lower`), as JassHelper does: an instance's field
  `f` of `S` the array `s__S_f` (a field array of `k` by `this * k + i`),
  a static one the global `s__S_f`; a method the function `s__S_m`,
  `this` first; an allocator but for `extends array` (a free list,
  `si__S_F`, `si__S_I`, `si__S_V`: `s__S__allocate` gives back the last
  instance given back or a new one, none (0) past the size, and sets the
  fields that have a value; `s__S_destroy` runs `onDestroy` and gives it
  back, but not one that isn't taken; `s__S_deallocate` only gives it
  back). `S.create` is the struct's own `create`, else `allocate`;
  `S(x)` and `thistype(x)` are `x`; modules are taken in.
  `F.evaluate(...)` and `F.execute(...)`, of a function or a method
  (`x.m.evaluate(...)`), go through its trigger, the arguments in globals
  (above). A family of several (an interface's structs, a struct and its
  children) shares its root's free list and keeps each instance's type
  (`si__R_type`); a parent's fields are the parent's, and allocating a
  child sets its parent's fields too. A method of an interface, a `stub`
  method and `destroy` go by the instance's type: `wcb__d_R_m` puts the
  arguments into globals of its own and evaluates the type's trigger
  (`wcb__dt_R_m[type]`), whose condition calls that struct's method (or
  its parent's) and keeps what it returns; a type without one returns
  the interface's `defaults`; destroying runs the struct's `onDestroy`,
  then its parents'. The triggers are made before any initializer runs.
  Every other call is straight, `super.m()` to the parent's.
  `S.typeid` is the struct's type, `x.getType()` the instance's. A struct
  must have every method of its interface that has no `defaults`. What
  `a` in `a.b` is comes from the types: parameters, locals, globals,
  fields, what functions and methods return, casts. A struct's type is
  `integer`. A `hook Name Target` (`Target` a function, or a struct's
  static method `S.m`): `h__Name`, taking and giving back what `Name`
  does (a native's from the game's `common.j` and `Blizzard.j`:
  `jass::Options::game`), calls each of its targets in turn, then
  `Name`; every call of `Name` but in the targets themselves (a hook
  calling what it hooks would run itself) is a call of it. A target
  taking something else is an error. A function interface `I` is `integer` too: `I.f` is `f`'s
  number among the functions taken as `I`'s, and `x.evaluate(...)` /
  `x.execute(...)` (`wcb__fi_I`, `wcb__fx_I`) put the arguments into
  `I`'s globals and evaluate or execute the trigger of that number
  (`wcb__fit_I[x]`), which calls the function; a function that doesn't
  take and return what `I` does is an error. Operators are methods
  named as JassHelper's: `a.x` and `set a.x = v` of a struct without a
  field `x` call `s__S__get_x` and `s__S__set_x`, `a[i]` and `set a[i] =
  v` of a struct (that isn't an array; `S[i]` a static one)
  `s__S__getindex` and `s__S__setindex`, `a < b` `s__S__lt(a, b)` (`a >
  b` is `b < a`, `<=` and `>=` the other's `not`). A `delegate` field
  gives its struct what its type has: `a.x` and `a.m()` of a struct
  without them are `a.d.x` and `a.d.m()`, through a delegate's own
  delegates too. Not lowered yet (errors): an operator of an interface
  or a `stub` one, a static `delegate` or an array of them, more than
  8190 instances.

- **`map`**: the map's own part of its script, out of its files, as World
  Editor makes it, made as the IR's nodes (`ir`'s `Expr::call`,
  `Stmt::set` ...: every output converts the same tree). Two parts around
  the rest of the code, whose `main`s merge with them in order:
  - `start`: `config` (`SetMapName`, players, teams, placement, start
    locations; `InitCustomPlayerSlots`, then with forces of the map's own
    `InitCustomTeams` (what each force's players share, allied victory),
    else each slot a user's and `InitGenericPlayerSlots`;
    `InitAllyPriorities` when any start location prefers another, whatever
    the flag says), and a `main` setting the map up: camera bounds
    (`war3map.w3i`'s, less the camera's margins), day and night models,
    terrain fog (its color stored blue, green, red), the map's weather
    over the whole map, water tint, sound environment (the tileset's when
    none), ambient sounds, music, then `InitSounds` (`war3map.w3s`),
    `CreateRegions` (`war3map.w3r`, each with its weather and its sound;
    two of a name declared once, each set),
    `CreateCameras` (`war3map.w3c`), `InitUpgrades`, `InitTechTree`,
    `InitRandomGroups` (`tables`), what's placed
    (`CreateAllDestructables`, `CreateAllItems`, `CreateAllUnits`:
    `placed`), `InitBlizzard` and `InitGlobals`. The triggers' `main`s
    follow (World Editor's `InitCustomTriggers`).
    - The players: a start location forced when it's fixed (1, not 2,
      whatever the map's flag); a race chosen when it's random, the
      players' settings aren't fixed, or the map is Reign of Chaos's
      (`war3map.w3i` 18); a start location's priority "low" when it's
      both. The game's placement (`MAP_PLACEMENT_USE_MAP_SETTINGS`) only
      with forces of the map's own none of which is allied, else teams
      together.
  - `end`: `RunInitializationTriggers`, the triggers of map
    initialization run once every `main` has.

  What the game's files decide is read from them (`GameData`, through the
  project's lookup): the camera's margins (`UI\MiscData.txt`), and by
  tileset the day and night models, the ambient sounds and the sound
  environment (`UI\WorldEditData.txt`), the labels of the game's sounds
  (`UI\SoundInfo\*.slk`, by their files). The triggers' variables and
  which triggers run at initialization are `wtg-rs`'s
  (`jass::map_variable`, `jass::initialization_triggers`). Random groups
  and item tables, upgrades and the tech tree are `tables`'.
  - Sounds: music is a string, its file; any other is made
    (`CreateSound`), with its label's settings when it's one of the
    game's (`SetSoundParamsFromLabel`), how long it plays, and each
    setting the file gives rather than World Editor's "default"; without
    a label, its channel, volume and pitch whatever they are (-1,
    4294967296.0 too).
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
  - Random ones (`uDNR`, `bDNR`, `iDNR`): the type set first
    (`unitID`, `itemID`: a column of one of the map's random groups,
    `gg_rg_000[1]`, the group by its number in the file and its global by
    its place; one of its own list by chance; any of a level), and the
    thing made only when one is chosen (not `-1`). A random unit is red
    unless it has a color.
  - Units: a function each for each owner's buildings and units
    (`CreateBuildingsForPlayer7`, `CreateUnitsForPlayer11`,
    `CreateNeutralHostile(Buildings)`, `CreateNeutralPassive(Buildings)`;
    the neutral owners are 12 and 15, or 24 and 27 from 1.29), by their
    numbers; `CreatePlayerBuildings` and `CreatePlayerUnits` even when
    empty. On each: a hero's level and attributes, life, mana, a gold
    mine's gold (any other's, 0 or 12500, isn't), a waygate's destination
    (none to a region the map hasn't), its color (the file's, else its
    type's team color; a mine's never), acquisition range (200 for
    "camp"), each hero ability chosen as many times as its level, the
    order that turns each ability's autocast on or off, inventory.
  - What something drops when it dies: a trigger on its death (a unit's
    change of owner too) and `<Doodad|Unit><its place in the file>_DropItems`,
    or one of the map's item tables' `ItemTable<number>_DropItems`;
    one item of each set that has any (a set short of 100% may drop none,
    `-1`; a random item `YlI1` is `ChooseRandomItemEx(ITEM_TYPE_ARTIFACT, 1)`).

  Within a type, World Editor's order of destructables and items is its
  own (neither their numbers nor their places in the file); ours is the
  file's. An angle is the file's in degrees: World Editor 6052 and 6059
  write each so, 6030 and 6031 (Blizzard's maps) one less in the last
  digit in about 2% of units, which no way of working it out from the
  file's `f32` matches. What hasn't been seen in a map yet, and is made
  as World Editor is known to: a hero's attributes, life, inventory,
  random items placed, random units of any level (`ChooseRandomCreep`,
  `ChooseRandomNPBuilding`).
- **`tables`**: what `war3map.w3i` says of the game beyond its players:
  `InitUpgrades` (each player's: an upgrade researched, its highest level
  counting from 1, `SetPlayerTechResearched`; unavailable from a level,
  `SetPlayerTechMaxAllowed`; a function per player that has any),
  `InitTechTree` (what each can't make, `SetPlayerTechMaxAllowed` 0),
  `InitRandomGroups` (each group's row chosen by its chances, its ids put
  in `gg_rg_<its place, 000>`, `-1` for an empty cell or no row) and each
  item table's `ItemTable<number>_DropItems`, as a thing's own drops.
- **`objects`**: what `placed` needs of the types, read as World Editor
  reads it: the game's tables (`UnitBalance.slk` `isbldg`, `unitUI.slk`
  `teamColor`, `UnitAbilities.slk` `abilList` (a gold mine has `Agld`),
  `ItemData.slk`, `AbilityData.slk` `hero`,
  `*AbilityFunc.txt` `Orderon`/`Orderoff`), then the map's object data,
  a custom object taking its standard one's. World Editor's quirks: a
  custom ability's order is always written, `""` when its standard one
  has none (a standard ability with none gets no call). An item of a set
  of drops is written as itself whatever its level (Diablo III beta's of
  11 and 13); CruiserCommand's `war3map.j` writes one of level 50 as none
  (`-1`), which nothing in its files explains.
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
- **`whole`**: a map's whole script (`whole::script`): the map's part
  made of its files, the code between its start and its end (the
  triggers' JASS, the map's custom script, the project's code, each with
  its dialect when its header names none: `from_jass::module_as`), built
  into the game's JASS. It knows no folders: every file comes from the
  caller's lookup (`Input::read`, and `read_patched` for the editor's
  files with their patches: `UI\WorldEditData.txt`): `war3map.w3i` (no
  map without it), `.w3r`, `.w3c`, `.w3s`, `.doo`, `Units.doo`, the
  object data, and the game's `UI\MiscData.txt`, `UI\SoundInfo\*.slk`,
  `Units\*.slk` and abilities' texts. `wc3-server`'s `build` gives it a
  project's.

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
Castle's whole `main` is 1475 calls. `MAPS` (a folder of maps' folders,
each as `MAP`) does it for many: of Blizzard's 108 melee maps (from `War3.mpq`,
`War3x.mpq` and `War3Patch.mpq`, which give `GAME_UI` and `GAME_FILES`
too), 104 make every function alike (of about 16,000 units' angles, 307
one off in the last digit); the four that don't are World Editor's for what their files
don't say (an old World Editor's script, a sound's distance taken from
its label's table, `ChooseRandomItem` where every other map of the same
World Editor has `ChooseRandomItemEx`, a tileset's missing sound
environment), listed with why in the test. The test takes `MAP` (the map's
files, with its object data), `WTG_TEXT`, `MAP_PATCH` (a project's
`TriggerData.patch.txt`), `GAME_UI` (the layers' `UI` folders, lowest
first, with `SoundInfo`; the `Units` folder next to each gives the
tables of `objects`), `GAME_FILES` (the game's sound files). The names
the code uses are World Editor's script's, but for what the map's part
makes; in `main`, what the map's code calls (JassHelper's initializers)
isn't the map's part.

The whole map (`the_whole_main_as_world_editors`) takes `MAP_TRIGGERS`
(the triggers' JASS) and `MAP_CODE` (the map's code: its `war3map.wct`'s
header and the custom text of each trigger that's on, vJASS and all),
builds them with the map's part, and compares the `main` with the one of
its `war3map.j` (World Editor's, or JassHelper's after it); with
`JASS_GAME` (folders with `common.j` and `Blizzard.j`) what's built must
have no problem of names, types or the game's grammar that the map's
`war3map.j` hasn't (`BUILT_OUT` keeps the script). Diablo III beta (a
mix of 237 triggers and a `war3map.wct` of plain JASS and a vJASS scope):
its map's part is World Editor's (12 angles one off in the last digit);
the whole map builds (24,846 lines), its `main` is JassHelper's 667
calls (211 triggers set up alike), and it checks clean, as its
`war3map.j`. The Castle with its code: 1475 calls, 534 triggers, its
`war3map.j`'s one problem and no other.
