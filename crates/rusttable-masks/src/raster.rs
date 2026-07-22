use crate::{MaskRoi, RasterMaskDescriptor};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fmt;

const MASK_BYTES_PER_VALUE: usize = std::mem::size_of::<f32>();

/// Canonical finite mask coverage. Values are always owned and immutable once
/// published; execution works on a private copy.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MaskRaster {
    width: u32,
    height: u32,
    values: Vec<f32>,
}

impl MaskRaster {
    /// Validates a dense row-major mask.
    pub fn new(width: u32, height: u32, values: Vec<f32>) -> Result<Self, MaskExecutionError> {
        if width == 0 || height == 0 {
            return Err(MaskExecutionError::InvalidDimensions);
        }
        let expected = usize::try_from(u64::from(width) * u64::from(height))
            .map_err(|_| MaskExecutionError::ArithmeticOverflow)?;
        if values.len() != expected {
            return Err(MaskExecutionError::DimensionsMismatch {
                expected,
                actual: values.len(),
            });
        }
        if values
            .iter()
            .any(|value| !value.is_finite() || !(0.0..=1.0).contains(value))
        {
            return Err(MaskExecutionError::InvalidCoverage);
        }
        Ok(Self {
            width,
            height,
            values,
        })
    }

    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }
    #[must_use]
    pub fn values(&self) -> &[f32] {
        &self.values
    }
    #[must_use]
    pub fn memory_bytes(&self) -> usize {
        self.values.len() * MASK_BYTES_PER_VALUE
    }
    #[must_use]
    pub fn identity(&self) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(b"rusttable.mask-raster.v1");
        hasher.update(self.width.to_le_bytes());
        hasher.update(self.height.to_le_bytes());
        for value in &self.values {
            hasher.update(value.to_bits().to_le_bytes());
        }
        hasher.finalize().into()
    }

    /// Returns a dense row-major copy for a checked half-open ROI.
    pub fn crop(&self, roi: MaskRoi) -> Result<Self, MaskExecutionError> {
        let right = roi
            .x()
            .checked_add(roi.width())
            .ok_or(MaskExecutionError::ArithmeticOverflow)?;
        let bottom = roi
            .y()
            .checked_add(roi.height())
            .ok_or(MaskExecutionError::ArithmeticOverflow)?;
        if roi.width() == 0 || roi.height() == 0 || right > self.width || bottom > self.height {
            return Err(MaskExecutionError::InvalidRoi);
        }
        let source_width =
            usize::try_from(self.width).map_err(|_| MaskExecutionError::ArithmeticOverflow)?;
        let x = usize::try_from(roi.x()).map_err(|_| MaskExecutionError::ArithmeticOverflow)?;
        let y = usize::try_from(roi.y()).map_err(|_| MaskExecutionError::ArithmeticOverflow)?;
        let width =
            usize::try_from(roi.width()).map_err(|_| MaskExecutionError::ArithmeticOverflow)?;
        let height =
            usize::try_from(roi.height()).map_err(|_| MaskExecutionError::ArithmeticOverflow)?;
        let mut values = Vec::with_capacity(
            width
                .checked_mul(height)
                .ok_or(MaskExecutionError::ArithmeticOverflow)?,
        );
        for row in y..y + height {
            let start = row
                .checked_mul(source_width)
                .and_then(|value| value.checked_add(x))
                .ok_or(MaskExecutionError::ArithmeticOverflow)?;
            let end = start
                .checked_add(width)
                .ok_or(MaskExecutionError::ArithmeticOverflow)?;
            values.extend_from_slice(
                self.values
                    .get(start..end)
                    .ok_or(MaskExecutionError::InvalidRoi)?,
            );
        }
        Self::new(roi.width(), roi.height(), values)
    }

    /// Applies one exact modifier without mutating the published raster.
    pub fn modified(&self, modifier: crate::MaskModifier) -> Result<Self, MaskExecutionError> {
        match modifier {
            crate::MaskModifier::Invert => Self::new(
                self.width,
                self.height,
                self.values.iter().map(|value| 1.0 - value).collect(),
            ),
            crate::MaskModifier::Opacity(opacity) => {
                if !opacity.is_finite() || !(0.0..=1.0).contains(&opacity) {
                    return Err(MaskExecutionError::InvalidOpacity);
                }
                Self::new(
                    self.width,
                    self.height,
                    self.values.iter().map(|value| value * opacity).collect(),
                )
            }
            crate::MaskModifier::Feather(radius) => self.feather(radius),
        }
    }

    /// Deterministic separable box feather. Edge samples are clamped, and the
    /// denominator is the actual sample count at every pixel.
    pub fn feather(&self, radius: u32) -> Result<Self, MaskExecutionError> {
        if radius == 0 {
            return Ok(self.clone());
        }
        let radius = usize::try_from(radius).map_err(|_| MaskExecutionError::ArithmeticOverflow)?;
        let width =
            usize::try_from(self.width).map_err(|_| MaskExecutionError::ArithmeticOverflow)?;
        let height =
            usize::try_from(self.height).map_err(|_| MaskExecutionError::ArithmeticOverflow)?;
        let mut horizontal = vec![0.0; self.values.len()];
        for y in 0..height {
            for x in 0..width {
                let start = x.saturating_sub(radius);
                let end = x.saturating_add(radius).min(width.saturating_sub(1));
                let count = end - start + 1;
                horizontal[y * width + x] = (start..=end)
                    .map(|sample| self.values[y * width + sample])
                    .sum::<f32>()
                    / count as f32;
            }
        }
        let mut output = vec![0.0; self.values.len()];
        for y in 0..height {
            for x in 0..width {
                let start = y.saturating_sub(radius);
                let end = y.saturating_add(radius).min(height.saturating_sub(1));
                let count = end - start + 1;
                output[y * width + x] = (start..=end)
                    .map(|sample| horizontal[sample * width + x])
                    .sum::<f32>()
                    / count as f32;
            }
        }
        Self::new(self.width, self.height, output)
    }

    pub(crate) fn combine(
        &self,
        other: &Self,
        mode: crate::CombinationMode,
    ) -> Result<Self, MaskExecutionError> {
        if self.width != other.width || self.height != other.height {
            return Err(MaskExecutionError::DimensionsMismatch {
                expected: self.values.len(),
                actual: other.values.len(),
            });
        }
        Self::new(
            self.width,
            self.height,
            self.values
                .iter()
                .zip(&other.values)
                .map(|(left, right)| mode.combine(*left, *right))
                .collect(),
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MaskExecutionError {
    ArithmeticOverflow,
    InvalidDimensions,
    DimensionsMismatch { expected: usize, actual: usize },
    InvalidCoverage,
    InvalidOpacity,
    InvalidRoi,
    MissingPublishedRaster([u8; 32]),
    MemoryBudgetExceeded { required: usize, budget: usize },
    OpaqueSource { version: u16 },
    AmbiguousConsumer { operation: u128 },
}

impl fmt::Display for MaskExecutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ArithmeticOverflow => formatter.write_str("mask arithmetic overflowed"),
            Self::InvalidDimensions => formatter.write_str("mask dimensions must be nonzero"),
            Self::DimensionsMismatch { expected, actual } => {
                write!(formatter, "mask has {actual} values, expected {expected}")
            }
            Self::InvalidCoverage => formatter.write_str("mask coverage must be finite in 0..=1"),
            Self::InvalidOpacity => formatter.write_str("mask opacity must be finite in 0..=1"),
            Self::InvalidRoi => {
                formatter.write_str("published mask does not match its descriptor ROI")
            }
            Self::MissingPublishedRaster(identity) => write!(
                formatter,
                "published raster mask {identity:02x?} is unavailable"
            ),
            Self::MemoryBudgetExceeded { required, budget } => write!(
                formatter,
                "mask store requires {required} bytes, budget is {budget}"
            ),
            Self::OpaqueSource { version } => write!(
                formatter,
                "mask source version {version} is opaque and blocking"
            ),
            Self::AmbiguousConsumer { operation } => {
                write!(formatter, "operation {operation} has multiple mask edges")
            }
        }
    }
}

impl std::error::Error for MaskExecutionError {}

/// A publication is the only cross-operation handoff for generated rasters.
#[derive(Debug, Clone, PartialEq)]
pub struct RasterMaskPublication {
    descriptor: RasterMaskDescriptor,
    raster: MaskRaster,
}

impl RasterMaskPublication {
    pub fn new(
        descriptor: RasterMaskDescriptor,
        raster: MaskRaster,
    ) -> Result<Self, MaskExecutionError> {
        let roi = descriptor.producer().roi();
        if roi.width() != raster.width() || roi.height() != raster.height() {
            return Err(MaskExecutionError::InvalidRoi);
        }
        Ok(Self { descriptor, raster })
    }
    #[must_use]
    pub fn descriptor(&self) -> &RasterMaskDescriptor {
        &self.descriptor
    }
    #[must_use]
    pub fn raster(&self) -> &MaskRaster {
        &self.raster
    }
}

#[derive(Debug, Clone, PartialEq)]
struct StoredMask {
    publication: RasterMaskPublication,
    last_used: u64,
}

/// Bounded deterministic host/GPU-neutral store. A consumer receives a clone
/// lease; no consumer can mutate or invalidate the published value.
#[derive(Debug, Clone, PartialEq)]
pub struct RasterMaskStore {
    budget: usize,
    resident: usize,
    clock: u64,
    entries: BTreeMap<[u8; 32], StoredMask>,
}

impl RasterMaskStore {
    #[must_use]
    pub fn new(budget: usize) -> Self {
        Self {
            budget,
            resident: 0,
            clock: 0,
            entries: BTreeMap::new(),
        }
    }
    #[must_use]
    pub const fn budget(&self) -> usize {
        self.budget
    }
    #[must_use]
    pub const fn resident_bytes(&self) -> usize {
        self.resident
    }
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Stable identity of resident raster content, excluding LRU state.
    #[must_use]
    pub fn identity(&self) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(b"rusttable.mask-store.v1");
        hasher.update((self.budget as u64).to_le_bytes());
        for (key, entry) in &self.entries {
            hasher.update(key);
            hasher.update(entry.publication.descriptor().cache_identity());
            hasher.update(entry.publication.raster().identity());
        }
        hasher.finalize().into()
    }

    /// Publishes atomically and evicts least-recently-used entries by full
    /// descriptor identity. An over-budget single raster is rejected.
    pub fn publish(
        &mut self,
        publication: RasterMaskPublication,
    ) -> Result<(), MaskExecutionError> {
        let key = publication.descriptor().cache_identity();
        let bytes = publication.raster().memory_bytes();
        if bytes > self.budget {
            return Err(MaskExecutionError::MemoryBudgetExceeded {
                required: bytes,
                budget: self.budget,
            });
        }
        if let Some(previous) = self.entries.remove(&key) {
            self.resident = self
                .resident
                .saturating_sub(previous.publication.raster().memory_bytes());
        }
        while self.resident.saturating_add(bytes) > self.budget {
            let Some((victim, entry)) = self
                .entries
                .iter()
                .min_by_key(|(identity, entry)| (entry.last_used, **identity))
                .map(|(identity, entry)| (*identity, entry.publication.raster().memory_bytes()))
            else {
                break;
            };
            self.entries.remove(&victim);
            self.resident = self.resident.saturating_sub(entry);
        }
        self.clock = self
            .clock
            .checked_add(1)
            .ok_or(MaskExecutionError::ArithmeticOverflow)?;
        self.resident = self
            .resident
            .checked_add(bytes)
            .ok_or(MaskExecutionError::ArithmeticOverflow)?;
        self.entries.insert(
            key,
            StoredMask {
                publication,
                last_used: self.clock,
            },
        );
        Ok(())
    }

    /// Consumes one exact publication and records use for deterministic eviction.
    pub fn consume(
        &mut self,
        descriptor: &RasterMaskDescriptor,
    ) -> Result<MaskLease, MaskExecutionError> {
        let key = descriptor.cache_identity();
        self.clock = self
            .clock
            .checked_add(1)
            .ok_or(MaskExecutionError::ArithmeticOverflow)?;
        let entry = self
            .entries
            .get_mut(&key)
            .ok_or(MaskExecutionError::MissingPublishedRaster(key))?;
        entry.last_used = self.clock;
        Ok(MaskLease {
            descriptor: entry.publication.descriptor.clone(),
            raster: entry.publication.raster.clone(),
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct MaskLease {
    descriptor: RasterMaskDescriptor,
    raster: MaskRaster,
}

impl MaskLease {
    #[must_use]
    pub fn descriptor(&self) -> &RasterMaskDescriptor {
        &self.descriptor
    }
    #[must_use]
    pub fn raster(&self) -> &MaskRaster {
        &self.raster
    }
}
