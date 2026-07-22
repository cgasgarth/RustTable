use rusttable_color::{
    AdaptationMethod, AlphaTransform, BlackPointCompensation, BuiltinColorTransformPlanner,
    ColorEncoding, ColorRole, ColorTransformPlanner, ColorTransformRequest, ExtendedRange,
    Precision, RenderingIntent, TransformPlan, TransformStep,
};
use rusttable_image::{CancellationToken, ImageDimensions};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;

pub type RgbaPixel = [f32; 4];

#[derive(Debug, Clone, PartialEq)]
pub struct DiagnosticFrame {
    dimensions: ImageDimensions,
    pixels: Vec<RgbaPixel>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticFrameError {
    PixelCountMismatch { expected: u64, actual: usize },
}

impl DiagnosticFrame {
    /// Creates a checked row-major RGBA frame.
    ///
    /// # Errors
    ///
    /// Returns a count mismatch when the pixel buffer does not exactly cover
    /// the dimensions.
    pub fn new(
        dimensions: ImageDimensions,
        pixels: Vec<RgbaPixel>,
    ) -> Result<Self, DiagnosticFrameError> {
        let expected =
            dimensions
                .pixel_count()
                .map_err(|_| DiagnosticFrameError::PixelCountMismatch {
                    expected: u64::MAX,
                    actual: pixels.len(),
                })?;
        if expected != u64::try_from(pixels.len()).unwrap_or(u64::MAX) {
            return Err(DiagnosticFrameError::PixelCountMismatch {
                expected,
                actual: pixels.len(),
            });
        }
        Ok(Self { dimensions, pixels })
    }

    #[must_use]
    pub const fn dimensions(&self) -> ImageDimensions {
        self.dimensions
    }

    #[must_use]
    pub fn pixels(&self) -> &[RgbaPixel] {
        &self.pixels
    }

    #[must_use]
    pub fn into_pixels(self) -> Vec<RgbaPixel> {
        self.pixels
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiagnosticBackend {
    Cpu,
    Wgpu,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiagnosticPath {
    Cpu,
    Wgpu,
    CpuFallback,
}

#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiagnosticDescriptor {
    compatibility_id: &'static str,
    module_version: u16,
    hidden: bool,
    history: bool,
    export: bool,
    thumbnail: bool,
}

impl DiagnosticDescriptor {
    #[must_use]
    pub const fn new(compatibility_id: &'static str, module_version: u16) -> Self {
        Self {
            compatibility_id,
            module_version,
            hidden: true,
            history: false,
            export: false,
            thumbnail: false,
        }
    }

    #[must_use]
    pub const fn compatibility_id(self) -> &'static str {
        self.compatibility_id
    }

    #[must_use]
    pub const fn module_version(self) -> u16 {
        self.module_version
    }

    #[must_use]
    pub const fn hidden(self) -> bool {
        self.hidden
    }

    #[must_use]
    pub const fn affects_history(self) -> bool {
        self.history
    }

    #[must_use]
    pub const fn affects_export(self) -> bool {
        self.export
    }

    #[must_use]
    pub const fn affects_thumbnail(self) -> bool {
        self.thumbnail
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiagnosticFinding {
    InvalidState,
    InvalidThreshold,
    InvalidRawLevels,
    UnsupportedCfa,
    InvalidCfa,
    SourceUnavailable,
    DimensionMismatch,
    GeometryUnavailable,
    ProfileUnavailable,
    UnsupportedProfile,
    InvalidTransform,
    ResourceLimit,
    Cancelled,
    GpuUnavailable,
}

impl fmt::Display for DiagnosticFinding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidState => "diagnostic state is invalid",
            Self::InvalidThreshold => "diagnostic threshold is invalid",
            Self::InvalidRawLevels => "RAW black/white levels are invalid",
            Self::UnsupportedCfa => "the RAW CFA is unsupported by this diagnostic",
            Self::InvalidCfa => "the RAW CFA metadata is invalid",
            Self::SourceUnavailable => "the diagnostic source is unavailable",
            Self::DimensionMismatch => "diagnostic dimensions do not match the frame",
            Self::GeometryUnavailable => "the diagnostic geometry snapshot is unavailable",
            Self::ProfileUnavailable => "the diagnostic color profile is unavailable",
            Self::UnsupportedProfile => "the diagnostic color profile is unsupported",
            Self::InvalidTransform => "the diagnostic color transform is invalid",
            Self::ResourceLimit => "the diagnostic temporary buffer exceeds its limit",
            Self::Cancelled => "the diagnostic was cancelled",
            Self::GpuUnavailable => "the diagnostic GPU path is unavailable",
        })
    }
}

impl std::error::Error for DiagnosticFinding {}

/// A frozen reverse mapping from display/output coordinates into a source raster.
/// The mapping is immutable for one plan and therefore tile-order independent.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct DiagnosticGeometry {
    affine: [f64; 6],
}

impl DiagnosticGeometry {
    /// Creates a frozen reverse transform snapshot.
    ///
    /// # Errors
    ///
    /// Returns [`DiagnosticFinding::GeometryUnavailable`] for non-finite
    /// coefficients.
    pub fn new(affine: [f64; 6]) -> Result<Self, DiagnosticFinding> {
        affine
            .iter()
            .all(|value| value.is_finite())
            .then_some(Self { affine })
            .ok_or(DiagnosticFinding::GeometryUnavailable)
    }

    #[must_use]
    pub const fn identity() -> Self {
        Self {
            affine: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0],
        }
    }

    #[must_use]
    pub fn map(self, x: u32, y: u32) -> Option<(f64, f64)> {
        let x = f64::from(x);
        let y = f64::from(y);
        let mapped = (
            self.affine[0].mul_add(x, self.affine[1].mul_add(y, self.affine[2])),
            self.affine[3].mul_add(x, self.affine[4].mul_add(y, self.affine[5])),
        );
        (mapped.0.is_finite() && mapped.1.is_finite()).then_some(mapped)
    }

    #[must_use]
    pub fn identity_hash(self) -> [u8; 32] {
        let mut bytes = Vec::with_capacity(48);
        for value in self.affine {
            bytes.extend_from_slice(&value.to_bits().to_be_bytes());
        }
        Sha256::digest(bytes).into()
    }
}

pub(crate) fn check_cancelled(token: &CancellationToken) -> Result<(), DiagnosticFinding> {
    (!token.is_cancelled())
        .then_some(())
        .ok_or(DiagnosticFinding::Cancelled)
}

pub(crate) fn profile_plan(
    current: ColorEncoding,
    histogram: ColorEncoding,
) -> Result<TransformPlan, DiagnosticFinding> {
    if !current.is_explicit() || !histogram.is_explicit() {
        return Err(DiagnosticFinding::ProfileUnavailable);
    }
    let request = ColorTransformRequest::new(
        current,
        histogram,
        ColorRole::Analysis,
        RenderingIntent::Relative,
        BlackPointCompensation::Disabled,
        AdaptationMethod::Bradford,
        Precision::F32,
        AlphaTransform::Preserve,
        ExtendedRange::Extended,
        1,
    )
    .map_err(|_| DiagnosticFinding::InvalidTransform)?;
    BuiltinColorTransformPlanner
        .plan(&request)
        .map_err(|error| match error {
            rusttable_color::PlannerError::UnknownProfile => DiagnosticFinding::UnsupportedProfile,
            _ => DiagnosticFinding::InvalidTransform,
        })
}

pub(crate) fn apply_profile(
    plan: &TransformPlan,
    mut rgb: [f32; 3],
) -> Result<[f32; 3], DiagnosticFinding> {
    for step in plan.steps() {
        apply_step(step, &mut rgb)?;
    }
    if rgb.iter().all(|value| value.is_finite()) {
        Ok(rgb)
    } else {
        Err(DiagnosticFinding::InvalidTransform)
    }
}

fn apply_step(step: &TransformStep, rgb: &mut [f32; 3]) -> Result<(), DiagnosticFinding> {
    match step {
        TransformStep::Identity => {}
        TransformStep::Transfer { function, decode } => {
            for channel in rgb {
                *channel = if *decode {
                    function.decode(*channel)
                } else {
                    function.encode(*channel)
                }
                .map_err(|_| DiagnosticFinding::InvalidTransform)?;
            }
        }
        TransformStep::Matrix(matrix) => *rgb = matrix.apply(*rgb),
        TransformStep::Adaptation(adaptation) => *rgb = adaptation.matrix().apply(*rgb),
        TransformStep::XyzToLab { white_point } => {
            *rgb = rusttable_color::xyz_to_lab(*rgb, *white_point);
        }
        TransformStep::LabToXyz { white_point } => {
            *rgb = rusttable_color::lab_to_xyz(*rgb, *white_point);
        }
        TransformStep::Composite(composite) => {
            for child in composite.steps() {
                apply_step(child, rgb)?;
            }
        }
        TransformStep::Lut1D(_) | TransformStep::Lut3D(_) => {
            return Err(DiagnosticFinding::UnsupportedProfile);
        }
    }
    Ok(())
}

pub(crate) fn profile_luminance(
    profile: ColorEncoding,
    rgb: [f32; 3],
) -> Result<f32, DiagnosticFinding> {
    let linear = if profile.is_linear() {
        rgb
    } else {
        let transfer = profile
            .transfer()
            .ok_or(DiagnosticFinding::UnsupportedProfile)?;
        rgb.map(|value| {
            transfer
                .decode(value)
                .map_err(|_| DiagnosticFinding::InvalidTransform)
        })
        .into_iter()
        .collect::<Result<Vec<_>, _>>()?
        .try_into()
        .map_err(|_| DiagnosticFinding::InvalidTransform)?
    };
    let matrix = profile
        .builtin()
        .and_then(rusttable_color::BuiltinSpace::to_xyz_matrix)
        .ok_or(DiagnosticFinding::UnsupportedProfile)?
        .rows();
    let luminance = matrix[3].mul_add(
        linear[0],
        matrix[4].mul_add(linear[1], matrix[5] * linear[2]),
    );
    luminance
        .is_finite()
        .then_some(luminance)
        .ok_or(DiagnosticFinding::InvalidTransform)
}
