use super::*;

#[test]
fn game_data_from_its_files() {
    let g = GameData::read(
        "[CameraMargins]\n// cells\nleft=4\nright=4\ntop=2\nbottom=2\n",
        "[TerrainLights]\nL=Environment\\DNC\\L.mdl\n//#LINENOBETA\nY=Environment\\DNC\\Y.mdl\n[DayAmbience]\nY=CityScapeDay\n[Other]\nY=x\n",
    );
    assert_eq!(g.margins, [4.0, 4.0, 2.0, 2.0]);
    assert_eq!(g.terrain_lights.get(&'Y').map(String::as_str), Some("Environment\\DNC\\Y.mdl"));
    assert_eq!(g.day.get(&'Y').map(String::as_str), Some("CityScapeDay"));
    assert!(g.night.is_empty());
}

#[test]
fn the_end_runs_the_initialization_triggers() {
    let info = Info::default();
    let init = ["gg_trg_A".to_string(), "gg_trg_B".to_string()];
    let map = Map { info: &info, initialization: &init, globals: false };
    assert_eq!(
        end(&map),
        "function RunInitializationTriggers\n    ConditionalTriggerExecute(gg_trg_A)\n    ConditionalTriggerExecute(gg_trg_B)\nendfunction\n\nfunction main\n    RunInitializationTriggers()\nendfunction\n"
    );
}
