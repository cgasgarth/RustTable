#![expect(
    clippy::missing_errors_doc,
    clippy::too_many_arguments,
    reason = "checked DNG constructors expose the complete format contract"
)]

use std::path::PathBuf;

use rusttable_color::Matrix3;
use rusttable_image::{ImageDimensions, Orientation, Roi};

pub const DNG_SCHEMA_VERSION: u16 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DngCfaColor {
    Red,
    Green,
    Blue,
}

impl DngCfaColor {
    pub(crate) const fn plane(self) -> u8 {
        match self {
            Self::Red => 0,
            Self::Green => 1,
            Self::Blue => 2,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DngCfaPattern {
    colors: [[DngCfaColor; 2]; 2],
    phase: (u8, u8),
}

impl DngCfaPattern {
    #[must_use]
    pub const fn new(colors: [[DngCfaColor; 2]; 2], phase: (u8, u8)) -> Self {
        Self { colors, phase }
    }

    #[must_use]
    pub const fn colors(self) -> [[DngCfaColor; 2]; 2] {
        self.colors
    }

    #[must_use]
    pub const fn phase(self) -> (u8, u8) {
        self.phase
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DngLinearColor {
    CameraLinear,
    SrgbD65,
    Rec2020D65,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DngMetadataPolicy {
    DerivedOnly,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DngCfaDescriptor {
    dimensions: ImageDimensions,
    row_stride_samples: usize,
    samples: Vec<u16>,
    pattern: DngCfaPattern,
    orientation: Orientation,
    active_area: Option<Roi>,
    default_crop: Option<Roi>,
    masked_areas: Vec<Roi>,
    black: [u16; 4],
    white: [u16; 4],
    white_balance: [f32; 4],
    camera_to_xyz: Matrix3,
    camera_identity: [u8; 32],
    source_identity: [u8; 32],
    output_identity: [u8; 32],
}

impl DngCfaDescriptor {
    /// Constructs a checked single-plane Bayer descriptor.
    pub fn new(
        dimensions: ImageDimensions,
        row_stride_samples: usize,
        samples: Vec<u16>,
        pattern: DngCfaPattern,
        orientation: Orientation,
        active_area: Option<Roi>,
        default_crop: Option<Roi>,
        masked_areas: Vec<Roi>,
        black: [u16; 4],
        white: [u16; 4],
        white_balance: [f32; 4],
        camera_to_xyz: Matrix3,
        camera_identity: [u8; 32],
        source_identity: [u8; 32],
        output_identity: [u8; 32],
    ) -> Result<Self, DngError> {
        let width =
            usize::try_from(dimensions.width()).map_err(|_| DngError::ArithmeticOverflow)?;
        let height =
            usize::try_from(dimensions.height()).map_err(|_| DngError::ArithmeticOverflow)?;
        let required = row_stride_samples
            .checked_mul(height)
            .ok_or(DngError::ArithmeticOverflow)?;
        if row_stride_samples < width || samples.len() != required {
            return Err(DngError::InvalidLayout);
        }
        validate_rois(dimensions, active_area, default_crop, &masked_areas)?;
        let flat = pattern.colors.concat();
        if flat
            .iter()
            .filter(|color| **color == DngCfaColor::Red)
            .count()
            != 1
            || flat
                .iter()
                .filter(|color| **color == DngCfaColor::Blue)
                .count()
                != 1
            || flat
                .iter()
                .filter(|color| **color == DngCfaColor::Green)
                .count()
                != 2
            || pattern.phase.0 > 1
            || pattern.phase.1 > 1
        {
            return Err(DngError::InvalidCfa);
        }
        validate_calibration(black, white, white_balance, camera_to_xyz, camera_identity)?;
        for y in 0..height {
            for x in 0..width {
                let plane = plane_for(
                    pattern,
                    u32::try_from(x).map_err(|_| DngError::ArithmeticOverflow)?,
                    u32::try_from(y).map_err(|_| DngError::ArithmeticOverflow)?,
                );
                let index = y
                    .checked_mul(row_stride_samples)
                    .and_then(|row| row.checked_add(x))
                    .ok_or(DngError::ArithmeticOverflow)?;
                if samples[index] < black[plane] || samples[index] > white[plane] {
                    return Err(DngError::SampleOutsideRange { index });
                }
            }
        }
        if source_identity == [0; 32] || output_identity == [0; 32] {
            return Err(DngError::InvalidMetadata);
        }
        Ok(Self {
            dimensions,
            row_stride_samples,
            samples,
            pattern,
            orientation,
            active_area,
            default_crop,
            masked_areas,
            black,
            white,
            white_balance,
            camera_to_xyz,
            camera_identity,
            source_identity,
            output_identity,
        })
    }

    #[must_use]
    pub const fn dimensions(&self) -> ImageDimensions {
        self.dimensions
    }
    #[must_use]
    pub const fn row_stride_samples(&self) -> usize {
        self.row_stride_samples
    }
    #[must_use]
    pub fn samples(&self) -> &[u16] {
        &self.samples
    }
    #[must_use]
    pub const fn pattern(&self) -> DngCfaPattern {
        self.pattern
    }
    #[must_use]
    pub const fn orientation(&self) -> Orientation {
        self.orientation
    }
    #[must_use]
    pub const fn active_area(&self) -> Option<Roi> {
        self.active_area
    }
    #[must_use]
    pub const fn default_crop(&self) -> Option<Roi> {
        self.default_crop
    }
    #[must_use]
    pub fn masked_areas(&self) -> &[Roi] {
        &self.masked_areas
    }
    #[must_use]
    pub const fn black(&self) -> [u16; 4] {
        self.black
    }
    #[must_use]
    pub const fn white(&self) -> [u16; 4] {
        self.white
    }
    #[must_use]
    pub const fn white_balance(&self) -> [f32; 4] {
        self.white_balance
    }
    #[must_use]
    pub const fn camera_to_xyz(&self) -> Matrix3 {
        self.camera_to_xyz
    }
    #[must_use]
    pub const fn camera_identity(&self) -> [u8; 32] {
        self.camera_identity
    }
    #[must_use]
    pub const fn source_identity(&self) -> [u8; 32] {
        self.source_identity
    }
    #[must_use]
    pub const fn output_identity(&self) -> [u8; 32] {
        self.output_identity
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DngLinearDescriptor {
    dimensions: ImageDimensions,
    samples: Vec<u16>,
    color: DngLinearColor,
    black: [u16; 3],
    white: [u16; 3],
    orientation: Orientation,
    active_area: Option<Roi>,
    default_crop: Option<Roi>,
    masked_areas: Vec<Roi>,
    camera_to_xyz: Option<Matrix3>,
    camera_identity: [u8; 32],
    source_identity: [u8; 32],
    output_identity: [u8; 32],
}

impl DngLinearDescriptor {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        dimensions: ImageDimensions,
        samples: Vec<u16>,
        color: DngLinearColor,
        black: [u16; 3],
        white: [u16; 3],
        orientation: Orientation,
        active_area: Option<Roi>,
        default_crop: Option<Roi>,
        masked_areas: Vec<Roi>,
        camera_to_xyz: Option<Matrix3>,
        camera_identity: [u8; 32],
        source_identity: [u8; 32],
        output_identity: [u8; 32],
    ) -> Result<Self, DngError> {
        let pixels = usize::try_from(dimensions.width())
            .ok()
            .and_then(|w| usize::try_from(dimensions.height()).ok()?.checked_mul(w))
            .ok_or(DngError::ArithmeticOverflow)?;
        if samples.len() != pixels.checked_mul(3).ok_or(DngError::ArithmeticOverflow)? {
            return Err(DngError::InvalidLayout);
        }
        if black.iter().zip(white).any(|(b, w)| w <= *b)
            || camera_identity == [0; 32]
            || source_identity == [0; 32]
            || output_identity == [0; 32]
            || (matches!(color, DngLinearColor::CameraLinear) && camera_to_xyz.is_none())
        {
            return Err(DngError::InvalidMetadata);
        }
        if let Some(matrix) = camera_to_xyz {
            matrix.inverse().map_err(|_| DngError::InvalidCalibration)?;
        }
        validate_rois(dimensions, active_area, default_crop, &masked_areas)?;
        for (index, sample) in samples.iter().enumerate() {
            if *sample < black[index % 3] || *sample > white[index % 3] {
                return Err(DngError::SampleOutsideRange { index });
            }
        }
        Ok(Self {
            dimensions,
            samples,
            color,
            black,
            white,
            orientation,
            active_area,
            default_crop,
            masked_areas,
            camera_to_xyz,
            camera_identity,
            source_identity,
            output_identity,
        })
    }
    #[must_use]
    pub const fn dimensions(&self) -> ImageDimensions {
        self.dimensions
    }
    #[must_use]
    pub fn samples(&self) -> &[u16] {
        &self.samples
    }
    #[must_use]
    pub const fn color(&self) -> DngLinearColor {
        self.color
    }
    #[must_use]
    pub const fn black(&self) -> [u16; 3] {
        self.black
    }
    #[must_use]
    pub const fn white(&self) -> [u16; 3] {
        self.white
    }
    #[must_use]
    pub const fn orientation(&self) -> Orientation {
        self.orientation
    }
    #[must_use]
    pub const fn active_area(&self) -> Option<Roi> {
        self.active_area
    }
    #[must_use]
    pub const fn default_crop(&self) -> Option<Roi> {
        self.default_crop
    }
    #[must_use]
    pub fn masked_areas(&self) -> &[Roi] {
        &self.masked_areas
    }
    #[must_use]
    pub const fn camera_to_xyz(&self) -> Option<Matrix3> {
        self.camera_to_xyz
    }
    #[must_use]
    pub const fn camera_identity(&self) -> [u8; 32] {
        self.camera_identity
    }
    #[must_use]
    pub const fn source_identity(&self) -> [u8; 32] {
        self.source_identity
    }
    #[must_use]
    pub const fn output_identity(&self) -> [u8; 32] {
        self.output_identity
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum DngRawLayout {
    CfaBayerU16(DngCfaDescriptor),
    LinearRawRgbU16(DngLinearDescriptor),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DngCollisionPolicy {
    Fail,
    ReuseIdentical,
    Suffix,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DngLimits {
    pub max_encoded_bytes: u64,
    pub max_pixels: u64,
    pub max_preview_bytes: u64,
}

impl Default for DngLimits {
    fn default() -> Self {
        Self {
            max_encoded_bytes: 4 * 1024 * 1024 * 1024,
            max_pixels: 200_000_000,
            max_preview_bytes: 16 * 1024 * 1024,
        }
    }
}

impl DngLimits {
    pub fn checked(self) -> Result<Self, DngError> {
        if self.max_encoded_bytes == 0 || self.max_pixels == 0 {
            Err(DngError::InvalidLimits)
        } else {
            Ok(self)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DngPreview {
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DngOutputRequest {
    pub destination: PathBuf,
    pub layout: DngRawLayout,
    pub collision: DngCollisionPolicy,
    pub metadata_policy: DngMetadataPolicy,
    pub limits: DngLimits,
    pub preview: Option<DngPreview>,
}

impl DngOutputRequest {
    pub fn new(destination: PathBuf, layout: DngRawLayout) -> Result<Self, DngError> {
        let request = Self {
            destination,
            layout,
            collision: DngCollisionPolicy::Fail,
            metadata_policy: DngMetadataPolicy::DerivedOnly,
            limits: DngLimits::default(),
            preview: None,
        };
        request.validate()?;
        Ok(request)
    }
    pub fn validate(&self) -> Result<(), DngError> {
        if self.destination.file_name().is_none() || self.destination.as_os_str().is_empty() {
            return Err(DngError::InvalidDestination);
        }
        self.limits.checked()?;
        let dimensions = match &self.layout {
            DngRawLayout::CfaBayerU16(v) => v.dimensions(),
            DngRawLayout::LinearRawRgbU16(v) => v.dimensions(),
        };
        let pixels = u64::from(dimensions.width())
            .checked_mul(u64::from(dimensions.height()))
            .ok_or(DngError::ArithmeticOverflow)?;
        if pixels > self.limits.max_pixels {
            return Err(DngError::MemoryLimit);
        }
        if self.preview.is_some() {
            return Err(DngError::UnsupportedPreview);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DngOutputReceipt {
    pub schema_version: u16,
    pub artifact_identity: [u8; 32],
    pub pixel_hash: [u8; 32],
    pub encoded_bytes: u64,
    pub strip_count: u32,
    pub rows_per_strip: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DngPublished {
    pub destination: PathBuf,
    pub receipt: DngOutputReceipt,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DngProbe {
    pub layout: DngRawLayoutKind,
    pub dimensions: ImageDimensions,
    pub samples: Vec<u16>,
    pub artifact_identity: [u8; 32],
    pub source_identity: [u8; 32],
    pub pixel_hash: [u8; 32],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DngRawLayoutKind {
    CfaBayerU16,
    LinearRawRgbU16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DngError {
    InvalidDestination,
    DestinationExists,
    InvalidLayout,
    InvalidCfa,
    InvalidCalibration,
    InvalidMetadata,
    InvalidLimits,
    ArithmeticOverflow,
    MemoryLimit,
    UnsupportedPreview,
    SampleOutsideRange { index: usize },
    Cancelled,
    Io(String),
    Encode(String),
    Probe(String),
    RoundTripMismatch,
}

fn validate_calibration(
    black: [u16; 4],
    white: [u16; 4],
    wb: [f32; 4],
    matrix: Matrix3,
    camera: [u8; 32],
) -> Result<(), DngError> {
    if black.iter().zip(white).any(|(b, w)| w <= *b)
        || wb.iter().any(|gain| !gain.is_finite() || *gain <= 0.0)
        || camera == [0; 32]
    {
        return Err(DngError::InvalidCalibration);
    }
    matrix
        .inverse()
        .map_err(|_| DngError::InvalidCalibration)
        .map(|_| ())
}

fn validate_rois(
    dimensions: ImageDimensions,
    active: Option<Roi>,
    crop: Option<Roi>,
    masked: &[Roi],
) -> Result<(), DngError> {
    for roi in active.into_iter().chain(crop).chain(masked.iter().copied()) {
        if roi.is_empty() || roi.within(dimensions).is_err() {
            return Err(DngError::InvalidMetadata);
        }
    }
    if let (Some(active), Some(crop)) = (active, crop)
        && (crop.x() < active.x()
            || crop.y() < active.y()
            || crop.right() > active.right()
            || crop.bottom() > active.bottom())
    {
        return Err(DngError::InvalidMetadata);
    }
    Ok(())
}

fn plane_for(pattern: DngCfaPattern, x: u32, y: u32) -> usize {
    let target = (x as usize % 2, y as usize % 2);
    let mut green_plane = 1;
    for yy in 0..2 {
        for xx in 0..2 {
            let color = pattern.colors[(yy + usize::from(pattern.phase.1)) % 2]
                [(xx + usize::from(pattern.phase.0)) % 2];
            if (xx, yy) == target {
                return match color {
                    DngCfaColor::Red => 0,
                    DngCfaColor::Green => green_plane,
                    DngCfaColor::Blue => 3,
                };
            }
            if color == DngCfaColor::Green {
                green_plane += 1;
            }
        }
    }
    0
}
