//! Descriptor for Darktable's atomic legacy basic-adjustments operation.

use super::descriptor_operations::{default_io_contract, default_mask_blend};
use super::{
    CapabilityContract, DescriptorId, MigrationContract, OperationDescriptor, OperationFlags,
    ParameterDefault, ParameterDescriptor, ParameterKind, ParameterRole, RoiKind, TilingContract,
    UiHint,
};

#[must_use]
#[allow(clippy::too_many_lines)]
#[allow(clippy::missing_panics_doc)]
pub fn basicadj_descriptor() -> OperationDescriptor {
    OperationDescriptor {
        id: DescriptorId::new("basicadj", "rusttable.basicadj", 2, 2, 1).expect("static ID"),
        parameters: vec![
            scalar("black_point", -1.0, 1.0, 0.0, None, 1),
            scalar("exposure", -18.0, 18.0, 0.0, Some("ev"), 1),
            scalar("hlcompr", 0.0, 500.0, 0.0, Some("percent"), 1),
            scalar("hlcomprthresh", 0.0, 100.0, 0.0, Some("percent"), 1),
            scalar("contrast", -1.0, 5.0, 0.0, None, 1),
            ParameterDescriptor {
                id: "preserve_colors".to_owned(),
                kind: ParameterKind::Enum {
                    tags: vec![
                        "none".to_owned(),
                        "luminance".to_owned(),
                        "max".to_owned(),
                        "average".to_owned(),
                        "sum".to_owned(),
                        "norm".to_owned(),
                        "power".to_owned(),
                    ],
                },
                default: ParameterDefault::Enum("luminance".to_owned()),
                required: false,
                introduced_version: 1,
                removed_version: None,
                unit: None,
                step: Some(1.0),
                precision: 0,
                role: ParameterRole::Color,
                cache_affecting: true,
                animatable: false,
                ui_hint: Some("combo".to_owned()),
                condition: None,
            },
            scalar("middle_grey", 0.05, 100.0, 18.42, Some("percent"), 1),
            scalar("brightness", -4.0, 4.0, 0.0, None, 1),
            scalar("saturation", -1.0, 1.0, 0.0, None, 1),
            scalar("vibrance", -1.0, 1.0, 0.0, None, 2),
            scalar("clip", -1.0, 1.0, 0.0, None, 1),
        ],
        flags: OperationFlags::DEPRECATED
            .insert(OperationFlags::DETERMINISTIC_CPU)
            .insert(OperationFlags::TILEABLE)
            .insert(OperationFlags::COLOR)
            .insert(OperationFlags::BLENDING),
        stage: "scene-linear".to_owned(),
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
            required_features: Vec::new(),
            required_formats: Vec::new(),
            deterministic_cpu: true,
            deterministic_gpu: false,
            fallback_to_cpu: true,
            precision: "f32".to_owned(),
            modes: vec!["preview".to_owned(), "full".to_owned(), "export".to_owned()],
        },
        io: default_io_contract(),
        mask_blend: default_mask_blend(),
        migration: MigrationContract {
            source_versions: vec![1, 2],
            target_version: 2,
            opaque_unknown_allowed: true,
        },
        ui: Some(UiHint {
            label_key: "operation.basicadj".to_owned(),
            group_key: "group.basic".to_owned(),
            control: "basic-adjustments".to_owned(),
        }),
    }
}

fn scalar(
    id: &str,
    minimum: f64,
    maximum: f64,
    default: f64,
    unit: Option<&str>,
    introduced_version: u16,
) -> ParameterDescriptor {
    ParameterDescriptor {
        id: id.to_owned(),
        kind: ParameterKind::Scalar { minimum, maximum },
        default: ParameterDefault::Scalar(default),
        required: false,
        introduced_version,
        removed_version: None,
        unit: unit.map(str::to_owned),
        step: Some(0.001),
        precision: 3,
        role: ParameterRole::Processing,
        cache_affecting: true,
        animatable: true,
        ui_hint: Some("slider".to_owned()),
        condition: None,
    }
}
