//! Original, offline experimental detector. No third-party DSP implementation.
//! Scores are evidence, NOT calibrated probabilities; never auto-renames files.
use serde::{Deserialize, Serialize};
use std::{
    f32::consts::PI,
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom},
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
};

const RATE: usize = 11025;
const FFT: usize = 4096;
const HOP: usize = 2048;
const NOTES: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];
// Published Krumhansl–Kessler empirical profiles, numeric research data.
// Source: https://extras.humdrum.org/man/keycor/ . Algorithm/code is original.
const MAJOR: [f32; 12] = [
    6.35, 2.23, 3.48, 2.33, 4.38, 4.09, 2.52, 5.19, 2.39, 3.66, 2.29, 2.88,
];
const MINOR: [f32; 12] = [
    6.33, 2.68, 3.52, 5.38, 2.60, 3.53, 2.54, 4.75, 3.98, 2.69, 3.34, 3.17,
];
static ANALYSIS_LOCK: Mutex<()> = Mutex::new(());

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Candidate {
    pub root: String,
    pub mode: String,
    pub score: f32,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Analysis {
    pub algorithm: String,
    pub status: String,
    pub confidence: String,
    pub reason: String,
    pub candidate: Option<Candidate>,
    pub alternatives: Vec<Candidate>,
    pub margin: f32,
    #[serde(default)]
    pub section_candidates: Vec<Vec<Candidate>>,
    pub agreement: f32,
    pub analyzed_seconds: f32,
    pub tuning_cents: f32,
    pub auto_rename_allowed: bool,
    pub audio_path: String,
}
fn check(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Acquire) {
        Err("Key analysis cancelled".into())
    } else {
        Ok(())
    }
}

pub fn analyze(path: &Path, cancel: &AtomicBool) -> Result<Analysis, String> {
    analyze_impl(path, cancel, false, |_| {})
}

pub fn analyze_with_progress(
    path: &Path,
    cancel: &AtomicBool,
    on_progress: impl Fn(u8),
) -> Result<Analysis, String> {
    analyze_impl(path, cancel, true, on_progress)
}

fn analyze_impl(
    path: &Path,
    cancel: &AtomicBool,
    wait: bool,
    on_progress: impl Fn(u8),
) -> Result<Analysis, String> {
    check(cancel)?;
    let _guard = loop {
        match ANALYSIS_LOCK.try_lock() {
            Ok(guard) => break guard,
            Err(std::sync::TryLockError::WouldBlock) if wait => {
                check(cancel)?;
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            Err(std::sync::TryLockError::WouldBlock) => {
                return Err("Another key analysis is running".into())
            }
            Err(std::sync::TryLockError::Poisoned(_)) => {
                return Err(
                    "Key analyzer stopped unexpectedly; restart the app before retrying".into(),
                )
            }
        }
    };
    let path = crate::render::regular_path(path, false)?;
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(1);
    }
    let mut file = options.open(&path).map_err(|e| e.to_string())?;
    let before = file.metadata().map_err(|e| e.to_string())?;
    let info = crate::render::wave::inspect(&mut file)?;
    let total = (info.data_size / info.align) as usize;
    let limit = info.rate as usize * 20;
    let ranges = if total <= limit {
        vec![(0, total)]
    } else {
        let count = limit.min(total / 3);
        vec![
            (0, count),
            ((total - count) / 2, count),
            (total - count, count),
        ]
    };
    let frame_count: usize = ranges
        .iter()
        .map(|(_, count)| {
            let samples = if info.rate as usize == RATE {
                *count
            } else {
                (*count as f64 / (info.rate as f64 / RATE as f64)) as usize
            };
            if samples < FFT {
                0
            } else {
                (samples - FFT) / HOP + 1
            }
        })
        .sum();
    let work = frame_count * 2 + 1;
    let last = std::cell::Cell::new(255u8);
    let report = |done: usize| {
        let value = ((done * 100 / work).min(99)) as u8;
        if last.replace(value) != value {
            on_progress(value);
        }
    };
    report(0);
    let mut spectra = Vec::new();
    let mut groups = Vec::new();
    let mut seconds = 0.;
    for (start, count) in ranges {
        check(cancel)?;
        let samples = read_segment(&mut file, &info, start, count, cancel)?;
        seconds += samples.len() as f32 / RATE as f32;
        let first = spectra.len();
        for window in samples.windows(FFT).step_by(HOP) {
            check(cancel)?;
            spectra.push(spectrum(window));
            report(spectra.len());
        }
        groups.push(first..spectra.len());
    }
    check(cancel)?;
    let mut result = estimate_progress(&spectra, &groups, seconds, cancel, |frames| {
        report(spectra.len() + frames)
    })?;
    result.audio_path = path.to_string_lossy().into_owned();
    let after = file.metadata().map_err(|e| e.to_string())?;
    if before.len() != after.len() || before.modified().ok() != after.modified().ok() {
        return Err("Audio changed during key analysis".into());
    }
    if seconds < 3. {
        result.status = "inconclusive".into();
        result.reason = "Audio is too short to establish a key".into();
        result.candidate = None;
    }
    check(cancel)?;
    on_progress(100);
    Ok(result)
}

fn read_segment(
    file: &mut File,
    info: &crate::render::wave::WaveInfo,
    start: usize,
    count: usize,
    cancel: &AtomicBool,
) -> Result<Vec<f32>, String> {
    file.seek(SeekFrom::Start(
        info.data_offset + start as u64 * info.align,
    ))
    .map_err(|e| e.to_string())?;
    // Each read is at most 20 seconds, stereo/32-bit/192kHz (~31 MB raw).
    let mut raw = vec![0u8; count * info.align as usize];
    file.read_exact(&mut raw).map_err(|e| e.to_string())?;
    check(cancel)?;
    let width = (info.bits / 8) as usize;
    let mut mono = Vec::with_capacity(count);
    let mut left = Vec::with_capacity(count);
    let mut stereo_energy = 0f64;
    let mut mono_energy = 0f64;
    for frame in raw.chunks_exact(info.align as usize) {
        let mut sum = 0.;
        let mut first = 0.;
        for channel in 0..info.channels as usize {
            let bytes = &frame[channel * width..(channel + 1) * width];
            let value = match (info.code, info.bits) {
                (3, 32) => f32::from_le_bytes(bytes.try_into().unwrap()),
                (1, 16) => i16::from_le_bytes(bytes.try_into().unwrap()) as f32 / 32768.,
                (1, 24) => {
                    ((i32::from_le_bytes([bytes[0], bytes[1], bytes[2], 0]) << 8) >> 8) as f32
                        / 8388608.
                }
                (1, 32) => i32::from_le_bytes(bytes.try_into().unwrap()) as f32 / 2147483648.,
                _ => return Err("Unsupported WAV samples".into()),
            };
            if !value.is_finite() {
                return Err("WAV contains non-finite samples".into());
            }
            let value = value.clamp(-16., 16.);
            if channel == 0 {
                first = value;
            }
            sum += value;
            stereo_energy += (value as f64).powi(2) / info.channels as f64;
        }
        let value = sum / info.channels as f32;
        mono_energy += (value as f64).powi(2);
        mono.push(value);
        left.push(first);
    }
    drop(raw);
    // Preserve useful tonal content for anti-phase stereo instead of cancelling it.
    if mono_energy < stereo_energy * 0.01 {
        mono = left;
    } else {
        drop(left);
    }
    if info.rate as usize == RATE {
        return Ok(mono);
    }
    // Fixed 24-tap/64-phase windowed-sinc low-pass resampler, no FFT aliasing.
    let ratio = info.rate as f64 / RATE as f64;
    let cutoff = 0.45f64.min(0.45 / ratio);
    let mut kernels = [[0f32; 24]; 64];
    for (phase, kernel) in kernels.iter_mut().enumerate() {
        let mut norm = 0.;
        for (tap, value) in kernel.iter_mut().enumerate() {
            let x = tap as f64 - 11. - phase as f64 / 64.;
            let sinc = if x.abs() < 1e-8 {
                2. * cutoff
            } else {
                (2. * std::f64::consts::PI * cutoff * x).sin() / (std::f64::consts::PI * x)
            };
            *value = (sinc * (0.5 + 0.5 * (std::f64::consts::PI * x / 12.).cos())) as f32;
            norm += *value;
        }
        for value in kernel {
            *value /= norm;
        }
    }
    let length = (mono.len() as f64 / ratio) as usize;
    let mut output = Vec::with_capacity(length);
    for i in 0..length {
        if i % 4096 == 0 {
            check(cancel)?;
        }
        let position = i as f64 * ratio;
        let base = position.floor() as isize;
        let phase = ((position.fract() * 64.) as usize).min(63);
        let mut sum = 0.;
        for (tap, weight) in kernels[phase].iter().enumerate() {
            let index = (base + tap as isize - 11).clamp(0, mono.len() as isize - 1) as usize;
            sum += mono[index] * weight;
        }
        output.push(sum);
    }
    Ok(output)
}

fn spectrum(samples: &[f32]) -> Vec<f32> {
    let mean = samples.iter().sum::<f32>() / FFT as f32;
    let mut real: Vec<f32> = samples
        .iter()
        .enumerate()
        .map(|(i, x)| (x - mean) * (0.5 - 0.5 * (2. * PI * i as f32 / (FFT - 1) as f32).cos()))
        .collect();
    let mut imag = vec![0.; FFT];
    let mut j = 0;
    for i in 1..FFT {
        let mut bit = FFT >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j ^= bit;
        if i < j {
            real.swap(i, j);
        }
    }
    let mut size = 2;
    while size <= FFT {
        let angle = -2. * PI / size as f32;
        let (wi, wr) = angle.sin_cos();
        for begin in (0..FFT).step_by(size) {
            let (mut ur, mut ui) = (1., 0.);
            for offset in 0..size / 2 {
                let a = begin + offset;
                let b = a + size / 2;
                let tr = ur * real[b] - ui * imag[b];
                let ti = ur * imag[b] + ui * real[b];
                real[b] = real[a] - tr;
                imag[b] = imag[a] - ti;
                real[a] += tr;
                imag[a] += ti;
                (ur, ui) = (ur * wr - ui * wi, ur * wi + ui * wr);
            }
        }
        size *= 2;
    }
    (0..=FFT / 2)
        .map(|i| (real[i] * real[i] + imag[i] * imag[i]).sqrt() / FFT as f32)
        .collect()
}
fn median(values: &mut [f32]) -> f32 {
    let middle = values.len() / 2;
    values.select_nth_unstable_by(middle, |a, b| a.total_cmp(b));
    values[middle]
}
fn peak_pitch(frame: &[f32], bin: usize) -> f32 {
    let a = frame[bin - 1].max(1e-12).ln();
    let b = frame[bin].max(1e-12).ln();
    let c = frame[bin + 1].max(1e-12).ln();
    let denom = a - 2. * b + c;
    let offset = if denom.abs() > 1e-8 {
        (0.5 * (a - c) / denom).clamp(-0.5, 0.5)
    } else {
        0.
    };
    69. + 12. * (((bin as f32 + offset) * RATE as f32 / FFT as f32) / 440.).log2()
}
fn correlate(chroma: &[f32; 12], profile: &[f32; 12], root: usize) -> f32 {
    let a = chroma.iter().sum::<f32>() / 12.;
    let b = profile.iter().sum::<f32>() / 12.;
    let (mut dot, mut aa, mut bb) = (0., 0., 0.);
    for i in 0..12 {
        let x = chroma[(i + root) % 12] - a;
        let y = profile[i] - b;
        dot += x * y;
        aa += x * x;
        bb += y * y;
    }
    if aa * bb < 1e-15 {
        0.
    } else {
        dot / (aa * bb).sqrt()
    }
}
fn rank(chroma: &[f32; 12]) -> Vec<Candidate> {
    let mut candidates = Vec::with_capacity(24);
    for (root, name) in NOTES.iter().enumerate() {
        for (mode, profile) in [("major", &MAJOR), ("minor", &MINOR)] {
            candidates.push(Candidate {
                root: (*name).into(),
                mode: mode.into(),
                score: correlate(chroma, profile, root),
            });
        }
    }
    candidates.sort_by(|a, b| b.score.total_cmp(&a.score));
    candidates
}

fn section_support(candidate: &Candidate, sections: &[Vec<Candidate>]) -> f32 {
    sections
        .iter()
        .filter(|ranked| {
            ranked.first().is_some_and(|winner| {
                ranked.iter().any(|c| {
                    c.root == candidate.root
                        && c.mode == candidate.mode
                        && c.score >= 0.55
                        && winner.score - c.score <= 0.05
                })
            })
        })
        .count() as f32
        / sections.len().max(1) as f32
}

#[cfg(test)]
fn estimate(
    spectra: &[Vec<f32>],
    groups: &[std::ops::Range<usize>],
    seconds: f32,
    cancel: &AtomicBool,
) -> Result<Analysis, String> {
    estimate_progress(spectra, groups, seconds, cancel, |_| {})
}
fn estimate_progress(
    spectra: &[Vec<f32>],
    groups: &[std::ops::Range<usize>],
    seconds: f32,
    cancel: &AtomicBool,
    on_frame: impl Fn(usize),
) -> Result<Analysis, String> {
    let mut result = Analysis {
        algorithm: "1ce-key-v2-experimental".into(),
        status: "inconclusive".into(),
        confidence: "uncalibrated".into(),
        reason: "Insufficient harmonic evidence".into(),
        candidate: None,
        alternatives: vec![],
        section_candidates: vec![],
        margin: 0.,
        agreement: 0.,
        analyzed_seconds: seconds,
        tuning_cents: 0.,
        auto_rename_allowed: false,
        audio_path: String::new(),
    };
    let low = (55. * FFT as f32 / RATE as f32).ceil() as usize;
    let high = (4000. * FFT as f32 / RATE as f32) as usize;
    let (mut sx, mut sy) = (0., 0.);
    for frame in spectra {
        let max = frame.iter().copied().fold(0f32, f32::max);
        for bin in low..high {
            if frame[bin] > max * 0.03 && frame[bin] > frame[bin - 1] && frame[bin] > frame[bin + 1]
            {
                let pitch = peak_pitch(frame, bin);
                let phase = (pitch - pitch.round()) * 2. * PI;
                let weight = frame[bin];
                sx += phase.cos() * weight;
                sy += phase.sin() * weight;
            }
        }
    }
    let tuning = if sx.abs() + sy.abs() > 1e-8 {
        sy.atan2(sx) / (2. * PI)
    } else {
        0.
    };
    result.tuning_cents = tuning * 100.;
    let mut all = [0f32; 12];
    let mut section_keys = Vec::new();
    let mut total_energy = 0.;
    for group in groups {
        let mut section = [0f32; 12];
        for t in group.clone() {
            check(cancel)?;
            let frame = &spectra[t];
            let max = frame.iter().copied().fold(0f32, f32::max);
            if max < 0.00001 {
                on_frame(t + 1);
                continue;
            }
            let mut chroma = [0.; 12];
            for bin in low..high {
                if frame[bin] < max * 0.03
                    || frame[bin] <= frame[bin - 1]
                    || frame[bin] <= frame[bin + 1]
                {
                    continue;
                }
                let mut times = [0f32; 9];
                let mut n = 0;
                for other in t.saturating_sub(4).max(group.start)..(t + 5).min(group.end) {
                    times[n] = spectra[other][bin];
                    n += 1;
                }
                let harmonic = median(&mut times[..n]);
                let mut freqs = [0f32; 17];
                freqs.copy_from_slice(&frame[bin - 8..=bin + 8]);
                let percussive = median(&mut freqs);
                let mask =
                    harmonic * harmonic / (harmonic * harmonic + percussive * percussive + 1e-15);
                let pitch = peak_pitch(frame, bin) - tuning;
                let distance = (pitch - pitch.round()).abs();
                if distance > 0.4 {
                    continue;
                }
                let index = (pitch.round() as i32).rem_euclid(12) as usize;
                chroma[index] += frame[bin].sqrt() * mask * (1. - distance / 0.5);
            }
            let mass = chroma.iter().sum::<f32>();
            total_energy += mass;
            if mass > 1e-6 {
                for i in 0..12 {
                    section[i] += chroma[i] / mass;
                }
            }
            on_frame(t + 1);
        }
        let mass = section.iter().sum::<f32>();
        if mass > 0. {
            for i in 0..12 {
                all[i] += section[i] / mass;
            }
            section_keys.push(rank(&section));
        }
    }
    if total_energy < 1e-4 {
        return Ok(result);
    }
    let mass = all.iter().sum::<f32>();
    if mass <= 1e-6 {
        return Ok(result);
    }
    for value in &mut all {
        *value /= mass;
    }
    let mut candidates = rank(&all);
    let top = candidates[0].clone();
    result.margin = (top.score - candidates[1].score).max(0.);
    // A section can support the global estimate even when a very close
    // alternative wins locally. Exact winner equality rejected stable songs.
    result.agreement = section_support(&top, &section_keys);
    result.section_candidates = section_keys
        .iter()
        .map(|r| r.iter().take(3).cloned().collect())
        .collect();
    let supported = all.iter().filter(|x| **x > 0.07).count();
    // Conservative screening only; these are NOT calibrated confidence thresholds.
    if supported >= 3 && top.score > 0.55 && result.margin > 0.015 && result.agreement >= 0.66 {
        result.status = "needs_review".into();
        result.reason = "Experimental key estimate; confirm before naming".into();
        result.candidate = Some(top);
    } else {
        result.reason = if supported < 3 {
            "Too few pitch classes to establish major/minor"
        } else {
            "Ambiguous key/mode or insufficient agreement between sections"
        }
        .into();
    }
    candidates.truncate(3);
    result.alternatives = candidates;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(root: usize, minor: bool) -> Vec<f32> {
        fixture_tuned(root, minor, 0.)
    }
    fn fixture_tuned(root: usize, minor: bool, cents: f32) -> Vec<f32> {
        // Independent musical cadence: I/iv/V/I (major I/IV/V/I), bass + triads.
        let third = if minor { 3 } else { 4 };
        let subthird = if minor { 8 } else { 9 };
        let chords = [[0, third, 7], [5, subthird, 0], [7, 11, 2], [0, third, 7]];
        let mut samples = Vec::new();
        for chord in chords.iter().cycle().take(8) {
            for i in 0..RATE {
                let time = i as f32 / RATE as f32;
                let envelope = (time * 30.).min(1.) * ((1. - time) * 20.).min(1.);
                let mut value = 0.;
                for note in chord {
                    let hz = 440.
                        * 2f32.powf((48. + root as f32 + *note as f32 - 69.) / 12. + cents / 1200.);
                    value += (2. * PI * hz * time).sin() * 0.15;
                }
                // Tonic is established by the cadence, not a profile-weight histogram.
                samples.push(value * envelope);
            }
        }
        samples
    }
    fn from_samples(samples: &[f32]) -> Analysis {
        let spectra: Vec<_> = samples.windows(FFT).step_by(HOP).map(spectrum).collect();
        let len = spectra.len();
        estimate(
            &spectra,
            &[0..len],
            samples.len() as f32 / RATE as f32,
            &AtomicBool::new(false),
        )
        .unwrap()
    }
    #[test]
    fn section_consensus_accepts_near_ties_but_not_unrelated_keys() {
        let candidate = Candidate {
            root: "E".into(),
            mode: "minor".into(),
            score: 0.72,
        };
        let rival = Candidate {
            root: "G".into(),
            mode: "major".into(),
            score: 0.74,
        };
        let close = vec![rival.clone(), candidate.clone()];
        let strong_rival = Candidate {
            score: 0.9,
            ..rival
        };
        assert_eq!(
            section_support(
                &candidate,
                &[close.clone(), close.clone(), vec![candidate.clone()]]
            ),
            1.
        );
        assert!(
            section_support(
                &candidate,
                &[
                    vec![strong_rival.clone(), candidate.clone()],
                    vec![strong_rival, candidate.clone()],
                    close
                ]
            ) < 0.66
        );
    }
    #[test]
    fn known_major_and_minor_cadences() {
        for (root, minor) in (0..12).flat_map(|root| [(root, false), (root, true)]) {
            let result = from_samples(&fixture(root, minor));
            let expected = if minor { "minor" } else { "major" };
            assert_eq!(
                (&result.alternatives[0].root, &result.alternatives[0].mode),
                (&NOTES[root].to_string(), &expected.to_string()),
                "{result:?}"
            );
            assert!(
                result.candidate.is_some(),
                "Expected usable synthetic cadence estimate: {result:?}"
            );
            assert!(!result.auto_rename_allowed);
            assert_eq!(result.confidence, "uncalibrated");
        }
    }
    #[test]
    fn silence_single_tone_noise_and_cancellation_are_not_confident_keys() {
        let silence = from_samples(&vec![0.; RATE * 4]);
        assert!(silence.candidate.is_none());
        let tone: Vec<_> = (0..RATE * 4)
            .map(|i| (2. * PI * 440. * i as f32 / RATE as f32).sin() * 0.3)
            .collect();
        assert!(from_samples(&tone).candidate.is_none());
        let mut seed = 123u32;
        let noise: Vec<_> = (0..RATE * 4)
            .map(|_| {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                (seed as f64 / u32::MAX as f64 - 0.5) as f32
            })
            .collect();
        assert!(from_samples(&noise).candidate.is_none());
        assert!(check(&AtomicBool::new(true)).is_err());
    }
    #[test]
    fn wav_decoding_resampling_and_source_preservation() {
        let dir =
            std::env::temp_dir().join(format!("onece-key-test-{}", crate::render::attempt_id()));
        std::fs::create_dir(&dir).unwrap();
        struct Cleanup(std::path::PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = Cleanup(dir.clone());
        let samples = fixture(0, false);
        for (rate, channels, code, bits, antiphase) in [
            (8000, 1, 1, 16, false),
            (44100, 2, 1, 24, false),
            (48000, 2, 3, 32, true),
            (192000, 1, 1, 32, false),
        ] {
            let mut data = Vec::new();
            for i in 0..rate * 8 {
                let value = samples[i * RATE / rate];
                for ch in 0..channels {
                    let x = if antiphase && ch == 1 { -value } else { value };
                    match (code, bits) {
                        (1, 16) => data.extend(((x * 32767.) as i16).to_le_bytes()),
                        (1, 24) => {
                            data.extend_from_slice(&((x * 8388607.) as i32).to_le_bytes()[..3])
                        }
                        (1, 32) => data.extend(((x * 2147483647.) as i32).to_le_bytes()),
                        (3, 32) => data.extend(x.to_le_bytes()),
                        _ => unreachable!(),
                    }
                }
            }
            let mut bytes = b"RIFF".to_vec();
            bytes.extend((36 + data.len() as u32).to_le_bytes());
            bytes.extend(b"WAVEfmt ");
            bytes.extend(16u32.to_le_bytes());
            bytes.extend((code as u16).to_le_bytes());
            bytes.extend((channels as u16).to_le_bytes());
            bytes.extend((rate as u32).to_le_bytes());
            let align = channels * bits / 8;
            bytes.extend((rate as u32 * align as u32).to_le_bytes());
            bytes.extend((align as u16).to_le_bytes());
            bytes.extend((bits as u16).to_le_bytes());
            bytes.extend(b"data");
            bytes.extend((data.len() as u32).to_le_bytes());
            bytes.extend(data);
            let path = dir.join(format!("{rate}-{bits}.wav"));
            std::fs::write(&path, &bytes).unwrap();
            let progress = std::cell::RefCell::new(Vec::new());
            let result = analyze_with_progress(&path, &AtomicBool::new(false), |value| {
                progress.borrow_mut().push(value)
            })
            .unwrap();
            let top = &result.alternatives[0];
            assert_eq!(
                (top.root.as_str(), top.mode.as_str()),
                ("C", "major"),
                "{rate}/{bits}: {result:?}"
            );
            let values = progress.borrow();
            assert_eq!(values.first(), Some(&0));
            assert_eq!(values.last(), Some(&100));
            assert!(values.windows(2).all(|v| v[0] <= v[1]));
            drop(values);
            if rate == 8000 {
                let cancel = AtomicBool::new(false);
                let last = std::cell::Cell::new(0);
                let stopped = analyze_with_progress(&path, &cancel, |value| {
                    last.set(value);
                    if value >= 10 {
                        cancel.store(true, Ordering::Release);
                    }
                });
                assert!(stopped.unwrap_err().contains("cancelled"));
                assert!(last.get() < 100);
            }
            assert_eq!(std::fs::read(&path).unwrap(), bytes);
        }
        let path = dir.join("bad.wav");
        std::fs::write(&path, b"not wave").unwrap();
        assert!(analyze_with_progress(&path, &AtomicBool::new(false), |_| {}).is_err());
        assert!(analyze(&path, &AtomicBool::new(true))
            .unwrap_err()
            .contains("cancelled"));
    }
    #[test]
    fn conflicting_sections_are_inconclusive() {
        let mut spectra = Vec::new();
        let mut groups = Vec::new();
        for root in [0, 3, 6] {
            let first = spectra.len();
            spectra.extend(fixture(root, false).windows(FFT).step_by(HOP).map(spectrum));
            groups.push(first..spectra.len());
        }
        let result = estimate(&spectra, &groups, 24., &AtomicBool::new(false)).unwrap();
        assert!(result.candidate.is_none(), "{result:?}");
    }
    #[test]
    fn detuning_and_percussion_do_not_change_a_known_cadence() {
        let mut samples = fixture_tuned(0, false, 22.);
        let mut seed = 1u32;
        for (i, value) in samples.iter_mut().enumerate() {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            let noise = (seed as f64 / u32::MAX as f64 - 0.5) as f32;
            let beat_time = (i % (RATE / 2)) as f32 / RATE as f32;
            *value += noise * (-beat_time * 70.).exp() * 0.7;
        }
        let result = from_samples(&samples);
        let top = &result.alternatives[0];
        assert_eq!(
            (top.root.as_str(), top.mode.as_str()),
            ("C", "major"),
            "{result:?}"
        );
        assert!((result.tuning_cents - 22.).abs() < 5., "{result:?}");
    }
    #[test]
    #[ignore = "Explicit local read-only WAV analysis"]
    fn real_key_smoke() {
        let path = std::env::var("ONECE_KEY_SMOKE_WAV").expect("Provide a WAV path");
        let result = analyze(Path::new(&path), &AtomicBool::new(false)).unwrap();
        println!("{}", serde_json::to_string_pretty(&result).unwrap());
        assert!(!result.auto_rename_allowed);
    }
}

#[tauri::command]
pub async fn analyze_audio_key(path: String) -> Result<Analysis, String> {
    tauri::async_runtime::spawn_blocking(move || analyze(Path::new(&path), &AtomicBool::new(false)))
        .await
        .map_err(|e| e.to_string())?
}
