use super::{FINALSCALE_PARAMETER_BYTES, FINALSCALE_PARAMETER_VERSION};
use crate::FiniteF32;
use std::fmt;

/// The request modes accepted by the final render stage.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RenderSizeRequest {
    Original,
    Exact {
        width: u32,
        height: u32,
    },
    FitWithin {
        width: u32,
        height: u32,
    },
    LongEdge(u32),
    ShortEdge(u32),
    Megapixels(FiniteF32),
    Print {
        width_mm: FiniteF32,
        height_mm: FiniteF32,
        dpi: FiniteF32,
    },
    PipelineScale(FiniteF32),
}

impl RenderSizeRequest {
    #[must_use]
    pub const fn exact(width: u32, height: u32) -> Self {
        Self::Exact { width, height }
    }

    #[must_use]
    pub const fn fit_within(width: u32, height: u32) -> Self {
        Self::FitWithin { width, height }
    }

    #[must_use]
    pub const fn long_edge(edge: u32) -> Self {
        Self::LongEdge(edge)
    }

    #[must_use]
    pub const fn short_edge(edge: u32) -> Self {
        Self::ShortEdge(edge)
    }

    pub fn megapixels(value: f32) -> Result<Self, RenderSizeRequestError> {
        Ok(Self::Megapixels(finite(value)?))
    }

    pub fn print(width_mm: f32, height_mm: f32, dpi: f32) -> Result<Self, RenderSizeRequestError> {
        Ok(Self::Print {
            width_mm: finite(width_mm)?,
            height_mm: finite(height_mm)?,
            dpi: finite(dpi)?,
        })
    }

    pub fn pipeline_scale(value: f32) -> Result<Self, RenderSizeRequestError> {
        Ok(Self::PipelineScale(finite(value)?))
    }

    pub(crate) fn validate(&self) -> Result<(), RenderSizeRequestError> {
        match self {
            Self::Original => Ok(()),
            Self::Exact { width, height } | Self::FitWithin { width, height } => {
                if *width == 0 || *height == 0 {
                    Err(RenderSizeRequestError::ZeroDimension)
                } else {
                    Ok(())
                }
            }
            Self::LongEdge(edge) | Self::ShortEdge(edge) => {
                if *edge == 0 {
                    Err(RenderSizeRequestError::ZeroDimension)
                } else {
                    Ok(())
                }
            }
            Self::Megapixels(value) | Self::PipelineScale(value) => positive(value.get()),
            Self::Print {
                width_mm,
                height_mm,
                dpi,
            } => {
                positive(width_mm.get())?;
                positive(height_mm.get())?;
                positive(dpi.get())
            }
        }
    }

    /// Returns the canonical bytes used when a pixelpipe snapshots this request.
    #[must_use]
    pub fn identity_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(32);
        match self {
            Self::Original => bytes.push(0),
            Self::Exact { width, height } => {
                bytes.push(1);
                bytes.extend(width.to_le_bytes());
                bytes.extend(height.to_le_bytes());
            }
            Self::FitWithin { width, height } => {
                bytes.push(2);
                bytes.extend(width.to_le_bytes());
                bytes.extend(height.to_le_bytes());
            }
            Self::LongEdge(edge) => {
                bytes.push(3);
                bytes.extend(edge.to_le_bytes());
            }
            Self::ShortEdge(edge) => {
                bytes.push(4);
                bytes.extend(edge.to_le_bytes());
            }
            Self::Megapixels(value) => {
                bytes.push(5);
                bytes.extend(value.get().to_bits().to_le_bytes());
            }
            Self::Print {
                width_mm,
                height_mm,
                dpi,
            } => {
                bytes.push(6);
                bytes.extend(width_mm.get().to_bits().to_le_bytes());
                bytes.extend(height_mm.get().to_bits().to_le_bytes());
                bytes.extend(dpi.get().to_bits().to_le_bytes());
            }
            Self::PipelineScale(value) => {
                bytes.push(7);
                bytes.extend(value.get().to_bits().to_le_bytes());
            }
        }
        bytes
    }
}

fn finite(value: f32) -> Result<FiniteF32, RenderSizeRequestError> {
    FiniteF32::new(value).map_err(|_| RenderSizeRequestError::NonFinite)
}

fn positive(value: f32) -> Result<(), RenderSizeRequestError> {
    if value.is_finite() && value > 0.0 {
        Ok(())
    } else if value.is_finite() {
        Err(RenderSizeRequestError::NonPositive)
    } else {
        Err(RenderSizeRequestError::NonFinite)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderSizeRequestError {
    ZeroDimension,
    NonFinite,
    NonPositive,
}

impl fmt::Display for RenderSizeRequestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::ZeroDimension => "render dimensions must be nonzero",
            Self::NonFinite => "render size values must be finite",
            Self::NonPositive => "render size values must be positive",
        })
    }
}

impl std::error::Error for RenderSizeRequestError {}

/// Interpolation kernels available to the final render stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FinalScaleKernel {
    Nearest,
    Bilinear,
    Bicubic,
    Lanczos,
}

impl FinalScaleKernel {
    #[must_use]
    pub const fn support(self) -> u32 {
        match self {
            Self::Nearest => 0,
            Self::Bilinear => 1,
            Self::Bicubic => 2,
            Self::Lanczos => 3,
        }
    }

    #[must_use]
    pub const fn tag(self) -> u8 {
        match self {
            Self::Nearest => 0,
            Self::Bilinear => 1,
            Self::Bicubic => 2,
            Self::Lanczos => 3,
        }
    }
}

/// Pipeline context is explicit so preview kernels cannot leak into export.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RenderQualityKind {
    Preview,
    Thumbnail,
    ImageFinal,
    Export,
    Print,
}

impl RenderQualityKind {
    #[must_use]
    pub const fn tag(self) -> u8 {
        match self {
            Self::Preview => 0,
            Self::Thumbnail => 1,
            Self::ImageFinal => 2,
            Self::Export => 3,
            Self::Print => 4,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RenderQuality {
    kind: RenderQualityKind,
    kernel: FinalScaleKernel,
}

impl RenderQuality {
    #[must_use]
    pub const fn new(kind: RenderQualityKind, kernel: FinalScaleKernel) -> Self {
        Self { kind, kernel }
    }

    #[must_use]
    pub const fn preview(kernel: FinalScaleKernel) -> Self {
        Self::new(RenderQualityKind::Preview, kernel)
    }

    #[must_use]
    pub const fn thumbnail(kernel: FinalScaleKernel) -> Self {
        Self::new(RenderQualityKind::Thumbnail, kernel)
    }

    #[must_use]
    pub const fn image_final(kernel: FinalScaleKernel) -> Self {
        Self::new(RenderQualityKind::ImageFinal, kernel)
    }

    #[must_use]
    pub const fn export(kernel: FinalScaleKernel) -> Self {
        Self::new(RenderQualityKind::Export, kernel)
    }

    #[must_use]
    pub const fn print_quality(kernel: FinalScaleKernel) -> Self {
        Self::new(RenderQualityKind::Print, kernel)
    }

    #[must_use]
    pub const fn kind(self) -> RenderQualityKind {
        self.kind
    }

    #[must_use]
    pub const fn kernel(self) -> FinalScaleKernel {
        self.kernel
    }
}

impl Default for RenderQuality {
    fn default() -> Self {
        Self::image_final(FinalScaleKernel::Bilinear)
    }
}

/// Version-one darktable history. The upstream payload is an `int dummy`; it
/// is retained byte-for-byte rather than decoded through a C layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FinalScaleParametersV1 {
    raw: [u8; FINALSCALE_PARAMETER_BYTES],
}

impl FinalScaleParametersV1 {
    #[must_use]
    pub const fn new(raw: [u8; FINALSCALE_PARAMETER_BYTES]) -> Self {
        Self { raw }
    }

    #[must_use]
    pub const fn raw(self) -> [u8; FINALSCALE_PARAMETER_BYTES] {
        self.raw
    }

    #[must_use]
    pub const fn to_bytes(self) -> [u8; FINALSCALE_PARAMETER_BYTES] {
        self.raw
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, FinalScaleCodecError> {
        if bytes.len() != FINALSCALE_PARAMETER_BYTES {
            return Err(FinalScaleCodecError::InvalidLength {
                expected: FINALSCALE_PARAMETER_BYTES,
                actual: bytes.len(),
            });
        }
        Ok(Self::new(bytes.try_into().expect("checked payload length")))
    }
}

impl Default for FinalScaleParametersV1 {
    fn default() -> Self {
        Self::new([0; FINALSCALE_PARAMETER_BYTES])
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FinalScaleHistory {
    V1(FinalScaleParametersV1),
    Opaque { version: u16, bytes: Vec<u8> },
}

impl FinalScaleHistory {
    pub fn decode(version: u16, bytes: &[u8]) -> Result<Self, FinalScaleCodecError> {
        if version == FINALSCALE_PARAMETER_VERSION {
            Ok(Self::V1(FinalScaleParametersV1::from_bytes(bytes)?))
        } else {
            Ok(Self::Opaque {
                version,
                bytes: bytes.to_vec(),
            })
        }
    }

    #[must_use]
    pub const fn version(&self) -> u16 {
        match self {
            Self::V1(_) => FINALSCALE_PARAMETER_VERSION,
            Self::Opaque { version, .. } => *version,
        }
    }

    #[must_use]
    pub fn payload(&self) -> &[u8] {
        match self {
            Self::V1(parameters) => &parameters.raw,
            Self::Opaque { bytes, .. } => bytes,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FinalScaleCodecError {
    InvalidLength { expected: usize, actual: usize },
}

impl fmt::Display for FinalScaleCodecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLength { expected, actual } => {
                write!(
                    formatter,
                    "finalscale payload has {actual} bytes; expected {expected}"
                )
            }
        }
    }
}

impl std::error::Error for FinalScaleCodecError {}
