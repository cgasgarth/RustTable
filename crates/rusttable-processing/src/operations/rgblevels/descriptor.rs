use rusttable_color::ColorEncoding;

use crate::descriptor::{
    AlphaPolicy, CapabilityContract, DescriptorId, ImagePredicate, InputOutputContract,
    MaskBlendContract, MigrationContract, NonFinitePolicy, OperationDescriptor, OperationFlags,
    ParameterDefault, ParameterDescriptor, ParameterKind, ParameterRole, RoiKind, TilingContract,
};

use super::{
    RGBLEVELS_COMPATIBILITY_ID, RGBLEVELS_RUST_ID, RGBLEVELS_SCHEMA_VERSION, RgbLevelsParametersV1,
};

/// Source-derived processing contract for Darktable RGB Levels.
///
/// The operation-local fourth lane is an ignorable SIMD spare rather than
/// straight alpha. Pixelpipe tracks straight alpha separately and preserves it
/// across publication. Imported masks and outer blending remain unavailable
/// instead of being approximated by the bounded CPU route.
#[must_use]
pub fn rgblevels_descriptor() -> OperationDescriptor {
    let defaults = RgbLevelsParametersV1::defaults();
    OperationDescriptor {
        id: DescriptorId::new(
            RGBLEVELS_COMPATIBILITY_ID,
            RGBLEVELS_RUST_ID,
            RGBLEVELS_SCHEMA_VERSION,
            RGBLEVELS_SCHEMA_VERSION,
            1,
        )
        .expect("static RGB Levels ID"),
        parameters: vec![
            enum_parameter(
                "autoscale",
                &["linked_channels", "independent_channels"],
                "linked_channels",
            ),
            enum_parameter(
                "preserve_colors",
                &[
                    "none",
                    "luminance",
                    "max",
                    "average",
                    "sum",
                    "norm",
                    "power",
                ],
                "luminance",
            ),
            ParameterDescriptor {
                id: "levels".to_owned(),
                kind: ParameterKind::Matrix {
                    rows: 3,
                    columns: 3,
                    minimum: 0.0,
                    maximum: 1.0,
                },
                default: ParameterDefault::Matrix(
                    defaults
                        .levels
                        .into_iter()
                        .flatten()
                        .map(f64::from)
                        .collect(),
                ),
                required: false,
                introduced_version: RGBLEVELS_SCHEMA_VERSION,
                removed_version: None,
                unit: None,
                step: Some(0.001),
                precision: 6,
                role: ParameterRole::Processing,
                cache_affecting: true,
                animatable: true,
                ui_hint: Some("levels-matrix".to_owned()),
                condition: None,
            },
        ],
        flags: OperationFlags::MULTI_INSTANCE
            .insert(OperationFlags::HISTORY_VISIBLE)
            .insert(OperationFlags::TILEABLE)
            .insert(OperationFlags::DETERMINISTIC_CPU)
            .insert(OperationFlags::COLOR),
        stage: "display-referred-linear-rgb".to_owned(),
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
            required_features: vec!["deterministic-row-major".to_owned()],
            required_formats: vec!["rgba32float".to_owned()],
            deterministic_cpu: true,
            deterministic_gpu: false,
            fallback_to_cpu: true,
            precision: "f32 RGB 65536-entry per-channel LUT".to_owned(),
            modes: vec!["preview".to_owned(), "full".to_owned(), "export".to_owned()],
        },
        io: rgb_io(),
        mask_blend: MaskBlendContract {
            consumes_mask: false,
            publishes_mask: false,
            blend_if: false,
            geometry: false,
            analysis: false,
        },
        migration: MigrationContract {
            source_versions: vec![RGBLEVELS_SCHEMA_VERSION],
            target_version: RGBLEVELS_SCHEMA_VERSION,
            opaque_unknown_allowed: true,
        },
        ui: None,
    }
}

fn enum_parameter(id: &str, tags: &[&str], default: &str) -> ParameterDescriptor {
    ParameterDescriptor {
        id: id.to_owned(),
        kind: ParameterKind::Enum {
            tags: tags.iter().map(|tag| (*tag).to_owned()).collect(),
        },
        default: ParameterDefault::Enum(default.to_owned()),
        required: false,
        introduced_version: RGBLEVELS_SCHEMA_VERSION,
        removed_version: None,
        unit: None,
        step: None,
        precision: 0,
        role: ParameterRole::Processing,
        cache_affecting: true,
        animatable: false,
        ui_hint: Some("combo".to_owned()),
        condition: None,
    }
}

fn rgb_io() -> InputOutputContract {
    let encodings = vec![
        ColorEncoding::LinearSrgbD65,
        ColorEncoding::LinearDisplayP3D65,
        ColorEncoding::LinearRec2020D65,
    ];
    InputOutputContract {
        input: ImagePredicate {
            channels: 4,
            alpha: AlphaPolicy::Preserve,
            encodings: encodings.clone(),
            nonfinite: NonFinitePolicy::Reject,
        },
        output: ImagePredicate {
            channels: 4,
            alpha: AlphaPolicy::Preserve,
            encodings,
            nonfinite: NonFinitePolicy::Reject,
        },
        derives_output_encoding: false,
    }
}
