use half::f16;

use super::{ConversionPolicy, ConversionRange, NonFinitePolicy, NumericalError, RoundingPolicy};

/// Converts an `f32` to `u8` with explicit range, rounding, and non-finite policy.
///
/// # Errors
///
/// Rejects non-finite input and out-of-range input when clamping is disabled.
pub fn f32_to_u8(value: f32, policy: ConversionPolicy) -> Result<u8, NumericalError> {
    if !value.is_finite() {
        return Err(NumericalError::NonFinite);
    }
    let value = match policy.range {
        ConversionRange::Reject if !(0.0..=255.0).contains(&value) => {
            return Err(NumericalError::OutOfRange);
        }
        ConversionRange::Reject => value,
        ConversionRange::Clamp => value.clamp(0.0, 255.0),
    };
    let rounded = match policy.rounding {
        RoundingPolicy::NearestTiesEven => value.round_ties_even(),
        RoundingPolicy::TowardZero => value.trunc(),
    };
    // The selected range policy proves the rounded value is within `u8` bounds.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let converted = rounded as u8;
    Ok(converted)
}

/// Produces canonical IEEE binary16 storage bits from an `f32`.
///
/// # Errors
///
/// Rejects non-finite input when selected by policy.
pub fn canonical_f16_bits(value: f32, policy: NonFinitePolicy) -> Result<u16, NumericalError> {
    let value = normalize_non_finite(value, policy)?;
    if value.is_nan() && policy == NonFinitePolicy::CanonicalizeNaN {
        return Ok(0x7e00);
    }
    Ok(f16::from_f32(value).to_bits())
}

/// Expands canonical IEEE binary16 storage bits for CPU `f32` arithmetic.
///
/// # Errors
///
/// Rejects non-finite values when selected by policy.
pub fn canonical_f16_to_f32(bits: u16, policy: NonFinitePolicy) -> Result<f32, NumericalError> {
    normalize_non_finite(f16::from_bits(bits).to_f32(), policy)
}

fn normalize_non_finite(value: f32, policy: NonFinitePolicy) -> Result<f32, NumericalError> {
    match policy {
        NonFinitePolicy::Reject if !value.is_finite() => Err(NumericalError::NonFinite),
        NonFinitePolicy::CanonicalizeNaN if value.is_nan() => Ok(f32::from_bits(0x7fc0_0000)),
        _ => Ok(value),
    }
}
