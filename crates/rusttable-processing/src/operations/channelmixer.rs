//! Darktable-compatible legacy channel mixer processing leaf.
//!
//! This module is a direct port of `src/iop/channelmixer.c` and its
//! `data/kernels/extended.cl::channelmixer` companion.  It intentionally stops
//! at the processing boundary: operation registration, history import, GPU
//! binding, pixelpipe dispatch, and UI wiring remain integration seams.
//!
//! The native module stores three seven-element arrays followed by a C enum.
//! The v1 payload omits that enum.  The codecs below spell out the little-endian
//! ABI rather than relying on Rust layout or the host architecture.

#![forbid(unsafe_code)]
#![allow(
    clippy::missing_errors_doc,
    clippy::float_cmp,
    clippy::cast_precision_loss,
    reason = "the compatibility codec and closed f32 point operation have conventional contracts"
)]

use std::fmt;

use rusttable_color::ColorEncoding;

use crate::FiniteF32;
use crate::descriptor::{
    AlphaPolicy, CapabilityContract, DescriptorId, ImagePredicate, InputOutputContract,
    MaskBlendContract, MigrationContract, NonFinitePolicy, OperationDescriptor, OperationFlags,
    ParameterDefault, ParameterDescriptor, ParameterKind, ParameterRole, RoiKind, TilingContract,
    UiHint,
};

pub const CHANNEL_MIXER_COMPATIBILITY_ID: &str = "channelmixer";
pub const CHANNEL_MIXER_RUST_ID: &str = "rusttable.channelmixer";
pub const CHANNEL_MIXER_SCHEMA_VERSION: u16 = 2;
pub const CHANNEL_MIXER_OUTPUT_COUNT: usize = 7;
pub const CHANNEL_MIXER_V1_PARAMETER_BYTES: usize = 3 * CHANNEL_MIXER_OUTPUT_COUNT * 4;
pub const CHANNEL_MIXER_V2_PARAMETER_BYTES: usize = CHANNEL_MIXER_V1_PARAMETER_BYTES + 4;
pub const CHANNEL_MIXER_PARAMETER_MIN: f32 = -1.0;
pub const CHANNEL_MIXER_PARAMETER_MAX: f32 = 1.0;
pub const CHANNEL_MIXER_GUI_MIN: f32 = -2.0;
pub const CHANNEL_MIXER_GUI_MAX: f32 = 2.0;

const HUE: usize = 0;
const LIGHTNESS: usize = 2;
const RED: usize = 3;
const GREEN: usize = 4;
const BLUE: usize = 5;
const GRAY: usize = 6;
const HSL_OUTPUTS: std::ops::RangeInclusive<usize> = HUE..=LIGHTNESS;
const RGB_OUTPUTS: std::ops::RangeInclusive<usize> = RED..=BLUE;
#[allow(
    clippy::excessive_precision,
    reason = "the native colorspaces.h constant is ported exactly"
)]
const MIN_HSL_DENOMINATOR: f32 = 1.525_878_906_25e-5_f32;

/// The native algorithm enum, including its exact signed 32-bit ABI width.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum ChannelMixerAlgorithm {
    V1 = 0,
    V2 = 1,
}

impl TryFrom<i32> for ChannelMixerAlgorithm {
    type Error = ChannelMixerCodecError;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::V1),
            1 => Ok(Self::V2),
            value => Err(Self::Error::UnknownAlgorithm(value)),
        }
    }
}

impl From<ChannelMixerAlgorithm> for i32 {
    fn from(value: ChannelMixerAlgorithm) -> Self {
        value as Self
    }
}

/// Native v1 payload in source declaration order.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChannelMixerParametersV1 {
    pub red: [f32; CHANNEL_MIXER_OUTPUT_COUNT],
    pub green: [f32; CHANNEL_MIXER_OUTPUT_COUNT],
    pub blue: [f32; CHANNEL_MIXER_OUTPUT_COUNT],
}

impl ChannelMixerParametersV1 {
    #[must_use]
    pub const fn new(
        red: [f32; CHANNEL_MIXER_OUTPUT_COUNT],
        green: [f32; CHANNEL_MIXER_OUTPUT_COUNT],
        blue: [f32; CHANNEL_MIXER_OUTPUT_COUNT],
    ) -> Self {
        Self { red, green, blue }
    }

    #[must_use]
    ///
    /// # Panics
    ///
    /// Never panics: the internal v2 buffer is always large enough for the v1 prefix.
    pub fn to_bytes(self) -> [u8; CHANNEL_MIXER_V1_PARAMETER_BYTES] {
        encode_arrays(self.red, self.green, self.blue, None)[..CHANNEL_MIXER_V1_PARAMETER_BYTES]
            .try_into()
            .expect("the v2 prefix is the native v1 payload")
    }

    ///
    /// # Panics
    ///
    /// Never panics: the fixed-size reads follow an exact length check.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ChannelMixerCodecError> {
        if bytes.len() != CHANNEL_MIXER_V1_PARAMETER_BYTES {
            return Err(ChannelMixerCodecError::InvalidLength {
                expected: CHANNEL_MIXER_V1_PARAMETER_BYTES,
                actual: bytes.len(),
            });
        }
        let (red, green, blue) = decode_arrays(bytes);
        Ok(Self::new(red, green, blue))
    }
}

/// Native current v2 payload in source declaration order.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChannelMixerParametersV2 {
    pub red: [f32; CHANNEL_MIXER_OUTPUT_COUNT],
    pub green: [f32; CHANNEL_MIXER_OUTPUT_COUNT],
    pub blue: [f32; CHANNEL_MIXER_OUTPUT_COUNT],
    pub algorithm_version: ChannelMixerAlgorithm,
}

impl ChannelMixerParametersV2 {
    #[must_use]
    pub const fn new(
        red: [f32; CHANNEL_MIXER_OUTPUT_COUNT],
        green: [f32; CHANNEL_MIXER_OUTPUT_COUNT],
        blue: [f32; CHANNEL_MIXER_OUTPUT_COUNT],
        algorithm_version: ChannelMixerAlgorithm,
    ) -> Self {
        Self {
            red,
            green,
            blue,
            algorithm_version,
        }
    }

    #[must_use]
    pub const fn defaults() -> Self {
        let mut red = [0.0; CHANNEL_MIXER_OUTPUT_COUNT];
        let mut green = [0.0; CHANNEL_MIXER_OUTPUT_COUNT];
        let mut blue = [0.0; CHANNEL_MIXER_OUTPUT_COUNT];
        red[RED] = 1.0;
        green[GREEN] = 1.0;
        blue[BLUE] = 1.0;
        Self::new(red, green, blue, ChannelMixerAlgorithm::V2)
    }

    #[must_use]
    pub fn to_bytes(self) -> [u8; CHANNEL_MIXER_V2_PARAMETER_BYTES] {
        encode_arrays(
            self.red,
            self.green,
            self.blue,
            Some(self.algorithm_version),
        )
    }

    ///
    /// # Panics
    ///
    /// Never panics: the fixed-size reads follow an exact length check.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ChannelMixerCodecError> {
        if bytes.len() != CHANNEL_MIXER_V2_PARAMETER_BYTES {
            return Err(ChannelMixerCodecError::InvalidLength {
                expected: CHANNEL_MIXER_V2_PARAMETER_BYTES,
                actual: bytes.len(),
            });
        }
        let (red, green, blue) = decode_arrays(bytes);
        let raw_algorithm = i32::from_le_bytes(
            bytes[CHANNEL_MIXER_V1_PARAMETER_BYTES..]
                .try_into()
                .expect("payload length was checked"),
        );
        let algorithm_version = raw_algorithm.try_into()?;
        Ok(Self::new(red, green, blue, algorithm_version))
    }
}

fn encode_arrays(
    red: [f32; CHANNEL_MIXER_OUTPUT_COUNT],
    green: [f32; CHANNEL_MIXER_OUTPUT_COUNT],
    blue: [f32; CHANNEL_MIXER_OUTPUT_COUNT],
    algorithm: Option<ChannelMixerAlgorithm>,
) -> [u8; CHANNEL_MIXER_V2_PARAMETER_BYTES] {
    let mut bytes = [0_u8; CHANNEL_MIXER_V2_PARAMETER_BYTES];
    for (index, value) in red.into_iter().chain(green).chain(blue).enumerate() {
        let offset = index * std::mem::size_of::<f32>();
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    if let Some(algorithm) = algorithm {
        bytes[CHANNEL_MIXER_V1_PARAMETER_BYTES..]
            .copy_from_slice(&i32::from(algorithm).to_le_bytes());
    }
    bytes
}

fn decode_arrays(
    bytes: &[u8],
) -> (
    [f32; CHANNEL_MIXER_OUTPUT_COUNT],
    [f32; CHANNEL_MIXER_OUTPUT_COUNT],
    [f32; CHANNEL_MIXER_OUTPUT_COUNT],
) {
    let read = |index: usize| {
        let offset = index * std::mem::size_of::<f32>();
        f32::from_le_bytes(
            bytes[offset..offset + 4]
                .try_into()
                .expect("payload length was checked"),
        )
    };
    (
        std::array::from_fn(&read),
        std::array::from_fn(|index| read(CHANNEL_MIXER_OUTPUT_COUNT + index)),
        std::array::from_fn(|index| read(2 * CHANNEL_MIXER_OUTPUT_COUNT + index)),
    )
}

/// Typed known history plus byte-exact retention for future versions.
#[derive(Debug, Clone, PartialEq)]
pub enum ChannelMixerHistory {
    V1(ChannelMixerParametersV1),
    V2(ChannelMixerParametersV2),
    Opaque { version: u16, bytes: Vec<u8> },
}

impl ChannelMixerHistory {
    pub fn decode(version: u16, bytes: &[u8]) -> Result<Self, ChannelMixerCodecError> {
        match version {
            1 => Ok(Self::V1(ChannelMixerParametersV1::from_bytes(bytes)?)),
            CHANNEL_MIXER_SCHEMA_VERSION => {
                Ok(Self::V2(ChannelMixerParametersV2::from_bytes(bytes)?))
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
            Self::V2(_) => CHANNEL_MIXER_SCHEMA_VERSION,
            Self::Opaque { version, .. } => *version,
        }
    }

    #[must_use]
    pub fn payload(&self) -> Vec<u8> {
        match self {
            Self::V1(parameters) => {
                parameters.to_bytes()[..CHANNEL_MIXER_V1_PARAMETER_BYTES].to_vec()
            }
            Self::V2(parameters) => parameters.to_bytes().to_vec(),
            Self::Opaque { bytes, .. } => bytes.clone(),
        }
    }

    pub fn current(&self) -> Result<ChannelMixerParametersV2, ChannelMixerCodecError> {
        match self {
            Self::V1(parameters) => Ok(migrate_v1_to_v2(*parameters)),
            Self::V2(parameters) => Ok(*parameters),
            Self::Opaque { version, .. } => {
                Err(ChannelMixerCodecError::UnsupportedVersion(*version))
            }
        }
    }
}

/// Reproduces the complete native `legacy_params(old_version == 1)` branch.
#[must_use]
pub fn migrate_v1_to_v2(parameters: ChannelMixerParametersV1) -> ChannelMixerParametersV2 {
    let mut red = [0.0; CHANNEL_MIXER_OUTPUT_COUNT];
    let mut green = [0.0; CHANNEL_MIXER_OUTPUT_COUNT];
    let mut blue = [0.0; CHANNEL_MIXER_OUTPUT_COUNT];

    // The old gray values are always copied first.
    red[GRAY] = parameters.red[GRAY];
    green[GRAY] = parameters.green[GRAY];
    blue[GRAY] = parameters.blue[GRAY];

    // Version 1 did not use RGB mixing while gray was enabled.
    if red[GRAY] == 0.0 && green[GRAY] == 0.0 && blue[GRAY] == 0.0 {
        red[RED..=BLUE].copy_from_slice(&parameters.red[RED..=BLUE]);
        green[RED..=BLUE].copy_from_slice(&parameters.green[RED..=BLUE]);
        blue[RED..=BLUE].copy_from_slice(&parameters.blue[RED..=BLUE]);
    }

    // HSL mixing occupies the first three output slots in all versions.
    red[HUE..=LIGHTNESS].copy_from_slice(&parameters.red[HUE..=LIGHTNESS]);
    green[HUE..=LIGHTNESS].copy_from_slice(&parameters.green[HUE..=LIGHTNESS]);
    blue[HUE..=LIGHTNESS].copy_from_slice(&parameters.blue[HUE..=LIGHTNESS]);

    ChannelMixerParametersV2::new(red, green, blue, ChannelMixerAlgorithm::V1)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChannelMixerCodecError {
    InvalidLength { expected: usize, actual: usize },
    UnknownAlgorithm(i32),
    UnsupportedVersion(u16),
}

impl fmt::Display for ChannelMixerCodecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLength { expected, actual } => write!(
                formatter,
                "Channel Mixer payload has {actual} bytes; expected {expected}"
            ),
            Self::UnknownAlgorithm(value) => write!(
                formatter,
                "Channel Mixer algorithm version {value} is unknown"
            ),
            Self::UnsupportedVersion(version) => {
                write!(
                    formatter,
                    "Channel Mixer version {version} is opaque and unsupported"
                )
            }
        }
    }
}

impl std::error::Error for ChannelMixerCodecError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChannelMixerParameterError {
    NonFinite { plane: &'static str, index: usize },
}

impl fmt::Display for ChannelMixerParameterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFinite { plane, index } => {
                write!(formatter, "Channel Mixer {plane}[{index}] is non-finite")
            }
        }
    }
}

impl std::error::Error for ChannelMixerParameterError {}

/// Checked finite runtime state. Native commit does not clamp array values to
/// the `-1..=1` metadata range, so every finite persisted value is retained.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ChannelMixerConfig {
    red: [FiniteF32; CHANNEL_MIXER_OUTPUT_COUNT],
    green: [FiniteF32; CHANNEL_MIXER_OUTPUT_COUNT],
    blue: [FiniteF32; CHANNEL_MIXER_OUTPUT_COUNT],
    algorithm_version: ChannelMixerAlgorithm,
}

impl ChannelMixerConfig {
    pub fn new(
        red: [f32; CHANNEL_MIXER_OUTPUT_COUNT],
        green: [f32; CHANNEL_MIXER_OUTPUT_COUNT],
        blue: [f32; CHANNEL_MIXER_OUTPUT_COUNT],
        algorithm_version: ChannelMixerAlgorithm,
    ) -> Result<Self, ChannelMixerParameterError> {
        Self::try_from(ChannelMixerParametersV2::new(
            red,
            green,
            blue,
            algorithm_version,
        ))
    }

    #[must_use]
    ///
    /// # Panics
    ///
    /// Never panics: the source-defined defaults are finite.
    pub fn defaults() -> Self {
        Self::try_from(ChannelMixerParametersV2::defaults())
            .expect("Channel Mixer defaults are finite")
    }

    #[must_use]
    pub fn red(self) -> [f32; CHANNEL_MIXER_OUTPUT_COUNT] {
        self.red.map(FiniteF32::get)
    }

    #[must_use]
    pub fn green(self) -> [f32; CHANNEL_MIXER_OUTPUT_COUNT] {
        self.green.map(FiniteF32::get)
    }

    #[must_use]
    pub fn blue(self) -> [f32; CHANNEL_MIXER_OUTPUT_COUNT] {
        self.blue.map(FiniteF32::get)
    }

    #[must_use]
    pub const fn algorithm_version(self) -> ChannelMixerAlgorithm {
        self.algorithm_version
    }

    #[must_use]
    pub fn parameters(self) -> ChannelMixerParametersV2 {
        ChannelMixerParametersV2::new(
            self.red(),
            self.green(),
            self.blue(),
            self.algorithm_version,
        )
    }
}

impl TryFrom<ChannelMixerParametersV2> for ChannelMixerConfig {
    type Error = ChannelMixerParameterError;

    fn try_from(parameters: ChannelMixerParametersV2) -> Result<Self, Self::Error> {
        Ok(Self {
            red: finite_array(parameters.red, "red")?,
            green: finite_array(parameters.green, "green")?,
            blue: finite_array(parameters.blue, "blue")?,
            algorithm_version: parameters.algorithm_version,
        })
    }
}

fn finite_array(
    values: [f32; CHANNEL_MIXER_OUTPUT_COUNT],
    plane: &'static str,
) -> Result<[FiniteF32; CHANNEL_MIXER_OUTPUT_COUNT], ChannelMixerParameterError> {
    let mut output = [FiniteF32::from_proven_finite(0.0); CHANNEL_MIXER_OUTPUT_COUNT];
    for (index, value) in values.into_iter().enumerate() {
        output[index] = FiniteF32::new(value)
            .map_err(|_| ChannelMixerParameterError::NonFinite { plane, index })?;
    }
    Ok(output)
}

/// Four-channel native float sample. RGB is processed and channel four is
/// copied bit-for-bit; the separate normal-blend entry point replaces it with
/// local blend coverage just as Darktable's RGB blend helper does.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChannelMixerPixel {
    channels: [f32; 4],
}

impl ChannelMixerPixel {
    #[must_use]
    pub const fn new(red: f32, green: f32, blue: f32, alpha: f32) -> Self {
        Self {
            channels: [red, green, blue, alpha],
        }
    }

    #[must_use]
    pub const fn from_channels(channels: [f32; 4]) -> Self {
        Self { channels }
    }

    #[must_use]
    pub const fn channels(self) -> [f32; 4] {
        self.channels
    }

    #[must_use]
    pub const fn red(self) -> f32 {
        self.channels[0]
    }

    #[must_use]
    pub const fn green(self) -> f32 {
        self.channels[1]
    }

    #[must_use]
    pub const fn blue(self) -> f32 {
        self.channels[2]
    }

    #[must_use]
    pub const fn alpha(self) -> f32 {
        self.channels[3]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChannelMixerOperationMode {
    Rgb,
    Gray,
    HslV1,
    HslV2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelMixerExecutionError {
    Cancelled,
}

impl fmt::Display for ChannelMixerExecutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cancelled => formatter.write_str("Channel Mixer execution was cancelled"),
        }
    }
}

impl std::error::Error for ChannelMixerExecutionError {}

/// Immutable committed state equivalent to native `commit_params`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChannelMixerPlan {
    config: ChannelMixerConfig,
    hsl_matrix: [f32; 9],
    rgb_matrix: [f32; 9],
    operation_mode: ChannelMixerOperationMode,
}

impl ChannelMixerPlan {
    #[must_use]
    pub fn new(config: ChannelMixerConfig) -> Self {
        let parameters = config.parameters();
        let mut hsl_matrix = [0.0; 9];
        let mut rgb_matrix = [0.0; 9];
        let mut hsl_mix_mode = false;

        // HSL mixer matrix: source planes are columns and output channels are rows.
        for (output, offset) in HSL_OUTPUTS.zip([0, 3, 6]) {
            hsl_matrix[offset] = parameters.red[output];
            hsl_matrix[offset + 1] = parameters.green[output];
            hsl_matrix[offset + 2] = parameters.blue[output];
            hsl_mix_mode |= parameters.red[output] != 0.0
                || parameters.green[output] != 0.0
                || parameters.blue[output] != 0.0;
        }

        // RGB mixer matrix is committed before the gray recomputation.
        for (output, offset) in RGB_OUTPUTS.zip([0, 3, 6]) {
            rgb_matrix[offset] = parameters.red[output];
            rgb_matrix[offset + 1] = parameters.green[output];
            rgb_matrix[offset + 2] = parameters.blue[output];
        }

        let gray_mix = [
            parameters.red[GRAY],
            parameters.green[GRAY],
            parameters.blue[GRAY],
        ];
        let gray_mix_mode = gray_mix[0] != 0.0 || gray_mix[1] != 0.0 || gray_mix[2] != 0.0;
        if gray_mix_mode {
            let mixed_gray: [f32; 3] = std::array::from_fn(|index| {
                gray_mix[0] * rgb_matrix[index]
                    + gray_mix[1] * rgb_matrix[index + 3]
                    + gray_mix[2] * rgb_matrix[index + 6]
            });
            let (rows, remainder) = rgb_matrix.as_chunks_mut::<3>();
            debug_assert!(remainder.is_empty());
            for row in rows {
                row.copy_from_slice(&mixed_gray);
            }
        }

        let operation_mode = if config.algorithm_version() == ChannelMixerAlgorithm::V1 {
            ChannelMixerOperationMode::HslV1
        } else if hsl_mix_mode {
            ChannelMixerOperationMode::HslV2
        } else if gray_mix_mode {
            ChannelMixerOperationMode::Gray
        } else {
            ChannelMixerOperationMode::Rgb
        };

        Self {
            config,
            hsl_matrix,
            rgb_matrix,
            operation_mode,
        }
    }

    /// Checked equivalent of constructing a native committed piece from raw
    /// persisted parameters.
    pub fn commit_params(
        parameters: ChannelMixerParametersV2,
    ) -> Result<Self, ChannelMixerParameterError> {
        Ok(Self::new(ChannelMixerConfig::try_from(parameters)?))
    }

    #[must_use]
    pub const fn config(self) -> ChannelMixerConfig {
        self.config
    }

    #[must_use]
    pub const fn operation_mode(self) -> ChannelMixerOperationMode {
        self.operation_mode
    }

    #[must_use]
    pub const fn hsl_matrix(self) -> [f32; 9] {
        self.hsl_matrix
    }

    #[must_use]
    pub const fn rgb_matrix(self) -> [f32; 9] {
        self.rgb_matrix
    }

    /// Executes the native CPU point path and preserves the source alpha word.
    #[must_use]
    pub fn execute(&self, input: &[ChannelMixerPixel]) -> Vec<ChannelMixerPixel> {
        input
            .iter()
            .map(|pixel| self.process_pixel(*pixel))
            .collect()
    }

    /// Executes without publishing a partial output when cancellation is
    /// requested. The callback is polled before the first pixel and before each
    /// subsequent pixel, matching the leaf's bounded point-loop boundary.
    pub fn execute_with_cancellation<F>(
        &self,
        input: &[ChannelMixerPixel],
        mut cancelled: F,
    ) -> Result<Vec<ChannelMixerPixel>, ChannelMixerExecutionError>
    where
        F: FnMut() -> bool,
    {
        if cancelled() {
            return Err(ChannelMixerExecutionError::Cancelled);
        }
        let mut output = Vec::with_capacity(input.len());
        for pixel in input {
            if cancelled() {
                return Err(ChannelMixerExecutionError::Cancelled);
            }
            output.push(self.process_pixel(*pixel));
        }
        Ok(output)
    }

    /// Applies Darktable's bounded RGB normal blend. The fourth output lane is
    /// local coverage, not source image alpha; the unblended point path above
    /// is the alpha-preserving API.
    #[must_use]
    pub fn execute_normal_blend(
        &self,
        input: &[ChannelMixerPixel],
        mask: Option<&[f32]>,
        opacity: f32,
    ) -> Vec<ChannelMixerPixel> {
        debug_assert!(mask.is_none_or(|values| values.len() == input.len()));
        let candidates = self.execute(input);
        input
            .iter()
            .zip(candidates)
            .enumerate()
            .map(|(index, (source, candidate))| {
                let coverage = mask.map_or(opacity, |values| values[index] * opacity);
                let source = source.channels();
                let candidate = candidate.channels();
                ChannelMixerPixel::new(
                    clamp_simd(source[0] * (1.0 - coverage) + candidate[0] * coverage),
                    clamp_simd(source[1] * (1.0 - coverage) + candidate[1] * coverage),
                    clamp_simd(source[2] * (1.0 - coverage) + candidate[2] * coverage),
                    coverage,
                )
            })
            .collect()
    }

    /// Cancellation-aware normal blend with the same no-partial-publication
    /// guarantee as [`Self::execute_with_cancellation`].
    pub fn execute_normal_blend_with_cancellation<F>(
        &self,
        input: &[ChannelMixerPixel],
        mask: Option<&[f32]>,
        opacity: f32,
        cancelled: F,
    ) -> Result<Vec<ChannelMixerPixel>, ChannelMixerExecutionError>
    where
        F: FnMut() -> bool,
    {
        let candidates = self.execute_with_cancellation(input, cancelled)?;
        debug_assert!(mask.is_none_or(|values| values.len() == input.len()));
        Ok(input
            .iter()
            .zip(candidates)
            .enumerate()
            .map(|(index, (source, candidate))| {
                let coverage = mask.map_or(opacity, |values| values[index] * opacity);
                let source = source.channels();
                let candidate = candidate.channels();
                ChannelMixerPixel::new(
                    clamp_simd(source[0] * (1.0 - coverage) + candidate[0] * coverage),
                    clamp_simd(source[1] * (1.0 - coverage) + candidate[1] * coverage),
                    clamp_simd(source[2] * (1.0 - coverage) + candidate[2] * coverage),
                    coverage,
                )
            })
            .collect())
    }

    fn process_pixel(&self, pixel: ChannelMixerPixel) -> ChannelMixerPixel {
        let input = [pixel.red(), pixel.green(), pixel.blue()];
        let output = match self.operation_mode {
            ChannelMixerOperationMode::Rgb => [
                nonnegative(matrix_dot(self.rgb_matrix, 0, input)),
                nonnegative(matrix_dot(self.rgb_matrix, 1, input)),
                nonnegative(matrix_dot(self.rgb_matrix, 2, input)),
            ],
            ChannelMixerOperationMode::Gray => {
                let gray = nonnegative(matrix_dot(self.rgb_matrix, 0, input));
                [gray, gray, gray]
            }
            ChannelMixerOperationMode::HslV1 => self.process_hsl_v1(input),
            ChannelMixerOperationMode::HslV2 => self.process_hsl_v2(input),
        };
        ChannelMixerPixel::new(output[0], output[1], output[2], pixel.alpha())
    }

    fn process_hsl_v1(&self, input: [f32; 3]) -> [f32; 3] {
        // Native v1 clamps only the first term of each HSL row.
        let hmix = clamp_simd(input[0] * self.hsl_matrix[0])
            + input[1] * self.hsl_matrix[1]
            + input[2] * self.hsl_matrix[2];
        let smix = clamp_simd(input[0] * self.hsl_matrix[3])
            + input[1] * self.hsl_matrix[4]
            + input[2] * self.hsl_matrix[5];
        let lmix = clamp_simd(input[0] * self.hsl_matrix[6])
            + input[1] * self.hsl_matrix[7]
            + input[2] * self.hsl_matrix[8];

        let rgb = if hmix != 0.0 || smix != 0.0 || lmix != 0.0 {
            let (mut h, mut s, mut l) = rgb_to_hsl(input);
            if hmix != 0.0 {
                h = hmix;
            }
            if smix != 0.0 {
                s = smix;
            }
            if lmix != 0.0 {
                l = lmix;
            }
            hsl_to_rgb(h, s, l)
        } else {
            input
        };

        [
            clamp_simd(matrix_dot(self.rgb_matrix, 0, rgb)),
            clamp_simd(matrix_dot(self.rgb_matrix, 1, rgb)),
            clamp_simd(matrix_dot(self.rgb_matrix, 2, rgb)),
        ]
    }

    fn process_hsl_v2(&self, input: [f32; 3]) -> [f32; 3] {
        let hmix = clamp_simd(matrix_dot(self.hsl_matrix, 0, input));
        let smix = clamp_simd(matrix_dot(self.hsl_matrix, 1, input));
        let lmix = clamp_simd(matrix_dot(self.hsl_matrix, 2, input));

        let rgb = if hmix != 0.0 || smix != 0.0 || lmix != 0.0 {
            // Native rgb2hsl expects all values clipped in v2 only.
            let clipped = [
                clamp_simd(input[0]),
                clamp_simd(input[1]),
                clamp_simd(input[2]),
            ];
            let (mut h, mut s, mut l) = rgb_to_hsl(clipped);
            if hmix != 0.0 {
                h = hmix;
            }
            if smix != 0.0 {
                s = smix;
            }
            if lmix != 0.0 {
                l = lmix;
            }
            hsl_to_rgb(h, s, l)
        } else {
            input
        };

        [
            nonnegative(matrix_dot(self.rgb_matrix, 0, rgb)),
            nonnegative(matrix_dot(self.rgb_matrix, 1, rgb)),
            nonnegative(matrix_dot(self.rgb_matrix, 2, rgb)),
        ]
    }
}

/// Exact native `clamp_simd`, including its fail-closed NaN behavior.
fn clamp_simd(value: f32) -> f32 {
    if value > 0.0 {
        if value < 1.0 { value } else { 1.0 }
    } else {
        0.0
    }
}

/// Exact native `fmaxf(value, 0.0f)` boundary used by RGB and v2 paths.
fn nonnegative(value: f32) -> f32 {
    if value > 0.0 { value } else { 0.0 }
}

fn matrix_dot(matrix: [f32; 9], row: usize, input: [f32; 3]) -> f32 {
    let offset = row * 3;
    matrix[offset] * input[0] + matrix[offset + 1] * input[1] + matrix[offset + 2] * input[2]
}

fn c_fmax(first: f32, second: f32) -> f32 {
    if first.is_nan() {
        second
    } else if second.is_nan() || first > second {
        first
    } else if second > first {
        second
    } else if first == 0.0 {
        if first.is_sign_positive() {
            first
        } else {
            second
        }
    } else {
        first
    }
}

fn c_fmin(first: f32, second: f32) -> f32 {
    if first.is_nan() {
        second
    } else if second.is_nan() || first < second {
        first
    } else if second < first {
        second
    } else if first == 0.0 {
        if first.is_sign_negative() {
            first
        } else {
            second
        }
    } else {
        first
    }
}

/// Port of `src/common/colorspaces.h::rgb2hsl` in source operation order.
#[allow(
    clippy::float_cmp,
    clippy::manual_midpoint,
    reason = "the native colorspaces.h conversion uses exact comparisons and its overflow-visible grouping"
)]
fn rgb_to_hsl(rgb: [f32; 3]) -> (f32, f32, f32) {
    let pmax = c_fmax(c_fmax(rgb[0], rgb[1]), rgb[2]);
    let pmin = c_fmin(c_fmin(rgb[0], rgb[1]), rgb[2]);
    let delta = pmax - pmin;
    let mut hue = 0.0;
    let mut saturation = 0.0;
    let lightness = (pmin + pmax) / 2.0;

    if delta != 0.0 {
        saturation = if lightness < 0.5 {
            delta / c_fmax(pmax + pmin, MIN_HSL_DENOMINATOR)
        } else {
            delta / c_fmax(2.0 - pmax - pmin, MIN_HSL_DENOMINATOR)
        };

        if pmax == rgb[0] {
            hue = (rgb[1] - rgb[2]) / delta;
        } else if pmax == rgb[1] {
            hue = 2.0 + (rgb[2] - rgb[0]) / delta;
        } else if pmax == rgb[2] {
            hue = 4.0 + (rgb[0] - rgb[1]) / delta;
        }
        hue /= 6.0;
        if hue < 0.0 {
            hue += 1.0;
        } else if hue > 1.0 {
            hue -= 1.0;
        }
    }
    (hue, saturation, lightness)
}

fn hue_to_rgb(m1: f32, m2: f32, hue: f32) -> f32 {
    if hue < 1.0 {
        m1 + (m2 - m1) * hue
    } else if hue < 3.0 {
        m2
    } else if hue < 4.0 {
        m1 + (m2 - m1) * (4.0 - hue)
    } else {
        m1
    }
}

/// Port of `src/common/colorspaces.h::hsl2rgb` in source operation order.
fn hsl_to_rgb(mut hue: f32, saturation: f32, lightness: f32) -> [f32; 3] {
    if saturation == 0.0 {
        return [lightness; 3];
    }
    let m2 = if lightness < 0.5 {
        lightness * (1.0 + saturation)
    } else {
        lightness + saturation - lightness * saturation
    };
    let m1 = 2.0 * lightness - m2;
    hue *= 6.0;
    [
        hue_to_rgb(m1, m2, if hue < 4.0 { hue + 2.0 } else { hue - 4.0 }),
        hue_to_rgb(m1, m2, hue),
        hue_to_rgb(m1, m2, if hue > 2.0 { hue - 2.0 } else { hue + 4.0 }),
    ]
}

#[must_use]
///
/// # Panics
///
/// Never panics: the static descriptor identity is valid by construction.
pub fn channelmixer_descriptor() -> OperationDescriptor {
    let defaults = ChannelMixerParametersV2::defaults();
    OperationDescriptor {
        id: DescriptorId::new(
            CHANNEL_MIXER_COMPATIBILITY_ID,
            CHANNEL_MIXER_RUST_ID,
            CHANNEL_MIXER_SCHEMA_VERSION,
            CHANNEL_MIXER_SCHEMA_VERSION,
            1,
        )
        .expect("static Channel Mixer ID"),
        parameters: vec![
            array_parameter("red", defaults.red, 1),
            array_parameter("green", defaults.green, 1),
            array_parameter("blue", defaults.blue, 1),
            ParameterDescriptor {
                id: "algorithm_version".to_owned(),
                kind: ParameterKind::Enum {
                    tags: vec!["v1".to_owned(), "v2".to_owned()],
                },
                default: ParameterDefault::Enum("v2".to_owned()),
                required: false,
                introduced_version: CHANNEL_MIXER_SCHEMA_VERSION,
                removed_version: None,
                unit: None,
                step: None,
                precision: 0,
                role: ParameterRole::Processing,
                cache_affecting: true,
                animatable: false,
                ui_hint: None,
                condition: None,
            },
        ],
        flags: OperationFlags::DEPRECATED
            .insert(OperationFlags::STYLE_ELIGIBLE)
            .insert(OperationFlags::HISTORY_VISIBLE)
            .insert(OperationFlags::TILEABLE)
            .insert(OperationFlags::DETERMINISTIC_CPU)
            .insert(OperationFlags::COLOR)
            .insert(OperationFlags::MASKS)
            .insert(OperationFlags::BLENDING),
        stage: "display-referred-rgb".to_owned(),
        roi: RoiKind::Identity,
        tiling: TilingContract {
            overlap_pixels: 0,
            alignment_pixels: 1,
            minimum_tile_edge: 1,
            preferred_tile_edge: 256,
            temporary_multiplier_milli: 1000,
            input_multiplier_milli: 1000,
            output_multiplier_milli: 1000,
        },
        capability: CapabilityContract {
            cpu_supported: true,
            gpu_tier: None,
            required_features: vec![],
            required_formats: vec!["rgba32float".to_owned()],
            deterministic_cpu: true,
            deterministic_gpu: false,
            fallback_to_cpu: true,
            precision: "f32 RGB matrix with native HSL and lower/upper clamps".to_owned(),
            modes: vec!["preview".to_owned(), "full".to_owned(), "export".to_owned()],
        },
        io: rgb_io(),
        mask_blend: MaskBlendContract {
            consumes_mask: true,
            publishes_mask: false,
            blend_if: true,
            geometry: false,
            analysis: false,
        },
        migration: MigrationContract {
            source_versions: vec![1, CHANNEL_MIXER_SCHEMA_VERSION],
            target_version: CHANNEL_MIXER_SCHEMA_VERSION,
            opaque_unknown_allowed: true,
        },
        ui: Some(UiHint {
            label_key: "operation.channelmixer".to_owned(),
            group_key: "group.grading".to_owned(),
            control: "channelmixer".to_owned(),
        }),
    }
}

fn array_parameter(
    id: &str,
    default: [f32; CHANNEL_MIXER_OUTPUT_COUNT],
    introduced_version: u16,
) -> ParameterDescriptor {
    ParameterDescriptor {
        id: id.to_owned(),
        kind: ParameterKind::Vector {
            dimensions: u8::try_from(CHANNEL_MIXER_OUTPUT_COUNT)
                .expect("Channel Mixer has seven vector dimensions"),
            minimum: f64::from(CHANNEL_MIXER_PARAMETER_MIN),
            maximum: f64::from(CHANNEL_MIXER_PARAMETER_MAX),
        },
        default: ParameterDefault::Vector(default.into_iter().map(f64::from).collect()),
        required: false,
        introduced_version,
        removed_version: None,
        unit: Some("factor".to_owned()),
        step: Some(0.001),
        precision: 3,
        role: ParameterRole::Processing,
        cache_affecting: true,
        animatable: true,
        ui_hint: Some("slider".to_owned()),
        condition: None,
    }
}

fn rgb_io() -> InputOutputContract {
    let image = ImagePredicate {
        channels: 4,
        alpha: AlphaPolicy::Preserve,
        encodings: vec![ColorEncoding::LinearSrgbD65],
        nonfinite: NonFinitePolicy::Reject,
    };
    InputOutputContract {
        input: image.clone(),
        output: image,
        derives_output_encoding: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn values(seed: f32) -> [f32; CHANNEL_MIXER_OUTPUT_COUNT] {
        std::array::from_fn(|index| seed + index as f32)
    }

    #[test]
    fn native_byte_layout_is_three_arrays_then_i32_algorithm() {
        let parameters = ChannelMixerParametersV2::new(
            values(1.0),
            values(11.0),
            values(21.0),
            ChannelMixerAlgorithm::V2,
        );
        let bytes = parameters.to_bytes();
        assert_eq!(CHANNEL_MIXER_V1_PARAMETER_BYTES, 84);
        assert_eq!(CHANNEL_MIXER_V2_PARAMETER_BYTES, 88);
        assert_eq!(&bytes[0..4], &1.0_f32.to_le_bytes());
        assert_eq!(&bytes[28..32], &11.0_f32.to_le_bytes());
        assert_eq!(&bytes[56..60], &21.0_f32.to_le_bytes());
        assert_eq!(&bytes[84..88], &1_i32.to_le_bytes());
        assert_eq!(ChannelMixerParametersV2::from_bytes(&bytes), Ok(parameters));

        let legacy = ChannelMixerParametersV1::new(values(-1.0), values(7.0), values(15.0));
        let legacy_bytes = legacy.to_bytes();
        assert_eq!(legacy_bytes.len(), CHANNEL_MIXER_V1_PARAMETER_BYTES);
        assert_eq!(
            ChannelMixerParametersV1::from_bytes(&legacy_bytes),
            Ok(legacy)
        );
    }

    #[test]
    fn v1_migration_copies_hsl_and_conditionally_copies_rgb() {
        let mut red = values(1.0);
        let mut green = values(11.0);
        let mut blue = values(21.0);
        red[GRAY] = 0.0;
        green[GRAY] = -0.0;
        blue[GRAY] = 0.0;
        let migrated = migrate_v1_to_v2(ChannelMixerParametersV1::new(red, green, blue));
        assert_eq!(migrated.algorithm_version, ChannelMixerAlgorithm::V1);
        assert_eq!(&migrated.red[0..3], &red[0..3]);
        assert_eq!(&migrated.green[RED..=BLUE], &green[RED..=BLUE]);
        assert_eq!(&migrated.blue[RED..=BLUE], &blue[RED..=BLUE]);

        red[GRAY] = 0.25;
        green[RED] = 99.0;
        let migrated_gray = migrate_v1_to_v2(ChannelMixerParametersV1::new(red, green, blue));
        assert_eq!(migrated_gray.red[GRAY], red[GRAY]);
        assert_eq!(migrated_gray.green[GRAY], green[GRAY]);
        assert_eq!(migrated_gray.green[RED], 0.0);
    }

    #[test]
    fn defaults_and_source_ranges_are_descriptor_visible() {
        let defaults = ChannelMixerParametersV2::defaults();
        assert_eq!(defaults.red[RED], 1.0);
        assert_eq!(defaults.green[GREEN], 1.0);
        assert_eq!(defaults.blue[BLUE], 1.0);
        assert_eq!(defaults.algorithm_version, ChannelMixerAlgorithm::V2);

        let descriptor = channelmixer_descriptor();
        descriptor.validate().expect("descriptor");
        assert_eq!(descriptor.parameters.len(), 4);
        for parameter in &descriptor.parameters[..3] {
            assert!(matches!(
                parameter.kind,
                ParameterKind::Vector {
                    dimensions: 7,
                    minimum: -1.0,
                    maximum: 1.0
                }
            ));
        }
        assert!(matches!(
            descriptor.parameters[3].kind,
            ParameterKind::Enum { .. }
        ));
        assert_eq!(descriptor.io.input.channels, 4);
        assert_eq!(descriptor.io.input.alpha, AlphaPolicy::Preserve);
        assert_eq!(
            descriptor.io.input.encodings,
            [ColorEncoding::LinearSrgbD65]
        );
        assert_eq!(descriptor.io.input.nonfinite, NonFinitePolicy::Reject);
    }

    #[test]
    fn defaults_are_identity_and_preserve_alpha_bits() {
        let plan = ChannelMixerPlan::new(ChannelMixerConfig::defaults());
        assert_eq!(plan.operation_mode(), ChannelMixerOperationMode::Rgb);
        let source = ChannelMixerPixel::new(0.2, 0.4, 0.8, f32::from_bits(0x3f41_2345));
        let output = plan.execute(&[source])[0];
        assert_eq!(output.red(), source.red());
        assert_eq!(output.green(), source.green());
        assert_eq!(output.blue(), source.blue());
        assert_eq!(output.alpha().to_bits(), source.alpha().to_bits());
    }

    #[test]
    fn rgb_gray_and_hsl_modes_follow_commit_precedence() {
        let mut red = [0.0; CHANNEL_MIXER_OUTPUT_COUNT];
        let mut green = [0.0; CHANNEL_MIXER_OUTPUT_COUNT];
        let mut blue = [0.0; CHANNEL_MIXER_OUTPUT_COUNT];
        red[RED] = 1.0;
        green[GREEN] = 1.0;
        blue[BLUE] = 1.0;
        red[GRAY] = 1.0;
        let gray_plan = ChannelMixerPlan::commit_params(ChannelMixerParametersV2::new(
            red,
            green,
            blue,
            ChannelMixerAlgorithm::V2,
        ))
        .expect("finite parameters");
        assert_eq!(gray_plan.operation_mode(), ChannelMixerOperationMode::Gray);
        red[HUE] = 0.25;
        let hsl_plan = ChannelMixerPlan::commit_params(ChannelMixerParametersV2::new(
            red,
            green,
            blue,
            ChannelMixerAlgorithm::V2,
        ))
        .expect("finite parameters");
        assert_eq!(hsl_plan.operation_mode(), ChannelMixerOperationMode::HslV2);
        let legacy_plan = ChannelMixerPlan::commit_params(ChannelMixerParametersV2::new(
            red,
            green,
            blue,
            ChannelMixerAlgorithm::V1,
        ))
        .expect("finite parameters");
        assert_eq!(
            legacy_plan.operation_mode(),
            ChannelMixerOperationMode::HslV1
        );
    }

    #[test]
    fn gray_commit_replaces_every_rgb_output_row_with_the_mixed_gray_row() {
        let mut red = [0.0; CHANNEL_MIXER_OUTPUT_COUNT];
        let mut green = [0.0; CHANNEL_MIXER_OUTPUT_COUNT];
        let mut blue = [0.0; CHANNEL_MIXER_OUTPUT_COUNT];
        red[RED] = 1.0;
        green[GREEN] = 1.0;
        blue[BLUE] = 1.0;
        red[GRAY] = 0.2;
        green[GRAY] = 0.3;
        blue[GRAY] = 0.5;
        let plan = ChannelMixerPlan::commit_params(ChannelMixerParametersV2::new(
            red,
            green,
            blue,
            ChannelMixerAlgorithm::V2,
        ))
        .expect("finite parameters");
        assert_eq!(
            plan.rgb_matrix(),
            [0.2, 0.3, 0.5, 0.2, 0.3, 0.5, 0.2, 0.3, 0.5]
        );
        let output = plan.execute(&[ChannelMixerPixel::new(0.1, 0.4, 0.8, 1.0)])[0];
        assert_eq!(output.channels(), [0.54, 0.54, 0.54, 1.0]);
    }

    #[test]
    fn hsl_v1_clamps_the_first_term_while_hsl_v2_clamps_the_complete_dot() {
        let mut red = [0.0; CHANNEL_MIXER_OUTPUT_COUNT];
        let mut green = [0.0; CHANNEL_MIXER_OUTPUT_COUNT];
        let mut blue = [0.0; CHANNEL_MIXER_OUTPUT_COUNT];
        red[HUE] = 2.0;
        green[HUE] = -1.0;
        red[RED] = 1.0;
        green[GREEN] = 1.0;
        blue[BLUE] = 1.0;
        let input = ChannelMixerPixel::new(0.8, 0.2, 0.1, 0.75);
        let legacy = ChannelMixerPlan::commit_params(ChannelMixerParametersV2::new(
            red,
            green,
            blue,
            ChannelMixerAlgorithm::V1,
        ))
        .expect("finite parameters");
        let current = ChannelMixerPlan::commit_params(ChannelMixerParametersV2::new(
            red,
            green,
            blue,
            ChannelMixerAlgorithm::V2,
        ))
        .expect("finite parameters");
        let (_, saturation, lightness) = rgb_to_hsl([input.red(), input.green(), input.blue()]);
        assert_eq!(
            legacy.execute(&[input])[0].channels(),
            [
                clamp_simd(hsl_to_rgb(0.8, saturation, lightness)[0]),
                clamp_simd(hsl_to_rgb(0.8, saturation, lightness)[1]),
                clamp_simd(hsl_to_rgb(0.8, saturation, lightness)[2]),
                input.alpha(),
            ]
        );
        assert_eq!(
            current.execute(&[input])[0].channels(),
            [
                nonnegative(hsl_to_rgb(1.0, saturation, lightness)[0]),
                nonnegative(hsl_to_rgb(1.0, saturation, lightness)[1]),
                nonnegative(hsl_to_rgb(1.0, saturation, lightness)[2]),
                input.alpha(),
            ]
        );
    }

    #[test]
    fn edge_values_are_lower_clamped_but_not_upper_clamped_in_rgb_mode() {
        let mut red = [0.0; CHANNEL_MIXER_OUTPUT_COUNT];
        let mut green = [0.0; CHANNEL_MIXER_OUTPUT_COUNT];
        let mut blue = [0.0; CHANNEL_MIXER_OUTPUT_COUNT];
        red[RED] = 2.0;
        green[GREEN] = 2.0;
        blue[BLUE] = 2.0;
        let plan = ChannelMixerPlan::new(
            ChannelMixerConfig::new(red, green, blue, ChannelMixerAlgorithm::V2)
                .expect("finite parameters"),
        );
        let output = plan.execute(&[ChannelMixerPixel::new(-1.0, 0.5, 0.75, 0.25)])[0];
        assert_eq!(output.red(), 0.0);
        assert_eq!(output.green(), 1.0);
        assert_eq!(output.blue(), 1.5);
    }

    #[test]
    fn nonfinite_parameters_and_malformed_payloads_fail_closed() {
        let mut red = [0.0; CHANNEL_MIXER_OUTPUT_COUNT];
        red[2] = f32::NAN;
        assert_eq!(
            ChannelMixerConfig::new(
                red,
                [0.0; CHANNEL_MIXER_OUTPUT_COUNT],
                [0.0; CHANNEL_MIXER_OUTPUT_COUNT],
                ChannelMixerAlgorithm::V2,
            ),
            Err(ChannelMixerParameterError::NonFinite {
                plane: "red",
                index: 2
            })
        );
        assert!(matches!(
            ChannelMixerParametersV2::from_bytes(&[0; 87]),
            Err(ChannelMixerCodecError::InvalidLength { .. })
        ));
        let mut bytes = ChannelMixerParametersV2::defaults().to_bytes();
        bytes[84..].copy_from_slice(&99_i32.to_le_bytes());
        assert_eq!(
            ChannelMixerParametersV2::from_bytes(&bytes),
            Err(ChannelMixerCodecError::UnknownAlgorithm(99))
        );
        let opaque = ChannelMixerHistory::decode(9, &[1, 2, 3]).expect("opaque history");
        assert!(opaque.current().is_err());
    }

    #[test]
    fn nonfinite_input_reaches_native_clamps_without_corrupting_alpha() {
        let plan = ChannelMixerPlan::new(ChannelMixerConfig::defaults());
        let source = ChannelMixerPixel::new(f32::NAN, f32::INFINITY, -f32::INFINITY, 0.5);
        let output = plan.execute(&[source])[0];
        // The native dot products still evaluate `0 * infinity` before
        // `fmaxf`, so the NaN result is fail-closed to zero in every lane.
        assert_eq!(output.channels(), [0.0, 0.0, 0.0, 0.5]);
    }

    #[test]
    fn bounded_normal_blend_uses_mask_coverage_in_alpha() {
        let plan = ChannelMixerPlan::new(ChannelMixerConfig::defaults());
        let source = ChannelMixerPixel::new(0.0, 0.2, 0.4, 0.9);
        let output = plan.execute_normal_blend(&[source], Some(&[0.5]), 0.5)[0];
        assert_eq!(output.channels(), [0.0, 0.2, 0.4, 0.25]);
    }

    #[test]
    fn cancellation_publishes_no_partial_result() {
        let plan = ChannelMixerPlan::new(ChannelMixerConfig::defaults());
        let input = vec![ChannelMixerPixel::new(0.1, 0.2, 0.3, 0.4); 8];
        let mut polls = 0;
        let result = plan.execute_with_cancellation(&input, || {
            polls += 1;
            polls > 2
        });
        assert_eq!(result, Err(ChannelMixerExecutionError::Cancelled));
        assert!(polls >= 3);
    }

    #[test]
    fn deterministic_execution_repeats_the_same_float_words() {
        let mut red = [0.0; CHANNEL_MIXER_OUTPUT_COUNT];
        let mut green = [0.0; CHANNEL_MIXER_OUTPUT_COUNT];
        let mut blue = [0.0; CHANNEL_MIXER_OUTPUT_COUNT];
        red[RED] = 0.4;
        red[GREEN] = 0.3;
        green[RED] = 0.2;
        green[GREEN] = 0.8;
        blue[BLUE] = 1.0;
        let plan = ChannelMixerPlan::new(
            ChannelMixerConfig::new(red, green, blue, ChannelMixerAlgorithm::V2)
                .expect("finite parameters"),
        );
        let input = [
            ChannelMixerPixel::new(0.1, 0.6, 0.8, 0.2),
            ChannelMixerPixel::new(0.9, 0.2, 0.3, 0.7),
        ];
        let first = plan.execute(&input);
        let second = plan.execute(&input);
        assert_eq!(
            first
                .iter()
                .flat_map(|pixel| pixel.channels().map(f32::to_bits))
                .collect::<Vec<_>>(),
            second
                .iter()
                .flat_map(|pixel| pixel.channels().map(f32::to_bits))
                .collect::<Vec<_>>()
        );
    }
}
