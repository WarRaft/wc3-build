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
    let map = Map { info: &info, variables: &[], initialization: &init, regions: None, cameras: None, sounds: None, duration: &|_| None };
    let out = crate::jass::build(&end(&map)).unwrap();
    assert!(out.contains("function RunInitializationTriggers takes nothing returns nothing\n    call ConditionalTriggerExecute(gg_trg_A)\n    call ConditionalTriggerExecute(gg_trg_B)\nendfunction\n"), "{out}");
    assert!(out.ends_with("function main takes nothing returns nothing\n    call RunInitializationTriggers()\nendfunction\n"), "{out}");
}

#[test]
fn regions_and_cameras() {
    let info = Info::default();
    let region = |name: &str, l: f32, b: f32, r: f32, t: f32| w3r_rs::Region::new(name, l, b, r, t, 0);
    let mut rain = region("REG Rain", -64.0, 0.0, 64.0, 128.0);
    rain.weather = w3i_rs::Rawcode::from(b"RAhr");
    let mut water = region("Water", 0.0, 0.0, 768.0, 640.0);
    water.sound = "gg_snd_Water".into();
    let regions = w3r_rs::Regions { version: 5, regions: vec![rain, water] };
    let cam = w3c_rs::Camera::new("Cam 1", 10.25, 0.0);
    let cameras = w3c_rs::Cameras { version: 0, layout: w3c_rs::Layout::Classic, cameras: vec![cam] };
    let map = Map { info: &info, variables: &[], initialization: &[], regions: Some(&regions), cameras: Some(&cameras), sounds: None, duration: &|_| None };
    let out = crate::jass::build(&start(&map, &GameData::default()).unwrap()).unwrap();
    for part in [
        "    rect gg_rct_REG_Rain = null\n    rect gg_rct_Water = null\n    camerasetup gg_cam_Cam_1 = null\n",
        "function CreateRegions takes nothing returns nothing\n    local weathereffect we\n    set gg_rct_REG_Rain = Rect(-64.0, 0.0, 64.0, 128.0)\n    set we = AddWeatherEffect(gg_rct_REG_Rain, 'RAhr')\n    call EnableWeatherEffect(we, true)\n    set gg_rct_Water = Rect(0.0, 0.0, 768.0, 640.0)\n    call SetSoundPosition(gg_snd_Water, 384.0, 320.0, 384.0)\n    call RegisterStackedSound(gg_snd_Water, true, 768.0, 640.0)\nendfunction\n",
        "    set gg_cam_Cam_1 = CreateCameraSetup()\n    call CameraSetupSetField(gg_cam_Cam_1, CAMERA_FIELD_ZOFFSET, 0.0, 0.0)\n    call CameraSetupSetField(gg_cam_Cam_1, CAMERA_FIELD_ROTATION, 90.0, 0.0)\n",
        "    call CameraSetupSetField(gg_cam_Cam_1, CAMERA_FIELD_FIELD_OF_VIEW, 70.0, 0.0)\n",
        "    call CameraSetupSetDestPosition(gg_cam_Cam_1, 10.2, 0.0, 0.0)\n",
        "    call CreateRegions()\n    call CreateCameras()\n    call InitBlizzard()\n",
    ] {
        assert!(out.contains(part), "{part}\n---\n{out}");
    }
}

#[test]
fn sounds_as_world_editor_makes_them() {
    use w3s_rs::{DEFAULT_INT, DEFAULT_REAL, MUSIC, SOUND_3D};
    let mut game = GameData::default();
    game.add_sound_info("ID;PWXL;N;E\nC;Y1;X1;K\"SoundName\"\nC;X2;K\"FileNames\"\nC;X3;K\"DirectoryBase\"\nC;Y2;X1;K\"Click\"\nC;X2;K\"Click1.wav,Click2.wav\"\nC;X3;K\"Sound\\Interface\\\"\nE\n");
    assert_eq!(game.sound_labels.get("sound\\interface\\click2.wav").map(String::as_str), Some("Click"));
    let sound = |name: &str, file: &str, flags: u32| {
        let mut s = w3s_rs::Sound::new(name, file);
        s.flags = flags;
        s
    };
    let music = sound("gg_snd_Theme", "war3mapImported\\Theme.mp3", MUSIC);
    let mut click = sound("gg_snd_Click", "Sound\\Interface\\Click2.wav", SOUND_3D);
    click.eax = "DefaultEAXON".into();
    for v in [&mut click.channel, &mut click.volume, &mut click.cone_outside_volume] {
        *v = DEFAULT_INT;
    }
    for v in [&mut click.pitch, &mut click.min_distance, &mut click.max_distance, &mut click.distance_cutoff, &mut click.cone_inside, &mut click.cone_outside, &mut click.cone_x, &mut click.cone_y, &mut click.cone_z] {
        *v = DEFAULT_REAL;
    }
    let mut voice = sound("gg_snd_Voice", "war3mapImported\\Voice.mp3", 0);
    voice.eax = String::new();
    voice.channel = 0;
    voice.volume = 127;
    voice.pitch = 1.0;
    let sounds = w3s_rs::Sounds { version: 1, sounds: vec![music, click, voice] };
    let info = Info::default();
    let duration = |f: &str| f.ends_with("Voice.mp3").then_some(5010).or(f.ends_with("Click2.wav").then_some(476));
    let map = Map { info: &info, variables: &[], initialization: &[], regions: None, cameras: None, sounds: Some(&sounds), duration: &duration };
    let out = crate::jass::build(&start(&map, &game).unwrap()).unwrap();
    for part in [
        "    string gg_snd_Theme\n    sound gg_snd_Click = null\n    sound gg_snd_Voice = null\n",
        "function InitSounds takes nothing returns nothing\n    set gg_snd_Theme = \"war3mapImported\\\\Theme.mp3\"\n    set gg_snd_Click = CreateSound(\"Sound\\\\Interface\\\\Click2.wav\", false, true, false, 10, 10, \"DefaultEAXON\")\n    call SetSoundParamsFromLabel(gg_snd_Click, \"Click\")\n    call SetSoundDuration(gg_snd_Click, 476)\n",
        "    set gg_snd_Voice = CreateSound(\"war3mapImported\\\\Voice.mp3\", false, false, false, 10, 10, \"\")\n    call SetSoundDuration(gg_snd_Voice, 5010)\n    call SetSoundChannel(gg_snd_Voice, 0)\n    call SetSoundVolume(gg_snd_Voice, 127)\n    call SetSoundPitch(gg_snd_Voice, 1.0)\nendfunction\n",
        "    call InitSounds()\n    call InitBlizzard()\n",
    ] {
        assert!(out.contains(part), "{part}\n---\n{out}");
    }
}
