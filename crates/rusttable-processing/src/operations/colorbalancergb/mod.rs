//! Bounded standalone Rust CPU leaf for retained Darktable
//! `src/iop/colorbalancergb.c`.
//!
//! The leaf owns the native v1–v5 ABI and migrations, commit-time
//! coefficients, internal shadows/midtones/highlights opacity masks, D50
//! profile matrices adapted to D65, JzAzBz and darktable UCS 2022 equations,
//! gamut LUTs, identity-ROI tiling, cancellation, and transactional CPU
//! publication.  It deliberately does not register itself in shared hubs.
//!
//! External pixelpipe mask/blending, history materialization, GPU/OpenCL,
//! GUI/presets, profile acquisition, and production routing remain fail-closed
//! until their owning seams are ported.  The native CPU process writes the
//! fourth lane through RGB matrix arithmetic; this leaf records that exact
//! native CPU behavior as zero rather than silently promising alpha
//! preservation.  The retained OpenCL kernel's explicit alpha copy is a
//! separate, deferred capability.

#![forbid(unsafe_code)]
#![allow(
    unused_imports,
    clippy::doc_markdown,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    dead_code,
    reason = "standalone bounded leaf is intentionally not exported through shared hubs"
)]

pub mod codec;
pub mod execution;
pub mod math;
pub mod source_map;

pub use codec::{
    COLORBALANCERGB_FIELD_COUNT_V1, COLORBALANCERGB_FIELD_COUNT_V2, COLORBALANCERGB_FIELD_COUNT_V3,
    COLORBALANCERGB_FIELD_COUNT_V4, COLORBALANCERGB_INTROSPECTION_VERSION,
    COLORBALANCERGB_V1_PARAMETER_BYTES, COLORBALANCERGB_V2_PARAMETER_BYTES,
    COLORBALANCERGB_V3_PARAMETER_BYTES, COLORBALANCERGB_V4_PARAMETER_BYTES,
    COLORBALANCERGB_V5_PARAMETER_BYTES, ColorBalanceRgbCodecError, ColorBalanceRgbHistory,
    ColorBalanceRgbParametersV1, ColorBalanceRgbParametersV2, ColorBalanceRgbParametersV3,
    ColorBalanceRgbParametersV4, ColorBalanceRgbParametersV5, ColorBalanceRgbSaturationFormula,
    ColorBalanceRgbV5Abi, migrate_v1_to_v5, migrate_v2_to_v5, migrate_v3_to_v5, migrate_v4_to_v5,
};
pub use execution::{
    ANGLE_SHIFT_DEGREES, COLORBALANCERGB_COMPATIBILITY_ID, COLORBALANCERGB_RUST_ID,
    ColorBalanceRgbAlphaBehavior, ColorBalanceRgbCapabilities, ColorBalanceRgbCapabilityError,
    ColorBalanceRgbCoefficients, ColorBalanceRgbConfig, ColorBalanceRgbExecutionError,
    ColorBalanceRgbMaskWeights, ColorBalanceRgbParameterError, ColorBalanceRgbPlan,
    ColorBalanceRgbProfile, ColorBalanceRgbProfileError, ColorBalanceRgbTiling, MASK_LUMA_EXPONENT,
    capabilities, opacity_masks, tiling,
};

use rusttable_color::ColorEncoding;

use crate::descriptor::{
    AlphaPolicy, CapabilityContract, DescriptorId, ImagePredicate, InputOutputContract,
    MaskBlendContract, MigrationContract, NonFinitePolicy, OperationDescriptor, OperationFlags,
    ParameterDefault, ParameterDescriptor, ParameterKind, ParameterRole, RoiKind, TilingContract,
};

/// Operation-local descriptor.  It is not exported through registry or
/// evaluator hubs until the cross-crate operation seam is owned.
#[must_use]
#[expect(
    clippy::too_many_lines,
    reason = "Keep native parameter declaration order together in the descriptor"
)]
pub fn colorbalancergb_descriptor() -> OperationDescriptor {
    OperationDescriptor {
        id: DescriptorId::new(
            COLORBALANCERGB_COMPATIBILITY_ID,
            COLORBALANCERGB_RUST_ID,
            COLORBALANCERGB_INTROSPECTION_VERSION,
            COLORBALANCERGB_INTROSPECTION_VERSION,
            1,
        )
        .expect("static Color Balance RGB descriptor identity"),
        parameters: vec![
            scalar("shadows_Y", -1.0, 1.0, 0.0, 1, "luminance"),
            scalar("shadows_C", 0.0, 1.0, 0.0, 1, "chroma"),
            scalar("shadows_H", 0.0, 360.0, 0.0, 1, "hue"),
            scalar("midtones_Y", -1.0, 1.0, 0.0, 1, "luminance"),
            scalar("midtones_C", 0.0, 1.0, 0.0, 1, "chroma"),
            scalar("midtones_H", 0.0, 360.0, 0.0, 1, "hue"),
            scalar("highlights_Y", -1.0, 1.0, 0.0, 1, "luminance"),
            scalar("highlights_C", 0.0, 1.0, 0.0, 1, "chroma"),
            scalar("highlights_H", 0.0, 360.0, 0.0, 1, "hue"),
            scalar("global_Y", -1.0, 1.0, 0.0, 1, "luminance"),
            scalar("global_C", 0.0, 1.0, 0.0, 1, "chroma"),
            scalar("global_H", 0.0, 360.0, 0.0, 1, "hue"),
            scalar("shadows_weight", 0.0, 3.0, 1.0, 1, "falloff"),
            scalar("white_fulcrum", -16.0, 16.0, 0.0, 1, "stops"),
            scalar("highlights_weight", 0.0, 3.0, 1.0, 1, "falloff"),
            scalar("chroma_shadows", -1.0, 1.0, 0.0, 1, "factor"),
            scalar("chroma_highlights", -1.0, 1.0, 0.0, 1, "factor"),
            scalar("chroma_global", -1.0, 1.0, 0.0, 1, "factor"),
            scalar("chroma_midtones", -1.0, 1.0, 0.0, 1, "factor"),
            scalar("saturation_global", -1.0, 1.0, 0.0, 1, "factor"),
            scalar("saturation_highlights", -1.0, 1.0, 0.0, 1, "factor"),
            scalar("saturation_midtones", -1.0, 1.0, 0.0, 1, "factor"),
            scalar("saturation_shadows", -1.0, 1.0, 0.0, 1, "factor"),
            scalar("hue_angle", -180.0, 180.0, 0.0, 1, "degrees"),
            scalar("brilliance_global", -1.0, 1.0, 0.0, 2, "factor"),
            scalar("brilliance_highlights", -1.0, 1.0, 0.0, 2, "factor"),
            scalar("brilliance_midtones", -1.0, 1.0, 0.0, 2, "factor"),
            scalar("brilliance_shadows", -1.0, 1.0, 0.0, 2, "factor"),
            scalar("mask_grey_fulcrum", 0.0, 1.0, 0.1845, 3, "normalized"),
            scalar("vibrance", -1.0, 1.0, 0.0, 4, "factor"),
            scalar("grey_fulcrum", 0.0, 1.0, 0.1845, 4, "normalized"),
            scalar("contrast", -1.0, 1.0, 0.0, 4, "factor"),
            ParameterDescriptor {
                id: "saturation_formula".to_owned(),
                kind: ParameterKind::Enum {
                    tags: vec!["jzazbz-2021".to_owned(), "darktable-ucs-2022".to_owned()],
                },
                default: ParameterDefault::Enum("darktable-ucs-2022".to_owned()),
                required: true,
                introduced_version: 5,
                removed_version: None,
                unit: None,
                step: None,
                precision: 0,
                role: ParameterRole::Color,
                cache_affecting: true,
                animatable: false,
                ui_hint: None,
                condition: None,
            },
        ],
        flags: OperationFlags::STYLE_ELIGIBLE
            .insert(OperationFlags::HISTORY_VISIBLE)
            .insert(OperationFlags::TILEABLE)
            .insert(OperationFlags::DETERMINISTIC_CPU)
            .insert(OperationFlags::COLOR)
            .insert(OperationFlags::MASKS)
            .insert(OperationFlags::BLENDING),
        stage: "scene-referred-rgb-profile-d50".to_owned(),
        roi: RoiKind::Identity,
        tiling: TilingContract {
            overlap_pixels: 0,
            alignment_pixels: 1,
            minimum_tile_edge: 1,
            preferred_tile_edge: 256,
            temporary_multiplier_milli: 1000,
            input_multiplier_milli: 1000,
            output_multiplier_milli: 1000,
        },
        capability: CapabilityContract {
            cpu_supported: true,
            gpu_tier: None,
            required_features: vec![
                "profile-rgb-matrix-d50".to_owned(),
                "internal-luma-masks".to_owned(),
            ],
            required_formats: vec!["rgb-f32x4-profile".to_owned()],
            deterministic_cpu: true,
            deterministic_gpu: false,
            fallback_to_cpu: false,
            precision: "scalar f32 CPU with native JzAzBz/UCS equations; GPU unavailable"
                .to_owned(),
            modes: vec!["preview".to_owned(), "full".to_owned(), "export".to_owned()],
        },
        io: rgb_io(),
        mask_blend: MaskBlendContract {
            consumes_mask: true,
            publishes_mask: false,
            blend_if: false,
            geometry: false,
            analysis: false,
        },
        migration: MigrationContract {
            source_versions: vec![1, 2, 3, 4, COLORBALANCERGB_INTROSPECTION_VERSION],
            target_version: COLORBALANCERGB_INTROSPECTION_VERSION,
            opaque_unknown_allowed: true,
        },
        ui: None,
    }
}

fn scalar(
    id: &str,
    minimum: f64,
    maximum: f64,
    default: f64,
    introduced_version: u16,
    unit: &str,
) -> ParameterDescriptor {
    ParameterDescriptor {
        id: id.to_ascii_lowercase(),
        kind: ParameterKind::Scalar { minimum, maximum },
        default: ParameterDefault::Scalar(default),
        required: false,
        introduced_version,
        removed_version: None,
        unit: Some(unit.to_owned()),
        step: Some(0.01),
        precision: 5,
        role: ParameterRole::Color,
        cache_affecting: true,
        animatable: true,
        ui_hint: None,
        condition: None,
    }
}

fn rgb_io() -> InputOutputContract {
    let image = ImagePredicate {
        channels: 4,
        alpha: AlphaPolicy::Replace,
        encodings: vec![ColorEncoding::Unspecified],
        nonfinite: NonFinitePolicy::Reject,
    };
    InputOutputContract {
        input: image.clone(),
        output: image,
        derives_output_encoding: false,
    }
}
