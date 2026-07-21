#![allow(clippy::missing_panics_doc, clippy::too_many_lines)]

use crate::descriptor::{
    AlphaPolicy, CapabilityContract, DescriptorId, ImagePredicate, InputOutputContract,
    MaskBlendContract, MigrationContract, NonFinitePolicy, OperationDescriptor, OperationFlags,
    ParameterDefault, ParameterDescriptor, ParameterKind, ParameterRole, TilingContract, UiHint,
};
use rusttable_color::ColorEncoding;

#[must_use]
pub fn watermark_descriptor() -> OperationDescriptor {
    let image = ImagePredicate {
        channels: 3,
        alpha: AlphaPolicy::Ignore,
        encodings: vec![ColorEncoding::LinearSrgbD65],
        nonfinite: NonFinitePolicy::Reject,
    };
    OperationDescriptor {
        id: DescriptorId::new(
            super::WATERMARK_COMPATIBILITY_ID,
            super::WATERMARK_RUST_ID,
            super::WATERMARK_SCHEMA_VERSION,
            super::WATERMARK_PARAMETER_VERSION,
            super::WATERMARK_IMPLEMENTATION_VERSION,
        )
        .expect("static watermark descriptor ID"),
        parameters: vec![
            ParameterDescriptor {
                id: "template_hash".to_owned(),
                kind: ParameterKind::ContentRef,
                default: ParameterDefault::ContentRef(String::new()),
                required: true,
                introduced_version: 7,
                removed_version: None,
                unit: None,
                step: None,
                precision: 0,
                role: ParameterRole::Presentation,
                cache_affecting: true,
                animatable: false,
                ui_hint: Some("managed-asset".to_owned()),
                condition: None,
            },
            scalar("opacity", 0.0, 1.0, 1.0, ParameterRole::Processing),
            scalar("scale", 0.0, 8.0, 0.25, ParameterRole::Geometry),
            ParameterDescriptor {
                id: "scale_mode".to_owned(),
                kind: ParameterKind::Enum {
                    tags: ["width", "height", "fit"]
                        .into_iter()
                        .map(str::to_owned)
                        .collect(),
                },
                default: ParameterDefault::Enum("width".to_owned()),
                required: true,
                introduced_version: 7,
                removed_version: None,
                step: Some(1.0),
                precision: 0,
                unit: None,
                role: ParameterRole::Geometry,
                cache_affecting: true,
                animatable: false,
                ui_hint: Some("combo".to_owned()),
                condition: None,
            },
            ParameterDescriptor {
                id: "anchor".to_owned(),
                kind: ParameterKind::Enum {
                    tags: [
                        "top-left",
                        "top",
                        "top-right",
                        "left",
                        "center",
                        "right",
                        "bottom-left",
                        "bottom",
                        "bottom-right",
                    ]
                    .into_iter()
                    .map(str::to_owned)
                    .collect(),
                },
                default: ParameterDefault::Enum("bottom-right".to_owned()),
                required: true,
                introduced_version: 7,
                removed_version: None,
                unit: None,
                step: Some(1.0),
                precision: 0,
                role: ParameterRole::Geometry,
                cache_affecting: true,
                animatable: false,
                ui_hint: Some("combo".to_owned()),
                condition: None,
            },
            scalar(
                "x_offset",
                -1_000_000.0,
                1_000_000.0,
                0.0,
                ParameterRole::Geometry,
            ),
            scalar(
                "y_offset",
                -1_000_000.0,
                1_000_000.0,
                0.0,
                ParameterRole::Geometry,
            ),
            scalar("rotation", -3600.0, 3600.0, 0.0, ParameterRole::Geometry),
            ParameterDescriptor {
                id: "color".to_owned(),
                kind: ParameterKind::Color {
                    allow_external_profile: false,
                },
                default: ParameterDefault::Color(ColorEncoding::Srgb),
                required: true,
                introduced_version: 7,
                removed_version: None,
                unit: None,
                step: None,
                precision: 4,
                role: ParameterRole::Color,
                cache_affecting: true,
                animatable: false,
                ui_hint: Some("color".to_owned()),
                condition: None,
            },
            ParameterDescriptor {
                id: "expand_variables".to_owned(),
                kind: ParameterKind::Bool,
                default: ParameterDefault::Bool(true),
                required: true,
                introduced_version: 7,
                removed_version: None,
                unit: None,
                step: None,
                precision: 0,
                role: ParameterRole::Presentation,
                cache_affecting: true,
                animatable: false,
                ui_hint: Some("check".to_owned()),
                condition: None,
            },
        ],
        flags: OperationFlags::HISTORY_VISIBLE
            .insert(OperationFlags::FULL_IMAGE)
            .insert(OperationFlags::DETERMINISTIC_CPU)
            .insert(OperationFlags::SCALE)
            .insert(OperationFlags::COLOR)
            .insert(OperationFlags::BLENDING),
        stage: "scene-linear".to_owned(),
        roi: crate::descriptor::RoiKind::FullImage,
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
            required_features: Vec::new(),
            required_formats: Vec::new(),
            deterministic_cpu: true,
            deterministic_gpu: false,
            fallback_to_cpu: true,
            precision: "premultiplied sRGB asset, linear-light scalar composite".to_owned(),
            modes: vec!["preview".to_owned(), "full".to_owned(), "export".to_owned()],
        },
        io: InputOutputContract {
            input: image.clone(),
            output: image,
            derives_output_encoding: false,
        },
        mask_blend: MaskBlendContract {
            consumes_mask: false,
            publishes_mask: false,
            blend_if: false,
            geometry: true,
            analysis: false,
        },
        migration: MigrationContract {
            source_versions: (1..=7).collect(),
            target_version: 7,
            opaque_unknown_allowed: true,
        },
        ui: Some(UiHint {
            label_key: "operation.watermark".to_owned(),
            group_key: "group.output".to_owned(),
            control: "managed-watermark".to_owned(),
        }),
    }
}

fn scalar(
    id: &str,
    minimum: f64,
    maximum: f64,
    default: f64,
    role: ParameterRole,
) -> ParameterDescriptor {
    ParameterDescriptor {
        id: id.to_owned(),
        kind: ParameterKind::Scalar { minimum, maximum },
        default: ParameterDefault::Scalar(default),
        required: true,
        introduced_version: 7,
        removed_version: None,
        unit: None,
        step: Some(0.001),
        precision: 3,
        role,
        cache_affecting: true,
        animatable: false,
        ui_hint: Some("slider".to_owned()),
        condition: None,
    }
}
