#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    reason = "the shared f32 raster/noise boundary intentionally narrows checked coordinates"
)]

use std::fmt;

use sha2::{Digest, Sha256};

use crate::{FiniteF32, LinearRgb, RasterDimensions, RgbChannel};

/// Maximum native bytes encoded by one core text parameter.
pub(crate) const NATIVE_PAYLOAD_CHUNK_BYTES: usize = 2_048;

pub(crate) fn encode_native_payload_chunks(bytes: &[u8]) -> Vec<String> {
    bytes
        .chunks(NATIVE_PAYLOAD_CHUNK_BYTES)
        .map(|chunk| {
            let mut encoded = String::with_capacity(chunk.len() * 2);
            for byte in chunk {
                use std::fmt::Write as _;
                write!(encoded, "{byte:02x}").expect("writing to a String cannot fail");
            }
            encoded
        })
        .collect()
}

pub(crate) fn decode_native_payload_chunks(
    chunks: &[&str],
    expected_bytes: usize,
) -> Result<Vec<u8>, String> {
    let encoded_bytes = chunks.iter().try_fold(0_usize, |total, chunk| {
        if chunk.len() % 2 != 0 {
            return Err("native payload chunks must contain even-length hexadecimal".to_owned());
        }
        total
            .checked_add(chunk.len())
            .ok_or_else(|| "native payload chunk length overflowed".to_owned())
    })?;
    if encoded_bytes != expected_bytes.saturating_mul(2) {
        return Err(format!(
            "native payload encodes {} bytes; expected {expected_bytes}",
            encoded_bytes / 2
        ));
    }
    let mut output = Vec::new();
    output
        .try_reserve_exact(expected_bytes)
        .map_err(|_| format!("native payload could not allocate {expected_bytes} bytes"))?;
    for chunk in chunks {
        let (pairs, remainder) = chunk.as_bytes().as_chunks::<2>();
        debug_assert!(remainder.is_empty());
        for pair in pairs {
            let high = hex_digit(pair[0])
                .ok_or_else(|| "native payload contains non-hexadecimal text".to_owned())?;
            let low = hex_digit(pair[1])
                .ok_or_else(|| "native payload contains non-hexadecimal text".to_owned())?;
            output.push((high << 4) | low);
        }
    }
    Ok(output)
}

const fn hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

pub(crate) fn rgb_to_hsl(rgb: [f32; 3]) -> (f32, f32, f32) {
    let [red, green, blue] = rgb;
    let maximum = red.max(green).max(blue);
    let minimum = red.min(green).min(blue);
    let delta = maximum - minimum;
    let lightness = f32::midpoint(minimum, maximum);
    if delta.to_bits() == 0.0_f32.to_bits() {
        return (0.0, 0.0, lightness);
    }
    let denominator = if lightness < 0.5 {
        (maximum + minimum).max(1.525_878_9e-5)
    } else {
        (2.0 - maximum - minimum).max(1.525_878_9e-5)
    };
    let saturation = delta / denominator;
    let mut hue = if maximum.to_bits() == red.to_bits() {
        (green - blue) / delta
    } else if maximum.to_bits() == green.to_bits() {
        2.0 + (blue - red) / delta
    } else {
        4.0 + (red - green) / delta
    } / 6.0;
    if hue < 0.0 {
        hue += 1.0;
    } else if hue > 1.0 {
        hue -= 1.0;
    }
    (hue, saturation, lightness)
}

#[must_use]
pub(crate) fn hsl_to_rgb(hue: f32, saturation: f32, lightness: f32) -> [f32; 3] {
    if saturation.to_bits() == 0.0_f32.to_bits() {
        return [lightness; 3];
    }
    let second = if lightness < 0.5 {
        lightness * (1.0 + saturation)
    } else {
        lightness + saturation - lightness * saturation
    };
    let first = 2.0 * lightness - second;
    let angle = hue * 6.0;
    [
        hue_to_rgb(
            first,
            second,
            if angle < 4.0 {
                angle + 2.0
            } else {
                angle - 4.0
            },
        ),
        hue_to_rgb(first, second, angle),
        hue_to_rgb(
            first,
            second,
            if angle > 2.0 {
                angle - 2.0
            } else {
                angle + 4.0
            },
        ),
    ]
}

fn hue_to_rgb(first: f32, second: f32, hue: f32) -> f32 {
    if hue < 1.0 {
        first + (second - first) * hue
    } else if hue < 3.0 {
        second
    } else if hue < 4.0 {
        first + (second - first) * (4.0 - hue)
    } else {
        first
    }
}

/// The mutable two-word TEA state used by Darktable's per-operation dither.
///
/// The native implementation zero-initializes one state per worker, replaces
/// word zero at the start of each row, and carries word one through every row.
/// Rust executes the canonical CPU operation serially, so one state represents
/// the operation's worker state without exposing scheduling-dependent output.
#[derive(Debug, Clone, Copy)]
pub(crate) struct TeaState {
    words: [u32; 2],
}

impl TeaState {
    #[must_use]
    pub(crate) const fn new() -> Self {
        Self { words: [0; 2] }
    }

    pub(crate) fn set_row(&mut self, row: usize, raster_height: u32) {
        let row = u32::try_from(row).expect("validated row fits native unsigned int");
        self.words[0] = row.wrapping_mul(raster_height);
    }

    pub(crate) fn encrypt_and_tpdf(&mut self) -> f32 {
        encrypt_tea(&mut self.words);
        tpdf(self.words[0])
    }
}

fn encrypt_tea(state: &mut [u32; 2]) {
    const KEY: [u32; 4] = [0xa341_316c, 0xc801_3ea4, 0xad90_777d, 0x7e95_761e];
    let mut sum = 0_u32;
    for _ in 0..8 {
        sum = sum.wrapping_add(0x9e37_79b9);
        state[0] = state[0].wrapping_add(
            ((state[1] << 4).wrapping_add(KEY[0]))
                ^ state[1].wrapping_add(sum)
                ^ ((state[1] >> 5).wrapping_add(KEY[1])),
        );
        state[1] = state[1].wrapping_add(
            ((state[0] << 4).wrapping_add(KEY[2]))
                ^ state[0].wrapping_add(sum)
                ^ ((state[0] >> 5).wrapping_add(KEY[3])),
        );
    }
}

fn tpdf(random: u32) -> f32 {
    let fraction = random as f32 / u32::MAX as f32;
    if fraction < 0.5 {
        (2.0 * fraction).sqrt() - 1.0
    } else {
        1.0 - (2.0 * (1.0 - fraction)).sqrt()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationExecutionError {
    Cancelled,
    MemoryBudgetExceeded { required: usize, budget: usize },
    AllocationFailed { required: usize },
    DimensionsMismatch { expected: usize, actual: usize },
    NonFiniteResult { pixel: usize, channel: RgbChannel },
    NoReconstructionEvidence,
    UnsupportedCapability(&'static str),
    GeometryRequiresFrameBoundary,
}

impl fmt::Display for OperationExecutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cancelled => formatter.write_str("operation execution was cancelled"),
            Self::MemoryBudgetExceeded { required, budget } => {
                write!(
                    formatter,
                    "operation requires {required} bytes, budget is {budget}"
                )
            }
            Self::AllocationFailed { required } => {
                write!(
                    formatter,
                    "operation could not allocate a required {required}-byte buffer"
                )
            }
            Self::DimensionsMismatch { expected, actual } => {
                write!(
                    formatter,
                    "operation expected {expected} pixels, got {actual}"
                )
            }
            Self::NonFiniteResult { pixel, channel } => {
                write!(
                    formatter,
                    "operation produced a non-finite {channel:?} at pixel {pixel}"
                )
            }
            Self::NoReconstructionEvidence => {
                formatter.write_str("operation found no trustworthy reconstruction evidence")
            }
            Self::UnsupportedCapability(reason) => {
                write!(formatter, "unsupported operation capability: {reason}")
            }
            Self::GeometryRequiresFrameBoundary => formatter
                .write_str("geometry operation requires a frame-boundary pixelpipe execution"),
        }
    }
}

impl std::error::Error for OperationExecutionError {}

#[derive(Debug, Clone, PartialEq)]
pub struct ReconstructionDiagnostics {
    pub(crate) affected: Vec<bool>,
    pub(crate) candidate: Vec<bool>,
    pub(crate) confidence: Vec<f32>,
    pub(crate) contribution: Vec<LinearRgb>,
}

impl ReconstructionDiagnostics {
    pub(crate) fn new(pixel_count: usize) -> Self {
        let zero = LinearRgb::new(
            FiniteF32::new(0.0).expect("zero is finite"),
            FiniteF32::new(0.0).expect("zero is finite"),
            FiniteF32::new(0.0).expect("zero is finite"),
        );
        Self {
            affected: vec![false; pixel_count],
            candidate: vec![false; pixel_count],
            confidence: vec![0.0; pixel_count],
            contribution: vec![zero; pixel_count],
        }
    }

    #[must_use]
    pub fn affected(&self) -> &[bool] {
        &self.affected
    }

    #[must_use]
    pub fn candidate(&self) -> &[bool] {
        &self.candidate
    }

    #[must_use]
    pub fn confidence(&self) -> &[f32] {
        &self.confidence
    }

    #[must_use]
    pub fn contribution(&self) -> &[LinearRgb] {
        &self.contribution
    }

    pub(crate) fn affected_count(&self) -> usize {
        self.affected.iter().filter(|value| **value).count()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReconstructionBudget {
    maximum_bytes: usize,
}

impl ReconstructionBudget {
    #[must_use]
    pub const fn new(maximum_bytes: usize) -> Self {
        Self { maximum_bytes }
    }

    #[must_use]
    pub const fn maximum_bytes(self) -> usize {
        self.maximum_bytes
    }
}

impl Default for ReconstructionBudget {
    fn default() -> Self {
        Self::new(512 * 1024 * 1024)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReconstructionReceipt {
    compatibility_name: &'static str,
    schema_version: u16,
    input_digest: [u8; 32],
    output_digest: [u8; 32],
    affected_pixels: usize,
    candidate_pixels: usize,
}

impl ReconstructionReceipt {
    pub(crate) fn new(
        compatibility_name: &'static str,
        schema_version: u16,
        input: &[LinearRgb],
        output: &[LinearRgb],
        diagnostics: &ReconstructionDiagnostics,
    ) -> Self {
        Self {
            compatibility_name,
            schema_version,
            input_digest: digest_pixels(input),
            output_digest: digest_pixels(output),
            affected_pixels: diagnostics.affected_count(),
            candidate_pixels: diagnostics.candidate.iter().filter(|value| **value).count(),
        }
    }

    #[must_use]
    pub const fn compatibility_name(&self) -> &'static str {
        self.compatibility_name
    }

    #[must_use]
    pub const fn schema_version(&self) -> u16 {
        self.schema_version
    }

    #[must_use]
    pub const fn input_digest(&self) -> [u8; 32] {
        self.input_digest
    }

    #[must_use]
    pub const fn output_digest(&self) -> [u8; 32] {
        self.output_digest
    }

    #[must_use]
    pub const fn affected_pixels(&self) -> usize {
        self.affected_pixels
    }

    #[must_use]
    pub const fn candidate_pixels(&self) -> usize {
        self.candidate_pixels
    }
}

pub(crate) fn validate_shape(
    dimensions: RasterDimensions,
    pixels: &[LinearRgb],
) -> Result<(), OperationExecutionError> {
    let expected = usize::try_from(dimensions.pixel_count()).map_err(|_| {
        OperationExecutionError::DimensionsMismatch {
            expected: usize::MAX,
            actual: pixels.len(),
        }
    })?;
    if expected == pixels.len() {
        Ok(())
    } else {
        Err(OperationExecutionError::DimensionsMismatch {
            expected,
            actual: pixels.len(),
        })
    }
}

pub(crate) fn checked_bytes(
    pixel_count: usize,
    buffers: usize,
    budget: ReconstructionBudget,
) -> Result<(), OperationExecutionError> {
    let required = pixel_count
        .checked_mul(buffers)
        .and_then(|value| value.checked_mul(std::mem::size_of::<LinearRgb>()))
        .and_then(|value| value.checked_add(pixel_count.saturating_mul(16)))
        .ok_or(OperationExecutionError::MemoryBudgetExceeded {
            required: usize::MAX,
            budget: budget.maximum_bytes(),
        })?;
    if required <= budget.maximum_bytes() {
        Ok(())
    } else {
        Err(OperationExecutionError::MemoryBudgetExceeded {
            required,
            budget: budget.maximum_bytes(),
        })
    }
}

pub(crate) fn luma(pixel: LinearRgb) -> f32 {
    0.2126 * pixel.red().get() + 0.7152 * pixel.green().get() + 0.0722 * pixel.blue().get()
}

pub(crate) fn chroma(pixel: LinearRgb) -> (f32, f32) {
    let lightness = luma(pixel);
    (
        pixel.red().get() - lightness,
        pixel.blue().get() - lightness,
    )
}

pub(crate) fn from_luma_chroma(lightness: f32, chroma: (f32, f32)) -> Option<LinearRgb> {
    let red = lightness + chroma.0;
    let blue = lightness + chroma.1;
    let green = (lightness - 0.2126 * red - 0.0722 * blue) / 0.7152;
    let values = [red, green, blue];
    if values.iter().all(|value| value.is_finite()) {
        Some(LinearRgb::new(
            FiniteF32::new(red).ok()?,
            FiniteF32::new(green).ok()?,
            FiniteF32::new(blue).ok()?,
        ))
    } else {
        None
    }
}

pub(crate) fn neighborhood(
    dimensions: RasterDimensions,
    index: usize,
    radius: u32,
) -> impl Iterator<Item = usize> {
    let width = usize::try_from(dimensions.width()).expect("validated width fits usize");
    let height = dimensions.height();
    let x = index % width;
    let y = index / width;
    let x0 = x.saturating_sub(usize::try_from(radius).expect("radius fits usize"));
    let y0 = y.saturating_sub(usize::try_from(radius).expect("radius fits usize"));
    let x1 = x
        .saturating_add(usize::try_from(radius).expect("radius fits usize"))
        .min(width.saturating_sub(1));
    let y1 = y
        .saturating_add(usize::try_from(radius).expect("radius fits usize"))
        .min(
            usize::try_from(height)
                .expect("height fits usize")
                .saturating_sub(1),
        );
    (y0..=y1).flat_map(move |row| {
        (x0..=x1).map(move |column| row.saturating_mul(width).saturating_add(column))
    })
}

pub(crate) fn apply_opacity(
    source: LinearRgb,
    candidate: LinearRgb,
    opacity: f32,
) -> Result<LinearRgb, ()> {
    let values = [
        source.red().get() + (candidate.red().get() - source.red().get()) * opacity,
        source.green().get() + (candidate.green().get() - source.green().get()) * opacity,
        source.blue().get() + (candidate.blue().get() - source.blue().get()) * opacity,
    ];
    Some(LinearRgb::new(
        FiniteF32::new(values[0]).map_err(|_| ())?,
        FiniteF32::new(values[1]).map_err(|_| ())?,
        FiniteF32::new(values[2]).map_err(|_| ())?,
    ))
    .ok_or(())
}

pub(crate) fn digest_pixels(pixels: &[LinearRgb]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"rusttable.reconstruction.raster.v1");
    for pixel in pixels {
        hasher.update(pixel.red().get().to_bits().to_le_bytes());
        hasher.update(pixel.green().get().to_bits().to_le_bytes());
        hasher.update(pixel.blue().get().to_bits().to_le_bytes());
    }
    hasher.finalize().into()
}
