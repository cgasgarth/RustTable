//! Bounded, operation-local CPU leaf for Darktable `src/iop/soften.c`.
//!
//! This module owns the native v1 parameter ABI, an operation-local descriptor,
//! the source-ordered HSL adjustment/eight-pass box mean/four-lane blend, and a
//! cancellation-safe publication candidate. Shared registry, history import,
//! pixelpipe, masks/blending, OpenCL, and GTK ownership remain explicitly
//! deferred. The root is intentionally named `leaf.rs`: the baseline still has
//! a provisional shared `operations/soften.rs`, which this bounded lane does not
//! modify or route into production.

#![forbid(unsafe_code)]
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::doc_markdown,
    clippy::excessive_precision,
    clippy::float_cmp,
    clippy::manual_midpoint,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::struct_excessive_bools,
    dead_code,
    reason = "the standalone source-shaped leaf retains native f32 casts and deferred public seams"
)]
#![expect(
    clippy::suboptimal_flops,
    reason = "Native Soften HSL and blend equations preserve source evaluation order and IEEE-754 parity."
)]

use std::fmt;
use std::mem::size_of;

use rusttable_color::ColorEncoding;
use rusttable_processing::common::box_filters::{
    BOX_ITERATIONS, BoxFilterError, CancellableBoxFilterError, box_mean_with_cancel,
    box_mean_with_cancel_scratch_bytes,
};
use rusttable_processing::descriptor::{
    AlphaPolicy, CapabilityContract, DescriptorId, ImagePredicate, InputOutputContract,
    MaskBlendContract, MigrationContract, NonFinitePolicy, OperationDescriptor, OperationFlags,
    ParameterDefault, ParameterDescriptor, ParameterKind, ParameterRole, RoiKind, TilingContract,
};

pub const SOFTEN_COMPATIBILITY_ID: &str = "soften";
pub const SOFTEN_RUST_ID: &str = "rusttable.soften";
pub const SOFTEN_SCHEMA_VERSION: u16 = 1;
pub const SOFTEN_PARAMETER_BYTES: usize = 16;
pub const SOFTEN_CHANNELS: usize = 4;
pub const SOFTEN_DEFAULT_SIZE: f32 = 50.0;
pub const SOFTEN_DEFAULT_SATURATION: f32 = 100.0;
pub const SOFTEN_DEFAULT_BRIGHTNESS: f32 = 0.33;
pub const SOFTEN_DEFAULT_AMOUNT: f32 = 50.0;
pub const SOFTEN_DEFAULT_ENABLED: bool = false;
pub const SOFTEN_DEFAULT_VISIBLE: bool = false;
pub const SOFTEN_DEFAULT_GROUPS: [&str; 2] = ["effect", "effects"];
pub const SOFTEN_DEFAULT_COLORSPACE: &str = "RGB";
pub const SOFTEN_SUPPORTS_BLENDING: bool = true;
pub const SOFTEN_ALLOW_TILING: bool = true;
pub const SOFTEN_MIGRATION_EDGES: &[(u16, u16)] = &[];

/// Exact five-string payload returned by native `description()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SoftenDescription {
    pub main_text: &'static str,
    pub purpose: &'static str,
    pub input: &'static str,
    pub process: &'static str,
    pub output: &'static str,
}

pub const SOFTEN_DESCRIPTION: SoftenDescription = SoftenDescription {
    main_text: "create a softened image using the Orton effect",
    purpose: "creative",
    input: "linear, RGB, display-referred",
    process: "linear, RGB",
    output: "linear, RGB, display-referred",
};

/// Versioned source evidence from `src/common/iop_order.c`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SoftenOperationOrderEvidence {
    pub table: &'static str,
    pub order: f32,
}

pub const SOFTEN_OPERATION_ORDERS: [SoftenOperationOrderEvidence; 5] = [
    SoftenOperationOrderEvidence {
        table: "legacy_order",
        order: 60.0,
    },
    SoftenOperationOrderEvidence {
        table: "v30_order",
        order: 66.0,
    },
    SoftenOperationOrderEvidence {
        table: "v50_order",
        order: 66.0,
    },
    SoftenOperationOrderEvidence {
        table: "v30_jpg_order",
        order: 66.0,
    },
    SoftenOperationOrderEvidence {
        table: "v50_jpg_order",
        order: 66.0,
    },
];

pub const SOFTEN_GPU_PROGRAM: u32 = 9;
pub const SOFTEN_GPU_KERNELS: [&str; 4] = [
    "soften_overexposed",
    "soften_hblur",
    "soften_vblur",
    "soften_mix",
];
pub const SOFTEN_GPU_EXECUTABLE: bool = false;
pub const SOFTEN_DEFAULT_MEMORY_BUDGET: usize = 512 * 1024 * 1024;

/// Native `dt_iop_soften_params_t` in declaration and byte order.
#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(C)]
pub struct SoftenParametersV1 {
    pub size: f32,
    pub saturation: f32,
    pub brightness: f32,
    pub amount: f32,
}

const _: () = assert!(size_of::<SoftenParametersV1>() == SOFTEN_PARAMETER_BYTES);

impl SoftenParametersV1 {
    #[must_use]
    pub const fn new(size: f32, saturation: f32, brightness: f32, amount: f32) -> Self {
        Self {
            size,
            saturation,
            brightness,
            amount,
        }
    }

    #[must_use]
    pub const fn defaults() -> Self {
        Self::new(
            SOFTEN_DEFAULT_SIZE,
            SOFTEN_DEFAULT_SATURATION,
            SOFTEN_DEFAULT_BRIGHTNESS,
            SOFTEN_DEFAULT_AMOUNT,
        )
    }

    #[must_use]
    pub fn to_bytes(self) -> [u8; SOFTEN_PARAMETER_BYTES] {
        let mut bytes = [0_u8; SOFTEN_PARAMETER_BYTES];
        write_f32(&mut bytes, 0, self.size);
        write_f32(&mut bytes, 4, self.saturation);
        write_f32(&mut bytes, 8, self.brightness);
        write_f32(&mut bytes, 12, self.amount);
        bytes
    }

    pub const fn from_bytes(bytes: &[u8]) -> Result<Self, SoftenCodecError> {
        if bytes.len() != SOFTEN_PARAMETER_BYTES {
            return Err(SoftenCodecError::InvalidLength {
                expected: SOFTEN_PARAMETER_BYTES,
                actual: bytes.len(),
            });
        }
        Ok(Self::new(
            read_f32(bytes, 0),
            read_f32(bytes, 4),
            read_f32(bytes, 8),
            read_f32(bytes, 12),
        ))
    }
}

/// Known native history plus byte-for-byte opaque future values.
#[derive(Debug, Clone, PartialEq)]
pub enum SoftenHistory {
    V1(SoftenParametersV1),
    Opaque { version: u16, bytes: Vec<u8> },
}

impl SoftenHistory {
    pub fn decode(version: u16, bytes: &[u8]) -> Result<Self, SoftenCodecError> {
        match version {
            SOFTEN_SCHEMA_VERSION => Ok(Self::V1(SoftenParametersV1::from_bytes(bytes)?)),
            _ => Ok(Self::Opaque {
                version,
                bytes: bytes.to_vec(),
            }),
        }
    }

    #[must_use]
    pub const fn version(&self) -> u16 {
        match self {
            Self::V1(_) => SOFTEN_SCHEMA_VERSION,
            Self::Opaque { version, .. } => *version,
        }
    }

    #[must_use]
    pub fn payload(&self) -> Vec<u8> {
        match self {
            Self::V1(parameters) => parameters.to_bytes().to_vec(),
            Self::Opaque { bytes, .. } => bytes.clone(),
        }
    }

    /// Materializes the only source-known schema. There are no legacy edges in
    /// `soften.c`; every other version remains opaque and non-executable.
    pub const fn migrate_to_current(&self) -> Result<SoftenParametersV1, SoftenCodecError> {
        match self {
            Self::V1(parameters) => Ok(*parameters),
            Self::Opaque { version, .. } => Err(SoftenCodecError::UnsupportedVersion(*version)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SoftenCodecError {
    InvalidLength { expected: usize, actual: usize },
    UnsupportedVersion(u16),
}

impl fmt::Display for SoftenCodecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLength { expected, actual } => write!(
                formatter,
                "soften payload has {actual} bytes; expected {expected}"
            ),
            Self::UnsupportedVersion(version) => {
                write!(
                    formatter,
                    "soften version {version} is opaque and unsupported"
                )
            }
        }
    }
}

impl std::error::Error for SoftenCodecError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SoftenParameterError {
    NonFinite(&'static str),
}

impl fmt::Display for SoftenParameterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFinite(name) => write!(formatter, "soften {name} is non-finite"),
        }
    }
}

impl std::error::Error for SoftenParameterError {}

/// Executable values committed by native `commit_params`.
///
/// The introspection minima and maxima describe sliders; `commit_params` copies
/// every value without clamping or range rejection. This safe leaf therefore
/// accepts every finite `f32` while explicitly failing closed on non-finite
/// values before planning or execution.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SoftenConfig {
    parameters: SoftenParametersV1,
}

impl TryFrom<SoftenParametersV1> for SoftenConfig {
    type Error = SoftenParameterError;

    fn try_from(parameters: SoftenParametersV1) -> Result<Self, Self::Error> {
        check_finite_parameter("size", parameters.size)?;
        check_finite_parameter("saturation", parameters.saturation)?;
        check_finite_parameter("brightness", parameters.brightness)?;
        check_finite_parameter("amount", parameters.amount)?;
        Ok(Self { parameters })
    }
}

impl SoftenConfig {
    pub fn new(parameters: SoftenParametersV1) -> Result<Self, SoftenParameterError> {
        Self::try_from(parameters)
    }

    #[must_use]
    pub fn defaults() -> Self {
        Self::new(SoftenParametersV1::defaults()).expect("native soften defaults are valid")
    }

    #[must_use]
    pub const fn parameters(self) -> SoftenParametersV1 {
        self.parameters
    }
}

/// Operation-local descriptor evidence. Calling this does not register the leaf.
#[must_use]
pub fn soften_descriptor() -> OperationDescriptor {
    OperationDescriptor {
        id: DescriptorId::new(
            SOFTEN_COMPATIBILITY_ID,
            SOFTEN_RUST_ID,
            SOFTEN_SCHEMA_VERSION,
            SOFTEN_SCHEMA_VERSION,
            1,
        )
        .expect("static soften descriptor identity"),
        parameters: vec![
            scalar_parameter("size", 0.0, 100.0, 50.0, "percent", 0.1, 1),
            scalar_parameter("saturation", 0.0, 100.0, 100.0, "percent", 0.1, 1),
            scalar_parameter(
                "brightness",
                -2.0,
                2.0,
                f64::from(SOFTEN_DEFAULT_BRIGHTNESS),
                "ev",
                0.01,
                2,
            ),
            scalar_parameter("amount", 0.0, 100.0, 50.0, "percent", 0.1, 1),
        ],
        flags: OperationFlags::MULTI_INSTANCE
            .insert(OperationFlags::STYLE_ELIGIBLE)
            .insert(OperationFlags::HISTORY_VISIBLE)
            .insert(OperationFlags::TILEABLE)
            .insert(OperationFlags::DETERMINISTIC_CPU)
            .insert(OperationFlags::COLOR)
            .insert(OperationFlags::BLENDING),
        stage: "display-referred-linear-rgb".to_owned(),
        roi: RoiKind::Neighborhood,
        tiling: TilingContract {
            // `tiling_callback()` resolves overlap from committed size and ROI
            // scales; `SoftenPlan::tiling()` exposes that dynamic value.
            overlap_pixels: 0,
            alignment_pixels: 1,
            minimum_tile_edge: 1,
            preferred_tile_edge: 256,
            temporary_multiplier_milli: 2100,
            input_multiplier_milli: 1000,
            output_multiplier_milli: 1000,
        },
        capability: CapabilityContract {
            cpu_supported: true,
            gpu_tier: None,
            required_features: vec!["dynamic-neighborhood-overlap".to_owned()],
            required_formats: vec!["rgba32float".to_owned()],
            deterministic_cpu: true,
            deterministic_gpu: false,
            fallback_to_cpu: true,
            precision: "source-ordered f32 HSL and eight-pass box mean".to_owned(),
            modes: vec!["preview".to_owned(), "full".to_owned(), "export".to_owned()],
        },
        io: soften_io(),
        mask_blend: MaskBlendContract {
            consumes_mask: false,
            publishes_mask: false,
            blend_if: false,
            geometry: false,
            analysis: false,
        },
        migration: MigrationContract {
            source_versions: vec![SOFTEN_SCHEMA_VERSION],
            target_version: SOFTEN_SCHEMA_VERSION,
            opaque_unknown_allowed: true,
        },
        // Native GTK composition is owned by a different lane. A generic UI is
        // intentionally not advertised by this descriptor.
        ui: None,
    }
}

fn scalar_parameter(
    id: &str,
    minimum: f64,
    maximum: f64,
    default: f64,
    unit: &str,
    step: f64,
    precision: u8,
) -> ParameterDescriptor {
    ParameterDescriptor {
        id: id.to_owned(),
        kind: ParameterKind::Scalar { minimum, maximum },
        default: ParameterDefault::Scalar(default),
        required: false,
        introduced_version: SOFTEN_SCHEMA_VERSION,
        removed_version: None,
        unit: Some(unit.to_owned()),
        step: Some(step),
        precision,
        role: ParameterRole::Processing,
        cache_affecting: true,
        animatable: true,
        ui_hint: None,
        condition: None,
    }
}

fn soften_io() -> InputOutputContract {
    let encodings = vec![
        ColorEncoding::LinearSrgbD65,
        ColorEncoding::LinearDisplayP3D65,
        ColorEncoding::LinearRec2020D65,
    ];
    InputOutputContract {
        input: ImagePredicate {
            channels: SOFTEN_CHANNELS as u8,
            alpha: AlphaPolicy::Replace,
            encodings: encodings.clone(),
            nonfinite: NonFinitePolicy::Reject,
        },
        output: ImagePredicate {
            channels: SOFTEN_CHANNELS as u8,
            alpha: AlphaPolicy::Replace,
            encodings,
            nonfinite: NonFinitePolicy::Reject,
        },
        derives_output_encoding: false,
    }
}

/// Capability facts kept separate from source metadata and shared registration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SoftenCapabilities {
    pub cpu: bool,
    pub gpu: bool,
    pub gtk: bool,
    pub history_materialization: bool,
    pub outer_blending_and_masks: bool,
    pub production_routing: bool,
}

#[must_use]
pub const fn capabilities() -> SoftenCapabilities {
    SoftenCapabilities {
        cpu: true,
        gpu: false,
        gtk: false,
        history_materialization: false,
        outer_blending_and_masks: false,
        production_routing: false,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SoftenPortStatus {
    Ported,
    RustAdaptation,
    ExistingDependency,
    ExplicitlyDeferred,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SoftenSourceMapEntry {
    pub native_file: &'static str,
    pub native_symbol: &'static str,
    pub rust_symbol: &'static str,
    pub status: SoftenPortStatus,
}

pub const SOFTEN_SOURCE_MAP: &[SoftenSourceMapEntry] = &[
    SoftenSourceMapEntry {
        native_file: "src/iop/soften.c",
        native_symbol: "DT_MODULE_INTROSPECTION(1) / dt_iop_soften_params_t",
        rust_symbol: "SoftenParametersV1 / SoftenHistory",
        status: SoftenPortStatus::Ported,
    },
    SoftenSourceMapEntry {
        native_file: "src/iop/soften.c",
        native_symbol: "dt_iop_soften_data_t",
        rust_symbol: "SoftenConfig fields / SoftenPlan committed immutable state",
        status: SoftenPortStatus::Ported,
    },
    SoftenSourceMapEntry {
        native_file: "src/iop/soften.c",
        native_symbol: "commit_params direct field copies; introspection bounds are slider metadata",
        rust_symbol: "SoftenConfig::new accepts every finite parameter value",
        status: SoftenPortStatus::Ported,
    },
    SoftenSourceMapEntry {
        native_file: "src/iop/soften.c",
        native_symbol: "init_pipe calloc(1, sizeof(dt_iop_soften_data_t))",
        rust_symbol: "SoftenPlan construction owns initialized committed state",
        status: SoftenPortStatus::RustAdaptation,
    },
    SoftenSourceMapEntry {
        native_file: "src/iop/soften.c",
        native_symbol: "cleanup_pipe free and piece->data = NULL",
        rust_symbol: "SoftenPlan automatic Drop with no nullable raw allocation",
        status: SoftenPortStatus::RustAdaptation,
    },
    SoftenSourceMapEntry {
        native_file: "src/iop/soften.c",
        native_symbol: "commit_params permits non-finite payload bits",
        rust_symbol: "check_finite_parameter fail-closed executable boundary",
        status: SoftenPortStatus::RustAdaptation,
    },
    SoftenSourceMapEntry {
        native_file: "src/iop/soften.c; src/common/box_filters.h",
        native_symbol: "finite negative int radius converts to invalid size_t neighborhood",
        rust_symbol: "soften_radius maps unrepresentable negative neighborhoods to zero",
        status: SoftenPortStatus::RustAdaptation,
    },
    SoftenSourceMapEntry {
        native_file: "src/develop/imageop.c; src/iop/CMakeLists.txt",
        native_symbol: "default_enabled = FALSE / add_iop(soften) without DEFAULT_VISIBLE",
        rust_symbol: "SOFTEN_DEFAULT_ENABLED / SOFTEN_DEFAULT_VISIBLE",
        status: SoftenPortStatus::Ported,
    },
    SoftenSourceMapEntry {
        native_file: "src/iop/soften.c",
        native_symbol: "name / flags / default_group / default_colorspace / description",
        rust_symbol: "soften_descriptor / SOFTEN_DESCRIPTION / SOFTEN_DEFAULT_*",
        status: SoftenPortStatus::Ported,
    },
    SoftenSourceMapEntry {
        native_file: "src/common/iop_order.c",
        native_symbol: "legacy_order / v30_order / v50_order / v30_jpg_order / v50_jpg_order",
        rust_symbol: "SOFTEN_OPERATION_ORDERS",
        status: SoftenPortStatus::Ported,
    },
    SoftenSourceMapEntry {
        native_file: "src/CMakeLists.txt; src/common/math.h",
        native_symbol: concat!(
            "Release -ffast",
            "-math selects __FAST_MATH__ dt_fast_hypotf = sqrtf(x*x+y*y)",
        ),
        rust_symbol: "release_fast_hypotf locks retained release-profile f32 operation order",
        status: SoftenPortStatus::RustAdaptation,
    },
    SoftenSourceMapEntry {
        native_file: "src/common/math.h",
        native_symbol: "CLIP comparison branches and NaN-to-zero semantics",
        rust_symbol: "clip_native",
        status: SoftenPortStatus::Ported,
    },
    SoftenSourceMapEntry {
        native_file: "src/iop/soften.c; src/common/colorspaces.h",
        native_symbol: "process / rgb2hsl / hsl2rgb",
        rust_symbol: "SoftenPlan::execute_* / adjust_rgb",
        status: SoftenPortStatus::Ported,
    },
    SoftenSourceMapEntry {
        native_file: "src/develop/imageop.c; src/common/imagebuf.c",
        native_symbol: "dt_iop_have_required_input_format / dt_iop_copy_image_roi copy-through",
        rust_symbol: "SoftenPlan::copy_through_unsupported_input",
        status: SoftenPortStatus::Ported,
    },
    SoftenSourceMapEntry {
        native_file: "src/develop/imageop.c",
        native_symbol: "required-format trouble message publication / debug logging",
        rust_symbol: "shared module trouble and logging routing",
        status: SoftenPortStatus::ExplicitlyDeferred,
    },
    SoftenSourceMapEntry {
        native_file: "src/common/box_filters.h; src/common/box_filters.cc",
        native_symbol: "BOX_ITERATIONS / dt_box_mean nonzero-radius passes",
        rust_symbol: "box_mean_with_cancel through soften_box_mean_with_cancel",
        status: SoftenPortStatus::ExistingDependency,
    },
    SoftenSourceMapEntry {
        native_file: "src/common/box_filters.cc",
        native_symbol: "dt_box_mean radius-zero eight horizontal/vertical pass iterations",
        rust_symbol: "soften_zero_radius_box_mean_with_cancel operation-local path",
        status: SoftenPortStatus::RustAdaptation,
    },
    SoftenSourceMapEntry {
        native_file: "src/common/imagebuf.c",
        native_symbol: "dt_iop_image_linear_blend",
        rust_symbol: "SoftenPlan four-lane publication blend",
        status: SoftenPortStatus::Ported,
    },
    SoftenSourceMapEntry {
        native_file: "src/iop/soften.c",
        native_symbol: "tiling_callback",
        rust_symbol: "SoftenPlan::tiling",
        status: SoftenPortStatus::Ported,
    },
    SoftenSourceMapEntry {
        native_file: "src/iop/soften.c; data/kernels/soften.cl",
        native_symbol: "process_cl / init_global / cleanup_global",
        rust_symbol: "GPU unavailable",
        status: SoftenPortStatus::ExplicitlyDeferred,
    },
    SoftenSourceMapEntry {
        native_file: "src/iop/soften.c",
        native_symbol: "gui_init / shared blending and history routing",
        rust_symbol: "GTK, import, registry, pixelpipe, masks, and outer blending",
        status: SoftenPortStatus::ExplicitlyDeferred,
    },
];

/// Four native float lanes. `soften.c::hsl2rgb` writes zero to lane four,
/// `dt_box_mean` filters all four lanes, then the final blend mixes that zero
/// with the source lane. It is therefore not an independently preserved alpha.
#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(transparent)]
pub struct SoftenPixel {
    channels: [f32; SOFTEN_CHANNELS],
}

const _: () = assert!(size_of::<SoftenPixel>() == size_of::<[f32; SOFTEN_CHANNELS]>());

impl SoftenPixel {
    #[must_use]
    pub const fn new(red: f32, green: f32, blue: f32, fourth: f32) -> Self {
        Self::from_channels([red, green, blue, fourth])
    }

    #[must_use]
    pub const fn from_channels(channels: [f32; SOFTEN_CHANNELS]) -> Self {
        Self { channels }
    }

    #[must_use]
    pub const fn channels(self) -> [f32; SOFTEN_CHANNELS] {
        self.channels
    }

    #[must_use]
    pub const fn fourth(self) -> f32 {
        self.channels[3]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SoftenDimensions {
    width: u32,
    height: u32,
}

impl SoftenDimensions {
    pub fn new(width: u32, height: u32) -> Result<Self, SoftenExecutionError> {
        if width == 0 || height == 0 {
            return Err(SoftenExecutionError::InvalidDimensions);
        }
        let dimensions = Self { width, height };
        let _ = dimensions.pixel_count()?;
        Ok(dimensions)
    }

    #[must_use]
    pub fn width(self) -> usize {
        usize::try_from(self.width).expect("u32 width fits usize")
    }

    #[must_use]
    pub fn height(self) -> usize {
        usize::try_from(self.height).expect("u32 height fits usize")
    }

    pub fn pixel_count(self) -> Result<usize, SoftenExecutionError> {
        self.width()
            .checked_mul(self.height())
            .ok_or(SoftenExecutionError::DimensionsTooLarge)
    }
}

/// Integer ROI geometry used by native `dt_iop_copy_image_roi`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SoftenRoi {
    x: i32,
    y: i32,
    dimensions: SoftenDimensions,
}

impl SoftenRoi {
    pub fn new(x: i32, y: i32, width: u32, height: u32) -> Result<Self, SoftenExecutionError> {
        Ok(Self {
            x,
            y,
            dimensions: SoftenDimensions::new(width, height)?,
        })
    }

    #[must_use]
    pub const fn x(self) -> i32 {
        self.x
    }

    #[must_use]
    pub const fn y(self) -> i32 {
        self.y
    }

    #[must_use]
    pub const fn dimensions(self) -> SoftenDimensions {
        self.dimensions
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SoftenTiling {
    pub factor: f32,
    pub factor_cl: f32,
    pub maxbuf: f32,
    pub overhead: u32,
    pub overlap: u32,
    pub align: u32,
}

/// Complete four-channel publication candidate.
#[derive(Debug, Clone, PartialEq)]
pub struct SoftenExecution {
    pub pixels: Vec<SoftenPixel>,
    pub input_format_problem: bool,
}

/// Packed copy-through candidate for an unsupported native input format.
///
/// `channels` is exactly `min(actual_channels, 4)`, matching the argument that
/// `dt_iop_have_required_input_format` passes to `dt_iop_copy_image_roi`.
#[derive(Debug, Clone, PartialEq)]
pub struct SoftenPackedExecution {
    pub samples: Vec<f32>,
    pub channels: usize,
    pub input_format_problem: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SoftenExecutionError {
    InvalidDimensions,
    DimensionsTooLarge,
    DimensionsMismatch {
        expected: usize,
        actual: usize,
    },
    SampleCountMismatch {
        expected: usize,
        actual: usize,
    },
    InvalidChannelCount {
        actual: usize,
    },
    RequiredFormatAlreadySatisfied,
    InvalidScale,
    MemoryBudgetExceeded {
        required: usize,
        budget: usize,
    },
    AllocationFailed {
        required: usize,
    },
    Cancelled,
    NonFiniteInput {
        pixel: usize,
        channel: usize,
    },
    NonFiniteResult {
        stage: &'static str,
        pixel: usize,
        channel: usize,
    },
    InternalFilterContract,
}

impl fmt::Display for SoftenExecutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidDimensions => formatter.write_str("soften dimensions must be nonzero"),
            Self::DimensionsTooLarge => {
                formatter.write_str("soften dimensions exceed supported arithmetic")
            }
            Self::DimensionsMismatch { expected, actual } => {
                write!(formatter, "soften expected {expected} pixels, got {actual}")
            }
            Self::SampleCountMismatch { expected, actual } => {
                write!(
                    formatter,
                    "soften expected {expected} samples, got {actual}"
                )
            }
            Self::InvalidChannelCount { actual } => {
                write!(
                    formatter,
                    "soften cannot copy an input with {actual} channels"
                )
            }
            Self::RequiredFormatAlreadySatisfied => formatter.write_str(
                "soften packed copy-through is only available for unsupported channel counts",
            ),
            Self::InvalidScale => {
                formatter.write_str("soften requires finite positive ROI and piece scales")
            }
            Self::MemoryBudgetExceeded { required, budget } => write!(
                formatter,
                "soften needs {required} bytes, above {budget} byte budget"
            ),
            Self::AllocationFailed { required } => {
                write!(formatter, "soften failed to allocate {required} bytes")
            }
            Self::Cancelled => formatter.write_str("soften execution was cancelled"),
            Self::NonFiniteInput { pixel, channel } => write!(
                formatter,
                "soften input pixel {pixel}, channel {channel} is non-finite"
            ),
            Self::NonFiniteResult {
                stage,
                pixel,
                channel,
            } => write!(
                formatter,
                "soften produced a non-finite value during {stage} at pixel {pixel}, channel {channel}"
            ),
            Self::InternalFilterContract => {
                formatter.write_str("soften box mean rejected a validated internal buffer")
            }
        }
    }
}

impl std::error::Error for SoftenExecutionError {}

/// Immutable committed CPU state. Full-image geometry controls native radius;
/// an expanded tile may be executed with separate raster dimensions, but crop,
/// scheduling, and publication into a production pixelpipe remain deferred.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SoftenPlan {
    config: SoftenConfig,
    full_dimensions: SoftenDimensions,
    roi_scale: f32,
    piece_scale: f32,
    radius: usize,
    memory_budget: usize,
}

impl SoftenPlan {
    pub fn new(
        config: SoftenConfig,
        full_dimensions: SoftenDimensions,
    ) -> Result<Self, SoftenExecutionError> {
        Self::new_with_budget(
            config,
            full_dimensions,
            1.0,
            1.0,
            SOFTEN_DEFAULT_MEMORY_BUDGET,
        )
    }

    pub fn new_with_scale(
        config: SoftenConfig,
        full_dimensions: SoftenDimensions,
        roi_scale: f32,
        piece_scale: f32,
    ) -> Result<Self, SoftenExecutionError> {
        Self::new_with_budget(
            config,
            full_dimensions,
            roi_scale,
            piece_scale,
            SOFTEN_DEFAULT_MEMORY_BUDGET,
        )
    }

    pub fn new_with_budget(
        config: SoftenConfig,
        full_dimensions: SoftenDimensions,
        roi_scale: f32,
        piece_scale: f32,
        memory_budget: usize,
    ) -> Result<Self, SoftenExecutionError> {
        let radius = soften_radius(
            config.parameters.size,
            full_dimensions,
            roi_scale,
            piece_scale,
        )?;
        Ok(Self {
            config,
            full_dimensions,
            roi_scale,
            piece_scale,
            radius,
            memory_budget,
        })
    }

    #[must_use]
    pub const fn config(self) -> SoftenConfig {
        self.config
    }

    #[must_use]
    pub const fn full_dimensions(self) -> SoftenDimensions {
        self.full_dimensions
    }

    #[must_use]
    pub const fn radius(self) -> usize {
        self.radius
    }

    #[must_use]
    pub const fn roi_scale(self) -> f32 {
        self.roi_scale
    }

    #[must_use]
    pub const fn piece_scale(self) -> f32 {
        self.piece_scale
    }

    pub fn tiling(self) -> Result<SoftenTiling, SoftenExecutionError> {
        let radius =
            u64::try_from(self.radius).map_err(|_| SoftenExecutionError::DimensionsTooLarge)?;
        let numerator = radius
            .checked_mul(
                radius
                    .checked_add(1)
                    .ok_or(SoftenExecutionError::DimensionsTooLarge)?,
            )
            .and_then(|value| value.checked_mul(u64::from(BOX_ITERATIONS)))
            .and_then(|value| value.checked_add(2))
            .ok_or(SoftenExecutionError::DimensionsTooLarge)?;
        let sigma = (numerator as f32 / 3.0_f32).sqrt();
        let overlap = (3.0_f32 * sigma).ceil();
        if !overlap.is_finite() || overlap > u32::MAX as f32 {
            return Err(SoftenExecutionError::DimensionsTooLarge);
        }
        Ok(SoftenTiling {
            factor: 2.1,
            factor_cl: 3.0,
            maxbuf: 1.0,
            overhead: 0,
            overlap: overlap as u32,
            align: 1,
        })
    }

    pub fn execute(&self, input: &[SoftenPixel]) -> Result<SoftenExecution, SoftenExecutionError> {
        self.execute_with_cancel(input, || false)
    }

    pub fn execute_with_cancel<F: Fn() -> bool>(
        &self,
        input: &[SoftenPixel],
        cancelled: F,
    ) -> Result<SoftenExecution, SoftenExecutionError> {
        self.execute_raster_with_cancel(input, self.full_dimensions, cancelled)
    }

    pub fn execute_raster(
        &self,
        input: &[SoftenPixel],
        raster_dimensions: SoftenDimensions,
    ) -> Result<SoftenExecution, SoftenExecutionError> {
        self.execute_raster_with_cancel(input, raster_dimensions, || false)
    }

    /// Executes only the supported native four-channel format.
    ///
    /// Unsupported formats use [`Self::copy_through_unsupported_input`], whose
    /// packed representation can preserve the actual copied channel count and
    /// distinct input/output ROI geometry without fabricating RGBA lanes.
    pub fn execute_raster_with_cancel<F: Fn() -> bool>(
        &self,
        input: &[SoftenPixel],
        raster_dimensions: SoftenDimensions,
        cancelled: F,
    ) -> Result<SoftenExecution, SoftenExecutionError> {
        let expected = raster_dimensions.pixel_count()?;
        if input.len() != expected {
            return Err(SoftenExecutionError::DimensionsMismatch {
                expected,
                actual: input.len(),
            });
        }
        if cancelled() {
            return Err(SoftenExecutionError::Cancelled);
        }
        validate_input(input, raster_dimensions, &cancelled)?;
        self.ensure_budget(raster_dimensions)?;
        self.execute_inner(input, raster_dimensions, &cancelled)
    }

    /// Ports the mismatch branch of `dt_iop_have_required_input_format` and
    /// `dt_iop_copy_image_roi` over packed samples.
    ///
    /// Native copy-through uses `min(actual_channels, 4)` lanes, crops by ROI
    /// origin when the output is smaller, and zero-pads unavailable output
    /// locations when it is larger. Non-finite samples are copied bit-for-bit;
    /// the native format guard returns before Soften processing inspects them.
    pub fn copy_through_unsupported_input(
        &self,
        input: &[f32],
        actual_channels: usize,
        roi_in: SoftenRoi,
        roi_out: SoftenRoi,
    ) -> Result<SoftenPackedExecution, SoftenExecutionError> {
        if actual_channels == 0 {
            return Err(SoftenExecutionError::InvalidChannelCount {
                actual: actual_channels,
            });
        }
        if actual_channels == SOFTEN_CHANNELS {
            return Err(SoftenExecutionError::RequiredFormatAlreadySatisfied);
        }
        let copied_channels = actual_channels.min(SOFTEN_CHANNELS);
        let expected = packed_sample_count(roi_in.dimensions, copied_channels)?;
        if input.len() != expected {
            return Err(SoftenExecutionError::SampleCountMismatch {
                expected,
                actual: input.len(),
            });
        }
        let output_samples = packed_sample_count(roi_out.dimensions, copied_channels)?;
        let publication_bytes = output_samples
            .checked_mul(size_of::<f32>())
            .ok_or(SoftenExecutionError::DimensionsTooLarge)?;
        if publication_bytes > self.memory_budget {
            return Err(SoftenExecutionError::MemoryBudgetExceeded {
                required: publication_bytes,
                budget: self.memory_budget,
            });
        }
        Ok(SoftenPackedExecution {
            samples: copy_packed_image_roi(
                input,
                copied_channels,
                roi_in,
                roi_out,
                publication_bytes,
            )?,
            channels: copied_channels,
            input_format_problem: true,
        })
    }

    fn execute_inner<F: Fn() -> bool>(
        &self,
        input: &[SoftenPixel],
        dimensions: SoftenDimensions,
        cancelled: &F,
    ) -> Result<SoftenExecution, SoftenExecutionError> {
        let width = dimensions.width();
        let height = dimensions.height();
        let pixel_count = dimensions.pixel_count()?;
        let sample_count = pixel_count
            .checked_mul(SOFTEN_CHANNELS)
            .ok_or(SoftenExecutionError::DimensionsTooLarge)?;
        let sample_bytes = sample_count
            .checked_mul(size_of::<f32>())
            .ok_or(SoftenExecutionError::DimensionsTooLarge)?;
        let mut adjusted = Vec::new();
        adjusted.try_reserve_exact(sample_count).map_err(|_| {
            SoftenExecutionError::AllocationFailed {
                required: sample_bytes,
            }
        })?;

        // Preserve the CPU source's promotion boundaries rather than replacing
        // the reciprocal with the algebraically equivalent `exp2(brightness)`.
        let saturation = (f64::from(self.config.parameters.saturation) / 100.0) as f32;
        let brightness = (1.0_f64 / f64::from((-self.config.parameters.brightness).exp2())) as f32;

        for (index, pixel) in input.iter().copied().enumerate() {
            if index % width == 0 && cancelled() {
                return Err(SoftenExecutionError::Cancelled);
            }
            let channels = pixel.channels();
            let rgb = adjust_rgb(
                [channels[0], channels[1], channels[2]],
                saturation,
                brightness,
                index,
            )?;
            // `hsl2rgb` always writes zero to lane four before the box mean.
            adjusted.extend_from_slice(&[rgb[0], rgb[1], rgb[2], 0.0]);
        }

        soften_box_mean_with_cancel(&mut adjusted, height, width, self.radius, cancelled)?;
        if cancelled() {
            return Err(SoftenExecutionError::Cancelled);
        }

        let amount = self.config.parameters.amount / 100.0_f32;
        let remainder = 1.0_f32 - amount;
        let mut output = Vec::new();
        output.try_reserve_exact(pixel_count).map_err(|_| {
            SoftenExecutionError::AllocationFailed {
                required: pixel_count.saturating_mul(size_of::<SoftenPixel>()),
            }
        })?;
        let (adjusted_pixels, remainder_samples) = adjusted.as_chunks::<SOFTEN_CHANNELS>();
        debug_assert!(remainder_samples.is_empty());
        for (index, processed) in adjusted_pixels.iter().enumerate() {
            if index % width == 0 && cancelled() {
                return Err(SoftenExecutionError::Cancelled);
            }
            let original = input[index].channels();
            let channels = std::array::from_fn(|channel| {
                amount * processed[channel] + remainder * original[channel]
            });
            for (channel, value) in channels.iter().copied().enumerate() {
                if !value.is_finite() {
                    return Err(SoftenExecutionError::NonFiniteResult {
                        stage: "linear blend",
                        pixel: index,
                        channel,
                    });
                }
            }
            output.push(SoftenPixel::from_channels(channels));
        }
        if cancelled() {
            return Err(SoftenExecutionError::Cancelled);
        }
        Ok(SoftenExecution {
            pixels: output,
            input_format_problem: false,
        })
    }

    fn ensure_budget(self, dimensions: SoftenDimensions) -> Result<(), SoftenExecutionError> {
        let required = required_bytes(dimensions, self.radius)?;
        if required > self.memory_budget {
            return Err(SoftenExecutionError::MemoryBudgetExceeded {
                required,
                budget: self.memory_budget,
            });
        }
        Ok(())
    }
}

fn soften_box_mean_with_cancel<F: Fn() -> bool>(
    buffer: &mut [f32],
    height: usize,
    width: usize,
    radius: usize,
    cancelled: &F,
) -> Result<(), SoftenExecutionError> {
    if radius == 0 {
        return soften_zero_radius_box_mean_with_cancel(buffer, height, width, cancelled);
    }
    box_mean_with_cancel(
        buffer,
        height,
        width,
        SOFTEN_CHANNELS as u32,
        radius,
        BOX_ITERATIONS,
        cancelled,
    )
    .map_err(map_cancellable_filter_error)
}

/// Operation-local radius-zero path for native `dt_box_mean` semantics.
///
/// The shared safe helper intentionally treats a zero radius as an identity,
/// while retained `_box_mean` still runs eight horizontal/vertical iterations.
/// Keeping this path local avoids changing other operations and preserves the
/// signed-zero effects of the source additions, subtractions, and divisions.
fn soften_zero_radius_box_mean_with_cancel<F: Fn() -> bool>(
    buffer: &mut [f32],
    height: usize,
    width: usize,
    cancelled: &F,
) -> Result<(), SoftenExecutionError> {
    let row_len = width
        .checked_mul(SOFTEN_CHANNELS)
        .ok_or(SoftenExecutionError::DimensionsTooLarge)?;
    debug_assert_eq!(buffer.len(), row_len * height);

    for _ in 0..BOX_ITERATIONS {
        if cancelled() {
            return Err(SoftenExecutionError::Cancelled);
        }
        for row in buffer.chunks_exact_mut(row_len) {
            let mut sum = [0.0_f32; SOFTEN_CHANNELS];
            let mut previous = [0.0_f32; SOFTEN_CHANNELS];
            for column in 0..width {
                if cancelled() {
                    return Err(SoftenExecutionError::Cancelled);
                }
                let offset = column * SOFTEN_CHANNELS;
                let current = std::array::from_fn(|channel| row[offset + channel]);
                for channel in 0..SOFTEN_CHANNELS {
                    if column != 0 {
                        sum[channel] -= previous[channel];
                    }
                    sum[channel] += current[channel];
                    row[offset + channel] = sum[channel] / 1.0_f32;
                }
                previous = current;
            }
        }

        for column in 0..row_len {
            let mut sum = 0.0_f32;
            let mut previous = 0.0_f32;
            for row in 0..height {
                if cancelled() {
                    return Err(SoftenExecutionError::Cancelled);
                }
                let offset = row * row_len + column;
                let current = buffer[offset];
                if row != 0 {
                    sum -= previous;
                }
                sum += current;
                buffer[offset] = sum / 1.0_f32;
                previous = current;
            }
        }
    }
    Ok(())
}

fn required_bytes(
    dimensions: SoftenDimensions,
    radius: usize,
) -> Result<usize, SoftenExecutionError> {
    let pixels = dimensions.pixel_count()?;
    let image_bytes = pixels
        .checked_mul(size_of::<SoftenPixel>())
        .ok_or(SoftenExecutionError::DimensionsTooLarge)?;
    let scratch_bytes = box_mean_with_cancel_scratch_bytes(
        dimensions.height(),
        dimensions.width(),
        SOFTEN_CHANNELS as u32,
        radius,
        BOX_ITERATIONS,
    )
    .map_err(map_filter_error)?;
    image_bytes
        .checked_mul(2)
        .and_then(|bytes| bytes.checked_add(scratch_bytes))
        .ok_or(SoftenExecutionError::DimensionsTooLarge)
}

fn validate_input<F: Fn() -> bool>(
    input: &[SoftenPixel],
    dimensions: SoftenDimensions,
    cancelled: &F,
) -> Result<(), SoftenExecutionError> {
    let width = dimensions.width();
    for (pixel, value) in input.iter().copied().enumerate() {
        if pixel % width == 0 && cancelled() {
            return Err(SoftenExecutionError::Cancelled);
        }
        for (channel, sample) in value.channels().into_iter().enumerate() {
            if !sample.is_finite() {
                return Err(SoftenExecutionError::NonFiniteInput { pixel, channel });
            }
        }
    }
    Ok(())
}

fn soften_radius(
    size: f32,
    dimensions: SoftenDimensions,
    roi_scale: f32,
    piece_scale: f32,
) -> Result<usize, SoftenExecutionError> {
    if !roi_scale.is_finite() || roi_scale <= 0.0 || !piece_scale.is_finite() || piece_scale <= 0.0
    {
        return Err(SoftenExecutionError::InvalidScale);
    }
    let width = dimensions.width as f32 * piece_scale;
    let height = dimensions.height as f32 * piece_scale;
    let maximum = release_fast_hypotf(width, height) * 0.01_f32;
    if !maximum.is_finite() || maximum > i32::MAX as f32 {
        return Err(SoftenExecutionError::DimensionsTooLarge);
    }
    let maximum_radius = maximum as i32;
    let capped_size = f64::from(size + 1.0_f32).min(100.0);
    let requested = f64::from(maximum_radius) * (capped_size / 100.0);
    if !requested.is_finite() || requested > f64::from(i32::MAX) {
        return Err(SoftenExecutionError::DimensionsTooLarge);
    }
    // A negative native `int radius` becomes an enormous `size_t` at the
    // `dt_box_mean` boundary. Safe Rust cannot reproduce that invalid lifetime
    // or allocation state, so finite negative requests remain executable as the
    // nearest representable empty neighborhood.
    if requested <= 0.0 {
        return Ok(0);
    }
    let requested_radius = requested as i32;
    let scaled = (requested_radius as f32 * roi_scale / piece_scale).ceil();
    if !scaled.is_finite() || scaled > i32::MAX as f32 {
        return Err(SoftenExecutionError::InvalidScale);
    }
    usize::try_from(maximum_radius.min(scaled as i32))
        .map_err(|_| SoftenExecutionError::DimensionsTooLarge)
}

/// Retained release-profile branch of `common/math.h::dt_fast_hypotf`.
fn release_fast_hypotf(x: f32, y: f32) -> f32 {
    let x_squared = x * x;
    let y_squared = y * y;
    (x_squared + y_squared).sqrt()
}

fn adjust_rgb(
    pixel: [f32; 3],
    saturation_scale: f32,
    brightness_scale: f32,
    index: usize,
) -> Result<[f32; 3], SoftenExecutionError> {
    let (hue, saturation, lightness) = rgb_to_hsl(pixel);
    let saturation = clip_native(saturation * saturation_scale);
    let lightness = clip_native(lightness * brightness_scale);
    let output = hsl_to_rgb(hue, saturation, lightness);
    for (channel, value) in output.iter().copied().enumerate() {
        if !value.is_finite() {
            return Err(SoftenExecutionError::NonFiniteResult {
                stage: "HSL adjustment",
                pixel: index,
                channel,
            });
        }
    }
    Ok(output)
}

/// Direct scalar port of `src/common/colorspaces.h::rgb2hsl`.
fn rgb_to_hsl(rgb: [f32; 3]) -> (f32, f32, f32) {
    let [red, green, blue] = rgb;
    let maximum = red.max(green).max(blue);
    let minimum = red.min(green).min(blue);
    let delta = maximum - minimum;
    let mut hue = 0.0_f32;
    let mut saturation = 0.0_f32;
    let lightness = (f64::from(minimum + maximum) / 2.0) as f32;

    if delta != 0.0_f32 {
        saturation = if lightness < 0.5_f32 {
            delta / (maximum + minimum).max(1.525_878_906_25e-5_f32)
        } else {
            let denominator = (2.0_f64 - f64::from(maximum) - f64::from(minimum)) as f32;
            delta / denominator.max(1.525_878_906_25e-5_f32)
        };
        let hue_angle = if maximum == red {
            (green - blue) / delta
        } else if maximum == green {
            (2.0_f64 + f64::from((blue - red) / delta)) as f32
        } else {
            (4.0_f64 + f64::from((red - green) / delta)) as f32
        };
        hue = (f64::from(hue_angle) / 6.0) as f32;
        if f64::from(hue) < 0.0 {
            hue = (f64::from(hue) + 1.0) as f32;
        } else if f64::from(hue) > 1.0 {
            hue = (f64::from(hue) - 1.0) as f32;
        }
    }
    (hue, saturation, lightness)
}

/// Direct scalar port of `src/common/colorspaces.h::hsl2rgb`.
fn hsl_to_rgb(hue: f32, saturation: f32, lightness: f32) -> [f32; 3] {
    if saturation == 0.0_f32 {
        return [lightness; 3];
    }
    let second = if lightness < 0.5_f32 {
        (f64::from(lightness) * (1.0 + f64::from(saturation))) as f32
    } else {
        lightness + saturation - lightness * saturation
    };
    let first = (2.0_f64 * f64::from(lightness) - f64::from(second)) as f32;
    let angle = hue * 6.0_f32;
    [
        hue_to_rgb(
            first,
            second,
            if angle < 4.0_f32 {
                angle + 2.0_f32
            } else {
                angle - 4.0_f32
            },
        ),
        hue_to_rgb(first, second, angle),
        hue_to_rgb(
            first,
            second,
            if angle > 2.0_f32 {
                angle - 2.0_f32
            } else {
                angle + 4.0_f32
            },
        ),
    ]
}

fn hue_to_rgb(first: f32, second: f32, hue: f32) -> f32 {
    if hue < 1.0_f32 {
        first + (second - first) * hue
    } else if hue < 3.0_f32 {
        second
    } else if hue < 4.0_f32 {
        first + (second - first) * (4.0_f32 - hue)
    } else {
        first
    }
}

fn clip_native(value: f32) -> f32 {
    if value >= 0.0_f32 {
        if value <= 1.0_f32 { value } else { 1.0_f32 }
    } else {
        0.0_f32
    }
}

const fn map_cancellable_filter_error(error: CancellableBoxFilterError) -> SoftenExecutionError {
    match error {
        CancellableBoxFilterError::Cancelled => SoftenExecutionError::Cancelled,
        CancellableBoxFilterError::Filter(error) => map_filter_error(error),
    }
}

const fn map_filter_error(error: BoxFilterError) -> SoftenExecutionError {
    match error {
        BoxFilterError::AllocationFailed { required_bytes } => {
            SoftenExecutionError::AllocationFailed {
                required: required_bytes,
            }
        }
        BoxFilterError::SizeOverflow => SoftenExecutionError::DimensionsTooLarge,
        BoxFilterError::BufferShape { expected, actual } => {
            SoftenExecutionError::DimensionsMismatch { expected, actual }
        }
        BoxFilterError::NonFiniteInput { sample } => SoftenExecutionError::NonFiniteResult {
            stage: "box mean",
            pixel: sample / SOFTEN_CHANNELS,
            channel: sample % SOFTEN_CHANNELS,
        },
        BoxFilterError::InvalidDimensions { .. }
        | BoxFilterError::UnsupportedChannels { .. }
        | BoxFilterError::ScratchShape { .. } => SoftenExecutionError::InternalFilterContract,
    }
}

fn packed_sample_count(
    dimensions: SoftenDimensions,
    channels: usize,
) -> Result<usize, SoftenExecutionError> {
    dimensions
        .pixel_count()?
        .checked_mul(channels)
        .ok_or(SoftenExecutionError::DimensionsTooLarge)
}

fn copy_packed_image_roi(
    input: &[f32],
    channels: usize,
    roi_in: SoftenRoi,
    roi_out: SoftenRoi,
    required_bytes: usize,
) -> Result<Vec<f32>, SoftenExecutionError> {
    let output_samples = packed_sample_count(roi_out.dimensions, channels)?;
    let mut output = Vec::new();
    output.try_reserve_exact(output_samples).map_err(|_| {
        SoftenExecutionError::AllocationFailed {
            required: required_bytes,
        }
    })?;

    // Preserve `dt_iop_copy_image_roi`'s dimension-only fast path: equal-sized
    // buffers are copied wholesale even if their ROI origins differ.
    if roi_in.dimensions == roi_out.dimensions {
        output.extend_from_slice(input);
        return Ok(output);
    }

    let delta_y = i64::from(roi_out.y) - i64::from(roi_in.y);
    let delta_x = i64::from(roi_out.x) - i64::from(roi_in.x);
    let input_width = roi_in.dimensions.width();
    let input_height = i64::from(roi_in.dimensions.height);
    let input_width_signed = i64::from(roi_in.dimensions.width);

    for output_row in 0..roi_out.dimensions.height() {
        let input_row =
            i64::try_from(output_row).expect("u32-backed output row fits i64") + delta_y;
        for output_column in 0..roi_out.dimensions.width() {
            let input_column =
                i64::try_from(output_column).expect("u32-backed output column fits i64") + delta_x;
            if input_row >= 0
                && input_row < input_height
                && input_column >= 0
                && input_column < input_width_signed
            {
                let input_pixel = usize::try_from(input_row)
                    .expect("non-negative bounded input row")
                    .checked_mul(input_width)
                    .and_then(|row| {
                        row.checked_add(
                            usize::try_from(input_column)
                                .expect("non-negative bounded input column"),
                        )
                    })
                    .ok_or(SoftenExecutionError::DimensionsTooLarge)?;
                let sample_start = input_pixel
                    .checked_mul(channels)
                    .ok_or(SoftenExecutionError::DimensionsTooLarge)?;
                let sample_end = sample_start
                    .checked_add(channels)
                    .ok_or(SoftenExecutionError::DimensionsTooLarge)?;
                output.extend_from_slice(&input[sample_start..sample_end]);
            } else {
                output.extend(std::iter::repeat_n(0.0, channels));
            }
        }
    }
    debug_assert_eq!(output.len(), output_samples);
    Ok(output)
}

const fn check_finite_parameter(
    name: &'static str,
    value: f32,
) -> Result<(), SoftenParameterError> {
    if !value.is_finite() {
        return Err(SoftenParameterError::NonFinite(name));
    }
    Ok(())
}

fn write_f32(bytes: &mut [u8], offset: usize, value: f32) {
    bytes[offset..offset + size_of::<f32>()].copy_from_slice(&value.to_le_bytes());
}

const fn read_f32(bytes: &[u8], offset: usize) -> f32 {
    f32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}
