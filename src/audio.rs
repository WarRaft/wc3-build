//! How long a sound file plays, as World Editor works it out for the map's
//! script (`SetSoundDuration`), in milliseconds, rounded:
//!
//! - WAV: the size of its data over its bytes a second.
//! - MP3: the file's size from its first frame, over the first frame's bit
//!   rate, as if every frame had it. A file of variable bit rate whose
//!   first frame is its Xing frame comes out longer than it plays, and a
//!   file with an ID3v1 tag at its end (`TAG`, 128 bytes) is taken for 128
//!   bytes longer than it is: World Editor's numbers all the same (every
//!   MP3 of The Castle, 78), which is what a map's script has.

/// How long `bytes`, a WAV or MP3 file, plays: `None` when it's neither,
/// or doesn't read.
pub fn duration_ms(bytes: &[u8]) -> Option<u32> {
    if bytes.starts_with(b"RIFF") { wav(bytes) } else { mp3(bytes) }
}

fn u32_le(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn wav(b: &[u8]) -> Option<u32> {
    if b.get(8..12)? != b"WAVE" {
        return None;
    }
    let (mut rate, mut size) = (None, None);
    let mut at = 12;
    while at + 8 <= b.len() {
        let id = &b[at..at + 4];
        let n = u32_le(b, at + 4)? as usize;
        match id {
            b"fmt " => rate = u32_le(b, at + 16),
            b"data" => size = Some(n),
            _ => {}
        }
        at += 8 + n + (n & 1);
    }
    let (rate, size) = (rate.filter(|&r| r > 0)?, size?);
    Some((size as f64 * 1000.0 / f64::from(rate)).round() as u32)
}

/// Kilobits a second, by bit rate index: MPEG-1 Layer III, MPEG-2 and 2.5
/// Layer III.
const BITRATES_1: [u32; 15] = [0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320];
const BITRATES_2: [u32; 15] = [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160];

fn mp3(b: &[u8]) -> Option<u32> {
    let mut at = 0;
    // An ID3v2 tag first: its size, seven bits a byte.
    if b.starts_with(b"ID3") && b.len() >= 10 {
        let size = (usize::from(b[6]) << 21) | (usize::from(b[7]) << 14) | (usize::from(b[8]) << 7) | usize::from(b[9]);
        at = 10 + size;
    }
    while at + 4 <= b.len() {
        let h = u32::from_be_bytes(b[at..at + 4].try_into().ok()?);
        let version = (h >> 19) & 3;
        let layer = (h >> 17) & 3;
        let index = ((h >> 12) & 15) as usize;
        let rate = (h >> 10) & 3;
        if (h >> 21) & 0x7FF != 0x7FF || version == 1 || layer != 1 || index == 0 || index == 15 || rate == 3 {
            at += 1;
            continue;
        }
        let kbps = if version == 3 { BITRATES_1[index] } else { BITRATES_2[index] };
        let id3v1 = if b.len() >= 128 && &b[b.len() - 128..b.len() - 125] == b"TAG" { 128 } else { 0 };
        let bits = (b.len() - at + id3v1) as f64 * 8.0;
        return Some((bits * 1000.0 / (f64::from(kbps) * 1000.0)).round() as u32);
    }
    None
}

#[cfg(test)]
#[path = "audio.test.rs"]
mod tests;
