use std::cmp::Ordering;

use super::{NonFinitePolicy, NumericalError};

/// Deterministic minimum with explicit non-finite handling and signed-zero ordering.
///
/// # Errors
///
/// Rejects non-finite values under [`NonFinitePolicy::Reject`].
pub fn ordered_min_f32(
    left: f32,
    right: f32,
    policy: NonFinitePolicy,
) -> Result<f32, NumericalError> {
    ordered_pair(left, right, policy, Ordering::Greater)
}

/// Deterministic maximum with explicit non-finite handling and signed-zero ordering.
///
/// # Errors
///
/// Rejects non-finite values under [`NonFinitePolicy::Reject`].
pub fn ordered_max_f32(
    left: f32,
    right: f32,
    policy: NonFinitePolicy,
) -> Result<f32, NumericalError> {
    ordered_pair(left, right, policy, Ordering::Less)
}

/// Deterministic clamp with explicit bounds and signed-zero behavior.
///
/// # Errors
///
/// Rejects inverted bounds and non-finite values under the selected policy.
pub fn ordered_clamp_f32(
    value: f32,
    lower: f32,
    upper: f32,
    policy: NonFinitePolicy,
) -> Result<f32, NumericalError> {
    let value = normalized(value, policy)?;
    let lower = normalized(lower, policy)?;
    let upper = normalized(upper, policy)?;
    if lower.is_nan() || upper.is_nan() {
        return Err(NumericalError::NonFinite);
    }
    if lower.total_cmp(&upper) == Ordering::Greater {
        return Err(NumericalError::InvertedBounds);
    }
    if value.is_nan() {
        return Ok(value);
    }
    if value.total_cmp(&lower) == Ordering::Less {
        Ok(lower)
    } else if value.total_cmp(&upper) == Ordering::Greater {
        Ok(upper)
    } else {
        Ok(value)
    }
}

/// Divides finite values without silently accepting zero or non-finite output.
///
/// # Errors
///
/// Rejects non-finite operands/results and either signed zero denominator.
pub fn finite_divide_f32(numerator: f32, denominator: f32) -> Result<f32, NumericalError> {
    if !numerator.is_finite() || !denominator.is_finite() {
        return Err(NumericalError::NonFinite);
    }
    if denominator == 0.0 {
        return Err(NumericalError::DivisionByZero);
    }
    let result = numerator / denominator;
    if result.is_finite() {
        Ok(result)
    } else {
        Err(NumericalError::NonFinite)
    }
}

/// Normalizes a finite circular value into `[0, period)`.
///
/// # Errors
///
/// Rejects non-finite values and non-positive periods.
pub fn normalize_circular_f32(value: f32, period: f32) -> Result<f32, NumericalError> {
    if !value.is_finite() || !period.is_finite() {
        return Err(NumericalError::NonFinite);
    }
    if period <= 0.0 {
        return Err(NumericalError::InvalidPeriod);
    }
    Ok(value.rem_euclid(period))
}

fn ordered_pair(
    left: f32,
    right: f32,
    policy: NonFinitePolicy,
    replace_when: Ordering,
) -> Result<f32, NumericalError> {
    let left = normalized(left, policy)?;
    let right = normalized(right, policy)?;
    if left.is_nan() {
        return Ok(left);
    }
    if right.is_nan() {
        return Ok(right);
    }
    if left.total_cmp(&right) == replace_when {
        Ok(right)
    } else {
        Ok(left)
    }
}

fn normalized(value: f32, policy: NonFinitePolicy) -> Result<f32, NumericalError> {
    match policy {
        NonFinitePolicy::Reject if !value.is_finite() => Err(NumericalError::NonFinite),
        NonFinitePolicy::CanonicalizeNaN if value.is_nan() => Ok(f32::from_bits(0x7fc0_0000)),
        _ => Ok(value),
    }
}
