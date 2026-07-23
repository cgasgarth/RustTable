use super::NumericalError;

/// Returns the representable-step distance between two finite `f32` values.
/// Signed zero values have distance one because their IEEE encodings are adjacent
/// in the total ordering used by this helper.
///
/// # Errors
///
/// Rejects non-finite values.
pub fn ulp_distance_f32(left: f32, right: f32) -> Result<u32, NumericalError> {
    if !left.is_finite() || !right.is_finite() {
        return Err(NumericalError::NonFinite);
    }
    Ok(ordered_bits(left).abs_diff(ordered_bits(right)))
}

const fn ordered_bits(value: f32) -> u32 {
    let bits = value.to_bits();
    if bits & 0x8000_0000 == 0 {
        bits | 0x8000_0000
    } else {
        !bits
    }
}
