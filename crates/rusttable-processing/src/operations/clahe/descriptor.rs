#![allow(clippy::missing_panics_doc)]

use rusttable_color::ColorEncoding;

use crate::descriptor::{
    AlphaPolicy, CapabilityContract, DescriptorId, ImagePredicate, InputOutputContract,
    MaskBlendContract, MigrationContract, NonFinitePolicy, OperationDescriptor, OperationFlags,
    ParameterDefault, ParameterDescriptor, ParameterKind, ParameterRole, RoiKind, TilingContract,
    UiHint,
};

#[must_use]
pub fn clahe_descriptor() -> OperationDescriptor {
    let scalar =
        |id: &str, minimum: f64, maximum: f64, default: f64, unit: &str| ParameterDescriptor {
            id: id.to_owned(),
            kind: ParameterKind::Scalar { minimum, maximum },
            default: ParameterDefault::Scalar(default),
            required: true,
            introduced_version: 1,
            removed_version: None,
            unit: Some(unit.to_owned()),
            step: Some(0.01),
            precision: 2,
            role: ParameterRole::Processing,
            cache_affecting: true,
            animatable: true,
            ui_hint: Some("slider".to_owned()),
            condition: None,
        };
    let image = ImagePredicate {
        channels: 4,
        alpha: AlphaPolicy::Preserve,
        encodings: vec![ColorEncoding::LinearSrgbD65],
        nonfinite: NonFinitePolicy::Reject,
    };
    OperationDescriptor {
        id: DescriptorId::new("clahe", "rusttable.clahe", 1, 1, 1).expect("static ID"),
        parameters: vec![
            scalar("radius", 0.0, 256.0, 64.0, "pixels"),
            scalar("slope", 1.0, 3.0, 1.25, "amount"),
        ],
        flags: OperationFlags::DEPRECATED
            .insert(OperationFlags::HIDDEN)
            .insert(OperationFlags::STYLE_ELIGIBLE)
            .insert(OperationFlags::HISTORY_VISIBLE)
            .insert(OperationFlags::FULL_IMAGE)
            .insert(OperationFlags::ANALYSIS)
            .insert(OperationFlags::DETERMINISTIC_CPU)
            .insert(OperationFlags::COLOR)
            .insert(OperationFlags::MASKS)
            .insert(OperationFlags::BLENDING),
        stage: "scene-linear-rgb".to_owned(),
        roi: RoiKind::FullImage,
        tiling: TilingContract {
            overlap_pixels: 0,
            alignment_pixels: 1,
            minimum_tile_edge: 1,
            preferred_tile_edge: 256,
            temporary_multiplier_milli: 2500,
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
            fallback_to_cpu: false,
            precision: "f32 scalar fixed-order HSL 257-bin CLAHE".to_owned(),
            modes: vec!["preview".to_owned(), "full".to_owned(), "export".to_owned()],
        },
        io: InputOutputContract {
            input: image.clone(),
            output: image,
            derives_output_encoding: false,
        },
        mask_blend: MaskBlendContract {
            consumes_mask: true,
            publishes_mask: false,
            blend_if: true,
            geometry: false,
            analysis: false,
        },
        migration: MigrationContract {
            source_versions: vec![1],
            target_version: 1,
            opaque_unknown_allowed: true,
        },
        ui: Some(UiHint {
            label_key: "operation.old_local_contrast".to_owned(),
            group_key: "group.effects".to_owned(),
            control: "clahe".to_owned(),
        }),
    }
}
