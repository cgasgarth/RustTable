//! Darktable-compatible RAW sensor preparation.
//!
//! `rawprepare` is deliberately independent of decoder code.  Decoders own
//! the source [`RawMosaic`]; this module owns the checked active-area crop,
//! black/white normalization, and the cache identity consumed by demosaic.

use crate::FiniteF32;
use rusttable_image::{BlackWhiteLevels, CfaDescriptor, ImageDimensions, RawMosaic, Roi};
use sha2::{Digest, Sha256};
use std::fmt;

pub const RAWPREPARE_COMPATIBILITY_ID: &str = "rawprepare";
pub const RAWPREPARE_SCHEMA_VERSION: u16 = 1;

/// Input-side configuration for one immutable sensor-preparation plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct RawPrepareConfig {
    active_area: Option<Roi>,
}

impl RawPrepareConfig {
    #[must_use]
    pub const fn new(active_area: Option<Roi>) -> Self {
        Self { active_area }
    }

    #[must_use]
    pub const fn active_area(self) -> Option<Roi> {
        self.active_area
    }
}

/// A normalized, cropped sensor plane.  Values remain unclipped so negative
/// black-level residuals and highlights above one remain available downstream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedRaw {
    dimensions: ImageDimensions,
    samples: Vec<FiniteF32>,
    cfa: CfaDescriptor,
    levels: BlackWhiteLevels,
    orientation: rusttable_image::Orientation,
}

impl NormalizedRaw {
    #[must_use]
    pub const fn dimensions(&self) -> ImageDimensions {
        self.dimensions
    }

    #[must_use]
    pub fn samples(&self) -> &[FiniteF32] {
        &self.samples
    }

    #[must_use]
    pub const fn cfa(&self) -> CfaDescriptor {
        self.cfa
    }

    #[must_use]
    pub const fn levels(&self) -> BlackWhiteLevels {
        self.levels
    }

    #[must_use]
    pub const fn orientation(&self) -> rusttable_image::Orientation {
        self.orientation
    }

    #[must_use]
    pub fn sample(&self, x: u32, y: u32) -> Option<FiniteF32> {
        if x >= self.dimensions.width() || y >= self.dimensions.height() {
            return None;
        }
        let width = usize::try_from(self.dimensions.width()).ok()?;
        let index = usize::try_from(y)
            .ok()?
            .checked_mul(width)?
            .checked_add(usize::try_from(x).ok()?)?;
        self.samples.get(index).copied()
    }
}

/// A checked sensor-normalization plan with stable cache identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawPreparePlan {
    config: RawPrepareConfig,
    source_dimensions: ImageDimensions,
    output_dimensions: ImageDimensions,
    cfa: CfaDescriptor,
    levels: BlackWhiteLevels,
    orientation: rusttable_image::Orientation,
    identity: [u8; 32],
}

impl RawPreparePlan {
    /// Compiles a plan against source metadata without reading pixel bytes.
    ///
    /// # Errors
    ///
    /// Returns an error when the requested active area is empty, out of bounds,
    /// or has invalid dimensions.
    pub fn new(raw: &RawMosaic, config: RawPrepareConfig) -> Result<Self, RawPrepareError> {
        let output_dimensions = config
            .active_area()
            .map(|roi| {
                roi.within(raw.dimensions())
                    .map_err(|_| RawPrepareError::ActiveAreaOutOfBounds)?;
                if roi.is_empty() {
                    return Err(RawPrepareError::EmptyActiveArea);
                }
                ImageDimensions::new(roi.width(), roi.height())
                    .map_err(|_| RawPrepareError::InvalidDimensions)
            })
            .transpose()?
            .unwrap_or(raw.dimensions());
        let cfa = config
            .active_area()
            .map_or(raw.cfa(), |roi| raw.cfa().after_crop(roi));
        let mut digest = Sha256::new();
        digest.update(RAWPREPARE_COMPATIBILITY_ID.as_bytes());
        digest.update(RAWPREPARE_SCHEMA_VERSION.to_le_bytes());
        digest.update(raw.dimensions().width().to_le_bytes());
        digest.update(raw.dimensions().height().to_le_bytes());
        digest.update(output_dimensions.width().to_le_bytes());
        digest.update(output_dimensions.height().to_le_bytes());
        digest.update(raw.levels().black().to_le_bytes());
        digest.update(raw.levels().white().to_le_bytes());
        if let Some(roi) = config.active_area() {
            digest.update(roi.x().to_le_bytes());
            digest.update(roi.y().to_le_bytes());
            digest.update(roi.width().to_le_bytes());
            digest.update(roi.height().to_le_bytes());
        }
        Ok(Self {
            config,
            source_dimensions: raw.dimensions(),
            output_dimensions,
            cfa,
            levels: raw.levels(),
            orientation: raw.orientation(),
            identity: digest.finalize().into(),
        })
    }

    #[must_use]
    pub const fn config(&self) -> RawPrepareConfig {
        self.config
    }

    #[must_use]
    pub const fn source_dimensions(&self) -> ImageDimensions {
        self.source_dimensions
    }

    #[must_use]
    pub const fn output_dimensions(&self) -> ImageDimensions {
        self.output_dimensions
    }

    #[must_use]
    pub const fn cfa(&self) -> CfaDescriptor {
        self.cfa
    }

    #[must_use]
    pub const fn identity(&self) -> [u8; 32] {
        self.identity
    }

    /// Executes normalization using the exact source bytes used to compile it.
    ///
    /// # Errors
    ///
    /// Returns an error when source metadata changed, cropping fails, or a
    /// normalized sample is non-finite.
    pub fn execute(&self, raw: &RawMosaic) -> Result<NormalizedRaw, RawPrepareError> {
        if raw.dimensions() != self.source_dimensions
            || raw.levels() != self.levels
            || raw.orientation() != self.orientation
        {
            return Err(RawPrepareError::SourceMetadataChanged);
        }
        let cropped = self
            .config
            .active_area()
            .map_or_else(|| Ok(raw.clone()), |roi| raw.crop(roi))
            .map_err(RawPrepareError::Crop)?;
        let denominator = f32::from(self.levels.white() - self.levels.black());
        let samples = cropped
            .samples()
            .iter()
            .copied()
            .enumerate()
            .map(|(index, sample)| {
                let value = (f32::from(sample) - f32::from(self.levels.black())) / denominator;
                FiniteF32::new(value).map_err(|_| RawPrepareError::NonFiniteSample { index })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(NormalizedRaw {
            dimensions: cropped.dimensions(),
            samples,
            cfa: cropped.cfa(),
            levels: cropped.levels(),
            orientation: cropped.orientation(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RawPrepareError {
    ActiveAreaOutOfBounds,
    EmptyActiveArea,
    InvalidDimensions,
    Crop(rusttable_image::RawMosaicError),
    SourceMetadataChanged,
    NonFiniteSample { index: usize },
}

impl fmt::Display for RawPrepareError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ActiveAreaOutOfBounds => {
                formatter.write_str("rawprepare active area is out of bounds")
            }
            Self::EmptyActiveArea => formatter.write_str("rawprepare active area is empty"),
            Self::InvalidDimensions => {
                formatter.write_str("rawprepare output dimensions are invalid")
            }
            Self::Crop(error) => write!(formatter, "rawprepare crop failed: {error}"),
            Self::SourceMetadataChanged => {
                formatter.write_str("rawprepare source metadata changed")
            }
            Self::NonFiniteSample { index } => {
                write!(formatter, "rawprepare sample {index} is non-finite")
            }
        }
    }
}

impl std::error::Error for RawPrepareError {}

#[cfg(test)]
mod tests {
    use super::*;
    use rusttable_image::{CfaPattern, Orientation};

    fn raw() -> RawMosaic {
        RawMosaic::new(
            ImageDimensions::new(2, 2).expect("dimensions"),
            2,
            vec![0, 1000, 2000, 4000],
            CfaPattern::bayer_rggb(),
            rusttable_image::CfaPhase::new(0, 0, CfaPattern::bayer_rggb()),
            BlackWhiteLevels::new(0, 4000).expect("levels"),
            Orientation::Normal,
        )
        .expect("raw")
    }

    #[test]
    fn normalizes_without_clipping_and_preserves_cfa() {
        let source = raw();
        let plan = RawPreparePlan::new(&source, RawPrepareConfig::default()).expect("plan");
        let output = plan.execute(&source).expect("execute");
        let expected = [0.0, 0.25, 0.5, 1.0];
        for (sample, expected) in output.samples().iter().zip(expected) {
            assert!((sample.get() - expected).abs() <= f32::EPSILON);
        }
        assert_eq!(output.cfa(), source.cfa());
    }

    #[test]
    fn crop_updates_dimensions_and_phase() {
        let source = RawMosaic::new(
            ImageDimensions::new(4, 4).expect("dimensions"),
            4,
            (0..16).map(|value| value * 100).collect(),
            CfaPattern::bayer_rggb(),
            rusttable_image::CfaPhase::new(0, 0, CfaPattern::bayer_rggb()),
            BlackWhiteLevels::new(0, 2000).expect("levels"),
            rusttable_image::Orientation::Normal,
        )
        .expect("raw");
        let roi = Roi::new(1, 1, 2, 2).expect("roi");
        let output = RawPreparePlan::new(&source, RawPrepareConfig::new(Some(roi)))
            .expect("plan")
            .execute(&source)
            .expect("execute");
        assert_eq!(
            output.dimensions(),
            ImageDimensions::new(2, 2).expect("dimensions")
        );
        assert_eq!(output.cfa().phase().x(), 1);
        assert_eq!(output.cfa().phase().y(), 1);
    }
}
