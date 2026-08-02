//! Bounded Color Correction CPU leaf ported from `src/iop/colorcorrection.c`.
//!
//! This operation-local module owns the native v1 history ABI, the empty
//! migration topology, a non-exported descriptor, checked `commit_params`
//! planning, an explicit separate-rounding scalar Lab adaptation, channel-four
//! preservation, finite failure handling, bounded cancellation, and
//! transactional output publication. Shared history/descriptor/registry export,
//! pixelpipe routing, ROI-aware format copy-through diagnostics, masks and outer
//! blending, OpenCL/WGPU,
//! preset registration, GTK, and app integration remain explicitly deferred.
//! Native presentation strings and exact preset tuples are retained as local
//! source evidence without claiming either shared descriptor or preset routing.
//!
//! The root is named `leaf.rs` rather than `mod.rs` because the requested
//! baseline still has a provisional shared `operations/colorcorrection.rs`.
//! Keeping this leaf operation-local avoids changing or shadowing that shared
//! module before an integration owner can replace its callers coherently.

#![forbid(unsafe_code)]
#![allow(
    clippy::missing_errors_doc,
    clippy::struct_excessive_bools,
    dead_code,
    reason = "the bounded leaf is path-mounted only by its focused contract test"
)]

use std::fmt;

use rusttable_color::ColorEncoding;

use crate::descriptor::{
    AlphaPolicy, CapabilityContract, DescriptorId, ImagePredicate, InputOutputContract,
    MaskBlendContract, MigrationContract, NonFinitePolicy, OperationDescriptor, OperationFlags,
    ParameterDefault, ParameterDescriptor, ParameterKind, ParameterRole, RoiKind, TilingContract,
};

#[path = "codec.rs"]
pub mod codec;
#[path = "execution.rs"]
pub mod execution;
#[path = "source_map.rs"]
pub mod source_map;

#[allow(unused_imports)]
pub use codec::{
    COLORCORRECTION_MIGRATION_EDGES, COLORCORRECTION_SCHEMA_VERSION,
    COLORCORRECTION_V1_PARAMETER_BYTES, ColorCorrectionCodecError, ColorCorrectionHistory,
    ColorCorrectionParametersV1,
};
#[allow(unused_imports)]
pub use execution::{
    COLORCORRECTION_CPU_ARITHMETIC_PROFILE, ColorCorrectionChannel, ColorCorrectionCoefficients,
    ColorCorrectionConfig, ColorCorrectionCpuArithmeticProfile, ColorCorrectionExecutionError,
    ColorCorrectionParameterError, ColorCorrectionPixel, ColorCorrectionPlan,
    ColorCorrectionPlanError,
};

pub const COLORCORRECTION_COMPATIBILITY_ID: &str = "colorcorrection";
pub const COLORCORRECTION_RUST_ID: &str = "rusttable.colorcorrection";
pub const COLORCORRECTION_NATIVE_NAME: &str = "color correction";
pub const COLORCORRECTION_NATIVE_DESCRIPTION: [&str; 5] = [
    "correct white balance selectively for blacks and whites",
    "corrective or creative",
    "non-linear, Lab, display-referred",
    "non-linear, Lab",
    "non-linear, Lab, display-referred",
];
pub const COLORCORRECTION_PRESET_BLEND_COLORSPACE: &str = "DEVELOP_BLEND_CS_RGB_DISPLAY";
pub const COLORCORRECTION_PARAMETER_ORDER: [&str; 5] = ["hia", "hib", "loa", "lob", "saturation"];
pub const COLORCORRECTION_DEFAULT_HIA: f32 = 0.0;
pub const COLORCORRECTION_DEFAULT_HIB: f32 = 0.0;
pub const COLORCORRECTION_DEFAULT_LOA: f32 = 0.0;
pub const COLORCORRECTION_DEFAULT_LOB: f32 = 0.0;
pub const COLORCORRECTION_DEFAULT_SATURATION: f32 = 1.0;
pub const COLORCORRECTION_ENDPOINT_MINIMUM: f64 = -40.0;
pub const COLORCORRECTION_ENDPOINT_MAXIMUM: f64 = 40.0;
pub const COLORCORRECTION_SATURATION_MINIMUM: f64 = -3.0;
pub const COLORCORRECTION_SATURATION_MAXIMUM: f64 = 3.0;
/// Native endpoints are custom-grid coordinates and never receive a Bauhaus
/// numeric control. `ParameterDescriptor` nevertheless requires a precision;
/// zero is an operation-local deferred sentinel, not a native UI claim.
pub const COLORCORRECTION_ENDPOINT_NATIVE_UI_PRECISION: Option<u8> = None;
pub const COLORCORRECTION_ENDPOINT_DEFERRED_DESCRIPTOR_PRECISION: u8 = 0;
/// `dt_bauhaus_slider_from_params` derives two digits from saturation's native
/// introspection range `[-3, 3]`.
pub const COLORCORRECTION_SATURATION_NATIVE_UI_PRECISION: u8 = 2;
pub const COLORCORRECTION_DEFAULT_COLORSPACE: &str = "Lab";
pub const COLORCORRECTION_DEFAULT_GROUPS: [&str; 2] = ["color", "grading"];
pub const COLORCORRECTION_ALLOW_TILING: bool = true;
pub const COLORCORRECTION_SUPPORTS_BLENDING: bool = true;
pub const COLORCORRECTION_GUI_INSET_LOGICAL_PIXELS: u32 = 5;
pub const COLORCORRECTION_GUI_KEY_STEP: f32 = 0.5;
pub const COLORCORRECTION_GPU_PROGRAM: u32 = 2;
pub const COLORCORRECTION_GPU_KERNEL: &str = "colorcorrection";
pub const COLORCORRECTION_GPU_EXECUTABLE: bool = false;
pub const COLORCORRECTION_KERNEL_ARGUMENT_ORDER: [&str; 5] =
    ["saturation", "a_scale", "a_base", "b_scale", "b_base"];

/// Exact untranslated strings returned by native `name` and `description`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColorCorrectionLeafPresentation {
    pub name: &'static str,
    pub description: [&'static str; 5],
}

#[must_use]
pub const fn colorcorrection_leaf_presentation() -> ColorCorrectionLeafPresentation {
    ColorCorrectionLeafPresentation {
        name: COLORCORRECTION_NATIVE_NAME,
        description: COLORCORRECTION_NATIVE_DESCRIPTION,
    }
}

/// Source evidence for one native `init_presets` call. Registration remains
/// deferred because it belongs to shared preset, history, and app hubs.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorCorrectionPresetEvidence {
    pub name: &'static str,
    pub parameters: ColorCorrectionParametersV1,
    pub enabled: bool,
    pub blend_color_space: &'static str,
}

pub const COLORCORRECTION_PRESET_EVIDENCE: [ColorCorrectionPresetEvidence; 3] = [
    ColorCorrectionPresetEvidence {
        name: "warm tone",
        parameters: ColorCorrectionParametersV1::new(0.0, 3.0, 0.0, 0.0, 1.0),
        enabled: true,
        blend_color_space: COLORCORRECTION_PRESET_BLEND_COLORSPACE,
    },
    ColorCorrectionPresetEvidence {
        name: "warming filter",
        parameters: ColorCorrectionParametersV1::new(-0.95, 4.5, 3.55, 0.0, 1.0),
        enabled: true,
        blend_color_space: COLORCORRECTION_PRESET_BLEND_COLORSPACE,
    },
    ColorCorrectionPresetEvidence {
        name: "cooling filter",
        parameters: ColorCorrectionParametersV1::new(0.95, -4.5, -3.55, -0.0, 1.0),
        enabled: true,
        blend_color_space: COLORCORRECTION_PRESET_BLEND_COLORSPACE,
    },
];

#[must_use]
pub const fn colorcorrection_preset_evidence() -> &'static [ColorCorrectionPresetEvidence; 3] {
    &COLORCORRECTION_PRESET_EVIDENCE
}

/// Operation-local descriptor evidence. This function is deliberately not
/// exported through the shared descriptor or registry modules.
#[must_use]
pub fn colorcorrection_leaf_descriptor() -> OperationDescriptor {
    let tiling = source_map::COLORCORRECTION_RUST_TRANSACTIONAL_TILING;
    OperationDescriptor {
        id: DescriptorId::new(
            COLORCORRECTION_COMPATIBILITY_ID,
            COLORCORRECTION_RUST_ID,
            COLORCORRECTION_SCHEMA_VERSION,
            COLORCORRECTION_SCHEMA_VERSION,
            1,
        )
        .expect("static Color Correction descriptor identity"),
        parameters: vec![
            endpoint_parameter("hia", COLORCORRECTION_DEFAULT_HIA),
            endpoint_parameter("hib", COLORCORRECTION_DEFAULT_HIB),
            endpoint_parameter("loa", COLORCORRECTION_DEFAULT_LOA),
            endpoint_parameter("lob", COLORCORRECTION_DEFAULT_LOB),
            saturation_parameter(),
        ],
        flags: OperationFlags::MULTI_INSTANCE
            .insert(OperationFlags::STYLE_ELIGIBLE)
            .insert(OperationFlags::HISTORY_VISIBLE)
            .insert(OperationFlags::TILEABLE)
            .insert(OperationFlags::DETERMINISTIC_CPU)
            .insert(OperationFlags::COLOR)
            .insert(OperationFlags::MASKS)
            .insert(OperationFlags::BLENDING),
        stage: "display-referred-lab-d50".to_owned(),
        roi: RoiKind::Identity,
        tiling: TilingContract {
            overlap_pixels: tiling.overlap_pixels,
            alignment_pixels: tiling.alignment_pixels,
            minimum_tile_edge: tiling.minimum_tile_edge,
            preferred_tile_edge: tiling.preferred_tile_edge,
            temporary_multiplier_milli: tiling.temporary_multiplier_milli,
            input_multiplier_milli: tiling.input_multiplier_milli,
            output_multiplier_milli: tiling.output_multiplier_milli,
        },
        capability: CapabilityContract {
            cpu_supported: true,
            gpu_tier: None,
            required_features: Vec::new(),
            required_formats: vec!["lab-f32x4".to_owned()],
            deterministic_cpu: true,
            deterministic_gpu: false,
            fallback_to_cpu: false,
            precision: "f32 endpoint subtraction, f64 division, f32 narrowing, and separate f32 process roundings"
                .to_owned(),
            modes: vec!["operation-local".to_owned()],
        },
        io: lab_io(),
        mask_blend: MaskBlendContract {
            consumes_mask: false,
            publishes_mask: false,
            blend_if: false,
            geometry: false,
            analysis: false,
        },
        migration: MigrationContract {
            source_versions: vec![COLORCORRECTION_SCHEMA_VERSION],
            target_version: COLORCORRECTION_SCHEMA_VERSION,
            opaque_unknown_allowed: true,
        },
        ui: None,
    }
}

fn endpoint_parameter(id: &str, default: f32) -> ParameterDescriptor {
    ParameterDescriptor {
        id: id.to_owned(),
        kind: ParameterKind::Scalar {
            minimum: COLORCORRECTION_ENDPOINT_MINIMUM,
            maximum: COLORCORRECTION_ENDPOINT_MAXIMUM,
        },
        default: ParameterDefault::Scalar(f64::from(default)),
        required: false,
        introduced_version: COLORCORRECTION_SCHEMA_VERSION,
        removed_version: None,
        unit: Some("Lab opponent".to_owned()),
        step: None,
        precision: COLORCORRECTION_ENDPOINT_DEFERRED_DESCRIPTOR_PRECISION,
        role: ParameterRole::Processing,
        cache_affecting: true,
        animatable: true,
        ui_hint: None,
        condition: None,
    }
}

fn saturation_parameter() -> ParameterDescriptor {
    ParameterDescriptor {
        id: "saturation".to_owned(),
        kind: ParameterKind::Scalar {
            minimum: COLORCORRECTION_SATURATION_MINIMUM,
            maximum: COLORCORRECTION_SATURATION_MAXIMUM,
        },
        default: ParameterDefault::Scalar(f64::from(COLORCORRECTION_DEFAULT_SATURATION)),
        required: false,
        introduced_version: COLORCORRECTION_SCHEMA_VERSION,
        removed_version: None,
        unit: Some("factor".to_owned()),
        step: None,
        precision: COLORCORRECTION_SATURATION_NATIVE_UI_PRECISION,
        role: ParameterRole::Processing,
        cache_affecting: true,
        animatable: true,
        ui_hint: None,
        condition: None,
    }
}

fn lab_io() -> InputOutputContract {
    let image = ImagePredicate {
        channels: 4,
        alpha: AlphaPolicy::Preserve,
        encodings: vec![ColorEncoding::LabD50],
        nonfinite: NonFinitePolicy::Reject,
    };
    InputOutputContract {
        input: image.clone(),
        output: image,
        derives_output_encoding: false,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorCorrectionCapabilityError {
    GpuUnavailable,
    GtkUnavailable,
    PresetRegistrationDeferred,
    FormatCopyThroughDeferred,
    ProductionRoutingDeferred,
}

impl fmt::Display for ColorCorrectionCapabilityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::GpuUnavailable => {
                formatter.write_str("Color Correction GPU execution is unavailable")
            }
            Self::GtkUnavailable => {
                formatter.write_str("Color Correction GTK controls are unavailable")
            }
            Self::PresetRegistrationDeferred => {
                formatter.write_str("Color Correction preset registration is deferred")
            }
            Self::FormatCopyThroughDeferred => formatter
                .write_str("Color Correction wrong-format copy-through diagnostics are deferred"),
            Self::ProductionRoutingDeferred => {
                formatter.write_str("Color Correction production routing is deferred")
            }
        }
    }
}

impl std::error::Error for ColorCorrectionCapabilityError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColorCorrectionCapabilities {
    pub cpu_supported: bool,
    pub history_codec_supported: bool,
    pub local_descriptor_supported: bool,
    pub gpu_supported: bool,
    pub gtk_supported: bool,
    pub presets_registered: bool,
    pub format_copy_through_supported: bool,
    pub masks_consumed: bool,
    pub outer_blending_deferred: bool,
    pub production_routing_deferred: bool,
    pub alpha_or_spare_preserved: bool,
}

impl ColorCorrectionCapabilities {
    #[must_use]
    pub const fn bounded_cpu_leaf() -> Self {
        Self {
            cpu_supported: true,
            history_codec_supported: true,
            local_descriptor_supported: true,
            gpu_supported: COLORCORRECTION_GPU_EXECUTABLE,
            gtk_supported: false,
            presets_registered: false,
            format_copy_through_supported: false,
            masks_consumed: false,
            outer_blending_deferred: true,
            production_routing_deferred: true,
            alpha_or_spare_preserved: true,
        }
    }

    pub const fn require_gpu(self) -> Result<(), ColorCorrectionCapabilityError> {
        if self.gpu_supported {
            Ok(())
        } else {
            Err(ColorCorrectionCapabilityError::GpuUnavailable)
        }
    }

    pub const fn require_gtk(self) -> Result<(), ColorCorrectionCapabilityError> {
        if self.gtk_supported {
            Ok(())
        } else {
            Err(ColorCorrectionCapabilityError::GtkUnavailable)
        }
    }

    pub const fn require_preset_registration(self) -> Result<(), ColorCorrectionCapabilityError> {
        if self.presets_registered {
            Ok(())
        } else {
            Err(ColorCorrectionCapabilityError::PresetRegistrationDeferred)
        }
    }

    pub const fn require_format_copy_through(self) -> Result<(), ColorCorrectionCapabilityError> {
        if self.format_copy_through_supported {
            Ok(())
        } else {
            Err(ColorCorrectionCapabilityError::FormatCopyThroughDeferred)
        }
    }

    pub const fn require_production_routing(self) -> Result<(), ColorCorrectionCapabilityError> {
        if self.production_routing_deferred {
            Err(ColorCorrectionCapabilityError::ProductionRoutingDeferred)
        } else {
            Ok(())
        }
    }
}

#[must_use]
pub const fn capabilities() -> ColorCorrectionCapabilities {
    ColorCorrectionCapabilities::bounded_cpu_leaf()
}
