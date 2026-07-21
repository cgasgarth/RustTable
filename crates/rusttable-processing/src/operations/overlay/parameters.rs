use super::OVERLAY_PARAMETER_BYTES;
use crate::{FiniteF32, FiniteF32Error};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum OverlayAnchor {
    TopLeft = 0,
    Top = 1,
    TopRight = 2,
    Left = 3,
    Center = 4,
    Right = 5,
    BottomLeft = 6,
    Bottom = 7,
    BottomRight = 8,
}
impl TryFrom<i32> for OverlayAnchor {
    type Error = OverlayCodecError;
    fn try_from(v: i32) -> Result<Self, Self::Error> {
        match v {
            0 => Ok(Self::TopLeft),
            1 => Ok(Self::Top),
            2 => Ok(Self::TopRight),
            3 => Ok(Self::Left),
            4 => Ok(Self::Center),
            5 => Ok(Self::Right),
            6 => Ok(Self::BottomLeft),
            7 => Ok(Self::Bottom),
            8 => Ok(Self::BottomRight),
            _ => Err(OverlayCodecError::InvalidEnum),
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum OverlayBaseScale {
    Image = 0,
    LargerBorder = 1,
    SmallerBorder = 2,
    MarkerHeight = 3,
    Advanced = 4,
}
impl TryFrom<i32> for OverlayBaseScale {
    type Error = OverlayCodecError;
    fn try_from(v: i32) -> Result<Self, Self::Error> {
        match v {
            0 => Ok(Self::Image),
            1 => Ok(Self::LargerBorder),
            2 => Ok(Self::SmallerBorder),
            3 => Ok(Self::MarkerHeight),
            4 => Ok(Self::Advanced),
            _ => Err(OverlayCodecError::InvalidEnum),
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum OverlayImageScale {
    Width = 1,
    Height = 2,
    Larger = 3,
    Smaller = 4,
}
impl TryFrom<i32> for OverlayImageScale {
    type Error = OverlayCodecError;
    fn try_from(v: i32) -> Result<Self, Self::Error> {
        match v {
            1 => Ok(Self::Width),
            2 => Ok(Self::Height),
            3 => Ok(Self::Larger),
            4 => Ok(Self::Smaller),
            _ => Err(OverlayCodecError::InvalidEnum),
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum OverlayReference {
    Width = 0,
    Height = 1,
}
impl TryFrom<i32> for OverlayReference {
    type Error = OverlayCodecError;
    fn try_from(v: i32) -> Result<Self, Self::Error> {
        match v {
            0 => Ok(Self::Width),
            1 => Ok(Self::Height),
            _ => Err(OverlayCodecError::InvalidEnum),
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OverlayInterpolation {
    Nearest,
    Bilinear,
    Bicubic,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OverlayEdge {
    Transparent,
    Clamp,
    Repeat,
    Mirror,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OverlayAlpha {
    Straight,
    Premultiplied,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OverlayChannel {
    Rgb,
    Red,
    Green,
    Blue,
    Alpha,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OverlayProfilePolicy {
    RequireEmbedded,
    WorkingSrgb,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OverlayConfig {
    pub asset_hash: [u8; 32],
    pub opacity: FiniteF32,
    pub scale: FiniteF32,
    pub xoffset: FiniteF32,
    pub yoffset: FiniteF32,
    pub anchor: OverlayAnchor,
    pub rotation_degrees: FiniteF32,
    pub base_scale: OverlayBaseScale,
    pub image_scale: OverlayImageScale,
    pub reference: OverlayReference,
    pub interpolation: OverlayInterpolation,
    pub edge: OverlayEdge,
    pub alpha: OverlayAlpha,
    pub channel: OverlayChannel,
    pub profile: OverlayProfilePolicy,
}
impl OverlayConfig {
    #[expect(
        clippy::too_many_arguments,
        reason = "matches the versioned Darktable parameter tuple"
    )]
    pub fn new(
        asset_hash: [u8; 32],
        opacity: f32,
        scale: f32,
        xoffset: f32,
        yoffset: f32,
        anchor: OverlayAnchor,
        rotation_degrees: f32,
        base_scale: OverlayBaseScale,
        image_scale: OverlayImageScale,
        reference: OverlayReference,
    ) -> Result<Self, OverlayCodecError> {
        let finite =
            |v: f32| FiniteF32::new(v).map_err(|_: FiniteF32Error| OverlayCodecError::NonFinite);
        let opacity = finite(opacity)?;
        if !(0.0..=1.0).contains(&opacity.get()) {
            return Err(OverlayCodecError::OutOfRange);
        }
        let scale = finite(scale)?;
        if !(0.01..=5.0).contains(&scale.get()) {
            return Err(OverlayCodecError::OutOfRange);
        }
        let xoffset = finite(xoffset)?;
        let yoffset = finite(yoffset)?;
        let rotation_degrees = finite(rotation_degrees)?;
        if !(-180.0..=180.0).contains(&rotation_degrees.get())
            || xoffset.get().abs() > 1.0
            || yoffset.get().abs() > 1.0
        {
            return Err(OverlayCodecError::OutOfRange);
        }
        Ok(Self {
            asset_hash,
            opacity,
            scale,
            xoffset,
            yoffset,
            anchor,
            rotation_degrees,
            base_scale,
            image_scale,
            reference,
            interpolation: OverlayInterpolation::Bilinear,
            edge: OverlayEdge::Transparent,
            alpha: OverlayAlpha::Straight,
            channel: OverlayChannel::Rgb,
            profile: OverlayProfilePolicy::WorkingSrgb,
        })
    }
    /// # Panics
    ///
    /// The built-in defaults are validated constants.
    pub fn defaults(asset_hash: [u8; 32]) -> Self {
        Self::new(
            asset_hash,
            1.0,
            1.0,
            0.0,
            0.0,
            OverlayAnchor::Center,
            0.0,
            OverlayBaseScale::Image,
            OverlayImageScale::Larger,
            OverlayReference::Width,
        )
        .expect("overlay defaults")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverlayParametersV1 {
    pub config: OverlayConfig,
    pub filename: Vec<u8>,
    raw: Vec<u8>,
}
impl OverlayParametersV1 {
    pub fn new(
        config: OverlayConfig,
        filename: impl AsRef<[u8]>,
    ) -> Result<Self, OverlayCodecError> {
        if filename.as_ref().len() > 1024 {
            return Err(OverlayCodecError::TooLarge);
        }
        Ok(Self {
            config,
            filename: filename.as_ref().to_vec(),
            raw: vec![0; OVERLAY_PARAMETER_BYTES],
        })
    }
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut b = self.raw.clone();
        put_f32(&mut b, 0, self.config.opacity.get() * 100.0);
        put_f32(&mut b, 4, self.config.scale.get() * 100.0);
        put_f32(&mut b, 8, self.config.xoffset.get());
        put_f32(&mut b, 12, self.config.yoffset.get());
        put_i32(&mut b, 16, self.config.anchor as i32);
        put_f32(&mut b, 20, self.config.rotation_degrees.get());
        put_i32(&mut b, 24, self.config.base_scale as i32);
        put_i32(&mut b, 28, self.config.image_scale as i32);
        put_i32(&mut b, 32, self.config.reference as i32);
        b[40..40 + self.filename.len()].copy_from_slice(&self.filename);
        b
    }
    pub fn from_bytes(b: &[u8]) -> Result<Self, OverlayCodecError> {
        if b.len() != OVERLAY_PARAMETER_BYTES {
            return Err(OverlayCodecError::InvalidLength);
        }
        let mut filename = Vec::new();
        filename.extend(b[40..1064].iter().take_while(|v| **v != 0));
        let hash = [0; 32];
        let config = OverlayConfig::new(
            hash,
            read_f32(b, 0)? / 100.0,
            read_f32(b, 4)? / 100.0,
            read_f32(b, 8)?,
            read_f32(b, 12)?,
            read_i32(b, 16)?.try_into()?,
            read_f32(b, 20)?,
            read_i32(b, 24)?.try_into()?,
            read_i32(b, 28)?.try_into()?,
            read_i32(b, 32)?.try_into()?,
        )?;
        Ok(Self {
            config,
            filename,
            raw: b.to_vec(),
        })
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OverlayHistory {
    V1(OverlayParametersV1),
    Opaque { version: u16, bytes: Vec<u8> },
}
pub fn decode_history(version: u16, b: &[u8]) -> Result<OverlayHistory, OverlayCodecError> {
    if version == 1 {
        Ok(OverlayHistory::V1(OverlayParametersV1::from_bytes(b)?))
    } else {
        Ok(OverlayHistory::Opaque {
            version,
            bytes: b.to_vec(),
        })
    }
}
pub fn migrate_history(h: OverlayHistory) -> Result<OverlayParametersV1, OverlayCodecError> {
    match h {
        OverlayHistory::V1(v) => Ok(v),
        OverlayHistory::Opaque { version, .. } => Err(OverlayCodecError::OpaqueSource(version)),
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OverlayCodecError {
    InvalidLength,
    InvalidEnum,
    NonFinite,
    OutOfRange,
    TooLarge,
    OpaqueSource(u16),
}
impl fmt::Display for OverlayCodecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid overlay parameters: {self:?}")
    }
}
impl std::error::Error for OverlayCodecError {}
fn put_f32(b: &mut [u8], o: usize, v: f32) {
    b[o..o + 4].copy_from_slice(&v.to_le_bytes());
}
fn put_i32(b: &mut [u8], o: usize, v: i32) {
    b[o..o + 4].copy_from_slice(&v.to_le_bytes());
}
fn read_f32(b: &[u8], o: usize) -> Result<f32, OverlayCodecError> {
    b.get(o..o + 4)
        .and_then(|v| v.try_into().ok())
        .map(f32::from_le_bytes)
        .ok_or(OverlayCodecError::InvalidLength)
}
fn read_i32(b: &[u8], o: usize) -> Result<i32, OverlayCodecError> {
    b.get(o..o + 4)
        .and_then(|v| v.try_into().ok())
        .map(i32::from_le_bytes)
        .ok_or(OverlayCodecError::InvalidLength)
}
