#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::many_single_char_names,
    clippy::missing_panics_doc,
    reason = "the deterministic f32 noise boundary uses checked raster coordinates"
)]

const OCTAVE_FREQUENCIES: [f32; 3] = [0.4910, 0.9441, 1.7280];
const OCTAVE_AMPLITUDES: [f32; 3] = [0.2340, 0.7850, 1.2150];
const AMPLITUDE_SUM: f32 = 2.234;

/// Counter hash shared by the scalar plan and the reflected point kernel.
#[must_use]
pub fn grain_hash(seed: u64, x: i64, y: i64, channel: u32, octave: u32) -> u32 {
    let mut value = (seed as u32)
        ^ (seed >> 32) as u32
        ^ (x as u32).wrapping_mul(0x9e37_79b9)
        ^ (y as u32).wrapping_mul(0x85eb_ca6b)
        ^ channel.wrapping_mul(0xc2b2_ae35)
        ^ octave.wrapping_mul(0x27d4_eb2d);
    value ^= value >> 16;
    value = value.wrapping_mul(0x7feb_352d);
    value ^= value >> 15;
    value = value.wrapping_mul(0x846c_a68b);
    value ^ (value >> 16)
}

/// Converts a hash to the exact [0, 1) float construction used by WGSL.
#[must_use]
pub fn hash_to_unit(value: u32) -> f32 {
    (value >> 8) as f32 / 16_777_216.0
}

#[must_use]
pub fn grain_noise(seed: u64, x: f32, y: f32, zoom: f32, channel: u32) -> f32 {
    OCTAVE_FREQUENCIES
        .into_iter()
        .zip(OCTAVE_AMPLITUDES)
        .enumerate()
        .map(|(octave, (frequency, amplitude))| {
            let octave = u32::try_from(octave).expect("three grain octaves fit u32");
            value_noise(
                seed,
                x / zoom * frequency,
                y / zoom * frequency,
                channel,
                octave,
            ) * amplitude
        })
        .sum::<f32>()
        / AMPLITUDE_SUM
}

fn value_noise(seed: u64, x: f32, y: f32, channel: u32, octave: u32) -> f32 {
    let x0 = x.floor();
    let y0 = y.floor();
    let fx = smoothstep(x - x0);
    let fy = smoothstep(y - y0);
    let x0 = x0 as i64;
    let y0 = y0 as i64;
    let a = signed_hash(seed, x0, y0, channel, octave);
    let b = signed_hash(seed, x0.saturating_add(1), y0, channel, octave);
    let c = signed_hash(seed, x0, y0.saturating_add(1), channel, octave);
    let d = signed_hash(
        seed,
        x0.saturating_add(1),
        y0.saturating_add(1),
        channel,
        octave,
    );
    lerp(lerp(a, b, fx), lerp(c, d, fx), fy)
}

fn signed_hash(seed: u64, x: i64, y: i64, channel: u32, octave: u32) -> f32 {
    hash_to_unit(grain_hash(seed, x, y, channel, octave)) * 2.0 - 1.0
}

fn smoothstep(value: f32) -> f32 {
    value * value * (3.0 - 2.0 * value)
}

fn lerp(left: f32, right: f32, amount: f32) -> f32 {
    left + (right - left) * amount
}
