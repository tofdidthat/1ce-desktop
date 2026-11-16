//! Structural RIFF/WAVE check, not an audio-quality or silence detector.
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::Path,
};

#[allow(dead_code)]
pub(super) fn validate(path: &Path) -> Result<(), String> {
    let mut file = File::open(path).map_err(|e| format!("Missing master WAV: {e}"))?;
    inspect(&mut file).map(|_| ())
}

pub(crate) struct WaveInfo {
    pub code: u16,
    pub channels: u32,
    pub rate: u32,
    pub bits: u32,
    pub data_offset: u64,
    pub data_size: u64,
    pub align: u64,
}
pub(crate) fn inspect(file: &mut File) -> Result<WaveInfo, String> {
    file.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
    let length = file.metadata().map_err(|e| e.to_string())?.len();
    let mut header = [0u8; 12];
    file.read_exact(&mut header)
        .map_err(|_| "Truncated WAV header")?;
    if &header[..4] != b"RIFF"
        || &header[8..] != b"WAVE"
        || u32::from_le_bytes(header[4..8].try_into().unwrap()) as u64 + 8 != length
    {
        return Err("Invalid or incomplete RIFF/WAVE file".into());
    }
    let mut fmt = None;
    let mut data = None;
    let mut offset = 12u64;
    let mut chunks = 0;
    while offset < length {
        chunks += 1;
        if chunks > 4096 {
            return Err("Too many WAV chunks".into());
        }
        if length - offset < 8 {
            return Err("Truncated WAV chunk".into());
        }
        file.seek(SeekFrom::Start(offset))
            .map_err(|e| e.to_string())?;
        let mut chunk = [0u8; 8];
        file.read_exact(&mut chunk).map_err(|e| e.to_string())?;
        let size = u32::from_le_bytes(chunk[4..].try_into().unwrap()) as u64;
        let end = offset + 8 + size;
        if end > length {
            return Err("WAV chunk exceeds file bounds".into());
        }
        if &chunk[..4] == b"fmt " {
            if fmt.is_some() || size < 16 {
                return Err("Invalid WAV format chunk".into());
            }
            let mut bytes = [0u8; 16];
            file.read_exact(&mut bytes).map_err(|e| e.to_string())?;
            let code = u16::from_le_bytes(bytes[0..2].try_into().unwrap());
            let channels = u16::from_le_bytes(bytes[2..4].try_into().unwrap()) as u32;
            let rate = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
            let byte_rate = u32::from_le_bytes(bytes[8..12].try_into().unwrap());
            let align = u16::from_le_bytes(bytes[12..14].try_into().unwrap()) as u32;
            let bits = u16::from_le_bytes(bytes[14..16].try_into().unwrap()) as u32;
            if !matches!((code, bits), (1, 16 | 24 | 32) | (3, 32))
                || !(1..=2).contains(&channels)
                || !(8000..=192000).contains(&rate)
                || align != channels * (bits / 8)
                || byte_rate != rate * align
            {
                return Err("Unsupported or inconsistent WAV format".into());
            }
            fmt = Some((code, channels, rate, bits, align as u64));
        } else if &chunk[..4] == b"data" {
            if data.is_some() || size == 0 {
                return Err("Empty or duplicate WAV audio data".into());
            }
            data = Some((offset + 8, size));
        }
        offset = end + (size & 1);
    }
    match (fmt, data) {
        (Some((code, channels, rate, bits, align)), Some((data_offset, data_size)))
            if data_size % align == 0 && offset == length =>
        {
            Ok(WaveInfo {
                code,
                channels,
                rate,
                bits,
                data_offset,
                data_size,
                align,
            })
        }
        _ => Err("Missing or misaligned WAV audio data".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn accepts_pcm_and_rejects_truncated_empty_and_fake_audio() {
        let root = std::env::temp_dir().join(format!("onece-wave-{}", crate::render::attempt_id()));
        std::fs::create_dir(&root).unwrap();
        let path = root.join("master.wav");
        let mut bytes = b"RIFF".to_vec();
        bytes.extend(40u32.to_le_bytes());
        bytes.extend(b"WAVEfmt ");
        bytes.extend(16u32.to_le_bytes());
        bytes.extend(1u16.to_le_bytes());
        bytes.extend(1u16.to_le_bytes());
        bytes.extend(44100u32.to_le_bytes());
        bytes.extend(88200u32.to_le_bytes());
        bytes.extend(2u16.to_le_bytes());
        bytes.extend(16u16.to_le_bytes());
        bytes.extend(b"data");
        bytes.extend(4u32.to_le_bytes());
        bytes.extend([1, 0, 2, 0]);
        std::fs::write(&path, &bytes).unwrap();
        assert!(validate(&path).is_ok());
        let mut empty = bytes[..44].to_vec();
        empty[4..8].copy_from_slice(&36u32.to_le_bytes());
        empty[40..44].copy_from_slice(&0u32.to_le_bytes());
        std::fs::write(&path, &empty).unwrap();
        assert!(validate(&path).is_err());
        let mut bad_rate = bytes.clone();
        bad_rate[28..32].copy_from_slice(&1u32.to_le_bytes());
        std::fs::write(&path, &bad_rate).unwrap();
        assert!(validate(&path).is_err());
        bytes.pop();
        std::fs::write(&path, &bytes).unwrap();
        assert!(validate(&path).is_err());
        std::fs::write(&path, b"not a real wave file").unwrap();
        assert!(validate(&path).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
}
