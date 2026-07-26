//! Scalar Color Zones pixel execution ported from `src/iop/colorzones.c`.
//!
//! This module implements the native CPU `process_v1`, `process_v3`, and
//! `process` branches and the authored default Lab blend. It deliberately makes
//! no display-mask, picker, histogram, preset, GPU, or UI claim.

#![allow(
    clippy::many_single_char_names,
    clippy::suboptimal_flops,
    reason = "the source-shaped Lab/LCh equations retain Darktable's names and operation order"
)]

use std::hash::{Hash, Hasher};

use super::curve::{
    COLORZONES_LUT_RESOLUTION, ColorZonesCompileError, ColorZonesLuts, compile_luts, lookup,
};
use super::{COLORZONES_CHANNELS, ColorZonesChannel, ColorZonesConfig, ColorZonesMode};

/// One native four-channel Lab sample.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorZonesPixel {
    channels: [f32; 4],
}

impl ColorZonesPixel {
    #[must_use]
    pub const fn new(lightness: f32, a: f32, b: f32, alpha: f32) -> Self {
        Self {
            channels: [lightness, a, b, alpha],
        }
    }

    #[must_use]
    pub const fn from_channels(channels: [f32; 4]) -> Self {
        Self { channels }
    }

    #[must_use]
    pub const fn channels(self) -> [f32; 4] {
        self.channels
    }

    #[must_use]
    pub const fn lightness(self) -> f32 {
        self.channels[0]
    }

    #[must_use]
    pub const fn a(self) -> f32 {
        self.channels[1]
    }

    #[must_use]
    pub const fn b(self) -> f32 {
        self.channels[2]
    }

    #[must_use]
    pub const fn alpha(self) -> f32 {
        self.channels[3]
    }
}

/// Immutable Color Zones CPU plan with committed native lookup tables.
///
/// Construction follows the native rebuild path from checked parameters. The
/// compiled plan is carried unchanged through registry preparation, canonical
/// evaluation, pixelpipe execution, and snapshot identity.
#[derive(Debug, Clone)]
pub struct ColorZonesPlan {
    config: ColorZonesConfig,
    luts: ColorZonesLuts,
}

impl PartialEq for ColorZonesPlan {
    fn eq(&self, other: &Self) -> bool {
        self.config == other.config
    }
}

impl Eq for ColorZonesPlan {}

impl Hash for ColorZonesPlan {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.config.hash(state);
    }
}

impl ColorZonesPlan {
    pub fn new(config: ColorZonesConfig) -> Result<Self, ColorZonesCompileError> {
        let luts = compile_luts(&config)?;
        Ok(Self { config, luts })
    }

    #[must_use]
    pub const fn config(&self) -> &ColorZonesConfig {
        &self.config
    }

    /// Returns the exact 65,536-entry committed LUT for one output channel.
    ///
    /// # Panics
    ///
    /// Panics only if this private plan's compiled-LUT length invariant is
    /// violated.
    #[must_use]
    pub fn lut(&self, channel: ColorZonesChannel) -> &[f32] {
        let start = channel.index() * COLORZONES_LUT_RESOLUTION;
        self.luts
            .get(start..start + COLORZONES_LUT_RESOLUTION)
            .expect("compiled Color Zones LUT storage has three complete channels")
    }

    /// Samples one committed curve with Darktable's native interpolating LUT lookup.
    #[must_use]
    pub fn sample_curve(&self, channel: ColorZonesChannel, input: f32) -> f32 {
        lookup(self.lut(channel), input)
    }

    /// Applies Darktable's native Smooth (`process_v3`) or Strong
    /// (`process_v1`) point kernel to Lab pixels.
    #[must_use]
    pub fn execute_lab(&self, input: &[ColorZonesPixel]) -> Vec<ColorZonesPixel> {
        let luts: [&[f32]; COLORZONES_CHANNELS] = [
            self.lut(ColorZonesChannel::Lightness),
            self.lut(ColorZonesChannel::Chroma),
            self.lut(ColorZonesChannel::Hue),
        ];
        input
            .iter()
            .copied()
            .map(|pixel| match self.config.mode() {
                ColorZonesMode::Smooth => process_smooth(pixel, self.config.channel(), &luts),
                ColorZonesMode::Strong => process_strong(pixel, self.config.channel(), &luts),
            })
            .collect()
    }

    /// Applies Darktable's default unbounded Lab normal blend after Color Zones.
    ///
    /// Imported Darktable rows remain outside this authored-operation seam while
    /// their arbitrary blend modes, blend-if state, and masks are still opaque.
    #[must_use]
    pub fn execute_lab_normal_blend(
        &self,
        input: &[ColorZonesPixel],
        mask: Option<&[f32]>,
        opacity: f32,
    ) -> Vec<ColorZonesPixel> {
        let candidates = self.execute_lab(input);
        Self::blend_lab_candidates(input, &candidates, mask, opacity)
    }

    /// Applies Darktable's default Lab normal blend to already evaluated
    /// candidates.
    ///
    /// This keeps opacity, masks, and preserved-channel behavior identical when
    /// the candidate pixels came from the source OpenCL-compatible WGPU kernel
    /// rather than the deliberately interpolating CPU kernel.
    #[must_use]
    pub fn blend_lab_candidates(
        input: &[ColorZonesPixel],
        candidates: &[ColorZonesPixel],
        mask: Option<&[f32]>,
        opacity: f32,
    ) -> Vec<ColorZonesPixel> {
        debug_assert_eq!(candidates.len(), input.len());
        debug_assert!(mask.is_none_or(|values| values.len() == input.len()));
        let inverse_scale = [1.0_f32 / 100.0, 1.0_f32 / 128.0, 1.0_f32 / 128.0];
        let scale = [100.0_f32, 128.0_f32, 128.0_f32];

        input
            .iter()
            .zip(candidates)
            .enumerate()
            .map(|(index, (source, candidate))| {
                let source = source.channels();
                let candidate = candidate.channels();
                let coverage = mask.map_or(opacity, |values| values[index] * opacity);
                let channels = std::array::from_fn(|channel| {
                    if channel == 3 {
                        source[channel]
                    } else {
                        let source = source[channel] * inverse_scale[channel];
                        let candidate = candidate[channel] * inverse_scale[channel];
                        (source * (1.0 - coverage) + candidate * coverage) * scale[channel]
                    }
                });
                ColorZonesPixel::from_channels(channels)
            })
            .collect()
    }
}

fn process_smooth(
    pixel: ColorZonesPixel,
    selection_channel: ColorZonesChannel,
    luts: &[&[f32]; COLORZONES_CHANNELS],
) -> ColorZonesPixel {
    let a = pixel.a();
    let b = pixel.b();
    let hue =
        ((b.atan2(a) + std::f32::consts::TAU) % std::f32::consts::TAU) / std::f32::consts::TAU;
    let chroma = smooth_chroma(a, b);

    let (select, blend) = match selection_channel {
        ColorZonesChannel::Lightness => ((pixel.lightness() / 100.0).min(1.0), 0.0),
        ColorZonesChannel::Chroma => ((chroma / 128.0).min(1.0), 0.0),
        ColorZonesChannel::Hue => {
            let inverse_chroma = 1.0 - chroma / 128.0;
            (hue, inverse_chroma * inverse_chroma)
        }
    };

    let lightness_modification = (blend * 0.5 + (1.0 - blend) * lookup(luts[0], select)) - 0.5;
    let hue_modification = (blend * 0.5 + (1.0 - blend) * lookup(luts[2], select)) - 0.5;
    // The native CPU path intentionally does not apply its low-chroma blend
    // to saturation.
    let chroma_modification = 2.0 * lookup(luts[1], select);
    let lightness = pixel.lightness() * 2.0_f32.powf(4.0 * lightness_modification);
    let adjusted_hue = std::f32::consts::TAU * (hue + hue_modification);

    ColorZonesPixel::new(
        lightness,
        adjusted_hue.cos() * chroma_modification * chroma,
        adjusted_hue.sin() * chroma_modification * chroma,
        pixel.alpha(),
    )
}

/// `process_v3` calls the fast-hypot path as `(b, a)`, whose release-build
/// contract is this source-order f32 square root rather than `hypotf`.
fn smooth_chroma(a: f32, b: f32) -> f32 {
    (b * b + a * a).sqrt()
}

fn process_strong(
    pixel: ColorZonesPixel,
    selection_channel: ColorZonesChannel,
    luts: &[&[f32]; COLORZONES_CHANNELS],
) -> ColorZonesPixel {
    let mut lch = lab_to_lch(pixel);
    let normalize_chroma = 1.0 / (128.0 * std::f32::consts::SQRT_2);
    let select = match selection_channel {
        ColorZonesChannel::Lightness => lch[0] * 0.01,
        ColorZonesChannel::Chroma => lch[1] * normalize_chroma,
        ColorZonesChannel::Hue => lch[2],
    };
    let select = clamp_unit_native(select);

    lch[0] *= 2.0_f32.powf(4.0 * (lookup(luts[0], select) - 0.5));
    lch[1] *= 2.0 * lookup(luts[1], select);
    lch[2] += lookup(luts[2], select) - 0.5;
    lch_to_lab(lch, pixel.alpha())
}

fn lab_to_lch(pixel: ColorZonesPixel) -> [f32; 3] {
    let hue_radians = pixel.b().atan2(pixel.a());
    let hue = if hue_radians > 0.0 {
        hue_radians / std::f32::consts::TAU
    } else {
        1.0 - hue_radians.abs() / std::f32::consts::TAU
    };
    [pixel.lightness(), pixel.a().hypot(pixel.b()), hue]
}

fn lch_to_lab(lch: [f32; 3], alpha: f32) -> ColorZonesPixel {
    let hue_radians = std::f32::consts::TAU * lch[2];
    ColorZonesPixel::new(
        lch[0],
        hue_radians.cos() * lch[1],
        hue_radians.sin() * lch[1],
        alpha,
    )
}

/// Exact comparison order from `GLib`'s `CLAMP(x, 0, 1)` macro.
#[allow(
    clippy::manual_clamp,
    reason = "the direct port preserves GLib CLAMP comparison order and signed-zero/NaN behavior"
)]
fn clamp_unit_native(value: f32) -> f32 {
    if value > 1.0 {
        1.0
    } else if value < 0.0 {
        0.0
    } else {
        value
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::{ColorZonesPixel, ColorZonesPlan, smooth_chroma};
    use crate::operations::colorzones::ColorZonesConfig;

    #[test]
    fn cloned_plans_share_one_exact_channel_major_lut_allocation() {
        let plan = ColorZonesPlan::new(ColorZonesConfig::defaults()).expect("default plan");
        let clone = plan.clone();

        assert_eq!(plan.luts.len(), 3 * super::COLORZONES_LUT_RESOLUTION);
        assert!(Arc::ptr_eq(&plan.luts, &clone.luts));
    }

    #[test]
    fn external_backend_candidates_use_native_lab_blend_and_preserve_alpha_bits() {
        let alpha = f32::from_bits(0x3eaa_aaab);
        let source = [ColorZonesPixel::new(40.0, -64.0, 32.0, alpha)];
        let candidate = [ColorZonesPixel::new(80.0, 64.0, -32.0, 1.0)];
        let blended = ColorZonesPlan::blend_lab_candidates(&source, &candidate, Some(&[0.5]), 0.5);

        assert!((blended[0].lightness() - 50.0).abs() <= 0.000_01);
        assert_eq!(blended[0].a().to_bits(), (-32.0_f32).to_bits());
        assert_eq!(blended[0].b().to_bits(), 16.0_f32.to_bits());
        assert_eq!(blended[0].alpha().to_bits(), alpha.to_bits());
    }

    #[test]
    fn smooth_fast_hypot_and_strong_lab_hypot_remain_bit_distinct() {
        let a = 1.0e-30_f32;
        let b = -1.0e-30_f32;
        let smooth = smooth_chroma(a, b);
        let strong = a.hypot(b);

        assert_eq!(smooth.to_bits(), 0.0_f32.to_bits());
        assert_ne!(strong.to_bits(), smooth.to_bits());
    }
}
