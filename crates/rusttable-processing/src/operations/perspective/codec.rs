#![allow(
    clippy::cast_lossless,
    clippy::cast_possible_truncation,
    clippy::float_cmp,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc
)]

use std::fmt;
use std::hash::{Hash, Hasher};

use crate::FiniteF32;

use super::geometry::Point;

pub const ASHIFT_COMPATIBILITY_ID: &str = "ashift";
pub const ASHIFT_RUST_ID: &str = "rusttable.ashift";
pub const ASHIFT_SCHEMA_VERSION: u16 = 1;
pub const ASHIFT_PARAMETER_VERSION: u16 = 5;
pub const ASHIFT_IMPLEMENTATION_VERSION: u16 = 1;
pub const ASHIFT_MAX_SAVED_LINES: usize = 50;
pub const ASHIFT_MAX_DIMENSION: u32 = 1 << 30;

const V1_BYTES: usize = 16;
const V2_BYTES: usize = 36;
const V3_BYTES: usize = 56;
const V4_BYTES: usize = 60;
const V5_BYTES: usize = 892;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum AutoMethod {
    None = 0,
    Automatic = 1,
    Quad = 2,
    Lines = 3,
}

impl TryFrom<i32> for AutoMethod {
    type Error = PerspectiveConfigError;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::None),
            1 => Ok(Self::Automatic),
            2 => Ok(Self::Quad),
            3 => Ok(Self::Lines),
            _ => Err(PerspectiveConfigError::UnknownMethod(value)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum LensModel {
    Generic = 0,
    Specific = 1,
}

impl TryFrom<i32> for LensModel {
    type Error = PerspectiveConfigError;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Generic),
            1 => Ok(Self::Specific),
            _ => Err(PerspectiveConfigError::UnknownLensModel(value)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum CropMode {
    Off = 0,
    Largest = 1,
    Aspect = 2,
}

impl TryFrom<i32> for CropMode {
    type Error = PerspectiveConfigError;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Off),
            1 => Ok(Self::Largest),
            2 => Ok(Self::Aspect),
            _ => Err(PerspectiveConfigError::UnknownCropMode(value)),
        }
    }
}

/// A compact bitset matching Darktable's automatic-fit axis choices.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FitAxis(u8);

impl FitAxis {
    pub const NONE: Self = Self(0);
    pub const ROTATION: Self = Self(1 << 0);
    pub const LENS_VERTICAL: Self = Self(1 << 1);
    pub const LENS_HORIZONTAL: Self = Self(1 << 2);
    pub const SHEAR: Self = Self(1 << 3);
    pub const LINES_VERTICAL: Self = Self(1 << 4);
    pub const LINES_HORIZONTAL: Self = Self(1 << 5);
    pub const BOTH: Self = Self(0b11_1111);

    #[must_use]
    pub const fn from_bits(bits: u8) -> Option<Self> {
        if bits & !Self::BOTH.0 == 0 {
            Some(Self(bits))
        } else {
            None
        }
    }

    #[must_use]
    pub const fn bits(self) -> u8 {
        self.0
    }

    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Quad {
    pub top_left: Point,
    pub top_right: Point,
    pub bottom_right: Point,
    pub bottom_left: Point,
}

impl Quad {
    #[must_use]
    pub const fn new(
        top_left: Point,
        top_right: Point,
        bottom_right: Point,
        bottom_left: Point,
    ) -> Self {
        Self {
            top_left,
            top_right,
            bottom_right,
            bottom_left,
        }
    }

    #[must_use]
    pub const fn points(self) -> [Point; 4] {
        [
            self.top_left,
            self.top_right,
            self.bottom_right,
            self.bottom_left,
        ]
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PerspectiveParametersV1 {
    pub rotation: f32,
    pub lensshift_v: f32,
    pub lensshift_h: f32,
    pub toggle: i32,
}

impl PerspectiveParametersV1 {
    #[must_use]
    pub const fn new(rotation: f32, lensshift_v: f32, lensshift_h: f32, toggle: i32) -> Self {
        Self {
            rotation,
            lensshift_v,
            lensshift_h,
            toggle,
        }
    }

    #[must_use]
    pub fn to_bytes(self) -> [u8; V1_BYTES] {
        let mut bytes = [0; V1_BYTES];
        write_f32(&mut bytes[0..4], self.rotation);
        write_f32(&mut bytes[4..8], self.lensshift_v);
        write_f32(&mut bytes[8..12], self.lensshift_h);
        bytes[12..16].copy_from_slice(&self.toggle.to_le_bytes());
        bytes
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PerspectiveParametersV2 {
    pub rotation: f32,
    pub lensshift_v: f32,
    pub lensshift_h: f32,
    pub focal_length: f32,
    pub crop_factor: f32,
    pub orthocorr: f32,
    pub aspect: f32,
    pub mode: i32,
    pub toggle: i32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PerspectiveParametersV3 {
    pub rotation: f32,
    pub lensshift_v: f32,
    pub lensshift_h: f32,
    pub focal_length: f32,
    pub crop_factor: f32,
    pub orthocorr: f32,
    pub aspect: f32,
    pub mode: i32,
    pub toggle: i32,
    pub crop_mode: i32,
    pub crop_left: f32,
    pub crop_right: f32,
    pub crop_top: f32,
    pub crop_bottom: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PerspectiveParametersV4 {
    pub rotation: f32,
    pub lensshift_v: f32,
    pub lensshift_h: f32,
    pub shear: f32,
    pub focal_length: f32,
    pub crop_factor: f32,
    pub orthocorr: f32,
    pub aspect: f32,
    pub mode: i32,
    pub toggle: i32,
    pub crop_mode: i32,
    pub crop_left: f32,
    pub crop_right: f32,
    pub crop_top: f32,
    pub crop_bottom: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PerspectiveParametersV5 {
    pub rotation: f32,
    pub lensshift_v: f32,
    pub lensshift_h: f32,
    pub shear: f32,
    pub focal_length: f32,
    pub crop_factor: f32,
    pub orthocorr: f32,
    pub aspect: f32,
    pub mode: i32,
    pub crop_mode: i32,
    pub crop_left: f32,
    pub crop_right: f32,
    pub crop_top: f32,
    pub crop_bottom: f32,
    pub last_drawn_lines: Vec<[f32; 4]>,
    pub last_quad: Option<Quad>,
}

impl PerspectiveParametersV5 {
    #[must_use]
    pub fn to_bytes(&self) -> [u8; V5_BYTES] {
        let mut bytes = [0; V5_BYTES];
        for (offset, value) in [
            self.rotation,
            self.lensshift_v,
            self.lensshift_h,
            self.shear,
            self.focal_length,
            self.crop_factor,
            self.orthocorr,
            self.aspect,
        ]
        .into_iter()
        .enumerate()
        {
            write_f32(&mut bytes[offset * 4..offset * 4 + 4], value);
        }
        bytes[32..36].copy_from_slice(&self.mode.to_le_bytes());
        bytes[36..40].copy_from_slice(&self.crop_mode.to_le_bytes());
        for (offset, value) in [
            self.crop_left,
            self.crop_right,
            self.crop_top,
            self.crop_bottom,
        ]
        .into_iter()
        .enumerate()
        {
            write_f32(&mut bytes[40 + offset * 4..44 + offset * 4], value);
        }
        for (index, line) in self
            .last_drawn_lines
            .iter()
            .take(ASHIFT_MAX_SAVED_LINES)
            .enumerate()
        {
            let offset = 56 + index * 16;
            for (component, value) in line.iter().copied().enumerate() {
                write_f32(
                    &mut bytes[offset + component * 4..offset + component * 4 + 4],
                    value,
                );
            }
        }
        let count = i32::try_from(self.last_drawn_lines.len().min(ASHIFT_MAX_SAVED_LINES))
            .expect("saved-line limit fits in i32");
        bytes[856..860].copy_from_slice(&count.to_le_bytes());
        if let Some(quad) = self.last_quad {
            for (index, point) in quad.points().into_iter().enumerate() {
                let offset = 860 + index * 8;
                write_f32(&mut bytes[offset..offset + 4], point.x() as f32);
                write_f32(&mut bytes[offset + 4..offset + 8], point.y() as f32);
            }
        }
        bytes
    }
}

impl Default for PerspectiveParametersV5 {
    fn default() -> Self {
        Self {
            rotation: 0.0,
            lensshift_v: 0.0,
            lensshift_h: 0.0,
            shear: 0.0,
            focal_length: 28.0,
            crop_factor: 1.0,
            orthocorr: 100.0,
            aspect: 1.0,
            mode: 0,
            crop_mode: 1,
            crop_left: 0.0,
            crop_right: 1.0,
            crop_top: 0.0,
            crop_bottom: 1.0,
            last_drawn_lines: Vec::new(),
            last_quad: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PerspectiveConfig {
    rotation: FiniteF32,
    lensshift_v: FiniteF32,
    lensshift_h: FiniteF32,
    shear: FiniteF32,
    focal_length: FiniteF32,
    crop_factor: FiniteF32,
    orthocorr: FiniteF32,
    aspect: FiniteF32,
    lens_model: LensModel,
    crop_mode: CropMode,
    crop_left: FiniteF32,
    crop_right: FiniteF32,
    crop_top: FiniteF32,
    crop_bottom: FiniteF32,
    method: AutoMethod,
    fit_axis: FitAxis,
    drawn_lines: Vec<[FiniteF32; 4]>,
    quad: Option<Quad>,
    opaque_source: Option<Vec<u8>>,
}

impl Eq for PerspectiveConfig {}

impl Hash for PerspectiveConfig {
    fn hash<H: Hasher>(&self, state: &mut H) {
        fn hash_f32<H: Hasher>(value: f32, state: &mut H) {
            value.to_bits().hash(state);
        }
        fn hash_f64<H: Hasher>(value: f64, state: &mut H) {
            let bits = if value == 0.0 { 0 } else { value.to_bits() };
            bits.hash(state);
        }
        for value in [
            self.rotation.get(),
            self.lensshift_v.get(),
            self.lensshift_h.get(),
            self.shear.get(),
            self.focal_length.get(),
            self.crop_factor.get(),
            self.orthocorr.get(),
            self.aspect.get(),
            self.crop_left.get(),
            self.crop_right.get(),
            self.crop_top.get(),
            self.crop_bottom.get(),
        ] {
            hash_f32(value, state);
        }
        self.lens_model.hash(state);
        self.crop_mode.hash(state);
        self.method.hash(state);
        self.fit_axis.hash(state);
        for line in &self.drawn_lines {
            for value in line {
                hash_f32(value.get(), state);
            }
        }
        if let Some(quad) = self.quad {
            1_u8.hash(state);
            for point in quad.points() {
                hash_f64(point.x(), state);
                hash_f64(point.y(), state);
            }
        } else {
            0_u8.hash(state);
        }
        self.opaque_source.hash(state);
    }
}

impl PerspectiveConfig {
    /// Validates and creates the current semantic configuration.
    pub fn from_parameters(value: PerspectiveParametersV5) -> Result<Self, PerspectiveConfigError> {
        let method = AutoMethod::try_from(value.mode)?;
        let crop_mode = CropMode::try_from(value.crop_mode)?;
        let lens_model = LensModel::Generic;
        let mut drawn_lines =
            Vec::with_capacity(value.last_drawn_lines.len().min(ASHIFT_MAX_SAVED_LINES));
        for line in value
            .last_drawn_lines
            .into_iter()
            .take(ASHIFT_MAX_SAVED_LINES)
        {
            drawn_lines.push([
                finite(line[0])?,
                finite(line[1])?,
                finite(line[2])?,
                finite(line[3])?,
            ]);
        }
        let config = Self {
            rotation: bounded(value.rotation, -180.0, 180.0, "rotation")?,
            lensshift_v: bounded(value.lensshift_v, -2.0, 2.0, "lensshift_v")?,
            lensshift_h: bounded(value.lensshift_h, -2.0, 2.0, "lensshift_h")?,
            shear: bounded(value.shear, -0.5, 0.5, "shear")?,
            focal_length: bounded(value.focal_length, 1.0, 2000.0, "focal_length")?,
            crop_factor: bounded(value.crop_factor, 0.5, 10.0, "crop_factor")?,
            orthocorr: bounded(value.orthocorr, 0.0, 100.0, "orthocorr")?,
            aspect: bounded(value.aspect, 0.5, 2.0, "aspect")?,
            lens_model,
            crop_mode,
            crop_left: bounded(value.crop_left, 0.0, 1.0, "crop_left")?,
            crop_right: bounded(value.crop_right, 0.0, 1.0, "crop_right")?,
            crop_top: bounded(value.crop_top, 0.0, 1.0, "crop_top")?,
            crop_bottom: bounded(value.crop_bottom, 0.0, 1.0, "crop_bottom")?,
            method,
            fit_axis: FitAxis::BOTH,
            drawn_lines,
            quad: value.last_quad,
            opaque_source: None,
        };
        if config.crop_left >= config.crop_right || config.crop_top >= config.crop_bottom {
            return Err(PerspectiveConfigError::InvalidCropRectangle);
        }
        if let Some(quad) = config.quad {
            for point in quad.points() {
                point
                    .validate()
                    .map_err(|_| PerspectiveConfigError::InvalidQuad)?;
            }
        }
        Ok(config)
    }

    #[must_use]
    pub fn with_method(mut self, method: AutoMethod, fit_axis: FitAxis) -> Self {
        self.method = method;
        self.fit_axis = fit_axis;
        self
    }

    #[must_use]
    pub fn with_lens_model(mut self, lens_model: LensModel) -> Self {
        self.lens_model = lens_model;
        self
    }

    #[must_use]
    pub fn with_quad(mut self, quad: Quad) -> Self {
        self.quad = Some(quad);
        self.method = AutoMethod::Quad;
        self
    }

    #[must_use]
    pub fn with_opaque_source(mut self, source: Vec<u8>) -> Self {
        self.opaque_source = Some(source);
        self
    }

    #[must_use]
    pub const fn rotation(&self) -> FiniteF32 {
        self.rotation
    }
    #[must_use]
    pub const fn lensshift_v(&self) -> FiniteF32 {
        self.lensshift_v
    }
    #[must_use]
    pub const fn lensshift_h(&self) -> FiniteF32 {
        self.lensshift_h
    }
    #[must_use]
    pub const fn shear(&self) -> FiniteF32 {
        self.shear
    }
    #[must_use]
    pub const fn focal_length(&self) -> FiniteF32 {
        self.focal_length
    }
    #[must_use]
    pub const fn crop_factor(&self) -> FiniteF32 {
        self.crop_factor
    }
    #[must_use]
    pub const fn orthocorr(&self) -> FiniteF32 {
        self.orthocorr
    }
    #[must_use]
    pub const fn aspect(&self) -> FiniteF32 {
        self.aspect
    }
    #[must_use]
    pub const fn lens_model(&self) -> LensModel {
        self.lens_model
    }
    #[must_use]
    pub const fn crop_mode(&self) -> CropMode {
        self.crop_mode
    }
    #[must_use]
    pub const fn method(&self) -> AutoMethod {
        self.method
    }
    #[must_use]
    pub const fn fit_axis(&self) -> FitAxis {
        self.fit_axis
    }
    #[must_use]
    pub fn crop_rectangle(&self) -> [f32; 4] {
        [
            self.crop_left.get(),
            self.crop_right.get(),
            self.crop_top.get(),
            self.crop_bottom.get(),
        ]
    }
    #[must_use]
    pub fn drawn_lines(&self) -> &[[FiniteF32; 4]] {
        &self.drawn_lines
    }
    #[must_use]
    pub const fn quad(&self) -> Option<Quad> {
        self.quad
    }
    #[must_use]
    pub fn opaque_source(&self) -> Option<&[u8]> {
        self.opaque_source.as_deref()
    }
}

impl Default for PerspectiveConfig {
    fn default() -> Self {
        Self::from_parameters(PerspectiveParametersV5::default()).expect("default is valid")
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum PerspectiveConfigError {
    NonFinite(&'static str),
    OutOfRange {
        field: &'static str,
        minimum: f32,
        maximum: f32,
    },
    UnknownMethod(i32),
    UnknownLensModel(i32),
    UnknownCropMode(i32),
    InvalidCropRectangle,
    InvalidQuad,
}

impl fmt::Display for PerspectiveConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFinite(field) => write!(formatter, "perspective {field} must be finite"),
            Self::OutOfRange {
                field,
                minimum,
                maximum,
            } => write!(
                formatter,
                "perspective {field} must be in [{minimum}, {maximum}]"
            ),
            Self::UnknownMethod(value) => write!(formatter, "unknown perspective method {value}"),
            Self::UnknownLensModel(value) => {
                write!(formatter, "unknown perspective lens model {value}")
            }
            Self::UnknownCropMode(value) => {
                write!(formatter, "unknown perspective crop mode {value}")
            }
            Self::InvalidCropRectangle => {
                formatter.write_str("perspective crop rectangle is empty or reversed")
            }
            Self::InvalidQuad => formatter.write_str("perspective quadrilateral is invalid"),
        }
    }
}

impl std::error::Error for PerspectiveConfigError {}

#[derive(Debug, Clone, PartialEq)]
pub enum PerspectiveHistory {
    V1(PerspectiveParametersV1),
    V2(PerspectiveParametersV2),
    V3(PerspectiveParametersV3),
    V4(PerspectiveParametersV4),
    V5(PerspectiveParametersV5),
    Opaque { version: u16, bytes: Vec<u8> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PerspectiveHistoryError {
    InvalidLength { expected: usize, actual: usize },
    InvalidEnum,
    NonFinite,
    InvalidQuad,
}

impl fmt::Display for PerspectiveHistoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLength { expected, actual } => write!(
                formatter,
                "perspective history has {actual} bytes; expected {expected}"
            ),
            Self::InvalidEnum => {
                formatter.write_str("perspective history contains an invalid enum")
            }
            Self::NonFinite => {
                formatter.write_str("perspective history contains a non-finite value")
            }
            Self::InvalidQuad => {
                formatter.write_str("perspective history contains an invalid quadrilateral")
            }
        }
    }
}

impl std::error::Error for PerspectiveHistoryError {}

pub fn decode_history(
    version: u16,
    bytes: &[u8],
) -> Result<PerspectiveHistory, PerspectiveHistoryError> {
    match version {
        1 => decode_v1(bytes).map(PerspectiveHistory::V1),
        2 => decode_v2(bytes).map(PerspectiveHistory::V2),
        3 => decode_v3(bytes).map(PerspectiveHistory::V3),
        4 => decode_v4(bytes).map(PerspectiveHistory::V4),
        5 => decode_v5(bytes).map(PerspectiveHistory::V5),
        _ => Ok(PerspectiveHistory::Opaque {
            version,
            bytes: bytes.to_vec(),
        }),
    }
}

pub fn migrate_history(
    version: u16,
    bytes: &[u8],
) -> Result<PerspectiveConfig, PerspectiveHistoryError> {
    let history = decode_history(version, bytes)?;
    let value = match history {
        PerspectiveHistory::V1(value) => PerspectiveParametersV5 {
            rotation: value.rotation,
            lensshift_v: value.lensshift_v,
            lensshift_h: value.lensshift_h,
            ..Default::default()
        },
        PerspectiveHistory::V2(value) => PerspectiveParametersV5 {
            rotation: value.rotation,
            lensshift_v: value.lensshift_v,
            lensshift_h: value.lensshift_h,
            focal_length: value.focal_length,
            crop_factor: value.crop_factor,
            orthocorr: value.orthocorr,
            aspect: value.aspect,
            mode: value.mode,
            ..Default::default()
        },
        PerspectiveHistory::V3(value) => PerspectiveParametersV5 {
            rotation: value.rotation,
            lensshift_v: value.lensshift_v,
            lensshift_h: value.lensshift_h,
            focal_length: value.focal_length,
            crop_factor: value.crop_factor,
            orthocorr: value.orthocorr,
            aspect: value.aspect,
            mode: value.mode,
            crop_mode: value.crop_mode,
            crop_left: value.crop_left,
            crop_right: value.crop_right,
            crop_top: value.crop_top,
            crop_bottom: value.crop_bottom,
            ..Default::default()
        },
        PerspectiveHistory::V4(value) => PerspectiveParametersV5 {
            rotation: value.rotation,
            lensshift_v: value.lensshift_v,
            lensshift_h: value.lensshift_h,
            shear: value.shear,
            focal_length: value.focal_length,
            crop_factor: value.crop_factor,
            orthocorr: value.orthocorr,
            aspect: value.aspect,
            mode: value.mode,
            crop_mode: value.crop_mode,
            crop_left: value.crop_left,
            crop_right: value.crop_right,
            crop_top: value.crop_top,
            crop_bottom: value.crop_bottom,
            ..Default::default()
        },
        PerspectiveHistory::V5(value) => value,
        PerspectiveHistory::Opaque { version, .. } => {
            return Err(PerspectiveHistoryError::InvalidLength {
                expected: usize::from(version),
                actual: 0,
            });
        }
    };
    PerspectiveConfig::from_parameters(value).map_err(|error| match error {
        PerspectiveConfigError::InvalidQuad => PerspectiveHistoryError::InvalidQuad,
        PerspectiveConfigError::NonFinite(_) => PerspectiveHistoryError::NonFinite,
        _ => PerspectiveHistoryError::InvalidEnum,
    })
}

fn finite(value: f32) -> Result<FiniteF32, PerspectiveConfigError> {
    FiniteF32::new(value).map_err(|_| PerspectiveConfigError::NonFinite("parameter"))
}

fn bounded(
    value: f32,
    minimum: f32,
    maximum: f32,
    field: &'static str,
) -> Result<FiniteF32, PerspectiveConfigError> {
    let value = finite(value).map_err(|_| PerspectiveConfigError::NonFinite(field))?;
    if value.get() < minimum || value.get() > maximum {
        return Err(PerspectiveConfigError::OutOfRange {
            field,
            minimum,
            maximum,
        });
    }
    Ok(value)
}

fn write_f32(target: &mut [u8], value: f32) {
    target.copy_from_slice(&value.to_le_bytes());
}

fn read_f32(source: &[u8]) -> Result<f32, PerspectiveHistoryError> {
    let value = f32::from_le_bytes(source.try_into().map_err(|_| {
        PerspectiveHistoryError::InvalidLength {
            expected: 4,
            actual: source.len(),
        }
    })?);
    value
        .is_finite()
        .then_some(value)
        .ok_or(PerspectiveHistoryError::NonFinite)
}

fn check(bytes: &[u8], expected: usize) -> Result<(), PerspectiveHistoryError> {
    (bytes.len() == expected)
        .then_some(())
        .ok_or(PerspectiveHistoryError::InvalidLength {
            expected,
            actual: bytes.len(),
        })
}

fn decode_v1(bytes: &[u8]) -> Result<PerspectiveParametersV1, PerspectiveHistoryError> {
    check(bytes, V1_BYTES)?;
    Ok(PerspectiveParametersV1::new(
        read_f32(&bytes[0..4])?,
        read_f32(&bytes[4..8])?,
        read_f32(&bytes[8..12])?,
        i32::from_le_bytes(bytes[12..16].try_into().expect("checked length")),
    ))
}

fn decode_v2(bytes: &[u8]) -> Result<PerspectiveParametersV2, PerspectiveHistoryError> {
    check(bytes, V2_BYTES)?;
    let values = read_floats(bytes, 7)?;
    Ok(PerspectiveParametersV2 {
        rotation: values[0],
        lensshift_v: values[1],
        lensshift_h: values[2],
        focal_length: values[3],
        crop_factor: values[4],
        orthocorr: values[5],
        aspect: values[6],
        mode: i32::from_le_bytes(bytes[28..32].try_into().expect("checked length")),
        toggle: i32::from_le_bytes(bytes[32..36].try_into().expect("checked length")),
    })
}

fn decode_v3(bytes: &[u8]) -> Result<PerspectiveParametersV3, PerspectiveHistoryError> {
    check(bytes, V3_BYTES)?;
    let values = read_floats(bytes, 7)?;
    Ok(PerspectiveParametersV3 {
        rotation: values[0],
        lensshift_v: values[1],
        lensshift_h: values[2],
        focal_length: values[3],
        crop_factor: values[4],
        orthocorr: values[5],
        aspect: values[6],
        mode: i32::from_le_bytes(bytes[28..32].try_into().expect("checked length")),
        toggle: i32::from_le_bytes(bytes[32..36].try_into().expect("checked length")),
        crop_mode: i32::from_le_bytes(bytes[36..40].try_into().expect("checked length")),
        crop_left: read_f32(&bytes[40..44])?,
        crop_right: read_f32(&bytes[44..48])?,
        crop_top: read_f32(&bytes[48..52])?,
        crop_bottom: read_f32(&bytes[52..56])?,
    })
}

fn decode_v4(bytes: &[u8]) -> Result<PerspectiveParametersV4, PerspectiveHistoryError> {
    check(bytes, V4_BYTES)?;
    let values = read_floats(bytes, 8)?;
    Ok(PerspectiveParametersV4 {
        rotation: values[0],
        lensshift_v: values[1],
        lensshift_h: values[2],
        shear: values[3],
        focal_length: values[4],
        crop_factor: values[5],
        orthocorr: values[6],
        aspect: values[7],
        mode: i32::from_le_bytes(bytes[32..36].try_into().expect("checked length")),
        toggle: i32::from_le_bytes(bytes[36..40].try_into().expect("checked length")),
        crop_mode: i32::from_le_bytes(bytes[40..44].try_into().expect("checked length")),
        crop_left: read_f32(&bytes[44..48])?,
        crop_right: read_f32(&bytes[48..52])?,
        crop_top: read_f32(&bytes[52..56])?,
        crop_bottom: read_f32(&bytes[56..60])?,
    })
}

fn decode_v5(bytes: &[u8]) -> Result<PerspectiveParametersV5, PerspectiveHistoryError> {
    check(bytes, V5_BYTES)?;
    let values = read_floats(bytes, 8)?;
    let count = i32::from_le_bytes(bytes[856..860].try_into().expect("checked length"));
    if !(0..=i32::try_from(ASHIFT_MAX_SAVED_LINES).expect("constant fits")).contains(&count) {
        return Err(PerspectiveHistoryError::InvalidEnum);
    }
    let mut last_drawn_lines = Vec::new();
    for index in 0..ASHIFT_MAX_SAVED_LINES {
        let offset = 56 + index * 16;
        let line = [
            read_f32(&bytes[offset..offset + 4])?,
            read_f32(&bytes[offset + 4..offset + 8])?,
            read_f32(&bytes[offset + 8..offset + 12])?,
            read_f32(&bytes[offset + 12..offset + 16])?,
        ];
        if index < usize::try_from(count).expect("validated count") && line != [0.0; 4] {
            last_drawn_lines.push(line);
        }
    }
    let quad_points = [
        Point::new(
            read_f32(&bytes[860..864])? as f64,
            read_f32(&bytes[864..868])? as f64,
        ),
        Point::new(
            read_f32(&bytes[868..872])? as f64,
            read_f32(&bytes[872..876])? as f64,
        ),
        Point::new(
            read_f32(&bytes[876..880])? as f64,
            read_f32(&bytes[880..884])? as f64,
        ),
        Point::new(
            read_f32(&bytes[884..888])? as f64,
            read_f32(&bytes[888..892])? as f64,
        ),
    ];
    let last_quad = (quad_points != [Point::new(0.0, 0.0); 4]).then(|| {
        Quad::new(
            quad_points[0],
            quad_points[1],
            quad_points[2],
            quad_points[3],
        )
    });
    Ok(PerspectiveParametersV5 {
        rotation: values[0],
        lensshift_v: values[1],
        lensshift_h: values[2],
        shear: values[3],
        focal_length: values[4],
        crop_factor: values[5],
        orthocorr: values[6],
        aspect: values[7],
        mode: i32::from_le_bytes(bytes[32..36].try_into().expect("checked length")),
        crop_mode: i32::from_le_bytes(bytes[36..40].try_into().expect("checked length")),
        crop_left: read_f32(&bytes[40..44])?,
        crop_right: read_f32(&bytes[44..48])?,
        crop_top: read_f32(&bytes[48..52])?,
        crop_bottom: read_f32(&bytes[52..56])?,
        last_drawn_lines,
        last_quad,
    })
}

fn read_floats(bytes: &[u8], count: usize) -> Result<Vec<f32>, PerspectiveHistoryError> {
    (0..count)
        .map(|index| read_f32(&bytes[index * 4..index * 4 + 4]))
        .collect()
}
