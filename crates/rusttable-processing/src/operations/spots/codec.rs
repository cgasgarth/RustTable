use std::fmt;

pub const SPOTS_SCHEMA_VERSION: u16 = 2;
pub const SPOTS_PARAMETER_BYTES_V1: usize = 1_472;
pub const SPOTS_PARAMETER_BYTES_V2: usize = 512;
const V1_SPOT_COUNT: usize = 32;
const V1_BODY_BYTES: usize = 4 + V1_SPOT_COUNT * 20;
const V1_PADDING_BYTES: usize = SPOTS_PARAMETER_BYTES_V1 - V1_BODY_BYTES;
const V2_ENTRIES: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpotsLegacySpot {
    x: f32,
    y: f32,
    source_x: f32,
    source_y: f32,
    radius: f32,
}

impl SpotsLegacySpot {
    pub fn new(
        x: f32,
        y: f32,
        source_x: f32,
        source_y: f32,
        radius: f32,
    ) -> Result<Self, SpotsCodecError> {
        if [x, y, source_x, source_y, radius]
            .into_iter()
            .any(|value| !value.is_finite())
            || radius < 0.0
        {
            return Err(SpotsCodecError::NonFiniteOrNegativeLegacySpot);
        }
        Ok(Self {
            x,
            y,
            source_x,
            source_y,
            radius,
        })
    }

    #[must_use]
    pub const fn x(self) -> f32 {
        self.x
    }

    #[must_use]
    pub const fn y(self) -> f32 {
        self.y
    }

    #[must_use]
    pub const fn source_x(self) -> f32 {
        self.source_x
    }

    #[must_use]
    pub const fn source_y(self) -> f32 {
        self.source_y
    }

    #[must_use]
    pub const fn radius(self) -> f32 {
        self.radius
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SpotsParametersV1 {
    spots: Vec<SpotsLegacySpot>,
    padding: Vec<u8>,
}

impl SpotsParametersV1 {
    pub fn new(spots: Vec<SpotsLegacySpot>) -> Result<Self, SpotsCodecError> {
        if spots.len() > V1_SPOT_COUNT {
            return Err(SpotsCodecError::TooManyLegacySpots {
                actual: spots.len(),
            });
        }
        Ok(Self {
            spots,
            padding: vec![0; V1_PADDING_BYTES],
        })
    }

    #[must_use]
    pub fn defaults() -> Self {
        Self::new(Vec::new()).expect("empty legacy spots are valid")
    }

    #[must_use]
    pub fn spots(&self) -> &[SpotsLegacySpot] {
        &self.spots
    }

    #[must_use]
    pub fn padding(&self) -> &[u8] {
        &self.padding
    }

    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = vec![0; SPOTS_PARAMETER_BYTES_V1];
        let count = i32::try_from(self.spots.len()).expect("legacy spot count is bounded");
        bytes[0..4].copy_from_slice(&count.to_le_bytes());
        for (index, spot) in self.spots.iter().enumerate() {
            let start = 4 + index * 20;
            for (offset, value) in [
                spot.x(),
                spot.y(),
                spot.source_x(),
                spot.source_y(),
                spot.radius(),
            ]
            .into_iter()
            .enumerate()
            {
                bytes[start + offset * 4..start + offset * 4 + 4]
                    .copy_from_slice(&value.to_le_bytes());
            }
        }
        bytes[V1_BODY_BYTES..].copy_from_slice(&self.padding);
        bytes
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, SpotsCodecError> {
        if bytes.len() != SPOTS_PARAMETER_BYTES_V1 {
            return Err(SpotsCodecError::InvalidLength {
                expected: SPOTS_PARAMETER_BYTES_V1,
                actual: bytes.len(),
            });
        }
        let count = i32::from_le_bytes(bytes[0..4].try_into().expect("checked length"));
        let count = usize::try_from(count)
            .map_err(|_| SpotsCodecError::InvalidLegacySpotCount { count })?;
        if count > V1_SPOT_COUNT {
            return Err(SpotsCodecError::InvalidLegacySpotCount {
                count: i32::try_from(count).unwrap_or(i32::MAX),
            });
        }
        let mut spots = Vec::with_capacity(count);
        for index in 0..count {
            let start = 4 + index * 20;
            let value = |offset: usize| {
                f32::from_le_bytes(
                    bytes[start + offset..start + offset + 4]
                        .try_into()
                        .expect("spot field is four bytes inside the checked legacy payload"),
                )
            };
            spots.push(SpotsLegacySpot::new(
                value(0),
                value(4),
                value(8),
                value(12),
                value(16),
            )?);
        }
        Ok(Self {
            spots,
            padding: bytes[V1_BODY_BYTES..].to_vec(),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SpotsMode {
    None,
    Clone,
    Heal,
    Unknown(i32),
}

impl SpotsMode {
    const fn raw(self) -> i32 {
        match self {
            Self::None => 0,
            Self::Clone => 1,
            Self::Heal => 2,
            Self::Unknown(value) => value,
        }
    }

    const fn from_raw(value: i32) -> Self {
        match value {
            0 => Self::None,
            1 => Self::Clone,
            2 => Self::Heal,
            value => Self::Unknown(value),
        }
    }

    #[must_use]
    pub const fn is_supported(self) -> bool {
        matches!(self, Self::None | Self::Clone | Self::Heal)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SpotsParametersV2 {
    form_ids: [u32; V2_ENTRIES],
    modes: [SpotsMode; V2_ENTRIES],
}

impl SpotsParametersV2 {
    #[must_use]
    pub fn defaults() -> Self {
        Self {
            form_ids: [0; V2_ENTRIES],
            modes: [SpotsMode::None; V2_ENTRIES],
        }
    }

    pub fn from_entries(
        entries: impl IntoIterator<Item = (u32, SpotsMode)>,
    ) -> Result<Self, SpotsCodecError> {
        let mut value = Self {
            form_ids: [0; V2_ENTRIES],
            modes: [SpotsMode::None; V2_ENTRIES],
        };
        for (index, (form_id, mode)) in entries.into_iter().enumerate() {
            if index == V2_ENTRIES {
                return Err(SpotsCodecError::TooManyEntries);
            }
            if form_id == 0 {
                return Err(SpotsCodecError::InvalidFormId);
            }
            if !mode.is_supported() {
                return Err(SpotsCodecError::UnsupportedMode(mode));
            }
            value.form_ids[index] = form_id;
            value.modes[index] = mode;
        }
        Ok(value)
    }

    #[must_use]
    pub const fn form_ids(&self) -> &[u32; V2_ENTRIES] {
        &self.form_ids
    }

    #[must_use]
    pub const fn modes(&self) -> &[SpotsMode; V2_ENTRIES] {
        &self.modes
    }

    #[must_use]
    pub fn ordered_entries(&self) -> Vec<(u32, SpotsMode)> {
        self.form_ids
            .into_iter()
            .zip(self.modes)
            .filter(|(form_id, mode)| *form_id != 0 && *mode != SpotsMode::None)
            .collect()
    }

    #[must_use]
    pub fn to_bytes(&self) -> [u8; SPOTS_PARAMETER_BYTES_V2] {
        let mut bytes = [0; SPOTS_PARAMETER_BYTES_V2];
        for (index, form_id) in self.form_ids.into_iter().enumerate() {
            bytes[index * 4..index * 4 + 4].copy_from_slice(&form_id.to_le_bytes());
        }
        for (index, mode) in self.modes.into_iter().enumerate() {
            let start = 256 + index * 4;
            bytes[start..start + 4].copy_from_slice(&mode.raw().to_le_bytes());
        }
        bytes
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, SpotsCodecError> {
        if bytes.len() != SPOTS_PARAMETER_BYTES_V2 {
            return Err(SpotsCodecError::InvalidLength {
                expected: SPOTS_PARAMETER_BYTES_V2,
                actual: bytes.len(),
            });
        }
        let mut form_ids = [0; V2_ENTRIES];
        let mut modes = [SpotsMode::None; V2_ENTRIES];
        for index in 0..V2_ENTRIES {
            let start = index * 4;
            let raw = i32::from_le_bytes(bytes[start..start + 4].try_into().expect("checked"));
            form_ids[index] = u32::try_from(raw).map_err(|_| SpotsCodecError::InvalidFormId)?;
            let mode_start = 256 + index * 4;
            modes[index] = SpotsMode::from_raw(i32::from_le_bytes(
                bytes[mode_start..mode_start + 4]
                    .try_into()
                    .expect("checked"),
            ));
        }
        if form_ids
            .iter()
            .zip(modes)
            .any(|(form_id, mode)| *form_id != 0 && mode == SpotsMode::None)
        {
            return Err(SpotsCodecError::InconsistentEntry);
        }
        Ok(Self { form_ids, modes })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum SpotsHistory {
    V1(SpotsParametersV1),
    V2(Box<SpotsParametersV2>),
    Opaque { version: u16, bytes: Vec<u8> },
}

impl SpotsHistory {
    pub fn decode(version: u16, bytes: &[u8]) -> Result<Self, SpotsCodecError> {
        match version {
            1 => Ok(Self::V1(SpotsParametersV1::from_bytes(bytes)?)),
            SPOTS_SCHEMA_VERSION => Ok(Self::V2(Box::new(SpotsParametersV2::from_bytes(bytes)?))),
            version => Ok(Self::Opaque {
                version,
                bytes: bytes.to_vec(),
            }),
        }
    }

    #[must_use]
    pub fn payload(&self) -> Vec<u8> {
        match self {
            Self::V1(value) => value.to_bytes(),
            Self::V2(value) => value.to_bytes().to_vec(),
            Self::Opaque { bytes, .. } => bytes.clone(),
        }
    }

    #[must_use]
    pub const fn version(&self) -> u16 {
        match self {
            Self::V1(_) => 1,
            Self::V2(_) => SPOTS_SCHEMA_VERSION,
            Self::Opaque { version, .. } => *version,
        }
    }

    #[must_use]
    pub const fn executable(&self) -> bool {
        matches!(self, Self::V1(_) | Self::V2(_))
    }
}

pub fn migrate_v1_to_v2(value: &SpotsParametersV1) -> Result<SpotsParametersV2, SpotsCodecError> {
    SpotsParametersV2::from_entries(value.spots().iter().enumerate().map(|(index, _)| {
        (
            u32::try_from(index + 1).unwrap_or(u32::MAX),
            SpotsMode::Heal,
        )
    }))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpotsCodecError {
    InvalidLength { expected: usize, actual: usize },
    InvalidLegacySpotCount { count: i32 },
    TooManyLegacySpots { actual: usize },
    NonFiniteOrNegativeLegacySpot,
    TooManyEntries,
    InvalidFormId,
    InconsistentEntry,
    UnsupportedMode(SpotsMode),
}

impl fmt::Display for SpotsCodecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLength { expected, actual } => {
                write!(
                    formatter,
                    "spots payload has {actual} bytes; expected {expected}"
                )
            }
            Self::InvalidLegacySpotCount { count } => {
                write!(formatter, "spots v1 count {count} is outside 0..=32")
            }
            Self::TooManyLegacySpots { actual } => {
                write!(
                    formatter,
                    "spots v1 contains {actual} entries; maximum is 32"
                )
            }
            Self::NonFiniteOrNegativeLegacySpot => {
                formatter.write_str("spots v1 geometry must be finite and nonnegative in radius")
            }
            Self::TooManyEntries => formatter.write_str("spots v2 contains more than 64 entries"),
            Self::InvalidFormId => formatter.write_str("spots form IDs must be positive"),
            Self::InconsistentEntry => {
                formatter.write_str("spots v2 form IDs and modes must be paired")
            }
            Self::UnsupportedMode(mode) => write!(formatter, "spots mode {mode:?} is unsupported"),
        }
    }
}

impl std::error::Error for SpotsCodecError {}
