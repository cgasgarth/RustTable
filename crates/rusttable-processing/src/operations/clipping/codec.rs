#![allow(clippy::ignored_unit_patterns, clippy::missing_errors_doc)]

use std::fmt;
use std::hash::{Hash, Hasher};

pub const CLIPPING_COMPATIBILITY_ID: &str = "clipping";
pub const CLIPPING_RUST_ID: &str = "rusttable.clipping";
pub const CLIPPING_SCHEMA_VERSION: u16 = 5;
pub const CLIPPING_PARAMETER_VERSION: u16 = 5;
pub const CLIPPING_IMPLEMENTATION_VERSION: u16 = 1;
pub const CLIPPING_MAX_DIMENSION: u32 = 1 << 30;

const V2_BYTES: usize = 28;
const V3_BYTES: usize = 28;
const V4_BYTES: usize = 76;
const V5_BYTES: usize = 84;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClippingParametersV2 {
    pub angle: f32,
    pub cx: f32,
    pub cy: f32,
    pub cw: f32,
    pub ch: f32,
    pub k_h: f32,
    pub k_v: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClippingParametersV3 {
    pub angle: f32,
    pub cx: f32,
    pub cy: f32,
    pub cw: f32,
    pub ch: f32,
    pub k_h: f32,
    pub k_v: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClippingParametersV4 {
    pub angle: f32,
    pub cx: f32,
    pub cy: f32,
    pub cw: f32,
    pub ch: f32,
    pub k_h: f32,
    pub k_v: f32,
    pub kxa: f32,
    pub kya: f32,
    pub kxb: f32,
    pub kyb: f32,
    pub kxc: f32,
    pub kyc: f32,
    pub kxd: f32,
    pub kyd: f32,
    pub k_type: i32,
    pub k_sym: i32,
    pub k_apply: i32,
    pub crop_auto: i32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClippingParametersV5 {
    pub angle: f32,
    pub cx: f32,
    pub cy: f32,
    pub cw: f32,
    pub ch: f32,
    pub k_h: f32,
    pub k_v: f32,
    pub kxa: f32,
    pub kya: f32,
    pub kxb: f32,
    pub kyb: f32,
    pub kxc: f32,
    pub kyc: f32,
    pub kxd: f32,
    pub kyd: f32,
    pub k_type: i32,
    pub k_sym: i32,
    pub k_apply: i32,
    pub crop_auto: bool,
    pub ratio_n: i32,
    pub ratio_d: i32,
}

impl Default for ClippingParametersV5 {
    fn default() -> Self {
        Self {
            angle: 0.0,
            cx: 0.0,
            cy: 0.0,
            cw: 1.0,
            ch: 1.0,
            k_h: 0.0,
            k_v: 0.0,
            kxa: 0.2,
            kya: 0.2,
            kxb: 0.8,
            kyb: 0.2,
            kxc: 0.8,
            kyc: 0.8,
            kxd: 0.2,
            kyd: 0.8,
            k_type: 0,
            k_sym: 0,
            k_apply: 0,
            crop_auto: true,
            ratio_n: -1,
            ratio_d: -1,
        }
    }
}

impl ClippingParametersV5 {
    #[must_use]
    pub fn to_bytes(self) -> [u8; V5_BYTES] {
        let mut bytes = [0_u8; V5_BYTES];
        for (index, value) in [
            self.angle, self.cx, self.cy, self.cw, self.ch, self.k_h, self.k_v, self.kxa, self.kya,
            self.kxb, self.kyb, self.kxc, self.kyc, self.kxd, self.kyd,
        ]
        .into_iter()
        .enumerate()
        {
            write_f32(&mut bytes[index * 4..index * 4 + 4], value);
        }
        write_i32(&mut bytes[60..64], self.k_type);
        write_i32(&mut bytes[64..68], self.k_sym);
        write_i32(&mut bytes[68..72], self.k_apply);
        bytes[72] = u8::from(self.crop_auto);
        write_i32(&mut bytes[76..80], self.ratio_n);
        write_i32(&mut bytes[80..84], self.ratio_d);
        bytes
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ClippingConfig {
    parameters: ClippingParametersV5,
    opaque_source: Option<Vec<u8>>,
}

impl ClippingConfig {
    pub fn new(parameters: ClippingParametersV5) -> Result<Self, ClippingParameterError> {
        if [
            parameters.angle,
            parameters.cx,
            parameters.cy,
            parameters.cw,
            parameters.ch,
            parameters.k_h,
            parameters.k_v,
            parameters.kxa,
            parameters.kya,
            parameters.kxb,
            parameters.kyb,
            parameters.kxc,
            parameters.kyc,
            parameters.kxd,
            parameters.kyd,
        ]
        .iter()
        .any(|value| !value.is_finite())
        {
            return Err(ClippingParameterError::NonFinite);
        }
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
    pub const fn parameters(&self) -> ClippingParametersV5 {
        self.parameters
    }

    #[must_use]
    pub fn opaque_source(&self) -> Option<&[u8]> {
        self.opaque_source.as_deref()
    }
}

impl Default for ClippingConfig {
    fn default() -> Self {
        Self::new(ClippingParametersV5::default()).expect("finite clipping defaults")
    }
}

impl Eq for ClippingConfig {}

impl Hash for ClippingConfig {
    fn hash<H: Hasher>(&self, state: &mut H) {
        for value in [
            self.parameters.angle,
            self.parameters.cx,
            self.parameters.cy,
            self.parameters.cw,
            self.parameters.ch,
            self.parameters.k_h,
            self.parameters.k_v,
            self.parameters.kxa,
            self.parameters.kya,
            self.parameters.kxb,
            self.parameters.kyb,
            self.parameters.kxc,
            self.parameters.kyc,
            self.parameters.kxd,
            self.parameters.kyd,
        ] {
            value.to_bits().hash(state);
        }
        self.parameters.k_type.hash(state);
        self.parameters.k_sym.hash(state);
        self.parameters.k_apply.hash(state);
        self.parameters.crop_auto.hash(state);
        self.parameters.ratio_n.hash(state);
        self.parameters.ratio_d.hash(state);
        self.opaque_source.hash(state);
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ClippingHistory {
    V2(ClippingParametersV2),
    V3(ClippingParametersV3),
    V4(ClippingParametersV4),
    V5(ClippingParametersV5),
    Opaque { version: u16, bytes: Vec<u8> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClippingCodecError {
    InvalidLength { expected: usize, actual: usize },
    NonFinite,
    InvalidMigration,
}

impl fmt::Display for ClippingCodecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLength { expected, actual } => {
                write!(
                    f,
                    "clipping payload has {actual} bytes; expected {expected}"
                )
            }
            Self::NonFinite => f.write_str("clipping payload contains a non-finite value"),
            Self::InvalidMigration => f.write_str("clipping history cannot be migrated"),
        }
    }
}
impl std::error::Error for ClippingCodecError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClippingParameterError {
    NonFinite,
}
impl fmt::Display for ClippingParameterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("clipping parameters must be finite")
    }
}
impl std::error::Error for ClippingParameterError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ClippingInterpolation {
    Nearest,
    Bilinear,
    Bicubic,
    Lanczos,
}

impl ClippingInterpolation {
    #[must_use]
    pub const fn all() -> [Self; 4] {
        [Self::Nearest, Self::Bilinear, Self::Bicubic, Self::Lanczos]
    }
    #[must_use]
    pub const fn support(self) -> u32 {
        match self {
            Self::Nearest => 0,
            Self::Bilinear => 1,
            Self::Bicubic => 2,
            Self::Lanczos => 3,
        }
    }
    #[must_use]
    pub const fn tag(self) -> u8 {
        match self {
            Self::Nearest => 0,
            Self::Bilinear => 1,
            Self::Bicubic => 2,
            Self::Lanczos => 3,
        }
    }
}

fn read_f32(bytes: &[u8]) -> f32 {
    f32::from_le_bytes(bytes.try_into().expect("checked f32 slice"))
}

fn read_i32(bytes: &[u8]) -> i32 {
    i32::from_le_bytes(bytes.try_into().expect("checked i32 slice"))
}

fn write_f32(bytes: &mut [u8], value: f32) {
    bytes.copy_from_slice(&value.to_le_bytes());
}

fn write_i32(bytes: &mut [u8], value: i32) {
    bytes.copy_from_slice(&value.to_le_bytes());
}

fn finite(values: &[f32]) -> Result<(), ClippingCodecError> {
    values
        .iter()
        .all(|value| value.is_finite())
        .then_some(())
        .ok_or(ClippingCodecError::NonFinite)
}

fn read_v2(bytes: &[u8]) -> Result<ClippingParametersV2, ClippingCodecError> {
    let values = (0..7).map(|index| read_f32(&bytes[index * 4..index * 4 + 4]));
    let values = values.collect::<Vec<_>>();
    finite(&values)?;
    let mut kh_bits = values[5].to_bits();
    let horizontal = kh_bits & 0x4000_0000 != 0;
    kh_bits &= !0x4000_0000;
    Ok(ClippingParametersV2 {
        angle: values[0],
        cx: values[1],
        cy: values[2],
        cw: values[3],
        ch: values[4],
        k_h: if horizontal {
            f32::from_bits(kh_bits)
        } else {
            0.0
        },
        k_v: if horizontal {
            0.0
        } else {
            f32::from_bits(kh_bits)
        },
    })
}

fn read_v3(bytes: &[u8]) -> Result<ClippingParametersV3, ClippingCodecError> {
    let v = read_v2(bytes)?;
    Ok(ClippingParametersV3 {
        angle: v.angle,
        cx: v.cx,
        cy: v.cy,
        cw: v.cw,
        ch: v.ch,
        k_h: v.k_h,
        k_v: v.k_v,
    })
}

fn read_v4(bytes: &[u8]) -> Result<ClippingParametersV4, ClippingCodecError> {
    let floats = (0..15)
        .map(|index| read_f32(&bytes[index * 4..index * 4 + 4]))
        .collect::<Vec<_>>();
    finite(&floats)?;
    let integer = |index: usize| read_i32(&bytes[index * 4..index * 4 + 4]);
    Ok(ClippingParametersV4 {
        angle: floats[0],
        cx: floats[1],
        cy: floats[2],
        cw: floats[3],
        ch: floats[4],
        k_h: floats[5],
        k_v: floats[6],
        kxa: floats[7],
        kya: floats[8],
        kxb: floats[9],
        kyb: floats[10],
        kxc: floats[11],
        kyc: floats[12],
        kxd: floats[13],
        kyd: floats[14],
        k_type: integer(60),
        k_sym: integer(64),
        k_apply: integer(68),
        crop_auto: integer(72),
    })
}

fn read_v5(bytes: &[u8]) -> Result<ClippingParametersV5, ClippingCodecError> {
    let v4 = read_v4(&bytes[..V4_BYTES])?;
    Ok(ClippingParametersV5 {
        angle: v4.angle,
        cx: v4.cx,
        cy: v4.cy,
        cw: v4.cw,
        ch: v4.ch,
        k_h: v4.k_h,
        k_v: v4.k_v,
        kxa: v4.kxa,
        kya: v4.kya,
        kxb: v4.kxb,
        kyb: v4.kyb,
        kxc: v4.kxc,
        kyc: v4.kyc,
        kxd: v4.kxd,
        kyd: v4.kyd,
        k_type: v4.k_type,
        k_sym: v4.k_sym,
        k_apply: v4.k_apply,
        crop_auto: bytes[72] != 0,
        ratio_n: read_i32(&bytes[76..80]),
        ratio_d: read_i32(&bytes[80..84]),
    })
}

pub fn decode_history(version: u16, bytes: &[u8]) -> Result<ClippingHistory, ClippingCodecError> {
    match version {
        2 => check_len(bytes, V2_BYTES).and_then(|_| read_v2(bytes).map(ClippingHistory::V2)),
        3 => check_len(bytes, V3_BYTES).and_then(|_| read_v3(bytes).map(ClippingHistory::V3)),
        4 => check_len(bytes, V4_BYTES).and_then(|_| read_v4(bytes).map(ClippingHistory::V4)),
        5 => check_len(bytes, V5_BYTES).and_then(|_| read_v5(bytes).map(ClippingHistory::V5)),
        _ => Ok(ClippingHistory::Opaque {
            version,
            bytes: bytes.to_vec(),
        }),
    }
}

fn check_len(bytes: &[u8], expected: usize) -> Result<(), ClippingCodecError> {
    (bytes.len() == expected)
        .then_some(())
        .ok_or(ClippingCodecError::InvalidLength {
            expected,
            actual: bytes.len(),
        })
}

pub fn migrate_history(history: &ClippingHistory) -> Result<ClippingConfig, ClippingCodecError> {
    let parameters = match history {
        ClippingHistory::V2(v) => ClippingParametersV5 {
            angle: v.angle,
            cx: v.cx,
            cy: v.cy,
            cw: v.cw,
            ch: v.ch,
            k_h: v.k_h,
            k_v: v.k_v,
            ..Default::default()
        },
        ClippingHistory::V3(v) => ClippingParametersV5 {
            angle: v.angle,
            cx: v.cx,
            cy: v.cy,
            cw: v.cw,
            ch: v.ch,
            k_h: v.k_h,
            k_v: v.k_v,
            ..Default::default()
        },
        ClippingHistory::V4(v) => ClippingParametersV5 {
            angle: v.angle,
            cx: v.cx,
            cy: v.cy,
            cw: v.cw,
            ch: v.ch,
            k_h: v.k_h,
            k_v: v.k_v,
            kxa: v.kxa,
            kya: v.kya,
            kxb: v.kxb,
            kyb: v.kyb,
            kxc: v.kxc,
            kyc: v.kyc,
            kxd: v.kxd,
            kyd: v.kyd,
            k_type: v.k_type,
            k_sym: v.k_sym,
            k_apply: v.k_apply,
            crop_auto: v.crop_auto != 0,
            ..Default::default()
        },
        ClippingHistory::V5(v) => *v,
        ClippingHistory::Opaque { .. } => return Err(ClippingCodecError::InvalidMigration),
    };
    ClippingConfig::new(parameters).map_err(|_| ClippingCodecError::NonFinite)
}
