use serde::{Deserialize, Serialize};
use std::fmt;
use std::hash::{Hash, Hasher};

pub const LENS_CORRECTION_PARAMETER_VERSION: u16 = 1;
pub const LENS_CORRECTION_PARAMETER_BYTES: usize = 0;
const LENS_CORRECTION_IDENTITY_PREFIX: &[u8] = b"rusttable.lenscorrection.parameters.v1";

/// Lensfun's projection families used by the operation contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LensGeometry {
    Rectilinear,
    Fisheye,
    Panoramic,
    Equirectangular,
    FisheyeOrthographic,
    FisheyeStereographic,
    FisheyeEquisolid,
    FisheyeThoby,
}

/// Whether the operation removes or applies the stored lens model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LensCorrectionMode {
    Correct,
    Distort,
}

/// Lensfun correction groups.  The value is deliberately a plain byte so it
/// remains stable in history and can be compared without a bitflags crate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CorrectionFlags(u8);

impl CorrectionFlags {
    pub const DISTORTION: Self = Self(1);
    pub const TCA: Self = Self(1 << 1);
    pub const VIGNETTING: Self = Self(1 << 2);
    pub const ALL: Self = Self(Self::DISTORTION.0 | Self::TCA.0 | Self::VIGNETTING.0);

    #[must_use]
    pub const fn empty() -> Self {
        Self(0)
    }

    #[must_use]
    pub const fn bits(self) -> u8 {
        self.0
    }

    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    #[must_use]
    pub const fn without(self, other: Self) -> Self {
        Self(self.0 & !other.0)
    }
}

impl Default for CorrectionFlags {
    fn default() -> Self {
        Self::ALL
    }
}

/// The semantic parameters persisted by `RustTable`. Camera and lens names
/// retain the EXIF spelling; matching applies `Lensfun`'s case-insensitive
/// lookup rules without mutating history.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LensCorrectionParametersV1 {
    pub method: LensCorrectionMethod,
    pub modify_flags: CorrectionFlags,
    pub mode: LensCorrectionMode,
    pub scale: f32,
    pub crop_factor: f32,
    pub focal_length: f32,
    pub aperture: f32,
    pub distance: f32,
    pub target_geometry: LensGeometry,
    pub camera: String,
    pub lens: String,
    pub tca_override: bool,
    pub tca_red: f32,
    pub tca_blue: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LensCorrectionMethod {
    Lensfun,
    OnlyVignetting,
}

impl LensCorrectionParametersV1 {
    /// Creates parameters after checking all values used by geometry or gain.
    ///
    /// # Errors
    ///
    /// Returns an error when an optical parameter is invalid.
    pub fn new(
        camera: impl Into<String>,
        lens: impl Into<String>,
        focal_length: f32,
        aperture: f32,
    ) -> Result<Self, LensCorrectionParameterError> {
        let parameters = Self {
            method: LensCorrectionMethod::Lensfun,
            modify_flags: CorrectionFlags::ALL,
            mode: LensCorrectionMode::Correct,
            scale: 1.0,
            crop_factor: 1.0,
            focal_length,
            aperture,
            distance: 1000.0,
            target_geometry: LensGeometry::Rectilinear,
            camera: camera.into(),
            lens: lens.into(),
            tca_override: false,
            tca_red: 1.0,
            tca_blue: 1.0,
        };
        parameters.validate()?;
        Ok(parameters)
    }

    /// # Errors
    ///
    /// Returns an error when a persisted value cannot produce finite geometry.
    pub fn validate(&self) -> Result<(), LensCorrectionParameterError> {
        for value in [
            self.scale,
            self.crop_factor,
            self.focal_length,
            self.aperture,
            self.distance,
            self.tca_red,
            self.tca_blue,
        ] {
            if !value.is_finite() {
                return Err(LensCorrectionParameterError::NonFinite);
            }
        }
        if !(0.1..=2.0).contains(&self.scale) {
            return Err(LensCorrectionParameterError::ScaleOutOfRange);
        }
        if self.crop_factor <= 0.0 || self.focal_length <= 0.0 || self.aperture <= 0.0 {
            return Err(LensCorrectionParameterError::NonPositiveOpticalValue);
        }
        if self.distance < 0.0 {
            return Err(LensCorrectionParameterError::NegativeDistance);
        }
        if self.camera.len() > 127 || self.lens.len() > 127 {
            return Err(LensCorrectionParameterError::NameTooLong);
        }
        if self.tca_override && !(0.99..=1.01).contains(&self.tca_red) {
            return Err(LensCorrectionParameterError::TcaOutOfRange);
        }
        if self.tca_override && !(0.99..=1.01).contains(&self.tca_blue) {
            return Err(LensCorrectionParameterError::TcaOutOfRange);
        }
        Ok(())
    }

    /// Returns the stable, version-framed bytes for this parameter value.
    ///
    /// The payload is the canonical V1 postcard encoding. Callers should use
    /// [`LensCorrectionConfig::canonical_identity_bytes`] when the pinned
    /// Lensfun snapshot must also participate in identity.
    #[must_use]
    pub fn canonical_identity_bytes(&self) -> Vec<u8> {
        canonical_identity_bytes(&encode_history(self))
    }
}

impl Default for LensCorrectionParametersV1 {
    fn default() -> Self {
        Self::new("", "", 50.0, 8.0).expect("finite lens correction defaults")
    }
}

/// A typed configuration plus untouched bytes from a future source version.
#[derive(Debug, Clone, PartialEq)]
pub struct LensCorrectionConfig {
    parameters: LensCorrectionParametersV1,
    opaque_source: Option<Vec<u8>>,
}

impl LensCorrectionConfig {
    /// # Errors
    ///
    /// Returns an error for invalid parameters.
    pub fn new(
        parameters: LensCorrectionParametersV1,
    ) -> Result<Self, LensCorrectionParameterError> {
        parameters.validate()?;
        Ok(Self {
            parameters,
            opaque_source: None,
        })
    }

    #[must_use]
    pub fn with_opaque_source(mut self, source: Vec<u8>) -> Self {
        self.opaque_source = Some(source);
        self
    }

    #[must_use]
    pub const fn parameters(&self) -> &LensCorrectionParametersV1 {
        &self.parameters
    }

    #[must_use]
    pub fn opaque_source(&self) -> Option<&[u8]> {
        self.opaque_source.as_deref()
    }

    /// Returns untouched imported bytes when present, otherwise the canonical
    /// V1 encoding for these parameters.
    #[must_use]
    pub fn history_bytes(&self) -> Vec<u8> {
        self.opaque_source
            .clone()
            .unwrap_or_else(|| encode_history(&self.parameters))
    }

    /// Returns stable identity bytes for pixelpipe snapshots.
    ///
    /// The identity includes the pinned Lensfun commit and either the typed
    /// V1 payload or untouched imported bytes retained by this config.
    #[must_use]
    pub fn canonical_identity_bytes(&self) -> Vec<u8> {
        let mut bytes = canonical_identity_bytes(&self.history_bytes());
        bytes.extend_from_slice(super::snapshot::LENSFUN_DATABASE_COMMIT.as_bytes());
        bytes
    }
}

impl Default for LensCorrectionConfig {
    fn default() -> Self {
        Self::new(LensCorrectionParametersV1::default()).expect("finite config defaults")
    }
}

impl Eq for LensCorrectionConfig {}

impl Hash for LensCorrectionConfig {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.parameters.method.hash(state);
        self.parameters.modify_flags.hash(state);
        self.parameters.mode.hash(state);
        for value in [
            self.parameters.scale,
            self.parameters.crop_factor,
            self.parameters.focal_length,
            self.parameters.aperture,
            self.parameters.distance,
            self.parameters.tca_red,
            self.parameters.tca_blue,
        ] {
            value.to_bits().hash(state);
        }
        self.parameters.target_geometry.hash(state);
        self.parameters.camera.hash(state);
        self.parameters.lens.hash(state);
        self.parameters.tca_override.hash(state);
        self.opaque_source.hash(state);
    }
}

/// Future history is retained instead of being silently interpreted as V1.
#[derive(Debug, Clone, PartialEq)]
pub enum LensCorrectionHistoryParameters {
    V1(LensCorrectionParametersV1),
    Opaque { version: u16, bytes: Vec<u8> },
}

/// Encodes the canonical postcard representation used by `RustTable` history.
///
/// # Panics
///
/// Panics only if the typed parameters cannot be serialized.
#[must_use]
pub fn encode_history(parameters: &LensCorrectionParametersV1) -> Vec<u8> {
    postcard::to_allocvec(parameters).expect("lens correction parameters are serializable")
}

/// # Errors
///
/// Returns an error when a V1 payload is malformed or fails validation.
pub fn decode_history(
    version: u16,
    bytes: &[u8],
) -> Result<LensCorrectionHistoryParameters, LensCorrectionCodecError> {
    if version != LENS_CORRECTION_PARAMETER_VERSION {
        return Ok(LensCorrectionHistoryParameters::Opaque {
            version,
            bytes: bytes.to_vec(),
        });
    }
    let parameters: LensCorrectionParametersV1 =
        postcard::from_bytes(bytes).map_err(|_| LensCorrectionCodecError::InvalidPayload)?;
    parameters
        .validate()
        .map_err(LensCorrectionCodecError::InvalidParameters)?;
    Ok(LensCorrectionHistoryParameters::V1(parameters))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LensCorrectionParameterError {
    NonFinite,
    ScaleOutOfRange,
    NonPositiveOpticalValue,
    NegativeDistance,
    NameTooLong,
    TcaOutOfRange,
}

impl fmt::Display for LensCorrectionParameterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NonFinite => "lens correction parameter is non-finite",
            Self::ScaleOutOfRange => "lens correction scale must be between 0.1 and 2.0",
            Self::NonPositiveOpticalValue => "lens correction optical values must be positive",
            Self::NegativeDistance => "lens correction focus distance cannot be negative",
            Self::NameTooLong => "lens correction camera and lens names are limited to 127 bytes",
            Self::TcaOutOfRange => "lens correction TCA override must be between 0.99 and 1.01",
        })
    }
}

impl std::error::Error for LensCorrectionParameterError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LensCorrectionCodecError {
    InvalidPayload,
    InvalidParameters(LensCorrectionParameterError),
}

impl fmt::Display for LensCorrectionCodecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPayload => {
                formatter.write_str("lens correction history payload is invalid")
            }
            Self::InvalidParameters(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for LensCorrectionCodecError {}

fn canonical_identity_bytes(payload: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(
        LENS_CORRECTION_IDENTITY_PREFIX
            .len()
            .saturating_add(2)
            .saturating_add(payload.len()),
    );
    bytes.extend_from_slice(LENS_CORRECTION_IDENTITY_PREFIX);
    bytes.extend_from_slice(&LENS_CORRECTION_PARAMETER_VERSION.to_le_bytes());
    bytes.extend_from_slice(payload);
    bytes
}
