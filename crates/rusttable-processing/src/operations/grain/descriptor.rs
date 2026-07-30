#![allow(
    clippy::missing_panics_doc,
    clippy::too_many_lines,
    reason = "descriptor construction is an explicit compatibility contract"
)]

use rusttable_color::ColorEncoding;

use crate::descriptor::{
    AlphaPolicy, CapabilityContract, DescriptorId, ImagePredicate, InputOutputContract,
    MaskBlendContract, MigrationContract, NonFinitePolicy, OperationDescriptor, OperationFlags,
    ParameterDefault, ParameterDescriptor, ParameterKind, ParameterRole, RoiKind, TilingContract,
    UiHint,
};

use super::parameters::GRAIN_V2_PARAMETER_BYTES;

pub const GRAIN_COMPATIBILITY_ID: &str = "grain";
pub const GRAIN_SCHEMA_VERSION: u16 = 2;

#[must_use]
pub fn grain_descriptor() -> OperationDescriptor {
    let scalar =
        |id: &str, minimum: f64, maximum: f64, default: f64, unit: &str| ParameterDescriptor {
            id: id.to_owned(),
            kind: ParameterKind::Scalar { minimum, maximum },
            default: ParameterDefault::Scalar(default),
            required: false,
            introduced_version: 1,
            removed_version: None,
            unit: Some(unit.to_owned()),
            step: Some(0.01),
            precision: 2,
            role: ParameterRole::Processing,
            cache_affecting: true,
            animatable: true,
            ui_hint: None,
            condition: None,
        };
    OperationDescriptor {
        id: DescriptorId::new(
            GRAIN_COMPATIBILITY_ID,
            "rusttable.grain",
            GRAIN_SCHEMA_VERSION,
            GRAIN_SCHEMA_VERSION,
            1,
        )
        .expect("static grain ID"),
        parameters: vec![
            ParameterDescriptor {
                id: "channel".to_owned(),
                kind: ParameterKind::Enum {
                    tags: ["hue", "saturation", "lightness", "rgb"]
                        .into_iter()
                        .map(str::to_owned)
                        .collect(),
                },
                default: ParameterDefault::Enum("lightness".to_owned()),
                required: false,
                introduced_version: 1,
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
            scalar("scale", 20.0 / 213.2, 6400.0 / 213.2, 1600.0 / 213.2, "iso"),
            scalar("strength", 0.0, 100.0, 25.0, "percent"),
            scalar("midtones_bias", 0.0, 100.0, 100.0, "percent"),
        ],
        flags: OperationFlags::STYLE_ELIGIBLE
            .insert(OperationFlags::HISTORY_VISIBLE)
            .insert(OperationFlags::TILEABLE)
            .insert(OperationFlags::DETERMINISTIC_CPU)
            .insert(OperationFlags::DETERMINISTIC_GPU)
            .insert(OperationFlags::COLOR)
            .insert(OperationFlags::BLENDING),
        stage: "scene-linear-rgb".to_owned(),
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
            gpu_tier: Some(1),
            required_features: Vec::new(),
            required_formats: vec!["rgba32float".to_owned()],
            deterministic_cpu: true,
            deterministic_gpu: true,
            fallback_to_cpu: true,
            precision: format!("f32 LUT/noise, {GRAIN_V2_PARAMETER_BYTES}-byte history"),
            modes: ["hue", "saturation", "lightness", "rgb"]
                .into_iter()
                .map(str::to_owned)
                .collect(),
        },
        io: rgb_io(),
        mask_blend: MaskBlendContract {
            consumes_mask: false,
            publishes_mask: false,
            blend_if: true,
            geometry: false,
            analysis: false,
        },
        migration: MigrationContract {
            source_versions: vec![1, 2],
            target_version: 2,
            opaque_unknown_allowed: true,
        },
        ui: Some(UiHint {
            label_key: "operation.grain".to_owned(),
            group_key: "group.effects".to_owned(),
            control: "grain".to_owned(),
        }),
    }
}

#[must_use]
pub const fn presets() -> &'static [()] {
    &[]
}

fn rgb_io() -> InputOutputContract {
    let image = ImagePredicate {
        channels: 3,
        alpha: AlphaPolicy::Preserve,
        encodings: vec![ColorEncoding::LinearSrgbD65],
        nonfinite: NonFinitePolicy::Reject,
    };
    InputOutputContract {
        input: image.clone(),
        output: image,
        derives_output_encoding: false,
    }
}
