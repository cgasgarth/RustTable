//! Lossless native Color Zones history codec and direct v1-v5 migrations.
//!
//! Source layout and migration lineage: `src/iop/colorzones.c`.

use std::fmt;

pub const COLORZONES_COMPATIBILITY_ID: &str = "colorzones";
pub const COLORZONES_RUST_ID: &str = "rusttable.colorzones";
pub const COLORZONES_SCHEMA_VERSION: u16 = 5;
pub const COLORZONES_CHANNELS: usize = 3;
pub const COLORZONES_MAX_NODES: usize = 20;
pub const COLORZONES_V1_BANDS: usize = 6;
pub const COLORZONES_LEGACY_BANDS: usize = 8;
pub const COLORZONES_V1_PARAMETER_BYTES: usize = 148;
pub const COLORZONES_V2_PARAMETER_BYTES: usize = 196;
pub const COLORZONES_V3_PARAMETER_BYTES: usize = 200;
pub const COLORZONES_V4_PARAMETER_BYTES: usize = 516;
pub const COLORZONES_V5_PARAMETER_BYTES: usize = 520;

const CATMULL_ROM: i32 = 1;
const SMOOTH_MODE: i32 = 0;
const SPLINES_V1: i32 = 0;
const SPLINES_V2: i32 = 1;

/// Raw native curve node.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorZonesNode {
    pub x: f32,
    pub y: f32,
}

impl ColorZonesNode {
    const ZERO: Self = Self::new(0.0, 0.0);

    #[must_use]
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

/// Raw native v1 payload in declaration order.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorZonesParametersV1 {
    pub channel: i32,
    pub equalizer_x: [[f32; COLORZONES_V1_BANDS]; COLORZONES_CHANNELS],
    pub equalizer_y: [[f32; COLORZONES_V1_BANDS]; COLORZONES_CHANNELS],
}

impl ColorZonesParametersV1 {
    #[must_use]
    pub const fn new(
        channel: i32,
        equalizer_x: [[f32; COLORZONES_V1_BANDS]; COLORZONES_CHANNELS],
        equalizer_y: [[f32; COLORZONES_V1_BANDS]; COLORZONES_CHANNELS],
    ) -> Self {
        Self {
            channel,
            equalizer_x,
            equalizer_y,
        }
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ColorZonesCodecError> {
        check_length(bytes, COLORZONES_V1_PARAMETER_BYTES)?;
        let mut decoder = Decoder::new(bytes);
        let channel = decoder.i32();
        let equalizer_x = decode_equalizer::<COLORZONES_V1_BANDS>(&mut decoder);
        let equalizer_y = decode_equalizer::<COLORZONES_V1_BANDS>(&mut decoder);
        debug_assert_eq!(decoder.offset, COLORZONES_V1_PARAMETER_BYTES);
        Ok(Self::new(channel, equalizer_x, equalizer_y))
    }

    #[must_use]
    pub fn to_bytes(&self) -> [u8; COLORZONES_V1_PARAMETER_BYTES] {
        let mut bytes = [0; COLORZONES_V1_PARAMETER_BYTES];
        let mut offset = 0;
        encode_i32(&mut bytes, &mut offset, self.channel);
        encode_equalizer(&mut bytes, &mut offset, &self.equalizer_x);
        encode_equalizer(&mut bytes, &mut offset, &self.equalizer_y);
        debug_assert_eq!(offset, COLORZONES_V1_PARAMETER_BYTES);
        bytes
    }
}

/// Raw native v2 payload in declaration order.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorZonesParametersV2 {
    pub channel: i32,
    pub equalizer_x: [[f32; COLORZONES_LEGACY_BANDS]; COLORZONES_CHANNELS],
    pub equalizer_y: [[f32; COLORZONES_LEGACY_BANDS]; COLORZONES_CHANNELS],
}

impl ColorZonesParametersV2 {
    #[must_use]
    pub const fn new(
        channel: i32,
        equalizer_x: [[f32; COLORZONES_LEGACY_BANDS]; COLORZONES_CHANNELS],
        equalizer_y: [[f32; COLORZONES_LEGACY_BANDS]; COLORZONES_CHANNELS],
    ) -> Self {
        Self {
            channel,
            equalizer_x,
            equalizer_y,
        }
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ColorZonesCodecError> {
        check_length(bytes, COLORZONES_V2_PARAMETER_BYTES)?;
        let mut decoder = Decoder::new(bytes);
        let channel = decoder.i32();
        let equalizer_x = decode_equalizer::<COLORZONES_LEGACY_BANDS>(&mut decoder);
        let equalizer_y = decode_equalizer::<COLORZONES_LEGACY_BANDS>(&mut decoder);
        debug_assert_eq!(decoder.offset, COLORZONES_V2_PARAMETER_BYTES);
        Ok(Self::new(channel, equalizer_x, equalizer_y))
    }

    #[must_use]
    pub fn to_bytes(&self) -> [u8; COLORZONES_V2_PARAMETER_BYTES] {
        let mut bytes = [0; COLORZONES_V2_PARAMETER_BYTES];
        let mut offset = 0;
        encode_i32(&mut bytes, &mut offset, self.channel);
        encode_equalizer(&mut bytes, &mut offset, &self.equalizer_x);
        encode_equalizer(&mut bytes, &mut offset, &self.equalizer_y);
        debug_assert_eq!(offset, COLORZONES_V2_PARAMETER_BYTES);
        bytes
    }
}

/// Raw native v3 payload in declaration order.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorZonesParametersV3 {
    pub channel: i32,
    pub equalizer_x: [[f32; COLORZONES_LEGACY_BANDS]; COLORZONES_CHANNELS],
    pub equalizer_y: [[f32; COLORZONES_LEGACY_BANDS]; COLORZONES_CHANNELS],
    pub strength: f32,
}

impl ColorZonesParametersV3 {
    #[must_use]
    pub const fn new(
        channel: i32,
        equalizer_x: [[f32; COLORZONES_LEGACY_BANDS]; COLORZONES_CHANNELS],
        equalizer_y: [[f32; COLORZONES_LEGACY_BANDS]; COLORZONES_CHANNELS],
        strength: f32,
    ) -> Self {
        Self {
            channel,
            equalizer_x,
            equalizer_y,
            strength,
        }
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ColorZonesCodecError> {
        check_length(bytes, COLORZONES_V3_PARAMETER_BYTES)?;
        let mut decoder = Decoder::new(bytes);
        let channel = decoder.i32();
        let equalizer_x = decode_equalizer::<COLORZONES_LEGACY_BANDS>(&mut decoder);
        let equalizer_y = decode_equalizer::<COLORZONES_LEGACY_BANDS>(&mut decoder);
        let strength = decoder.f32();
        debug_assert_eq!(decoder.offset, COLORZONES_V3_PARAMETER_BYTES);
        Ok(Self::new(channel, equalizer_x, equalizer_y, strength))
    }

    #[must_use]
    pub fn to_bytes(&self) -> [u8; COLORZONES_V3_PARAMETER_BYTES] {
        let mut bytes = [0; COLORZONES_V3_PARAMETER_BYTES];
        let mut offset = 0;
        encode_i32(&mut bytes, &mut offset, self.channel);
        encode_equalizer(&mut bytes, &mut offset, &self.equalizer_x);
        encode_equalizer(&mut bytes, &mut offset, &self.equalizer_y);
        encode_f32(&mut bytes, &mut offset, self.strength);
        debug_assert_eq!(offset, COLORZONES_V3_PARAMETER_BYTES);
        bytes
    }
}

/// Raw native v4 payload in declaration order.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorZonesParametersV4 {
    pub channel: i32,
    pub curves: [[ColorZonesNode; COLORZONES_MAX_NODES]; COLORZONES_CHANNELS],
    pub curve_num_nodes: [i32; COLORZONES_CHANNELS],
    pub curve_type: [i32; COLORZONES_CHANNELS],
    pub strength: f32,
    pub mode: i32,
}

impl ColorZonesParametersV4 {
    #[must_use]
    pub const fn new(
        channel: i32,
        curves: [[ColorZonesNode; COLORZONES_MAX_NODES]; COLORZONES_CHANNELS],
        curve_num_nodes: [i32; COLORZONES_CHANNELS],
        curve_type: [i32; COLORZONES_CHANNELS],
        strength: f32,
        mode: i32,
    ) -> Self {
        Self {
            channel,
            curves,
            curve_num_nodes,
            curve_type,
            strength,
            mode,
        }
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ColorZonesCodecError> {
        check_length(bytes, COLORZONES_V4_PARAMETER_BYTES)?;
        let mut decoder = Decoder::new(bytes);
        let channel = decoder.i32();
        let curves = decode_curves(&mut decoder);
        let curve_num_nodes = decode_i32_array(&mut decoder);
        let curve_type = decode_i32_array(&mut decoder);
        let strength = decoder.f32();
        let mode = decoder.i32();
        debug_assert_eq!(decoder.offset, COLORZONES_V4_PARAMETER_BYTES);
        Ok(Self::new(
            channel,
            curves,
            curve_num_nodes,
            curve_type,
            strength,
            mode,
        ))
    }

    #[must_use]
    pub fn to_bytes(&self) -> [u8; COLORZONES_V4_PARAMETER_BYTES] {
        let mut bytes = [0; COLORZONES_V4_PARAMETER_BYTES];
        let mut offset = 0;
        encode_i32(&mut bytes, &mut offset, self.channel);
        encode_curves(&mut bytes, &mut offset, &self.curves);
        encode_i32_array(&mut bytes, &mut offset, self.curve_num_nodes);
        encode_i32_array(&mut bytes, &mut offset, self.curve_type);
        encode_f32(&mut bytes, &mut offset, self.strength);
        encode_i32(&mut bytes, &mut offset, self.mode);
        debug_assert_eq!(offset, COLORZONES_V4_PARAMETER_BYTES);
        bytes
    }
}

/// Raw current native v5 payload in declaration order.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorZonesParametersV5 {
    pub channel: i32,
    pub curves: [[ColorZonesNode; COLORZONES_MAX_NODES]; COLORZONES_CHANNELS],
    pub curve_num_nodes: [i32; COLORZONES_CHANNELS],
    pub curve_type: [i32; COLORZONES_CHANNELS],
    pub strength: f32,
    pub mode: i32,
    pub splines_version: i32,
}

impl ColorZonesParametersV5 {
    #[must_use]
    pub const fn new(
        channel: i32,
        curves: [[ColorZonesNode; COLORZONES_MAX_NODES]; COLORZONES_CHANNELS],
        curve_num_nodes: [i32; COLORZONES_CHANNELS],
        curve_type: [i32; COLORZONES_CHANNELS],
        strength: f32,
        mode: i32,
        splines_version: i32,
    ) -> Self {
        Self {
            channel,
            curves,
            curve_num_nodes,
            curve_type,
            strength,
            mode,
            splines_version,
        }
    }

    /// Native v5 defaults from `_reset_parameters(..., hue, splines-v2)`.
    #[must_use]
    pub const fn defaults() -> Self {
        let mut curves = [[ColorZonesNode::ZERO; COLORZONES_MAX_NODES]; COLORZONES_CHANNELS];
        let mut channel = 0;
        while channel < COLORZONES_CHANNELS {
            curves[channel][0] = ColorZonesNode::new(0.25, 0.5);
            curves[channel][1] = ColorZonesNode::new(0.75, 0.5);
            channel += 1;
        }
        Self::new(
            2,
            curves,
            [2; COLORZONES_CHANNELS],
            [CATMULL_ROM; COLORZONES_CHANNELS],
            0.0,
            SMOOTH_MODE,
            SPLINES_V2,
        )
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ColorZonesCodecError> {
        check_length(bytes, COLORZONES_V5_PARAMETER_BYTES)?;
        let mut decoder = Decoder::new(bytes);
        let channel = decoder.i32();
        let curves = decode_curves(&mut decoder);
        let curve_num_nodes = decode_i32_array(&mut decoder);
        let curve_type = decode_i32_array(&mut decoder);
        let strength = decoder.f32();
        let mode = decoder.i32();
        let splines_version = decoder.i32();
        debug_assert_eq!(decoder.offset, COLORZONES_V5_PARAMETER_BYTES);
        Ok(Self::new(
            channel,
            curves,
            curve_num_nodes,
            curve_type,
            strength,
            mode,
            splines_version,
        ))
    }

    #[must_use]
    pub fn to_bytes(&self) -> [u8; COLORZONES_V5_PARAMETER_BYTES] {
        let mut bytes = [0; COLORZONES_V5_PARAMETER_BYTES];
        let mut offset = 0;
        encode_i32(&mut bytes, &mut offset, self.channel);
        encode_curves(&mut bytes, &mut offset, &self.curves);
        encode_i32_array(&mut bytes, &mut offset, self.curve_num_nodes);
        encode_i32_array(&mut bytes, &mut offset, self.curve_type);
        encode_f32(&mut bytes, &mut offset, self.strength);
        encode_i32(&mut bytes, &mut offset, self.mode);
        encode_i32(&mut bytes, &mut offset, self.splines_version);
        debug_assert_eq!(offset, COLORZONES_V5_PARAMETER_BYTES);
        bytes
    }
}

/// Typed known history with bounded enum size and opaque future retention.
#[derive(Debug, Clone, PartialEq)]
pub enum ColorZonesHistory {
    V1(Box<ColorZonesParametersV1>),
    V2(Box<ColorZonesParametersV2>),
    V3(Box<ColorZonesParametersV3>),
    V4(Box<ColorZonesParametersV4>),
    V5(Box<ColorZonesParametersV5>),
    Opaque { version: u16, bytes: Vec<u8> },
}

impl ColorZonesHistory {
    pub fn decode(version: u16, bytes: &[u8]) -> Result<Self, ColorZonesCodecError> {
        match version {
            1 => Ok(Self::V1(Box::new(ColorZonesParametersV1::from_bytes(
                bytes,
            )?))),
            2 => Ok(Self::V2(Box::new(ColorZonesParametersV2::from_bytes(
                bytes,
            )?))),
            3 => Ok(Self::V3(Box::new(ColorZonesParametersV3::from_bytes(
                bytes,
            )?))),
            4 => Ok(Self::V4(Box::new(ColorZonesParametersV4::from_bytes(
                bytes,
            )?))),
            COLORZONES_SCHEMA_VERSION => Ok(Self::V5(Box::new(
                ColorZonesParametersV5::from_bytes(bytes)?,
            ))),
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
            Self::V5(_) => COLORZONES_SCHEMA_VERSION,
            Self::Opaque { version, .. } => *version,
        }
    }

    #[must_use]
    pub fn payload(&self) -> Vec<u8> {
        match self {
            Self::V1(parameters) => parameters.to_bytes().to_vec(),
            Self::V2(parameters) => parameters.to_bytes().to_vec(),
            Self::V3(parameters) => parameters.to_bytes().to_vec(),
            Self::V4(parameters) => parameters.to_bytes().to_vec(),
            Self::V5(parameters) => parameters.to_bytes().to_vec(),
            Self::Opaque { bytes, .. } => bytes.clone(),
        }
    }

    pub fn current(&self) -> Result<ColorZonesParametersV5, ColorZonesCodecError> {
        match self {
            Self::V1(parameters) => Ok(migrate_v1_to_v5(**parameters)),
            Self::V2(parameters) => Ok(migrate_v2_to_v5(**parameters)),
            Self::V3(parameters) => Ok(migrate_v3_to_v5(**parameters)),
            Self::V4(parameters) => Ok(migrate_v4_to_v5(**parameters)),
            Self::V5(parameters) => Ok(**parameters),
            Self::Opaque { version, .. } => Err(ColorZonesCodecError::UnsupportedVersion(*version)),
        }
    }
}

/// Direct reproduction of Darktable's v1-to-v5 migration.
#[must_use]
pub fn migrate_v1_to_v5(parameters: ColorZonesParametersV1) -> ColorZonesParametersV5 {
    let mut migrated = legacy_migration_base(parameters.channel, 0.0);
    for channel in 0..COLORZONES_CHANNELS {
        migrated.curves[channel][0] = ColorZonesNode::new(
            parameters.equalizer_x[channel][0],
            parameters.equalizer_y[channel][0],
        );
        for node in 0..COLORZONES_V1_BANDS {
            let x = if node == 0 {
                parameters.equalizer_x[channel][node] + 0.001_f32
            } else if node == COLORZONES_V1_BANDS - 1 {
                parameters.equalizer_x[channel][node] - 0.001_f32
            } else {
                parameters.equalizer_x[channel][node]
            };
            migrated.curves[channel][node + 1] =
                ColorZonesNode::new(x, parameters.equalizer_y[channel][node]);
        }
        migrated.curves[channel][COLORZONES_LEGACY_BANDS - 1] = ColorZonesNode::new(
            parameters.equalizer_x[channel][COLORZONES_V1_BANDS - 1],
            parameters.equalizer_y[channel][COLORZONES_V1_BANDS - 1],
        );
    }
    migrated
}

/// Direct reproduction of Darktable's v2-to-v5 migration.
#[must_use]
pub fn migrate_v2_to_v5(parameters: ColorZonesParametersV2) -> ColorZonesParametersV5 {
    migrate_equalizer_v2_layout(
        parameters.channel,
        parameters.equalizer_x,
        parameters.equalizer_y,
        0.0,
    )
}

/// Direct reproduction of Darktable's v3-to-v5 migration.
#[must_use]
pub fn migrate_v3_to_v5(parameters: ColorZonesParametersV3) -> ColorZonesParametersV5 {
    migrate_equalizer_v2_layout(
        parameters.channel,
        parameters.equalizer_x,
        parameters.equalizer_y,
        parameters.strength,
    )
}

/// Direct reproduction of Darktable's v4-to-v5 migration.
#[must_use]
pub const fn migrate_v4_to_v5(parameters: ColorZonesParametersV4) -> ColorZonesParametersV5 {
    ColorZonesParametersV5::new(
        parameters.channel,
        parameters.curves,
        parameters.curve_num_nodes,
        parameters.curve_type,
        parameters.strength,
        parameters.mode,
        SPLINES_V1,
    )
}

fn legacy_migration_base(channel: i32, strength: f32) -> ColorZonesParametersV5 {
    ColorZonesParametersV5::new(
        channel,
        [[ColorZonesNode::ZERO; COLORZONES_MAX_NODES]; COLORZONES_CHANNELS],
        [i32::try_from(COLORZONES_LEGACY_BANDS).expect("legacy band count fits i32");
            COLORZONES_CHANNELS],
        [CATMULL_ROM; COLORZONES_CHANNELS],
        strength,
        SMOOTH_MODE,
        SPLINES_V1,
    )
}

fn migrate_equalizer_v2_layout(
    channel: i32,
    equalizer_x: [[f32; COLORZONES_LEGACY_BANDS]; COLORZONES_CHANNELS],
    equalizer_y: [[f32; COLORZONES_LEGACY_BANDS]; COLORZONES_CHANNELS],
    strength: f32,
) -> ColorZonesParametersV5 {
    let mut migrated = legacy_migration_base(channel, strength);
    for curve_channel in 0..COLORZONES_CHANNELS {
        for node in 0..COLORZONES_LEGACY_BANDS {
            migrated.curves[curve_channel][node] = ColorZonesNode::new(
                equalizer_x[curve_channel][node],
                equalizer_y[curve_channel][node],
            );
        }
    }
    migrated
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ColorZonesCodecError {
    InvalidLength { expected: usize, actual: usize },
    UnsupportedVersion(u16),
}

impl fmt::Display for ColorZonesCodecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLength { expected, actual } => write!(
                formatter,
                "Color Zones payload has {actual} bytes; expected {expected}"
            ),
            Self::UnsupportedVersion(version) => write!(
                formatter,
                "Color Zones version {version} is opaque and unsupported"
            ),
        }
    }
}

impl std::error::Error for ColorZonesCodecError {}

fn check_length(bytes: &[u8], expected: usize) -> Result<(), ColorZonesCodecError> {
    if bytes.len() == expected {
        Ok(())
    } else {
        Err(ColorZonesCodecError::InvalidLength {
            expected,
            actual: bytes.len(),
        })
    }
}

struct Decoder<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Decoder<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn i32(&mut self) -> i32 {
        i32::from_le_bytes(self.bytes_4())
    }

    fn f32(&mut self) -> f32 {
        f32::from_le_bytes(self.bytes_4())
    }

    fn bytes_4(&mut self) -> [u8; 4] {
        let end = self.offset + 4;
        let bytes = self.bytes[self.offset..end]
            .try_into()
            .expect("payload length was checked before decoding");
        self.offset = end;
        bytes
    }
}

fn decode_equalizer<const BANDS: usize>(
    decoder: &mut Decoder<'_>,
) -> [[f32; BANDS]; COLORZONES_CHANNELS] {
    std::array::from_fn(|_| std::array::from_fn(|_| decoder.f32()))
}

fn decode_curves(
    decoder: &mut Decoder<'_>,
) -> [[ColorZonesNode; COLORZONES_MAX_NODES]; COLORZONES_CHANNELS] {
    std::array::from_fn(|_| {
        std::array::from_fn(|_| ColorZonesNode::new(decoder.f32(), decoder.f32()))
    })
}

fn decode_i32_array(decoder: &mut Decoder<'_>) -> [i32; COLORZONES_CHANNELS] {
    std::array::from_fn(|_| decoder.i32())
}

fn encode_i32(bytes: &mut [u8], offset: &mut usize, value: i32) {
    encode_4(bytes, offset, value.to_le_bytes());
}

fn encode_f32(bytes: &mut [u8], offset: &mut usize, value: f32) {
    encode_4(bytes, offset, value.to_le_bytes());
}

fn encode_4(bytes: &mut [u8], offset: &mut usize, encoded: [u8; 4]) {
    let end = *offset + 4;
    bytes[*offset..end].copy_from_slice(&encoded);
    *offset = end;
}

fn encode_equalizer<const BANDS: usize>(
    bytes: &mut [u8],
    offset: &mut usize,
    equalizer: &[[f32; BANDS]; COLORZONES_CHANNELS],
) {
    for channel in equalizer {
        for value in channel {
            encode_f32(bytes, offset, *value);
        }
    }
}

fn encode_curves(
    bytes: &mut [u8],
    offset: &mut usize,
    curves: &[[ColorZonesNode; COLORZONES_MAX_NODES]; COLORZONES_CHANNELS],
) {
    for curve in curves {
        for node in curve {
            encode_f32(bytes, offset, node.x);
            encode_f32(bytes, offset, node.y);
        }
    }
}

fn encode_i32_array(bytes: &mut [u8], offset: &mut usize, values: [i32; COLORZONES_CHANNELS]) {
    for value in values {
        encode_i32(bytes, offset, value);
    }
}
