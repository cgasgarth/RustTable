//! Deterministic scalar demosaic for normalized Bayer and X-Trans mosaics.
//!
//! The operation keeps algorithm choice in the plan identity.  The initial
//! implementation provides the safe, canonical neighborhood path used for
//! previews and as a reference implementation for later quality-specific
//! algorithms.

use crate::rawprepare::NormalizedRaw;
use crate::{FiniteF32, LinearRgb, RasterDimensions};
use rusttable_image::{CfaColor, CfaPattern, ImageDimensions};
use sha2::{Digest, Sha256};
use std::fmt;

pub const DEMOSAIC_COMPATIBILITY_ID: &str = "demosaic";
pub const DEMOSAIC_SCHEMA_VERSION: u16 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DemosaicAlgorithm {
    /// The bounded bilinear reference path used for deterministic previews.
    Bilinear,
    /// Replicates a monochrome sensor sample into all three RGB channels.
    Monochrome,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DemosaicError {
    UnsupportedCfa,
    InvalidDimensions,
    SourceMetadataChanged,
    NonFiniteOutput { index: usize, channel: CfaColor },
}

impl fmt::Display for DemosaicError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedCfa => formatter.write_str("demosaic CFA contains unsupported colors"),
            Self::InvalidDimensions => formatter.write_str("demosaic dimensions are invalid"),
            Self::SourceMetadataChanged => formatter.write_str("demosaic source metadata changed"),
            Self::NonFiniteOutput { index, channel } => {
                write!(
                    formatter,
                    "demosaic output {channel:?} at pixel {index} is non-finite"
                )
            }
        }
    }
}

impl std::error::Error for DemosaicError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DemosaicPlan {
    algorithm: DemosaicAlgorithm,
    dimensions: ImageDimensions,
    cfa: CfaPattern,
    phase: rusttable_image::CfaPhase,
    identity: [u8; 32],
}

impl DemosaicPlan {
    /// Builds an immutable plan for one normalized sensor interpretation.
    ///
    /// # Errors
    ///
    /// Returns an error when the source CFA contains an unsupported clear
    /// pixel or when plan metadata cannot be represented.
    pub fn new(raw: &NormalizedRaw, algorithm: DemosaicAlgorithm) -> Result<Self, DemosaicError> {
        let cfa = raw.cfa().pattern();
        if matches!(cfa, CfaPattern::Bayer(pattern) if pattern.into_iter().flatten().any(|color| color == CfaColor::Clear))
        {
            return Err(DemosaicError::UnsupportedCfa);
        }
        let mut digest = Sha256::new();
        digest.update(DEMOSAIC_COMPATIBILITY_ID.as_bytes());
        digest.update(DEMOSAIC_SCHEMA_VERSION.to_le_bytes());
        digest.update([algorithm as u8]);
        digest.update(raw.dimensions().width().to_le_bytes());
        digest.update(raw.dimensions().height().to_le_bytes());
        digest.update(raw.cfa().phase().x().to_le_bytes());
        digest.update(raw.cfa().phase().y().to_le_bytes());
        Ok(Self {
            algorithm,
            dimensions: raw.dimensions(),
            cfa,
            phase: raw.cfa().phase(),
            identity: digest.finalize().into(),
        })
    }

    #[must_use]
    pub const fn algorithm(&self) -> DemosaicAlgorithm {
        self.algorithm
    }

    #[must_use]
    pub const fn identity(&self) -> [u8; 32] {
        self.identity
    }

    /// Executes scalar demosaic without clipping scene-linear values.
    ///
    /// # Errors
    ///
    /// Returns an error when the source metadata differs from the compiled
    /// plan, dimensions are invalid, or an output channel is non-finite.
    pub fn execute(&self, raw: &NormalizedRaw) -> Result<DemosaicedImage, DemosaicError> {
        if raw.dimensions() != self.dimensions
            || raw.cfa().pattern() != self.cfa
            || raw.cfa().phase() != self.phase
        {
            return Err(DemosaicError::SourceMetadataChanged);
        }
        let dimensions = RasterDimensions::new(self.dimensions.width(), self.dimensions.height())
            .map_err(|_| DemosaicError::InvalidDimensions)?;
        let width = usize::try_from(self.dimensions.width())
            .map_err(|_| DemosaicError::InvalidDimensions)?;
        let radius = match self.cfa {
            CfaPattern::Bayer(_) => 1,
            CfaPattern::XTrans(_) => 2,
        };
        let mut pixels = Vec::with_capacity(
            usize::try_from(
                self.dimensions
                    .pixel_count()
                    .map_err(|_| DemosaicError::InvalidDimensions)?,
            )
            .map_err(|_| DemosaicError::InvalidDimensions)?,
        );
        for index in 0..usize::try_from(
            self.dimensions
                .pixel_count()
                .map_err(|_| DemosaicError::InvalidDimensions)?,
        )
        .map_err(|_| DemosaicError::InvalidDimensions)?
        {
            let x = u32::try_from(index % width).map_err(|_| DemosaicError::InvalidDimensions)?;
            let y = u32::try_from(index / width).map_err(|_| DemosaicError::InvalidDimensions)?;
            let pixel = match self.algorithm {
                DemosaicAlgorithm::Monochrome => {
                    let value = raw
                        .sample(x, y)
                        .ok_or(DemosaicError::InvalidDimensions)?
                        .get();
                    rgb(value, value, value, index)?
                }
                DemosaicAlgorithm::Bilinear => {
                    let red = average_color(raw, x, y, CfaColor::Red, radius);
                    let green = average_color(raw, x, y, CfaColor::Green, radius);
                    let blue = average_color(raw, x, y, CfaColor::Blue, radius);
                    rgb(red, green, blue, index)?
                }
            };
            pixels.push(pixel);
        }
        Ok(DemosaicedImage {
            dimensions,
            pixels,
            source_cfa: raw.cfa(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DemosaicedImage {
    dimensions: RasterDimensions,
    pixels: Vec<LinearRgb>,
    source_cfa: rusttable_image::CfaDescriptor,
}

impl DemosaicedImage {
    #[must_use]
    pub const fn dimensions(&self) -> RasterDimensions {
        self.dimensions
    }

    #[must_use]
    pub fn pixels(&self) -> &[LinearRgb] {
        &self.pixels
    }

    #[must_use]
    pub const fn source_cfa(&self) -> rusttable_image::CfaDescriptor {
        self.source_cfa
    }
}

fn average_color(raw: &NormalizedRaw, x: u32, y: u32, wanted: CfaColor, radius: u32) -> f32 {
    let mut total = 0.0;
    let mut count = 0_u32;
    let width = raw.dimensions().width();
    let height = raw.dimensions().height();
    let x0 = x.saturating_sub(radius);
    let y0 = y.saturating_sub(radius);
    let x1 = x.saturating_add(radius).min(width.saturating_sub(1));
    let y1 = y.saturating_add(radius).min(height.saturating_sub(1));
    for sample_y in y0..=y1 {
        for sample_x in x0..=x1 {
            if raw
                .cfa()
                .pattern()
                .color_at(sample_x, sample_y, raw.cfa().phase())
                == wanted
                && let Some(value) = raw.sample(sample_x, sample_y)
            {
                total += value.get();
                count = count.saturating_add(1);
            }
        }
    }
    if count == 0 {
        raw.sample(x, y).map_or(0.0, FiniteF32::get)
    } else {
        let count = u16::try_from(count).expect("demosaic neighborhood is bounded");
        total / f32::from(count)
    }
}

fn rgb(red: f32, green: f32, blue: f32, index: usize) -> Result<LinearRgb, DemosaicError> {
    let values = [
        (red, CfaColor::Red),
        (green, CfaColor::Green),
        (blue, CfaColor::Blue),
    ];
    for (value, channel) in values {
        if !value.is_finite() {
            return Err(DemosaicError::NonFiniteOutput { index, channel });
        }
    }
    Ok(LinearRgb::new(
        FiniteF32::new(red).expect("finite red"),
        FiniteF32::new(green).expect("finite green"),
        FiniteF32::new(blue).expect("finite blue"),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{RawPrepareConfig, RawPreparePlan};
    use rusttable_image::{
        BlackWhiteLevels, CfaPattern, CfaPhase, ImageDimensions, Orientation, RawMosaic,
    };

    #[test]
    fn bayer_reference_is_deterministic_and_preserves_scene_headroom() {
        let pattern = CfaPattern::bayer_rggb();
        let raw = RawMosaic::new(
            ImageDimensions::new(2, 2).expect("dimensions"),
            2,
            vec![4000, 2000, 2000, 1000],
            pattern,
            CfaPhase::new(0, 0, pattern),
            BlackWhiteLevels::new(0, 4000).expect("levels"),
            Orientation::Normal,
        )
        .expect("raw");
        let normalized = RawPreparePlan::new(&raw, RawPrepareConfig::default())
            .expect("prepare")
            .execute(&raw)
            .expect("normalized");
        let plan = DemosaicPlan::new(&normalized, DemosaicAlgorithm::Bilinear).expect("plan");
        let first = plan.execute(&normalized).expect("first");
        let second = plan.execute(&normalized).expect("second");
        assert_eq!(first, second);
        assert_eq!(first.pixels().len(), 4);
        assert!(first.pixels()[0].red().get() > 0.9);
    }

    #[test]
    fn monochrome_replicates_one_sample_to_rgb() {
        let pattern = CfaPattern::bayer_rggb();
        let raw = RawMosaic::new(
            ImageDimensions::new(1, 1).expect("dimensions"),
            1,
            vec![1000],
            pattern,
            CfaPhase::new(0, 0, pattern),
            BlackWhiteLevels::new(0, 2000).expect("levels"),
            Orientation::Normal,
        )
        .expect("raw");
        let normalized = RawPreparePlan::new(&raw, RawPrepareConfig::default())
            .expect("prepare")
            .execute(&raw)
            .expect("normalized");
        let output = DemosaicPlan::new(&normalized, DemosaicAlgorithm::Monochrome)
            .expect("plan")
            .execute(&normalized)
            .expect("output");
        assert_eq!(output.pixels()[0].red(), output.pixels()[0].green());
        assert_eq!(output.pixels()[0].green(), output.pixels()[0].blue());
    }
}
