use super::*;

fn wav_bytes(rate: u32, data: usize) -> Vec<u8> {
    let mut b = b"RIFF\0\0\0\0WAVEfmt ".to_vec();
    b.extend(16u32.to_le_bytes());
    b.extend([1, 0, 1, 0]);
    b.extend(22050u32.to_le_bytes());
    b.extend(rate.to_le_bytes());
    b.extend([2, 0, 16, 0]);
    b.extend(b"LIST");
    b.extend(3u32.to_le_bytes());
    b.extend([0, 0, 0, 0]);
    b.extend(b"data");
    b.extend((data as u32).to_le_bytes());
    b.extend(vec![0; data]);
    b
}

#[test]
fn wav_its_data_over_its_rate() {
    // 44100 bytes a second, 49920 of data: 1131.97, rounded.
    assert_eq!(duration_ms(&wav_bytes(44100, 49920)), Some(1132));
    assert_eq!(duration_ms(b"RIFF\0\0\0\0WAVE"), None);
}

#[test]
fn mp3_from_its_first_frame() {
    // An ID3 tag of 6 bytes, a byte of junk, then frames of MPEG-1 Layer
    // III, 128 kbps, 44.1 kHz: 16000 bytes from the first is a second.
    let mut b = b"ID3\x03\0\0\0\0\0\x06abcdef\x00".to_vec();
    let frame = [0xFF, 0xFB, 0x90, 0x00];
    let start = b.len();
    b.extend(frame);
    b.resize(start + 16000, 0);
    assert_eq!(duration_ms(&b), Some(1000));
    // MPEG-2, 64 kbps: 8000 bytes a second.
    let mut b = vec![0xFF, 0xF3, 0x84, 0x00];
    b.resize(4000, 0);
    assert_eq!(duration_ms(&b), Some(500));
    // An ID3v1 tag at the end: 128 bytes more, as World Editor counts.
    let mut b = vec![0xFF, 0xF3, 0x84, 0x00];
    b.resize(4000 - 128, 0);
    b.extend(b"TAG");
    b.resize(4000, 0);
    assert_eq!(duration_ms(&b), Some(516));
    assert_eq!(duration_ms(b"not a sound"), None);
}
