use rusttable_processing::{FiniteF32, LinearRgb, WorkingRgbImage};

/// Darktable's scene-referred baseline for color RAW images, in exposure stops.
///
/// This mirrors `src/iop/exposure.c::reload_defaults` in the read-only
/// Darktable reference. Its no-monitor-profile output then resolves to the
/// built-in sRGB profile in `src/common/colorspaces.c` and `src/iop/colorout.c`.
pub const SCENE_REFERRED_RAW_EXPOSURE_STOPS: f32 = 0.7;

/// Linear gain corresponding to [`SCENE_REFERRED_RAW_EXPOSURE_STOPS`].
pub const SCENE_REFERRED_RAW_LINEAR_GAIN: f32 = 1.624_504_8;

/// Versioned terminal sRGB presentation selected by the render composition.
///
/// Raster inputs retain a colorimetric conversion. Scene-linear color RAWs use
/// Darktable's +0.7 EV baseline before the same conversion. The contract is
/// carried by render receipts so preview, filmstrip caches, and export cannot
/// silently disagree about the selected presentation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SrgbFallbackContract {
    #[default]
    Colorimetric,
    SceneReferredRawV1,
}

impl SrgbFallbackContract {
    #[must_use]
    pub const fn source_linear_gain(self) -> f32 {
        match self {
            Self::Colorimetric => 1.0,
            Self::SceneReferredRawV1 => SCENE_REFERRED_RAW_LINEAR_GAIN,
        }
    }

    pub(crate) fn apply(self, input: &WorkingRgbImage) -> WorkingRgbImage {
        if self == Self::Colorimetric {
            return input.clone();
        }
        let gain = self.source_linear_gain();
        let pixels = input
            .pixels()
            .map(|pixel| {
                LinearRgb::new(
                    scaled(pixel.red(), gain),
                    scaled(pixel.green(), gain),
                    scaled(pixel.blue(), gain),
                )
            })
            .collect();
        WorkingRgbImage::new_with_frame(input.dimensions(), pixels, input.frame())
            .expect("presentation preserves pixel count")
    }
}

fn scaled(value: FiniteF32, gain: f32) -> FiniteF32 {
    let value = (value.get() * gain).clamp(-f32::MAX, f32::MAX);
    FiniteF32::new(value).expect("finite input and bounded gain remain finite")
}
