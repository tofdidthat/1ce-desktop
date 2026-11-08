//! Read-only FL Studio project tempo extraction. Does not launch FL Studio or alter projects.
//! FLP format: FLhd header followed by FLdt events; event 156 = BPM * 1000.
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FlpTempo {
    pub bpm: f64,
    pub source: &'static str,
    /// More than one tempo-setting event can be ambiguous. This is NOT a reliable
    /// detector of tempo automation, which is currently unsupported.
    pub tempo_event_count: usize,
    pub needs_tempo_review: bool,
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.to_owned())
}

fn read_u16<R: Read>(reader: &mut R) -> io::Result<u16> {
    let mut b = [0_u8; 2];
    reader.read_exact(&mut b)?;
    Ok(u16::from_le_bytes(b))
}

fn read_u32<R: Read>(reader: &mut R) -> io::Result<u32> {
    let mut b = [0_u8; 4];
    reader.read_exact(&mut b)?;
    Ok(u32::from_le_bytes(b))
}

fn leb128<R: Read>(reader: &mut R, end: u64, pos: &mut u64) -> io::Result<u64> {
    let mut value = 0u64;
    for shift in (0..35).step_by(7) {
        if *pos >= end {
            return Err(invalid("Truncated FLP variable event length"));
        }
        let mut b = [0u8; 1];
        reader.read_exact(&mut b)?;
        *pos += 1;
        value |= u64::from(b[0] & 0x7f) << shift;
        if b[0] & 0x80 == 0 {
            if value > u64::from(u32::MAX) {
                return Err(invalid("FLP variable event length exceeds 32 bits"));
            }
            return Ok(value);
        }
    }
    Err(invalid("FLP variable event length uses more than five bytes"))
}

/// Parse only the base tempo events in an FLP; the caller must treat tempo
/// automation or multiple tempo events as potentially requiring manual review.
/// Works on any seekable stream so tests can use in-memory FLP fixtures.
pub fn parse_flp_tempo<R: Read + Seek>(reader: &mut R) -> io::Result<FlpTempo> {
    let file_size = reader.seek(SeekFrom::End(0))?;
    reader.seek(SeekFrom::Start(0))?;
    if file_size < 22 {
        return Err(invalid("FLP too small"));
    }
    let mut magic = [0u8; 4];
    reader.read_exact(&mut magic)?;
    if &magic != b"FLhd" {
        return Err(invalid("Missing FLhd header"));
    }
    let header_len = u64::from(read_u32(reader)?);
    if !(6..=1024).contains(&header_len) || 8u64 + header_len + 8 > file_size {
        return Err(invalid("Invalid FLhd length"));
    }
    reader.seek(SeekFrom::Start(8 + header_len))?;
    reader.read_exact(&mut magic)?;
    if &magic != b"FLdt" {
        return Err(invalid("Missing FLdt event block"));
    }
    let data_len = u64::from(read_u32(reader)?);
    let mut pos = 8u64 + header_len + 8;
    let end = pos.checked_add(data_len).ok_or_else(|| invalid("FLdt size overflow"))?;
    if data_len == 0 || end > file_size {
        return Err(invalid("Invalid or truncated FLdt event block"));
    }

    let mut first_modern: Option<f64> = None;
    let mut first_coarse: Option<u16> = None;
    let mut first_fine: Option<u16> = None;
    let mut tempo_events = 0usize;
    while pos < end {
        let mut id = [0u8; 1];
        reader.read_exact(&mut id)?;
        pos += 1;
        let len = match id[0] {
            0..=63 => 1u64,
            64..=127 => 2u64,
            128..=191 => 4u64,
            _ => leb128(reader, end, &mut pos)?,
        };
        let next = pos.checked_add(len).ok_or_else(|| invalid("Event length overflow"))?;
        if next > end {
            return Err(invalid("FLP event extends beyond FLdt"));
        }
        match id[0] {
            156 if first_modern.is_none() => {
                first_modern = Some(f64::from(read_u32(reader)?) / 1000.0);
                tempo_events += 1;
            }
            156 => { tempo_events += 1; },
            66 if first_coarse.is_none() => {
                first_coarse = Some(read_u16(reader)?);
                tempo_events += 1;
            }
            66 => { tempo_events += 1; },
            93 if first_fine.is_none() => {
                first_fine = Some(read_u16(reader)?);
            }
            _ => {}
        }
        // Absolute seek after each event prevents accidental desynchronization.
        reader.seek(SeekFrom::Start(next))?;
        pos = next;
    }

    let (bpm, source) = if let Some(modern) = first_modern {
        (modern, "modern-156")
    } else if let Some(coarse) = first_coarse {
        (f64::from(coarse) + f64::from(first_fine.unwrap_or(0)) / 1000.0, "legacy-66-93")
    } else {
        return Err(invalid("No known FLP tempo event"));
    };
    if !bpm.is_finite() || !(10.0..=999.0).contains(&bpm) {
        return Err(invalid("FLP tempo outside supported range"));
    }
    Ok(FlpTempo {
        bpm,
        source,
        tempo_event_count: tempo_events,
        needs_tempo_review: tempo_events > 1,
    })
}

pub fn read_flp_tempo(path: &Path) -> io::Result<FlpTempo> {
    let mut f = File::open(path)?;
    parse_flp_tempo(&mut f)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn flp(events: &[u8]) -> Vec<u8> {
        let mut bytes = b"FLhd".to_vec();
        bytes.extend_from_slice(&6u32.to_le_bytes());
        bytes.extend_from_slice(&[0, 0, 1, 0, 96, 0]);
        bytes.extend_from_slice(b"FLdt");
        bytes.extend_from_slice(&(events.len() as u32).to_le_bytes());
        bytes.extend_from_slice(events);
        bytes
    }
    fn modern(bpm_thousandths: u32) -> Vec<u8> {
        let mut ev = vec![156];
        ev.extend_from_slice(&bpm_thousandths.to_le_bytes());
        ev
    }
    #[test]
    fn reads_three_140_bpm_event_156_fixtures() {
        for _ in 0..3 {
            let parsed = parse_flp_tempo(&mut Cursor::new(flp(&modern(140_000)))).unwrap();
            assert_eq!(parsed.bpm, 140.0);
            assert_eq!(parsed.source, "modern-156");
            assert!(!parsed.needs_tempo_review);
        }
    }
    #[test]
    fn reads_fractional_modern_bpm_and_skips_variable_events() {
        let mut ev = vec![192, 0x82, 0x01]; // variable event with 130-byte payload
        ev.extend_from_slice(&[0u8; 130]);
        ev.extend(modern(137_500));
        let parsed = parse_flp_tempo(&mut Cursor::new(flp(&ev))).unwrap();
        assert_eq!(parsed.bpm, 137.5);
    }
    #[test]
    fn reads_legacy_whole_and_fractional_bpm() {
        let mut ev = vec![66];
        ev.extend_from_slice(&135u16.to_le_bytes());
        ev.push(93);
        ev.extend_from_slice(&250u16.to_le_bytes());
        let parsed = parse_flp_tempo(&mut Cursor::new(flp(&ev))).unwrap();
        assert_eq!(parsed.bpm, 135.25);
        assert_eq!(parsed.source, "legacy-66-93");
    }
    #[test]
    fn multiple_tempo_events_trigger_review() {
        let mut events = modern(140_000);
        events.extend(modern(120_000));
        let parsed = parse_flp_tempo(&mut Cursor::new(flp(&events))).unwrap();
        assert_eq!(parsed.bpm, 140.0);
        assert!(parsed.needs_tempo_review);
    }
    #[test]
    fn rejects_malformed_truncated_and_out_of_range() {
        for bytes in [
            b"not a project".to_vec(),
            flp(&[156, 1]),
            flp(&[192, 0x80]),
            flp(&modern(9_000)),
        ] {
            assert!(parse_flp_tempo(&mut Cursor::new(bytes)).is_err());
        }
    }
    #[test]
    fn legacy_missing_fine_uses_integer_only() {
        let mut ev = vec![66];
        ev.extend_from_slice(&100u16.to_le_bytes());
        let parsed = parse_flp_tempo(&mut Cursor::new(flp(&ev))).unwrap();
        assert_eq!(parsed.bpm, 100.0);
    }
}
