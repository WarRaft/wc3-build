//! Builds a Warcraft III map's script: every source into one model of the
//! code ([`ir`]), the game's JASS made of it ([`jass`]). See the README.

pub mod angelscript;
pub mod audio;
pub mod from_jass;
pub mod from_zinc;
pub mod ir;
pub mod jass;
pub mod lower;
pub mod lua;
pub mod map;
pub mod objects;
pub mod dead_code;
pub mod inline;
pub mod string_hash;
pub mod placed;
pub mod preprocess;
pub mod structs;
pub mod tables;
pub mod uglify;
pub mod whole;

/// The game's JASS of `files` (this toolchain's JASS, or the game's), in
/// their order: see [`jass`].
pub fn jass_of(files: &[from_jass::Source]) -> Result<String, Vec<ir::Error>> {
    jass::build(&from_jass::module(files)?)
}
