//! Scalar transfer functions with one explicit range and non-finite contract.
//!
//! Constants and piecewise equations follow IEC 61966-2-1 as reproduced by
//! [CSS Color 4](https://www.w3.org/TR/css-color-4/#color-conversion-code),
//! ITU-R BT.709-6, ITU-R BT.2020-2, and ITU-R BT.2100-3 Tables 4 and 5.
//! PQ values are normalized so linear `1.0` means 10,000 cd/m². HLG is the
//! scene-light OETF/inverse OETF; display OOTF policy is deliberately excluded.

use crate::{FiniteF32, TransferFunction};
use rusttable_core::numerics::{NonFinitePolicy, ordered::ordered_clamp_f32};
use std::fmt;

const SRGB_LINEAR_BREAK: f32 = 0.003_130_8;
const SRGB_ENCODED_BREAK: f32 = 0.040_45;
const REC709_ALPHA: f32 = 1.099;
const REC709_BETA: f32 = 0.018;
// Continuous BT.2020 curve coefficients, avoiding the rounded 10/12-bit join.
const REC2020_ALPHA: f32 = 1.099_296_8;
const REC2020_BETA: f32 = 0.018_053_97;

const PQ_M1: f32 = 2_610.0 / 16_384.0;
const PQ_M2: f32 = 2_523.0 / 4_096.0 * 128.0;
const PQ_C1: f32 = 3_424.0 / 4_096.0;
const PQ_C2: f32 = 2_413.0 / 4_096.0 * 32.0;
const PQ_C3: f32 = 2_392.0 / 4_096.0 * 32.0;

const HLG_A: f32 = 0.178_832_77;
const HLG_B: f32 = 0.284_668_92;
const HLG_C: f32 = 0.559_910_7;

/// Input-domain behavior selected for one transfer evaluation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferRange {
    /// Odd extension of SDR curves to negative and HDR values without clipping.
    Extended,
    /// Require an input in the standard normalized `[0, 1]` interval.
    StrictUnit,
    /// Clamp finite input to `[0, 1]` before evaluating the standard curve.
    ClampUnit,
}

/// Complete transfer evaluation policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransferPolicy {
    pub range: TransferRange,
    pub non_finite: NonFinitePolicy,
}

impl TransferPolicy {
    #[must_use]
    pub const fn new(range: TransferRange, non_finite: NonFinitePolicy) -> Self {
        Self { range, non_finite }
    }

    #[must_use]
    pub const fn extended() -> Self {
        Self::new(TransferRange::Extended, NonFinitePolicy::Reject)
    }

    #[must_use]
    pub const fn strict_unit() -> Self {
        Self::new(TransferRange::StrictUnit, NonFinitePolicy::Reject)
    }

    #[must_use]
    pub const fn clamped_unit() -> Self {
        Self::new(TransferRange::ClampUnit, NonFinitePolicy::Reject)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferDirection {
    Encode,
    Decode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferFunctionError {
    NonFinite,
    InvalidGamma,
    OutOfRange,
    UnsupportedExtendedRange,
    Overflow,
    LengthMismatch,
}

impl TransferFunction {
    /// Defines a pure power transfer with a finite positive exponent.
    pub fn gamma(value: f32) -> Result<Self, TransferFunctionError> {
        let value = FiniteF32::new(value).map_err(|_| TransferFunctionError::NonFinite)?;
        if value.get() <= 0.0 {
            return Err(TransferFunctionError::InvalidGamma);
        }
        Ok(Self::Gamma(value))
    }

    /// Decodes using the curve's lossless default domain.
    ///
    /// SDR curves use signed extended range. PQ and HLG require `[0, 1]`.
    pub fn decode(self, value: f32) -> Result<f32, TransferFunctionError> {
        self.decode_with_policy(value, self.default_policy())
    }

    /// Encodes using the curve's lossless default domain.
    ///
    /// SDR curves use signed extended range. PQ and HLG require `[0, 1]`.
    pub fn encode(self, value: f32) -> Result<f32, TransferFunctionError> {
        self.encode_with_policy(value, self.default_policy())
    }

    pub fn decode_with_policy(
        self,
        value: f32,
        policy: TransferPolicy,
    ) -> Result<f32, TransferFunctionError> {
        self.evaluate(value, TransferDirection::Decode, policy)
    }

    pub fn encode_with_policy(
        self,
        value: f32,
        policy: TransferPolicy,
    ) -> Result<f32, TransferFunctionError> {
        self.evaluate(value, TransferDirection::Encode, policy)
    }

    /// Evaluates a slice into caller-owned storage without per-sample allocation.
    ///
    /// Output before the first invalid sample remains written when evaluation fails.
    pub fn apply_slice(
        self,
        input: &[f32],
        output: &mut [f32],
        direction: TransferDirection,
        policy: TransferPolicy,
    ) -> Result<(), TransferFunctionError> {
        if input.len() != output.len() {
            return Err(TransferFunctionError::LengthMismatch);
        }
        for (&source, target) in input.iter().zip(output) {
            *target = self.evaluate(source, direction, policy)?;
        }
        Ok(())
    }

    const fn default_policy(self) -> TransferPolicy {
        match self {
            Self::Pq | Self::Hlg => TransferPolicy::strict_unit(),
            Self::Linear | Self::Srgb | Self::Rec709 | Self::Rec2020 | Self::Gamma(_) => {
                TransferPolicy::extended()
            }
        }
    }

    fn evaluate(
        self,
        value: f32,
        direction: TransferDirection,
        policy: TransferPolicy,
    ) -> Result<f32, TransferFunctionError> {
        let Some(value) = finite_or_policy(value, policy.non_finite)? else {
            return Ok(non_finite_value(value, policy.non_finite));
        };
        if matches!(policy.range, TransferRange::Extended) && matches!(self, Self::Pq | Self::Hlg) {
            return Err(TransferFunctionError::UnsupportedExtendedRange);
        }
        let value = match policy.range {
            TransferRange::StrictUnit if !(0.0..=1.0).contains(&value) => {
                return Err(TransferFunctionError::OutOfRange);
            }
            TransferRange::Extended | TransferRange::StrictUnit => value,
            TransferRange::ClampUnit => ordered_clamp_f32(value, 0.0, 1.0, NonFinitePolicy::Reject)
                .map_err(|_| TransferFunctionError::NonFinite)?,
        };

        let result = match self {
            Self::Pq => pq(value, direction),
            Self::Hlg => hlg(value, direction),
            curve => extended_sdr(curve, value, direction),
        };
        if result.is_finite() {
            Ok(result)
        } else {
            Err(TransferFunctionError::Overflow)
        }
    }
}

fn finite_or_policy(
    value: f32,
    policy: NonFinitePolicy,
) -> Result<Option<f32>, TransferFunctionError> {
    if value.is_finite() {
        return Ok(Some(value));
    }
    match policy {
        NonFinitePolicy::Reject => Err(TransferFunctionError::NonFinite),
        NonFinitePolicy::Preserve | NonFinitePolicy::CanonicalizeNaN => Ok(None),
    }
}

fn non_finite_value(value: f32, policy: NonFinitePolicy) -> f32 {
    if matches!(policy, NonFinitePolicy::CanonicalizeNaN) && value.is_nan() {
        f32::from_bits(0x7fc0_0000)
    } else {
        value
    }
}

fn extended_sdr(curve: TransferFunction, value: f32, direction: TransferDirection) -> f32 {
    let sign = if value.is_sign_negative() { -1.0 } else { 1.0 };
    let magnitude = value.abs();
    let result = match (curve, direction) {
        (TransferFunction::Linear, _) => magnitude,
        (TransferFunction::Srgb, TransferDirection::Encode) => {
            if magnitude <= SRGB_LINEAR_BREAK {
                12.92 * magnitude
            } else {
                1.055 * magnitude.powf(1.0 / 2.4) - 0.055
            }
        }
        (TransferFunction::Srgb, TransferDirection::Decode) => {
            if magnitude <= SRGB_ENCODED_BREAK {
                magnitude / 12.92
            } else {
                ((magnitude + 0.055) / 1.055).powf(2.4)
            }
        }
        (TransferFunction::Rec709, direction) => {
            rec_oetf(magnitude, direction, REC709_ALPHA, REC709_BETA)
        }
        (TransferFunction::Rec2020, direction) => {
            rec_oetf(magnitude, direction, REC2020_ALPHA, REC2020_BETA)
        }
        (TransferFunction::Gamma(gamma), TransferDirection::Encode) => {
            magnitude.powf(1.0 / gamma.get())
        }
        (TransferFunction::Gamma(gamma), TransferDirection::Decode) => magnitude.powf(gamma.get()),
        (TransferFunction::Pq | TransferFunction::Hlg, _) => unreachable!(),
    };
    sign * result
}

fn rec_oetf(magnitude: f32, direction: TransferDirection, alpha: f32, beta: f32) -> f32 {
    match direction {
        TransferDirection::Encode if magnitude < beta => 4.5 * magnitude,
        TransferDirection::Encode => alpha * magnitude.powf(0.45) - (alpha - 1.0),
        TransferDirection::Decode if magnitude < 4.5 * beta => magnitude / 4.5,
        TransferDirection::Decode => ((magnitude + (alpha - 1.0)) / alpha).powf(1.0 / 0.45),
    }
}

fn pq(value: f32, direction: TransferDirection) -> f32 {
    match direction {
        TransferDirection::Encode => {
            let powered = value.powf(PQ_M1);
            ((PQ_C1 + PQ_C2 * powered) / (1.0 + PQ_C3 * powered)).powf(PQ_M2)
        }
        TransferDirection::Decode => {
            let powered = value.powf(1.0 / PQ_M2);
            let numerator = (powered - PQ_C1).max(0.0);
            (numerator / (PQ_C2 - PQ_C3 * powered)).powf(1.0 / PQ_M1)
        }
    }
}

fn hlg(value: f32, direction: TransferDirection) -> f32 {
    match direction {
        TransferDirection::Encode if value <= 1.0 / 12.0 => (3.0 * value).sqrt(),
        TransferDirection::Encode => HLG_A * (12.0 * value - HLG_B).ln() + HLG_C,
        TransferDirection::Decode if value <= 0.5 => value * value / 3.0,
        TransferDirection::Decode => (((value - HLG_C) / HLG_A).exp() + HLG_B) / 12.0,
    }
}

impl fmt::Display for TransferFunctionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NonFinite => "transfer value is non-finite",
            Self::InvalidGamma => "gamma must be finite and positive",
            Self::OutOfRange => "transfer value is outside the selected unit domain",
            Self::UnsupportedExtendedRange => "PQ and HLG do not define signed extended range",
            Self::Overflow => "transfer evaluation overflowed",
            Self::LengthMismatch => "transfer input and output slice lengths differ",
        })
    }
}

impl std::error::Error for TransferFunctionError {}
