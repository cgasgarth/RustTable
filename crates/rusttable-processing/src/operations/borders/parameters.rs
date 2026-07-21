use crate::{FiniteF32, FiniteF32Error};
use std::fmt;

use super::{BORDERS_PARAMETER_BYTES_V1, BORDERS_PARAMETER_BYTES_V3, BORDERS_PARAMETER_BYTES_V4};

const ASPECT_IMAGE: f32 = 0.0;
const ASPECT_CONSTANT: f32 = -1.0;
const ASPECTS: [f32; 19] = [
    ASPECT_IMAGE,
    3.0,
    95.0 / 33.0,
    2.39,
    2.0,
    16.0 / 9.0,
    5.0 / 3.0,
    14.0 / 8.5,
    1.618_034,
    16.0 / 10.0,
    3.0 / 2.0,
    297.0 / 210.0,
    std::f32::consts::SQRT_2,
    7.0 / 5.0,
    4.0 / 3.0,
    11.0 / 8.5,
    14.0 / 11.0,
    5.0 / 4.0,
    1.0,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BordersColor([FiniteF32; 3]);

impl BordersColor {
    pub fn new(values: [f32; 3]) -> Result<Self, BordersCodecError> {
        Ok(Self([
            finite(values[0])?,
            finite(values[1])?,
            finite(values[2])?,
        ]))
    }
    pub const fn values(self) -> [FiniteF32; 3] {
        self.0
    }
    pub fn floats(self) -> [f32; 3] {
        self.0.map(FiniteF32::get)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BordersAspect {
    Image,
    Constant,
    Registered(u8),
    Custom(FiniteF32),
}

impl BordersAspect {
    pub fn custom(value: f32) -> Result<Self, BordersCodecError> {
        let value = finite(value)?;
        if value.get() <= 0.0 {
            return Err(BordersCodecError::InvalidAspect);
        }
        Ok(Self::Custom(value))
    }
    pub const fn registered(index: u8) -> Result<Self, BordersCodecError> {
        if index < 19 {
            Ok(Self::Registered(index))
        } else {
            Err(BordersCodecError::InvalidAspect)
        }
    }
    pub fn ratio(self) -> Option<f32> {
        match self {
            Self::Image => None,
            Self::Constant => Some(ASPECT_CONSTANT),
            Self::Registered(i) => ASPECTS.get(usize::from(i)).copied(),
            Self::Custom(v) => Some(v.get()),
        }
    }
    pub const fn storage_value(self) -> f32 {
        match self {
            Self::Image => ASPECT_IMAGE,
            Self::Constant => ASPECT_CONSTANT,
            Self::Registered(i) => ASPECTS[i as usize],
            Self::Custom(v) => v.get(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum BordersOrientation {
    Auto = 0,
    Portrait = 1,
    Landscape = 2,
}

impl TryFrom<i32> for BordersOrientation {
    type Error = BordersCodecError;
    fn try_from(value: i32) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Auto),
            1 => Ok(Self::Portrait),
            2 => Ok(Self::Landscape),
            _ => Err(BordersCodecError::InvalidOrientation(value)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum BordersBasis {
    Auto = 0,
    Width = 1,
    Height = 2,
    Shorter = 3,
    Longer = 4,
}

impl TryFrom<i32> for BordersBasis {
    type Error = BordersCodecError;
    fn try_from(value: i32) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Auto),
            1 => Ok(Self::Width),
            2 => Ok(Self::Height),
            3 => Ok(Self::Shorter),
            4 => Ok(Self::Longer),
            _ => Err(BordersCodecError::InvalidBasis(value)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BordersConfig {
    pub color: BordersColor,
    pub aspect: BordersAspect,
    pub orientation: BordersOrientation,
    pub size: FiniteF32,
    pub pos_h: FiniteF32,
    pub pos_v: FiniteF32,
    pub frame_size: FiniteF32,
    pub frame_offset: FiniteF32,
    pub frame_color: BordersColor,
    pub max_border_size: bool,
    pub basis: BordersBasis,
}

impl BordersConfig {
    #[expect(
        clippy::too_many_arguments,
        reason = "matches the versioned Darktable parameter tuple"
    )]
    pub fn new(
        color: [f32; 3],
        aspect: BordersAspect,
        orientation: BordersOrientation,
        size: f32,
        pos_h: f32,
        pos_v: f32,
        frame_size: f32,
        frame_offset: f32,
        frame_color: [f32; 3],
        max_border_size: bool,
        basis: BordersBasis,
    ) -> Result<Self, BordersCodecError> {
        let range = |value: f32, max: f32| {
            let value = finite(value)?;
            if !(0.0..=max).contains(&value.get()) {
                return Err(BordersCodecError::OutOfRange);
            }
            Ok(value)
        };
        Ok(Self {
            color: BordersColor::new(color)?,
            aspect,
            orientation,
            size: range(size, 0.5)?,
            pos_h: range(pos_h, 1.0)?,
            pos_v: range(pos_v, 1.0)?,
            frame_size: range(frame_size, 1.0)?,
            frame_offset: range(frame_offset, 1.0)?,
            frame_color: BordersColor::new(frame_color)?,
            max_border_size,
            basis,
        })
    }
    /// # Panics
    ///
    /// The built-in defaults are validated constants.
    pub fn defaults() -> Self {
        Self::new(
            [1.0; 3],
            BordersAspect::Constant,
            BordersOrientation::Auto,
            0.1,
            0.5,
            0.5,
            0.0,
            0.5,
            [0.0; 3],
            true,
            BordersBasis::Auto,
        )
        .expect("borders defaults")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BordersParametersV4 {
    pub config: BordersConfig,
    pub aspect_text: [u8; 20],
    pub pos_h_text: [u8; 20],
    pub pos_v_text: [u8; 20],
}

impl BordersParametersV4 {
    pub fn new(config: BordersConfig) -> Self {
        Self {
            config,
            aspect_text: [0; 20],
            pos_h_text: [0; 20],
            pos_v_text: [0; 20],
        }
    }
    pub fn to_bytes(&self) -> [u8; BORDERS_PARAMETER_BYTES_V4] {
        let mut b = [0; 120];
        put_color(&mut b, 0, self.config.color);
        b[12..16].copy_from_slice(&self.config.aspect.storage_value().to_le_bytes());
        b[16..36].copy_from_slice(&self.aspect_text);
        put_i32(&mut b, 36, self.config.orientation as i32);
        put_f32(&mut b, 40, self.config.size.get());
        put_f32(&mut b, 44, self.config.pos_h.get());
        b[48..68].copy_from_slice(&self.pos_h_text);
        put_f32(&mut b, 68, self.config.pos_v.get());
        b[72..92].copy_from_slice(&self.pos_v_text);
        put_f32(&mut b, 92, self.config.frame_size.get());
        put_f32(&mut b, 96, self.config.frame_offset.get());
        put_color(&mut b, 100, self.config.frame_color);
        put_i32(&mut b, 112, i32::from(self.config.max_border_size));
        put_i32(&mut b, 116, self.config.basis as i32);
        b
    }
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, BordersCodecError> {
        if bytes.len() != 120 {
            return Err(BordersCodecError::InvalidLength {
                expected: 120,
                actual: bytes.len(),
            });
        }
        let aspect = decode_aspect(read_f32(bytes, 12)?)?;
        let mut aspect_text = [0; 20];
        aspect_text.copy_from_slice(&bytes[16..36]);
        let mut horizontal_position_text = [0; 20];
        horizontal_position_text.copy_from_slice(&bytes[48..68]);
        let mut vertical_position_text = [0; 20];
        vertical_position_text.copy_from_slice(&bytes[72..92]);
        let config = BordersConfig::new(
            read_color(bytes, 0)?,
            aspect,
            read_i32(bytes, 36)?.try_into()?,
            read_f32(bytes, 40)?,
            read_f32(bytes, 44)?,
            read_f32(bytes, 68)?,
            read_f32(bytes, 92)?,
            read_f32(bytes, 96)?,
            read_color(bytes, 100)?,
            read_i32(bytes, 112)? != 0,
            read_i32(bytes, 116)?.try_into()?,
        )?;
        Ok(Self {
            config,
            aspect_text,
            pos_h_text: horizontal_position_text,
            pos_v_text: vertical_position_text,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BordersParametersV1 {
    pub color: BordersColor,
    pub aspect: FiniteF32,
    pub size: FiniteF32,
    raw: Vec<u8>,
}
impl BordersParametersV1 {
    pub fn new(color: [f32; 3], aspect: f32, size: f32) -> Result<Self, BordersCodecError> {
        Ok(Self {
            color: BordersColor::new(color)?,
            aspect: finite(aspect)?,
            size: finite(size)?,
            raw: vec![0; BORDERS_PARAMETER_BYTES_V1],
        })
    }
    pub fn from_bytes(b: &[u8]) -> Result<Self, BordersCodecError> {
        if b.len() != BORDERS_PARAMETER_BYTES_V1 {
            return Err(BordersCodecError::InvalidLength {
                expected: BORDERS_PARAMETER_BYTES_V1,
                actual: b.len(),
            });
        }
        Ok(Self {
            color: BordersColor::new(read_color(b, 0)?)?,
            aspect: finite(read_f32(b, 12)?)?,
            size: finite(read_f32(b, 16)?)?,
            raw: b.to_vec(),
        })
    }
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut b = self.raw.clone();
        put_color(&mut b, 0, self.color);
        put_f32(&mut b, 12, self.aspect.get());
        put_f32(&mut b, 16, self.size.get());
        b
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BordersParametersV2 {
    pub config: BordersConfig,
    raw: Vec<u8>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BordersParametersV3 {
    pub config: BordersConfig,
    raw: Vec<u8>,
}

macro_rules! legacy_impl {
    ($name:ident, $size:ident) => {
        impl $name {
            pub fn from_bytes(b: &[u8]) -> Result<Self, BordersCodecError> {
                if b.len() != super::$size {
                    return Err(BordersCodecError::InvalidLength {
                        expected: super::$size,
                        actual: b.len(),
                    });
                }
                let config = decode_legacy_config(b, false)?;
                Ok(Self {
                    config,
                    raw: b.to_vec(),
                })
            }
            pub fn to_bytes(&self) -> Vec<u8> {
                let mut b = self.raw.clone();
                encode_legacy_config(&mut b, self.config);
                b
            }
        }
    };
}
legacy_impl!(BordersParametersV2, BORDERS_PARAMETER_BYTES_V2);
legacy_impl!(BordersParametersV3, BORDERS_PARAMETER_BYTES_V3);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BordersHistory {
    V1(BordersParametersV1),
    V2(BordersParametersV2),
    V3(BordersParametersV3),
    V4(BordersParametersV4),
    Opaque { version: u16, bytes: Vec<u8> },
}
pub fn decode_history(version: u16, bytes: &[u8]) -> Result<BordersHistory, BordersCodecError> {
    match version {
        1 => Ok(BordersHistory::V1(BordersParametersV1::from_bytes(bytes)?)),
        2 => Ok(BordersHistory::V2(BordersParametersV2::from_bytes(bytes)?)),
        3 => Ok(BordersHistory::V3(BordersParametersV3::from_bytes(bytes)?)),
        4 => Ok(BordersHistory::V4(BordersParametersV4::from_bytes(bytes)?)),
        _ => Ok(BordersHistory::Opaque {
            version,
            bytes: bytes.to_vec(),
        }),
    }
}
pub fn migrate_history(history: BordersHistory) -> Result<BordersParametersV4, BordersCodecError> {
    match history {
        BordersHistory::V4(v) => Ok(v),
        BordersHistory::V3(v) => Ok(BordersParametersV4::new(migrate_v3(v.config))),
        BordersHistory::V2(v) => Ok(BordersParametersV4::new(migrate_v3(v.config))),
        BordersHistory::V1(v) => {
            let aspect = if v.aspect.get() < 1.0 {
                1.0 / v.aspect.get()
            } else {
                v.aspect.get()
            };
            let orientation = if v.aspect.get() > 1.0 {
                BordersOrientation::Landscape
            } else {
                BordersOrientation::Portrait
            };
            Ok(BordersParametersV4::new(BordersConfig::new(
                v.color.floats(),
                BordersAspect::custom(aspect)?,
                orientation,
                v.size.get(),
                0.5,
                0.5,
                0.0,
                0.5,
                [0.0; 3],
                false,
                BordersBasis::Auto,
            )?))
        }
        BordersHistory::Opaque { version, .. } => Err(BordersCodecError::OpaqueSource(version)),
    }
}
fn migrate_v3(mut c: BordersConfig) -> BordersConfig {
    c.basis = if matches!(c.aspect, BordersAspect::Constant) && !c.max_border_size {
        BordersBasis::Width
    } else {
        BordersBasis::Auto
    };
    c
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BordersCodecError {
    InvalidLength { expected: usize, actual: usize },
    InvalidFloat,
    InvalidAspect,
    InvalidOrientation(i32),
    InvalidBasis(i32),
    OutOfRange,
    OpaqueSource(u16),
    ArithmeticOverflow,
}
impl fmt::Display for BordersCodecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid borders parameters: {self:?}")
    }
}
impl std::error::Error for BordersCodecError {}
fn finite(v: f32) -> Result<FiniteF32, BordersCodecError> {
    FiniteF32::new(v).map_err(|_: FiniteF32Error| BordersCodecError::InvalidFloat)
}
fn put_f32(b: &mut [u8], o: usize, v: f32) {
    b[o..o + 4].copy_from_slice(&v.to_le_bytes());
}
fn put_i32(b: &mut [u8], o: usize, v: i32) {
    put_f32(b, o, f32::from_bits(v.cast_unsigned()));
}
fn read_f32(b: &[u8], o: usize) -> Result<f32, BordersCodecError> {
    b.get(o..o + 4)
        .and_then(|v| v.try_into().ok())
        .map(f32::from_le_bytes)
        .ok_or(BordersCodecError::ArithmeticOverflow)
}
fn read_i32(b: &[u8], o: usize) -> Result<i32, BordersCodecError> {
    Ok(f32::to_bits(read_f32(b, o)?).cast_signed())
}
fn put_color(b: &mut [u8], o: usize, c: BordersColor) {
    for (i, v) in c.floats().into_iter().enumerate() {
        put_f32(b, o + i * 4, v);
    }
}
fn read_color(b: &[u8], o: usize) -> Result<[f32; 3], BordersCodecError> {
    Ok([read_f32(b, o)?, read_f32(b, o + 4)?, read_f32(b, o + 8)?])
}
fn decode_aspect(v: f32) -> Result<BordersAspect, BordersCodecError> {
    if v == ASPECT_IMAGE {
        Ok(BordersAspect::Image)
    } else if v == ASPECT_CONSTANT {
        Ok(BordersAspect::Constant)
    } else if let Some(i) = ASPECTS.iter().position(|a| a.to_bits() == v.to_bits()) {
        Ok(BordersAspect::Registered(
            u8::try_from(i).expect("aspect index"),
        ))
    } else if v > 0.0 {
        Ok(BordersAspect::Custom(finite(v)?))
    } else {
        Err(BordersCodecError::InvalidAspect)
    }
}
fn decode_legacy_config(b: &[u8], max: bool) -> Result<BordersConfig, BordersCodecError> {
    let aspect = decode_aspect(read_f32(b, 12)?)?;
    BordersConfig::new(
        read_color(b, 0)?,
        aspect,
        read_i32(b, 36)?.try_into()?,
        read_f32(b, 40)?,
        read_f32(b, 44)?,
        read_f32(b, 68)?,
        read_f32(b, 92)?,
        read_f32(b, 96)?,
        read_color(b, 100)?,
        if b.len() >= BORDERS_PARAMETER_BYTES_V3 {
            read_i32(b, 112)? != 0
        } else {
            max
        },
        BordersBasis::Auto,
    )
}
fn encode_legacy_config(b: &mut [u8], c: BordersConfig) {
    put_color(b, 0, c.color);
    put_f32(b, 12, c.aspect.storage_value());
    put_i32(b, 36, c.orientation as i32);
    put_f32(b, 40, c.size.get());
    put_f32(b, 44, c.pos_h.get());
    put_f32(b, 68, c.pos_v.get());
    put_f32(b, 92, c.frame_size.get());
    put_f32(b, 96, c.frame_offset.get());
    put_color(b, 100, c.frame_color);
    if b.len() >= BORDERS_PARAMETER_BYTES_V3 {
        put_i32(b, 112, i32::from(c.max_border_size));
    }
}
