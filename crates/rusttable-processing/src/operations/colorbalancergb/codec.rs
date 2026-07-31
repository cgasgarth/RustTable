//! Native Color Balance RGB parameter ABI and history migration.
//!
//! Direct source lineage: `src/iop/colorbalancergb.c`, `legacy_params`, and
//! `dt_iop_colorbalancergb_params_t`.  The pinned native file uses a blind
//! declaration-order copy for every migration and appends the v5 enum after
//! 32 floats.  Rust therefore encodes the payload explicitly instead of
//! relying on host layout or enum representation.

use std::fmt;
use std::mem::size_of;

pub const COLORBALANCERGB_INTROSPECTION_VERSION: u16 = 5;
pub const COLORBALANCERGB_V1_PARAMETER_BYTES: usize = 24 * size_of::<f32>();
pub const COLORBALANCERGB_V2_PARAMETER_BYTES: usize = 28 * size_of::<f32>();
pub const COLORBALANCERGB_V3_PARAMETER_BYTES: usize = 29 * size_of::<f32>();
pub const COLORBALANCERGB_V4_PARAMETER_BYTES: usize = 32 * size_of::<f32>();
pub const COLORBALANCERGB_V5_PARAMETER_BYTES: usize = 32 * size_of::<f32>() + size_of::<i32>();

pub const COLORBALANCERGB_FIELD_COUNT_V1: usize = 24;
pub const COLORBALANCERGB_FIELD_COUNT_V2: usize = 28;
pub const COLORBALANCERGB_FIELD_COUNT_V3: usize = 29;
pub const COLORBALANCERGB_FIELD_COUNT_V4: usize = 32;

pub const RGB_RED: usize = 0;
pub const RGB_GREEN: usize = 1;
pub const RGB_BLUE: usize = 2;
pub const RGB_ALPHA: usize = 3;
pub const RGB_CHANNELS: usize = 4;

/// Layout witness for native v5: 32 contiguous floats followed by the
/// signed C enum's 4-byte representation.  Payload methods still use
/// explicit little-endian bytes and never transmute this witness.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct ColorBalanceRgbV5Abi {
    pub floats: [f32; 32],
    pub saturation_formula: i32,
}

const _: () = assert!(size_of::<ColorBalanceRgbV5Abi>() == COLORBALANCERGB_V5_PARAMETER_BYTES);

/// Native `dt_iop_colorbalancrgb_saturation_t` values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum ColorBalanceRgbSaturationFormula {
    JzAzBz = 0,
    DarktableUcs2022 = 1,
}

impl TryFrom<i32> for ColorBalanceRgbSaturationFormula {
    type Error = ColorBalanceRgbCodecError;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::JzAzBz),
            1 => Ok(Self::DarktableUcs2022),
            value => Err(Self::Error::UnknownSaturationFormula(value)),
        }
    }
}

impl From<ColorBalanceRgbSaturationFormula> for i32 {
    fn from(value: ColorBalanceRgbSaturationFormula) -> Self {
        value as Self
    }
}

/// Native v1 declaration order, 24 floats.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorBalanceRgbParametersV1 {
    pub shadows_y: f32,
    pub shadows_c: f32,
    pub shadows_h: f32,
    pub midtones_y: f32,
    pub midtones_c: f32,
    pub midtones_h: f32,
    pub highlights_y: f32,
    pub highlights_c: f32,
    pub highlights_h: f32,
    pub global_y: f32,
    pub global_c: f32,
    pub global_h: f32,
    pub shadows_weight: f32,
    pub white_fulcrum: f32,
    pub highlights_weight: f32,
    pub chroma_shadows: f32,
    pub chroma_highlights: f32,
    pub chroma_global: f32,
    pub chroma_midtones: f32,
    pub saturation_global: f32,
    pub saturation_highlights: f32,
    pub saturation_midtones: f32,
    pub saturation_shadows: f32,
    pub hue_angle: f32,
}

impl ColorBalanceRgbParametersV1 {
    #[must_use]
    #[allow(clippy::too_many_arguments, reason = "preserves native v1 field order")]
    pub const fn new(values: [f32; COLORBALANCERGB_FIELD_COUNT_V1]) -> Self {
        Self {
            shadows_y: values[0],
            shadows_c: values[1],
            shadows_h: values[2],
            midtones_y: values[3],
            midtones_c: values[4],
            midtones_h: values[5],
            highlights_y: values[6],
            highlights_c: values[7],
            highlights_h: values[8],
            global_y: values[9],
            global_c: values[10],
            global_h: values[11],
            shadows_weight: values[12],
            white_fulcrum: values[13],
            highlights_weight: values[14],
            chroma_shadows: values[15],
            chroma_highlights: values[16],
            chroma_global: values[17],
            chroma_midtones: values[18],
            saturation_global: values[19],
            saturation_highlights: values[20],
            saturation_midtones: values[21],
            saturation_shadows: values[22],
            hue_angle: values[23],
        }
    }

    #[must_use]
    pub const fn defaults() -> Self {
        Self::new([
            0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0,
            0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
        ])
    }

    #[must_use]
    pub fn to_bytes(self) -> [u8; COLORBALANCERGB_V1_PARAMETER_BYTES] {
        encode_f32s(self.values())
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ColorBalanceRgbCodecError> {
        Ok(Self::new(decode_f32s::<24>(
            bytes,
            COLORBALANCERGB_V1_PARAMETER_BYTES,
        )?))
    }

    #[must_use]
    pub const fn values(self) -> [f32; COLORBALANCERGB_FIELD_COUNT_V1] {
        [
            self.shadows_y,
            self.shadows_c,
            self.shadows_h,
            self.midtones_y,
            self.midtones_c,
            self.midtones_h,
            self.highlights_y,
            self.highlights_c,
            self.highlights_h,
            self.global_y,
            self.global_c,
            self.global_h,
            self.shadows_weight,
            self.white_fulcrum,
            self.highlights_weight,
            self.chroma_shadows,
            self.chroma_highlights,
            self.chroma_global,
            self.chroma_midtones,
            self.saturation_global,
            self.saturation_highlights,
            self.saturation_midtones,
            self.saturation_shadows,
            self.hue_angle,
        ]
    }
}

/// Native v2 declaration order: v1 plus brilliance global/highlights/midtones/shadows.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorBalanceRgbParametersV2 {
    pub v1: ColorBalanceRgbParametersV1,
    pub brilliance_global: f32,
    pub brilliance_highlights: f32,
    pub brilliance_midtones: f32,
    pub brilliance_shadows: f32,
}

impl ColorBalanceRgbParametersV2 {
    #[must_use]
    pub const fn new(v1: ColorBalanceRgbParametersV1, brilliance: [f32; 4]) -> Self {
        Self {
            v1,
            brilliance_global: brilliance[0],
            brilliance_highlights: brilliance[1],
            brilliance_midtones: brilliance[2],
            brilliance_shadows: brilliance[3],
        }
    }

    #[must_use]
    pub const fn defaults() -> Self {
        Self::new(ColorBalanceRgbParametersV1::defaults(), [0.0; 4])
    }

    #[must_use]
    pub fn to_bytes(self) -> [u8; COLORBALANCERGB_V2_PARAMETER_BYTES] {
        encode_f32s(self.values())
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ColorBalanceRgbCodecError> {
        Ok(Self::from_values(decode_f32s::<28>(
            bytes,
            COLORBALANCERGB_V2_PARAMETER_BYTES,
        )?))
    }

    #[must_use]
    pub const fn values(self) -> [f32; COLORBALANCERGB_FIELD_COUNT_V2] {
        let v1 = self.v1.values();
        [
            v1[0],
            v1[1],
            v1[2],
            v1[3],
            v1[4],
            v1[5],
            v1[6],
            v1[7],
            v1[8],
            v1[9],
            v1[10],
            v1[11],
            v1[12],
            v1[13],
            v1[14],
            v1[15],
            v1[16],
            v1[17],
            v1[18],
            v1[19],
            v1[20],
            v1[21],
            v1[22],
            v1[23],
            self.brilliance_global,
            self.brilliance_highlights,
            self.brilliance_midtones,
            self.brilliance_shadows,
        ]
    }

    fn from_values(values: [f32; 28]) -> Self {
        Self::new(
            ColorBalanceRgbParametersV1::new(values[..24].try_into().expect("fixed v2 prefix")),
            [values[24], values[25], values[26], values[27]],
        )
    }
}

/// Native v3 declaration order: v2 plus `mask_grey_fulcrum`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorBalanceRgbParametersV3 {
    pub v2: ColorBalanceRgbParametersV2,
    pub mask_grey_fulcrum: f32,
}

impl ColorBalanceRgbParametersV3 {
    #[must_use]
    pub const fn new(v2: ColorBalanceRgbParametersV2, mask_grey_fulcrum: f32) -> Self {
        Self {
            v2,
            mask_grey_fulcrum,
        }
    }

    #[must_use]
    pub const fn defaults() -> Self {
        Self::new(ColorBalanceRgbParametersV2::defaults(), 0.1845)
    }

    #[must_use]
    pub fn to_bytes(self) -> [u8; COLORBALANCERGB_V3_PARAMETER_BYTES] {
        encode_f32s(self.values())
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ColorBalanceRgbCodecError> {
        Ok(Self::from_values(decode_f32s::<29>(
            bytes,
            COLORBALANCERGB_V3_PARAMETER_BYTES,
        )?))
    }

    #[must_use]
    pub const fn values(self) -> [f32; COLORBALANCERGB_FIELD_COUNT_V3] {
        let v2 = self.v2.values();
        [
            v2[0],
            v2[1],
            v2[2],
            v2[3],
            v2[4],
            v2[5],
            v2[6],
            v2[7],
            v2[8],
            v2[9],
            v2[10],
            v2[11],
            v2[12],
            v2[13],
            v2[14],
            v2[15],
            v2[16],
            v2[17],
            v2[18],
            v2[19],
            v2[20],
            v2[21],
            v2[22],
            v2[23],
            v2[24],
            v2[25],
            v2[26],
            v2[27],
            self.mask_grey_fulcrum,
        ]
    }

    fn from_values(values: [f32; 29]) -> Self {
        Self::new(
            ColorBalanceRgbParametersV2::from_values(
                values[..28].try_into().expect("fixed v3 prefix"),
            ),
            values[28],
        )
    }
}

/// Native v4 declaration order: v3 plus vibrance, grey fulcrum, contrast.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorBalanceRgbParametersV4 {
    pub v3: ColorBalanceRgbParametersV3,
    pub vibrance: f32,
    pub grey_fulcrum: f32,
    pub contrast: f32,
}

impl ColorBalanceRgbParametersV4 {
    #[must_use]
    pub const fn new(
        v3: ColorBalanceRgbParametersV3,
        vibrance: f32,
        grey_fulcrum: f32,
        contrast: f32,
    ) -> Self {
        Self {
            v3,
            vibrance,
            grey_fulcrum,
            contrast,
        }
    }

    #[must_use]
    pub const fn defaults() -> Self {
        Self::new(ColorBalanceRgbParametersV3::defaults(), 0.0, 0.1845, 0.0)
    }

    #[must_use]
    pub fn to_bytes(self) -> [u8; COLORBALANCERGB_V4_PARAMETER_BYTES] {
        encode_f32s(self.values())
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ColorBalanceRgbCodecError> {
        Ok(Self::from_values(decode_f32s::<32>(
            bytes,
            COLORBALANCERGB_V4_PARAMETER_BYTES,
        )?))
    }

    #[must_use]
    pub const fn values(self) -> [f32; COLORBALANCERGB_FIELD_COUNT_V4] {
        let v3 = self.v3.values();
        [
            v3[0],
            v3[1],
            v3[2],
            v3[3],
            v3[4],
            v3[5],
            v3[6],
            v3[7],
            v3[8],
            v3[9],
            v3[10],
            v3[11],
            v3[12],
            v3[13],
            v3[14],
            v3[15],
            v3[16],
            v3[17],
            v3[18],
            v3[19],
            v3[20],
            v3[21],
            v3[22],
            v3[23],
            v3[24],
            v3[25],
            v3[26],
            v3[27],
            v3[28],
            self.vibrance,
            self.grey_fulcrum,
            self.contrast,
        ]
    }

    fn from_values(values: [f32; 32]) -> Self {
        Self::new(
            ColorBalanceRgbParametersV3::from_values(
                values[..29].try_into().expect("fixed v4 prefix"),
            ),
            values[29],
            values[30],
            values[31],
        )
    }
}

/// Native current v5 declaration order: v4 plus the i32 saturation formula.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorBalanceRgbParametersV5 {
    pub v4: ColorBalanceRgbParametersV4,
    pub saturation_formula: ColorBalanceRgbSaturationFormula,
}

impl ColorBalanceRgbParametersV5 {
    #[must_use]
    pub const fn new(
        v4: ColorBalanceRgbParametersV4,
        saturation_formula: ColorBalanceRgbSaturationFormula,
    ) -> Self {
        Self {
            v4,
            saturation_formula,
        }
    }

    #[must_use]
    pub const fn defaults() -> Self {
        // Native metadata declares enum=1 as the module default.  The named
        // JzAzBz preset in `init_presets` is distinct from this default.
        Self::new(
            ColorBalanceRgbParametersV4::new(
                ColorBalanceRgbParametersV3::new(
                    ColorBalanceRgbParametersV2::new(
                        ColorBalanceRgbParametersV1::defaults(),
                        [0.0; 4],
                    ),
                    0.1845,
                ),
                0.0,
                0.1845,
                0.0,
            ),
            ColorBalanceRgbSaturationFormula::DarktableUcs2022,
        )
    }

    /// Exact `legacy_params` migration seed, including its enum=1 and zero
    /// grey fulcrum/contrast fields before each old-version branch overwrites
    /// the fields it owns.
    #[must_use]
    pub const fn legacy_default_v5() -> Self {
        Self::new(
            ColorBalanceRgbParametersV4::new(
                ColorBalanceRgbParametersV3::new(
                    ColorBalanceRgbParametersV2::new(
                        ColorBalanceRgbParametersV1::defaults(),
                        [0.0; 4],
                    ),
                    0.1845,
                ),
                0.0,
                0.0,
                0.0,
            ),
            ColorBalanceRgbSaturationFormula::DarktableUcs2022,
        )
    }

    #[must_use]
    pub fn to_bytes(self) -> [u8; COLORBALANCERGB_V5_PARAMETER_BYTES] {
        let mut bytes = [0_u8; COLORBALANCERGB_V5_PARAMETER_BYTES];
        encode_f32s_into(&mut bytes, 0, self.v4.values());
        bytes[COLORBALANCERGB_V4_PARAMETER_BYTES..]
            .copy_from_slice(&i32::from(self.saturation_formula).to_le_bytes());
        bytes
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ColorBalanceRgbCodecError> {
        if bytes.len() != COLORBALANCERGB_V5_PARAMETER_BYTES {
            return Err(ColorBalanceRgbCodecError::InvalidLength {
                expected: COLORBALANCERGB_V5_PARAMETER_BYTES,
                actual: bytes.len(),
            });
        }
        let v4 =
            ColorBalanceRgbParametersV4::from_bytes(&bytes[..COLORBALANCERGB_V4_PARAMETER_BYTES])?;
        let saturation_formula = i32::from_le_bytes(
            bytes[COLORBALANCERGB_V4_PARAMETER_BYTES..]
                .try_into()
                .expect("checked v5 enum width"),
        )
        .try_into()?;
        Ok(Self::new(v4, saturation_formula))
    }
}

fn encode_f32s<const COUNT: usize, const BYTES: usize>(values: [f32; COUNT]) -> [u8; BYTES] {
    let mut bytes = [0_u8; BYTES];
    encode_f32s_into(&mut bytes, 0, values);
    bytes
}

fn encode_f32s_into<const BYTES: usize, const COUNT: usize>(
    bytes: &mut [u8; BYTES],
    start: usize,
    values: [f32; COUNT],
) {
    for (index, value) in values.into_iter().enumerate() {
        let offset = start + index * size_of::<f32>();
        bytes[offset..offset + size_of::<f32>()].copy_from_slice(&value.to_le_bytes());
    }
}

fn decode_f32s<const COUNT: usize>(
    bytes: &[u8],
    expected: usize,
) -> Result<[f32; COUNT], ColorBalanceRgbCodecError> {
    if bytes.len() != expected {
        return Err(ColorBalanceRgbCodecError::InvalidLength {
            expected,
            actual: bytes.len(),
        });
    }
    Ok(std::array::from_fn(|index| {
        let offset = index * size_of::<f32>();
        f32::from_le_bytes(
            bytes[offset..offset + size_of::<f32>()]
                .try_into()
                .expect("checked float width"),
        )
    }))
}

/// Typed known history and byte-preserved future values.
#[derive(Debug, Clone, PartialEq)]
pub enum ColorBalanceRgbHistory {
    V1(ColorBalanceRgbParametersV1),
    V2(ColorBalanceRgbParametersV2),
    V3(ColorBalanceRgbParametersV3),
    V4(ColorBalanceRgbParametersV4),
    V5(ColorBalanceRgbParametersV5),
    Opaque { version: u16, bytes: Vec<u8> },
}

impl ColorBalanceRgbHistory {
    pub fn decode(version: u16, bytes: &[u8]) -> Result<Self, ColorBalanceRgbCodecError> {
        match version {
            1 => Ok(Self::V1(ColorBalanceRgbParametersV1::from_bytes(bytes)?)),
            2 => Ok(Self::V2(ColorBalanceRgbParametersV2::from_bytes(bytes)?)),
            3 => Ok(Self::V3(ColorBalanceRgbParametersV3::from_bytes(bytes)?)),
            4 => Ok(Self::V4(ColorBalanceRgbParametersV4::from_bytes(bytes)?)),
            COLORBALANCERGB_INTROSPECTION_VERSION => {
                Ok(Self::V5(ColorBalanceRgbParametersV5::from_bytes(bytes)?))
            }
            _ => Ok(Self::Opaque {
                version,
                bytes: bytes.to_vec(),
            }),
        }
    }

    #[must_use]
    pub const fn version(&self) -> u16 {
        match self {
            Self::V1(_) => 1,
            Self::V2(_) => 2,
            Self::V3(_) => 3,
            Self::V4(_) => 4,
            Self::V5(_) => COLORBALANCERGB_INTROSPECTION_VERSION,
            Self::Opaque { version, .. } => *version,
        }
    }

    #[must_use]
    pub fn payload(&self) -> Vec<u8> {
        match self {
            Self::V1(value) => value.to_bytes().to_vec(),
            Self::V2(value) => value.to_bytes().to_vec(),
            Self::V3(value) => value.to_bytes().to_vec(),
            Self::V4(value) => value.to_bytes().to_vec(),
            Self::V5(value) => value.to_bytes().to_vec(),
            Self::Opaque { bytes, .. } => bytes.clone(),
        }
    }

    pub fn current(&self) -> Result<ColorBalanceRgbParametersV5, ColorBalanceRgbCodecError> {
        match self {
            Self::V1(value) => Ok(migrate_v1_to_v5(*value)),
            Self::V2(value) => Ok(migrate_v2_to_v5(*value)),
            Self::V3(value) => Ok(migrate_v3_to_v5(*value)),
            Self::V4(value) => Ok(migrate_v4_to_v5(*value)),
            Self::V5(value) => Ok(*value),
            Self::Opaque { version, .. } => {
                Err(ColorBalanceRgbCodecError::UnsupportedVersion(*version))
            }
        }
    }
}

#[must_use]
pub const fn migrate_v1_to_v5(value: ColorBalanceRgbParametersV1) -> ColorBalanceRgbParametersV5 {
    let mut v1 = value;
    v1.saturation_global /= 100.0;
    migrate_common(
        v1,
        [0.0; 4],
        0.1845,
        0.0,
        0.1845,
        0.0,
        ColorBalanceRgbSaturationFormula::JzAzBz,
    )
}

#[must_use]
pub const fn migrate_v2_to_v5(value: ColorBalanceRgbParametersV2) -> ColorBalanceRgbParametersV5 {
    migrate_common(
        value.v1,
        [
            value.brilliance_global,
            value.brilliance_highlights,
            value.brilliance_midtones,
            value.brilliance_shadows,
        ],
        0.1845,
        0.0,
        0.1845,
        0.0,
        ColorBalanceRgbSaturationFormula::JzAzBz,
    )
}

#[must_use]
pub const fn migrate_v3_to_v5(value: ColorBalanceRgbParametersV3) -> ColorBalanceRgbParametersV5 {
    migrate_common(
        value.v2.v1,
        [
            value.v2.brilliance_global,
            value.v2.brilliance_highlights,
            value.v2.brilliance_midtones,
            value.v2.brilliance_shadows,
        ],
        value.mask_grey_fulcrum,
        0.0,
        0.1845,
        0.0,
        ColorBalanceRgbSaturationFormula::JzAzBz,
    )
}

#[must_use]
pub const fn migrate_v4_to_v5(value: ColorBalanceRgbParametersV4) -> ColorBalanceRgbParametersV5 {
    migrate_common(
        value.v3.v2.v1,
        [
            value.v3.v2.brilliance_global,
            value.v3.v2.brilliance_highlights,
            value.v3.v2.brilliance_midtones,
            value.v3.v2.brilliance_shadows,
        ],
        value.v3.mask_grey_fulcrum,
        value.vibrance,
        value.grey_fulcrum,
        value.contrast,
        ColorBalanceRgbSaturationFormula::JzAzBz,
    )
}

const fn migrate_common(
    v1: ColorBalanceRgbParametersV1,
    brilliance: [f32; 4],
    mask_grey_fulcrum: f32,
    vibrance: f32,
    grey_fulcrum: f32,
    contrast: f32,
    saturation_formula: ColorBalanceRgbSaturationFormula,
) -> ColorBalanceRgbParametersV5 {
    ColorBalanceRgbParametersV5::new(
        ColorBalanceRgbParametersV4::new(
            ColorBalanceRgbParametersV3::new(
                ColorBalanceRgbParametersV2::new(v1, brilliance),
                mask_grey_fulcrum,
            ),
            vibrance,
            grey_fulcrum,
            contrast,
        ),
        saturation_formula,
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorBalanceRgbCodecError {
    InvalidLength { expected: usize, actual: usize },
    UnknownSaturationFormula(i32),
    UnsupportedVersion(u16),
}

impl fmt::Display for ColorBalanceRgbCodecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLength { expected, actual } => {
                write!(
                    formatter,
                    "colorbalancergb payload has {actual} bytes; expected {expected}"
                )
            }
            Self::UnknownSaturationFormula(value) => {
                write!(
                    formatter,
                    "colorbalancergb saturation formula {value} is unknown"
                )
            }
            Self::UnsupportedVersion(version) => {
                write!(
                    formatter,
                    "colorbalancergb version {version} is opaque and unsupported"
                )
            }
        }
    }
}

impl std::error::Error for ColorBalanceRgbCodecError {}
