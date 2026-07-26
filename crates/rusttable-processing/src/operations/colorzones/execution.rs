//! Scalar Color Zones pixel execution ported from `src/iop/colorzones.c`.
//!
//! This module implements the native CPU `process_v1`, `process_v3`, and
//! `process` branches. It deliberately makes no GPU, display-mask, blend,
//! descriptor, registry, or UI claim.

#![allow(
    clippy::many_single_char_names,
    clippy::suboptimal_flops,
    reason = "the source-shaped Lab/LCh equations retain Darktable's names and operation order"
)]

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
/// stateful `piece->data` cache lifecycle remains outside this standalone
/// execution slice until Color Zones is routed through the operation registry.
#[derive(Debug, Clone, PartialEq)]
pub struct ColorZonesPlan {
    config: ColorZonesConfig,
    luts: ColorZonesLuts,
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

    use super::{ColorZonesPlan, smooth_chroma};
    use crate::operations::colorzones::ColorZonesConfig;

    #[test]
    fn cloned_plans_share_one_exact_channel_major_lut_allocation() {
        let plan = ColorZonesPlan::new(ColorZonesConfig::defaults()).expect("default plan");
        let clone = plan.clone();

        assert_eq!(plan.luts.len(), 3 * super::COLORZONES_LUT_RESOLUTION);
        assert!(Arc::ptr_eq(&plan.luts, &clone.luts));
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
