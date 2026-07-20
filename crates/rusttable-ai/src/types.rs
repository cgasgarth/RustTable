use std::fmt;
use std::path::PathBuf;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use crate::ColorProfile;
use crate::tiff::TiffSettings;

/// Checked image dimensions used by the AI boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ImageDimensions {
    width: u32,
    height: u32,
}

impl ImageDimensions {
    pub const fn new(width: u32, height: u32) -> Result<Self, LinearRgbaImageError> {
        if width == 0 {
            return Err(LinearRgbaImageError::ZeroWidth);
        }
        if height == 0 {
            return Err(LinearRgbaImageError::ZeroHeight);
        }
        Ok(Self { width, height })
    }

    #[must_use]
    pub const fn width(self) -> u32 {
        self.width
    }

    #[must_use]
    pub const fn height(self) -> u32 {
        self.height
    }

    #[must_use]
    pub fn pixels(self) -> Option<usize> {
        usize::try_from(u64::from(self.width).checked_mul(u64::from(self.height))?).ok()
    }

    #[must_use]
    pub fn bytes(self, channels: usize) -> Option<u64> {
        u64::from(self.width)
            .checked_mul(u64::from(self.height))?
            .checked_mul(u64::try_from(channels).ok()?)
            .and_then(|bytes| bytes.checked_mul(4))
    }
}

/// Straight-alpha linear working-profile pixels. Values may be HDR or below zero,
/// but every value is finite so model planning never depends on NaN behavior.
#[derive(Debug, Clone, PartialEq)]
pub struct LinearRgbaImage {
    dimensions: ImageDimensions,
    pixels: Vec<[f32; 4]>,
}

impl LinearRgbaImage {
    pub fn new(
        dimensions: ImageDimensions,
        pixels: Vec<[f32; 4]>,
    ) -> Result<Self, LinearRgbaImageError> {
        let expected = dimensions.pixels().ok_or(LinearRgbaImageError::Overflow)?;
        if pixels.len() != expected {
            return Err(LinearRgbaImageError::PixelCount {
                expected,
                actual: pixels.len(),
            });
        }
        if pixels.iter().flatten().any(|value| !value.is_finite()) {
            return Err(LinearRgbaImageError::NonFinite);
        }
        Ok(Self { dimensions, pixels })
    }

    #[must_use]
    pub const fn dimensions(&self) -> ImageDimensions {
        self.dimensions
    }

    #[must_use]
    pub fn pixels(&self) -> &[[f32; 4]] {
        &self.pixels
    }

    #[must_use]
    pub fn pixel(&self, x: u32, y: u32) -> Option<[f32; 4]> {
        if x >= self.dimensions.width() || y >= self.dimensions.height() {
            return None;
        }
        let index = usize::try_from(y)
            .ok()?
            .checked_mul(usize::try_from(self.dimensions.width()).ok()?)?
            .checked_add(usize::try_from(x).ok()?)?;
        self.pixels.get(index).copied()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinearRgbaImageError {
    ZeroWidth,
    ZeroHeight,
    Overflow,
    PixelCount { expected: usize, actual: usize },
    NonFinite,
}

impl fmt::Display for LinearRgbaImageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid linear RGBA image: {self:?}")
    }
}

impl std::error::Error for LinearRgbaImageError {}

/// Only integral model scales with an independently qualified task are allowed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SuperResolutionScale {
    X2,
    X4,
}

impl SuperResolutionScale {
    #[must_use]
    pub const fn integer(self) -> u32 {
        match self {
            Self::X2 => 2,
            Self::X4 => 4,
        }
    }
}

/// Deep-shadow handling follows Darktable's whole-image, pre-tile decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ShadowPolicy {
    Disabled,
    Auto,
}

/// An explicit cancellation flag shared by a workflow and its injected services.
#[derive(Debug, Clone, Default)]
pub struct CancellationToken(Arc<AtomicBool>);

impl CancellationToken {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

/// Export and catalog settings for one immutable super-resolution request.
#[derive(Debug, Clone, PartialEq)]
pub struct SuperResolutionSettings {
    pub(crate) output_profile: ColorProfile,
    pub(crate) tiff: TiffSettings,
    pub(crate) destination: Option<PathBuf>,
    pub(crate) preserve_wide_gamut: bool,
    pub(crate) shadow_policy: ShadowPolicy,
    pub(crate) source_photo_id: u64,
    pub(crate) edit_revision: u64,
}

impl SuperResolutionSettings {
    pub fn new(output_profile: ColorProfile) -> Self {
        Self {
            output_profile,
            tiff: TiffSettings::default(),
            destination: None,
            preserve_wide_gamut: false,
            shadow_policy: ShadowPolicy::Auto,
            source_photo_id: 0,
            edit_revision: 0,
        }
    }

    #[must_use]
    pub const fn output_profile(&self) -> &ColorProfile {
        &self.output_profile
    }

    #[must_use]
    pub const fn tiff(&self) -> &TiffSettings {
        &self.tiff
    }

    #[must_use]
    pub const fn shadow_policy(&self) -> ShadowPolicy {
        self.shadow_policy
    }

    #[must_use]
    pub fn with_tiff(mut self, tiff: TiffSettings) -> Self {
        self.tiff = tiff;
        self
    }

    #[must_use]
    pub fn with_destination(mut self, destination: PathBuf) -> Self {
        self.destination = Some(destination);
        self
    }

    #[must_use]
    pub fn with_shadow_policy(mut self, policy: ShadowPolicy) -> Self {
        self.shadow_policy = policy;
        self
    }

    #[must_use]
    pub fn with_wide_gamut_preservation(mut self, preserve: bool) -> Self {
        self.preserve_wide_gamut = preserve;
        self
    }

    #[must_use]
    pub const fn with_source_identity(mut self, photo_id: u64, edit_revision: u64) -> Self {
        self.source_photo_id = photo_id;
        self.edit_revision = edit_revision;
        self
    }
}

pub type OutputProfile = ColorProfile;

/// A revision-pinned source render and the requested immutable output settings.
#[derive(Debug, Clone, PartialEq)]
pub struct SuperResolutionRequest {
    pub(crate) source: LinearRgbaImage,
    pub(crate) source_profile: ColorProfile,
    pub(crate) scale: SuperResolutionScale,
    pub(crate) settings: SuperResolutionSettings,
}

impl SuperResolutionRequest {
    pub fn new(
        source: LinearRgbaImage,
        source_profile: ColorProfile,
        scale: SuperResolutionScale,
    ) -> Self {
        let settings = SuperResolutionSettings::new(source_profile.clone());
        Self {
            source,
            source_profile,
            scale,
            settings,
        }
    }

    #[must_use]
    pub fn with_settings(mut self, settings: SuperResolutionSettings) -> Self {
        self.settings = settings;
        self
    }

    #[must_use]
    pub const fn source(&self) -> &LinearRgbaImage {
        &self.source
    }

    #[must_use]
    pub const fn source_profile(&self) -> &ColorProfile {
        &self.source_profile
    }

    #[must_use]
    pub const fn scale(&self) -> SuperResolutionScale {
        self.scale
    }

    #[must_use]
    pub const fn settings(&self) -> &SuperResolutionSettings {
        &self.settings
    }
}
