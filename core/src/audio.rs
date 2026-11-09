//! Decoding: the only place that knows how audio becomes samples.
//!
//! The prototype shells out to ffmpeg; this decodes in-process (symphonia) and
//! resamples with an FFT resampler (rubato). The whole file is decoded once
//! and windows are cut from it exactly, rather than seeking per window.

use std::path::Path;

use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::errors::Error as SymErr;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;

use crate::Res;

/// A decoded track: mono samples with the encoder's delay and padding trimmed
/// (gapless), and its length as ffprobe reports it, which counts them.
pub struct Decoded {
    pub samples: Vec<f32>,
    pub rate: u32,
    /// seconds, delay and padding included: windows are placed by this, as
    /// the prototype placed them, so their vectors match its cache
    pub duration: f64,
}

/// Mono samples at the file's own rate.
pub fn decode_native(path: &Path) -> Res<Decoded> {
    // Skip leading ID3v2 tags here: symphonia can mistake bytes inside a large
    // tag (cover art) for an MPEG layer 1 frame and decode nothing.
    let bytes = std::fs::read(path)?;
    let mut at = 0;
    while bytes.len() >= at + 10 && &bytes[at..at + 3] == b"ID3" {
        let h = &bytes[at..at + 10];
        let size = h[6..10].iter().fold(0usize, |s, b| (s << 7) | (b & 0x7f) as usize);
        at += 10 + size + if h[5] & 0x10 != 0 { 10 } else { 0 };
    }
    let body = bytes.get(at..).unwrap_or(&[]).to_vec();
    let mss = MediaSourceStream::new(Box::new(std::io::Cursor::new(body)), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let mut format = symphonia::default::get_probe().probe(
        &hint,
        mss,
        FormatOptions::default(),
        MetadataOptions::default(),
    )?;
    let codecs = symphonia::default::get_codecs();
    let opts = AudioDecoderOptions::default();
    let (track, mut dec) = format
        .tracks()
        .iter()
        .filter(|t| t.codec_params.as_ref().and_then(|p| p.audio()).is_some())
        .find_map(|t| Some((t, codecs.make_audio_decoder(t.codec_params.as_ref()?.audio()?, &opts).ok()?)))
        .ok_or("no audio track this build can decode")?;
    let id = track.id;
    let extra = track.delay.unwrap_or(0) as usize + track.padding.unwrap_or(0) as usize;
    let (mut out, mut rate, mut buf) = (Vec::<f32>::new(), 0u32, Vec::<f32>::new());
    loop {
        if crate::stopping() {
            return Err("stopped".into());
        }
        let packet = match format.next_packet() {
            Ok(Some(p)) => p,
            Ok(None) => break,
            Err(SymErr::IoError(_)) => break,
            Err(e) => return Err(e.into()),
        };
        if packet.track_id != id {
            continue;
        }
        let ab = match dec.decode(&packet) {
            Ok(ab) => ab,
            Err(SymErr::DecodeError(_)) => continue, // a corrupt frame, not a corrupt file
            Err(e) => return Err(e.into()),
        };
        let ch = ab.spec().channels().count().max(1);
        rate = ab.spec().rate();
        buf.resize(ab.samples_interleaved(), 0.0);
        ab.copy_to_slice_interleaved(&mut buf[..]);
        if ch == 1 {
            out.extend_from_slice(&buf);
        } else {
            // as ffmpeg -ac 1 downmixes: (L + R) / sqrt(2) for stereo, which
            // the prototype's embeddings were made with
            let k = if ch == 2 { std::f32::consts::FRAC_1_SQRT_2 } else { 1.0 / ch as f32 };
            out.extend(buf.chunks_exact(ch).map(|f| f.iter().sum::<f32>() * k));
        }
    }
    if rate == 0 || out.is_empty() {
        return Err(format!("no audio decoded: {}", path.display()).into());
    }
    let duration = (out.len() + extra) as f64 / rate as f64;
    Ok(Decoded { samples: out, rate, duration })
}

/// ffmpeg's default resampler (libswresample: Kaiser-windowed sinc, beta 9,
/// cutoff 0.97, 32 taps scaled by the ratio, exact rational phases, the start
/// mirrored), so the samples -- and every number computed from them -- match
/// what the prototype got from `ffmpeg -ar`.
pub fn resample(x: &[f32], from: u32, to: u32) -> Res<Vec<f32>> {
    if from == to || x.is_empty() {
        return Ok(x.to_vec());
    }
    let g = gcd(from as u64, to as u64);
    let (p, q) = ((to as u64 / g) as usize, (from as u64 / g) as usize);
    if p > 1024 {
        return Err(format!("unsupported resampling ratio {from} -> {to}").into());
    }
    let factor = (to as f64 * 0.97 / from as f64).min(1.0);
    let mut len = ((32.0 / factor).ceil() as usize).max(1);
    if len > 1 {
        len += len % 2;
    }
    let center = (len - 1) / 2;
    let beta = 9.0;
    // the filter bank: one row of taps per phase, normalised by phase 0's sum
    let mut bank = vec![0f64; p * len];
    let mut norm = 0.0;
    for ph in 0..p {
        for i in 0..len {
            let xx = std::f64::consts::PI * ((i as f64 - center as f64) - ph as f64 / p as f64) * factor;
            let mut y = if xx == 0.0 { 1.0 } else { xx.sin() / xx };
            let w = 2.0 * xx / (factor * len as f64 * std::f64::consts::PI);
            y *= bessel_i0(beta * (1.0 - w * w).max(0.0).sqrt());
            bank[ph * len + i] = y;
            if ph == 0 {
                norm += y;
            }
        }
    }
    let bank: Vec<f32> = bank.iter().map(|v| (v / norm) as f32).collect();
    let n_out = (x.len() as u64 * p as u64).div_ceil(q as u64) as usize;
    let at = |k: isize| -> f32 {
        // mirrored before the start, silence after the end
        let k = if k < 0 { -k } else { k } as usize;
        if k < x.len() { x[k] } else { 0.0 }
    };
    let mut out = Vec::with_capacity(n_out);
    for n in 0..n_out {
        // a whole track is a second or so of work: closing doesn't wait for it
        if n % (1 << 20) == 0 && crate::stopping() {
            return Err("stopped".into());
        }
        let pos = n as u64 * q as u64;
        let (s, ph) = ((pos / p as u64) as isize, (pos % p as u64) as usize);
        let taps = &bank[ph * len..(ph + 1) * len];
        let base = s - center as isize;
        let mut acc = 0f32;
        if base >= 0 && (base as usize + len) <= x.len() {
            let src = &x[base as usize..base as usize + len];
            for (a, b) in src.iter().zip(taps) {
                acc += a * b;
            }
        } else {
            for (i, t) in taps.iter().enumerate() {
                acc += at(base + i as isize) * t;
            }
        }
        out.push(acc);
    }
    Ok(out)
}

fn gcd(a: u64, b: u64) -> u64 {
    if b == 0 { a } else { gcd(b, a % b) }
}

/// Modified Bessel function of the first kind, order 0 (power series).
fn bessel_i0(x: f64) -> f64 {
    let (mut sum, mut term, h) = (1.0, 1.0, x * x / 4.0);
    for k in 1..200 {
        term *= h / (k * k) as f64;
        sum += term;
        if term < sum * 1e-17 {
            break;
        }
    }
    sum
}

/// Mono samples at `rate`.
pub fn decode(path: &Path, rate: u32) -> Res<Decoded> {
    let d = decode_native(path)?;
    Ok(Decoded { samples: resample(&d.samples, d.rate, rate)?, rate, duration: d.duration })
}

/// `n` windows of `seconds` spread evenly through the track, centred on
/// (i+1)/(n+1) of its length: DJ tracks have long intros and outros, so one
/// window from the start says little. A track shorter than one window gives
/// itself, once.
pub fn excerpts(d: &Decoded, n: usize, seconds: f64) -> Vec<&[f32]> {
    let (x, rate, total) = (&d.samples[..], d.rate, d.duration);
    let want = (seconds * rate as f64) as usize;
    if total <= seconds {
        return vec![&x[..want.min(x.len())]];
    }
    (0..n)
        .map(|i| {
            let centre = total * (i + 1) as f64 / (n + 1) as f64;
            let start = (centre - seconds / 2.0).max(0.0).min(total - seconds);
            let a = ((start * rate as f64) as usize).min(x.len());
            &x[a..(a + want).min(x.len())]
        })
        .filter(|w| !w.is_empty())
        .collect()
}
