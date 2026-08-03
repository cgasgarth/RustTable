//! Operation-local metadata ported from `src/iop/colorharmonizer.c`.
//!
//! This descriptor is deliberately not registered.  Its CPU capability records
//! the bounded leaf contract for a future integration owner; no UI, generic
//! controls, GPU binding, profile resolver, history router, or evaluator seam
//! is exposed by this lane.

use rusttable_color::ColorEncoding;

use crate::descriptor::{
    AlphaPolicy, CapabilityContract, DescriptorId, ImagePredicate, InputOutputContract,
    MaskBlendContract, MigrationContract, NonFinitePolicy, OperationDescriptor, OperationFlags,
    ParameterDefault, ParameterDescriptor, ParameterKind, ParameterRole, RoiKind, TilingContract,
};

use super::codec::{
    COLORHARMONIZER_DEFAULT_ANCHOR_HUE, COLORHARMONIZER_DEFAULT_NEUTRAL_PROTECTION,
    COLORHARMONIZER_DEFAULT_NUM_CUSTOM_NODES, COLORHARMONIZER_DEFAULT_PULL_STRENGTH,
    COLORHARMONIZER_DEFAULT_PULL_WIDTH, COLORHARMONIZER_DEFAULT_SMOOTHING,
    COLORHARMONIZER_SCHEMA_VERSION,
};

pub const COLORHARMONIZER_COMPATIBILITY_ID: &str = "colorharmonizer";
pub const COLORHARMONIZER_RUST_ID: &str = "rusttable.colorharmonizer";
/// The leaf is intentionally absent from the production registry.
pub const COLORHARMONIZER_REGISTERED: bool = false;
/// The native GTK and vectorscope surfaces are explicitly unavailable here.
pub const COLORHARMONIZER_UI_AVAILABLE: bool = false;
pub const COLORHARMONIZER_GPU_AVAILABLE: bool = false;

#[must_use]
#[expect(
    clippy::too_many_lines,
    reason = "Color Harmonizer descriptor fields mirror the native parameter contract in one declaration."
)]
pub fn colorharmonizer_descriptor() -> OperationDescriptor {
    OperationDescriptor {
        id: DescriptorId::new(
            COLORHARMONIZER_COMPATIBILITY_ID,
            COLORHARMONIZER_RUST_ID,
            COLORHARMONIZER_SCHEMA_VERSION,
            COLORHARMONIZER_SCHEMA_VERSION,
            1,
        )
        .expect("static Color Harmonizer descriptor identity"),
        parameters: vec![
            ParameterDescriptor {
                id: "rule".to_owned(),
                kind: ParameterKind::Enum {
                    tags: vec![
                        "monochromatic".to_owned(),
                        "analogous".to_owned(),
                        "analogous_complementary".to_owned(),
                        "complementary".to_owned(),
                        "split_complementary".to_owned(),
                        "dyad".to_owned(),
                        "triad".to_owned(),
                        "tetrad".to_owned(),
                        "square".to_owned(),
                        "custom".to_owned(),
                    ],
                },
                default: ParameterDefault::Enum("complementary".to_owned()),
                required: false,
                introduced_version: COLORHARMONIZER_SCHEMA_VERSION,
                removed_version: None,
                unit: None,
                step: None,
                precision: 0,
                role: ParameterRole::Processing,
                cache_affecting: true,
                animatable: false,
                ui_hint: None,
                condition: None,
            },
            scalar(
                "anchor_hue",
                0.0,
                1.0,
                COLORHARMONIZER_DEFAULT_ANCHOR_HUE,
                1,
            ),
            scalar(
                "pull_strength",
                0.0,
                1.0,
                COLORHARMONIZER_DEFAULT_PULL_STRENGTH,
                2,
            ),
            scalar(
                "neutral_protection",
                0.0,
                1.0,
                COLORHARMONIZER_DEFAULT_NEUTRAL_PROTECTION,
                2,
            ),
            scalar(
                "pull_width",
                0.25,
                4.0,
                COLORHARMONIZER_DEFAULT_PULL_WIDTH,
                2,
            ),
            vector("custom_hue", 4, 0.0, 1.0, vec![0.0, 0.25, 0.5, 0.75], 1),
            integer(
                "num_custom_nodes",
                2,
                4,
                i64::from(COLORHARMONIZER_DEFAULT_NUM_CUSTOM_NODES),
                0,
            ),
            vector("node_saturation", 4, 0.0, 2.0, vec![1.0; 4], 0),
            scalar("smoothing", 0.0, 2.0, COLORHARMONIZER_DEFAULT_SMOOTHING, 2),
        ],
        flags: OperationFlags::FULL_IMAGE
            .insert(OperationFlags::DETERMINISTIC_CPU)
            .insert(OperationFlags::COLOR),
        stage: "working-profile-rgb-ucs-jch".to_owned(),
        roi: RoiKind::FullImage,
        tiling: TilingContract {
            overlap_pixels: 0,
            alignment_pixels: 1,
            minimum_tile_edge: 1,
            preferred_tile_edge: 256,
            temporary_multiplier_milli: 2000,
            input_multiplier_milli: 1000,
            output_multiplier_milli: 1000,
        },
        capability: CapabilityContract {
            cpu_supported: true,
            gpu_tier: None,
            required_features: Vec::new(),
            required_formats: vec!["rgba32float".to_owned()],
            deterministic_cpu: true,
            deterministic_gpu: false,
            fallback_to_cpu: false,
            precision: "f32 Darktable UCS/JCH with explicit CAT16 and profile matrices".to_owned(),
            modes: Vec::new(),
        },
        io: InputOutputContract {
            input: ImagePredicate {
                channels: 4,
                alpha: AlphaPolicy::Preserve,
                encodings: vec![ColorEncoding::Unspecified],
                nonfinite: NonFinitePolicy::Reject,
            },
            output: ImagePredicate {
                channels: 4,
                alpha: AlphaPolicy::Preserve,
                encodings: vec![ColorEncoding::Unspecified],
                nonfinite: NonFinitePolicy::Reject,
            },
            derives_output_encoding: false,
        },
        mask_blend: MaskBlendContract {
            consumes_mask: false,
            publishes_mask: false,
            blend_if: false,
            geometry: false,
            analysis: false,
        },
        migration: MigrationContract {
            source_versions: vec![COLORHARMONIZER_SCHEMA_VERSION],
            target_version: COLORHARMONIZER_SCHEMA_VERSION,
            opaque_unknown_allowed: true,
        },
        ui: None,
    }
}

fn scalar(
    id: &str,
    minimum: f64,
    maximum: f64,
    default: f32,
    precision: u8,
) -> ParameterDescriptor {
    ParameterDescriptor {
        id: id.to_owned(),
        kind: ParameterKind::Scalar { minimum, maximum },
        default: ParameterDefault::Scalar(f64::from(default)),
        required: false,
        introduced_version: COLORHARMONIZER_SCHEMA_VERSION,
        removed_version: None,
        unit: Some("normalized".to_owned()),
        step: Some(0.01),
        precision,
        role: ParameterRole::Processing,
        cache_affecting: true,
        animatable: false,
        ui_hint: None,
        condition: None,
    }
}

fn vector(
    id: &str,
    dimensions: u8,
    minimum: f64,
    maximum: f64,
    default: Vec<f64>,
    precision: u8,
) -> ParameterDescriptor {
    ParameterDescriptor {
        id: id.to_owned(),
        kind: ParameterKind::Vector {
            dimensions,
            minimum,
            maximum,
        },
        default: ParameterDefault::Vector(default),
        required: false,
        introduced_version: COLORHARMONIZER_SCHEMA_VERSION,
        removed_version: None,
        unit: Some("normalized".to_owned()),
        step: Some(0.01),
        precision,
        role: ParameterRole::Processing,
        cache_affecting: true,
        animatable: false,
        ui_hint: None,
        condition: None,
    }
}

fn integer(
    id: &str,
    minimum: i64,
    maximum: i64,
    default: i64,
    precision: u8,
) -> ParameterDescriptor {
    ParameterDescriptor {
        id: id.to_owned(),
        kind: ParameterKind::Integer { minimum, maximum },
        default: ParameterDefault::Integer(default),
        required: false,
        introduced_version: COLORHARMONIZER_SCHEMA_VERSION,
        removed_version: None,
        unit: Some("count".to_owned()),
        step: Some(1.0),
        precision,
        role: ParameterRole::Processing,
        cache_affecting: true,
        animatable: false,
        ui_hint: None,
        condition: None,
    }
}
