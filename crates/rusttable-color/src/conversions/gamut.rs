use super::{ColorMathError, finite3};

/// Tests the normalized RGB cube exactly; this is not perceptual gamut mapping.
pub fn is_in_unit_gamut(rgb: [f32; 3]) -> Result<bool, ColorMathError> {
    Ok(finite3(rgb)?
        .into_iter()
        .all(|channel| (0.0..=1.0).contains(&channel)))
}

/// Clips finite RGB to the normalized cube; this is not perceptual gamut mapping.
pub fn clip_to_unit_gamut(rgb: [f32; 3]) -> Result<[f32; 3], ColorMathError> {
    Ok(finite3(rgb)?.map(|channel| channel.clamp(0.0, 1.0)))
}
