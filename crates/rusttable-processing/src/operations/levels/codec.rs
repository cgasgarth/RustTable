//! Native parameter ABI and history migration for `src/iop/levels.c`.

use std::fmt;

use crate::FiniteF32;

/// Stable retained-module name used by native history records.
pub const LEVELS_COMPATIBILITY_ID: &str = "levels";
/// Rust operation identity reserved for the later registry integration.
pub const LEVELS_RUST_ID: &str = "rusttable.levels";
/// Native `DT_MODULE_INTROSPECTION(2, dt_iop_levels_params_t)` version.
pub const LEVELS_SCHEMA_VERSION: u16 = 2;
/// Native four-float pixel layout: L, a, b, and alpha/spare.
pub const LEVELS_CHANNELS: usize = 4;
/// Bytes in the retained v1 payload (`float levels[3]; int levels_preset`).
pub const LEVELS_PARAMETER_BYTES_V1: usize = 16;
/// Bytes in the retained v2 payload (`enum`/int, three percentiles, three levels).
pub const LEVELS_PARAMETER_BYTES_V2: usize = 28;

pub const LEVELS_BLACK_MINIMUM: f32 = 0.0;
pub const LEVELS_BLACK_MAXIMUM: f32 = 100.0;
pub const LEVELS_BLACK_DEFAULT: f32 = 0.0;
pub const LEVELS_GRAY_MINIMUM: f32 = 0.0;
pub const LEVELS_GRAY_MAXIMUM: f32 = 100.0;
pub const LEVELS_DEFAULT_GRAY: f32 = 50.0;
pub const LEVELS_WHITE_MINIMUM: f32 = 0.0;
pub const LEVELS_WHITE_MAXIMUM: f32 = 100.0;
pub const LEVELS_WHITE_DEFAULT: f32 = 100.0;
/// Native `init()` defaults for the three manual Lab levels.
pub const LEVELS_DEFAULT_LEVELS: [f32; 3] = [0.0, 0.5, 1.0];

/// Native mode enum. The retained C enum has the ABI of a four-byte `int`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum LevelsMode {
    Manual = 0,
    Automatic = 1,
}

impl LevelsMode {
    #[must_use]
    pub const fn raw(self) -> i32 {
        self as i32
    }

    pub const fn from_raw(value: i32) -> Result<Self, LevelsCodecError> {
        match value {
            0 => Ok(Self::Manual),
            1 => Ok(Self::Automatic),
            _ => Err(LevelsCodecError::InvalidMode(value)),
        }
    }
}

/// Exact current native parameter declaration order.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LevelsParametersV2 {
    pub mode: LevelsMode,
    pub black: f32,
    pub gray: f32,
    pub white: f32,
    pub levels: [f32; 3],
}

impl LevelsParametersV2 {
    #[must_use]
    pub const fn defaults() -> Self {
        Self {
            mode: LevelsMode::Manual,
            black: LEVELS_BLACK_DEFAULT,
            gray: LEVELS_DEFAULT_GRAY,
            white: LEVELS_WHITE_DEFAULT,
            levels: LEVELS_DEFAULT_LEVELS,
        }
    }

    #[must_use]
    pub const fn new(
        mode: LevelsMode,
        black: f32,
        gray: f32,
        white: f32,
        levels: [f32; 3],
    ) -> Self {
        Self {
            mode,
            black,
            gray,
            white,
            levels,
        }
    }

    /// Encodes the native v2 fields in declaration order.
    #[must_use]
    pub fn to_bytes(self) -> [u8; LEVELS_PARAMETER_BYTES_V2] {
        let mut bytes = [0_u8; LEVELS_PARAMETER_BYTES_V2];
        bytes[0..4].copy_from_slice(&self.mode.raw().to_le_bytes());
        for (index, value) in [self.black, self.gray, self.white]
            .into_iter()
            .chain(self.levels)
            .enumerate()
        {
            let start = 4 + index * 4;
            bytes[start..start + 4].copy_from_slice(&value.to_le_bytes());
        }
        bytes
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, LevelsCodecError> {
        if bytes.len() != LEVELS_PARAMETER_BYTES_V2 {
            return Err(LevelsCodecError::InvalidLength {
                expected: LEVELS_PARAMETER_BYTES_V2,
                actual: bytes.len(),
            });
        }
        let mode = LevelsMode::from_raw(i32::from_le_bytes(
            bytes[0..4].try_into().expect("validated mode range"),
        ))?;
        let read = |start| {
            f32::from_le_bytes(
                bytes[start..start + 4]
                    .try_into()
                    .expect("validated parameter range"),
            )
        };
        Ok(Self::new(
            mode,
            read(4),
            read(8),
            read(12),
            [read(16), read(20), read(24)],
        ))
    }
}

/// Retained v1 payload accepted by `legacy_params`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LevelsParametersV1 {
    pub levels: [f32; 3],
    pub levels_preset: i32,
}

impl LevelsParametersV1 {
    #[must_use]
    pub const fn new(levels: [f32; 3], levels_preset: i32) -> Self {
        Self {
            levels,
            levels_preset,
        }
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, LevelsCodecError> {
        if bytes.len() != LEVELS_PARAMETER_BYTES_V1 {
            return Err(LevelsCodecError::InvalidLength {
                expected: LEVELS_PARAMETER_BYTES_V1,
                actual: bytes.len(),
            });
        }
        let read = |start| {
            f32::from_le_bytes(
                bytes[start..start + 4]
                    .try_into()
                    .expect("validated v1 parameter range"),
            )
        };
        Ok(Self::new(
            [read(0), read(4), read(8)],
            i32::from_le_bytes(bytes[12..16].try_into().expect("validated preset range")),
        ))
    }

    #[must_use]
    pub fn to_bytes(self) -> [u8; LEVELS_PARAMETER_BYTES_V1] {
        let mut bytes = [0_u8; LEVELS_PARAMETER_BYTES_V1];
        for (index, value) in self.levels.into_iter().enumerate() {
            let start = index * 4;
            bytes[start..start + 4].copy_from_slice(&value.to_le_bytes());
        }
        bytes[12..16].copy_from_slice(&self.levels_preset.to_le_bytes());
        bytes
    }
}

/// Direct native `legacy_params(old_version == 1)` conversion.
#[must_use]
pub const fn migrate_v1_to_v2(old: LevelsParametersV1) -> LevelsParametersV2 {
    LevelsParametersV2::new(
        LevelsMode::Manual,
        LEVELS_BLACK_DEFAULT,
        LEVELS_DEFAULT_GRAY,
        LEVELS_WHITE_DEFAULT,
        old.levels,
    )
}

/// Checked typed parameters used to prepare an execution plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LevelsConfig {
    mode: LevelsMode,
    black: FiniteF32,
    gray: FiniteF32,
    white: FiniteF32,
    levels: [FiniteF32; 3],
}

impl LevelsConfig {
    pub fn new(parameters: LevelsParametersV2) -> Result<Self, LevelsParameterError> {
        Self::try_from(parameters)
    }

    #[must_use]
    pub fn defaults() -> Self {
        Self::try_from(LevelsParametersV2::defaults()).expect("native levels defaults are finite")
    }

    #[must_use]
    pub const fn mode(self) -> LevelsMode {
        self.mode
    }

    #[must_use]
    pub const fn black(self) -> f32 {
        self.black.get()
    }

    #[must_use]
    pub const fn gray(self) -> f32 {
        self.gray.get()
    }

    #[must_use]
    pub const fn white(self) -> f32 {
        self.white.get()
    }

    #[must_use]
    pub const fn levels(self) -> [f32; 3] {
        [
            self.levels[0].get(),
            self.levels[1].get(),
            self.levels[2].get(),
        ]
    }

    #[must_use]
    pub const fn parameters(self) -> LevelsParametersV2 {
        LevelsParametersV2::new(
            self.mode,
            self.black.get(),
            self.gray.get(),
            self.white.get(),
            self.levels(),
        )
    }
}

impl TryFrom<LevelsParametersV2> for LevelsConfig {
    type Error = LevelsParameterError;

    fn try_from(parameters: LevelsParametersV2) -> Result<Self, Self::Error> {
        Ok(Self {
            mode: parameters.mode,
            black: finite("black", parameters.black)?,
            gray: finite("gray", parameters.gray)?,
            white: finite("white", parameters.white)?,
            levels: parameters
                .levels
                .map(|value| finite("level", value))
                .into_iter()
                .collect::<Result<Vec<_>, _>>()?
                .try_into()
                .expect("three native levels were collected"),
        })
    }
}

fn finite(name: &'static str, value: f32) -> Result<FiniteF32, LevelsParameterError> {
    FiniteF32::new(value).map_err(|_| LevelsParameterError::NonFinite(name))
}

/// Typed history with byte-preserving retention for unknown future versions.
#[derive(Debug, Clone, PartialEq)]
pub enum LevelsHistory {
    V2(LevelsParametersV2),
    Opaque { version: u16, bytes: Vec<u8> },
}

impl LevelsHistory {
    pub fn decode(version: u16, bytes: &[u8]) -> Result<Self, LevelsCodecError> {
        match version {
            1 => Ok(Self::V2(migrate_v1_to_v2(LevelsParametersV1::from_bytes(
                bytes,
            )?))),
            LEVELS_SCHEMA_VERSION => Ok(Self::V2(LevelsParametersV2::from_bytes(bytes)?)),
            _ => Ok(Self::Opaque {
                version,
                bytes: bytes.to_vec(),
            }),
        }
    }

    /// Converts a known retained history version through the native migration.
    pub fn migrate(version: u16, bytes: &[u8]) -> Result<LevelsParametersV2, LevelsCodecError> {
        match version {
            1 => Ok(migrate_v1_to_v2(LevelsParametersV1::from_bytes(bytes)?)),
            LEVELS_SCHEMA_VERSION => LevelsParametersV2::from_bytes(bytes),
            other => Err(LevelsCodecError::UnsupportedVersion(other)),
        }
    }

    #[must_use]
    pub fn payload(&self) -> Vec<u8> {
        match self {
            Self::V2(parameters) => parameters.to_bytes().to_vec(),
            Self::Opaque { bytes, .. } => bytes.clone(),
        }
    }

    #[must_use]
    pub const fn version(&self) -> u16 {
        match self {
            Self::V2(_) => LEVELS_SCHEMA_VERSION,
            Self::Opaque { version, .. } => *version,
        }
    }

    pub fn current(&self) -> Result<LevelsParametersV2, LevelsCodecError> {
        match self {
            Self::V2(parameters) => Ok(*parameters),
            Self::Opaque { version, .. } => Err(LevelsCodecError::UnsupportedVersion(*version)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LevelsCodecError {
    InvalidLength { expected: usize, actual: usize },
    InvalidMode(i32),
    UnsupportedVersion(u16),
}

impl fmt::Display for LevelsCodecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLength { expected, actual } => {
                write!(
                    formatter,
                    "levels payload has {actual} bytes; expected {expected}"
                )
            }
            Self::InvalidMode(mode) => write!(formatter, "levels mode {mode} is invalid"),
            Self::UnsupportedVersion(version) => {
                write!(
                    formatter,
                    "levels version {version} is opaque and unsupported"
                )
            }
        }
    }
}

impl std::error::Error for LevelsCodecError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LevelsParameterError {
    NonFinite(&'static str),
}

impl fmt::Display for LevelsParameterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFinite(name) => write!(formatter, "levels {name} is non-finite"),
        }
    }
}

impl std::error::Error for LevelsParameterError {}
