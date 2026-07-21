use rusttable_color::ColorEncoding;

use crate::descriptor::{
    AlphaPolicy, CapabilityContract, DescriptorId, ImagePredicate, InputOutputContract,
    MaskBlendContract, MigrationContract, NonFinitePolicy, OperationDescriptor, OperationFlags,
    ParameterDefault, ParameterDescriptor, ParameterKind, ParameterRole, RoiKind, TilingContract,
    UiHint,
};

#[must_use]
#[allow(clippy::missing_panics_doc, clippy::too_many_lines)]
pub fn defringe_descriptor() -> OperationDescriptor {
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
        encodings: vec![ColorEncoding::LabD50],
        nonfinite: NonFinitePolicy::Reject,
    };
    OperationDescriptor {
        id: DescriptorId::new("defringe", "rusttable.defringe", 1, 1, 1).expect("static ID"),
        parameters: vec![
            scalar("radius", 0.5, 20.0, 4.0, "pixels"),
            scalar("threshold", 0.5, 128.0, 20.0, "chroma squared"),
            ParameterDescriptor {
                id: "mode".to_owned(),
                kind: ParameterKind::Enum {
                    tags: vec![
                        "global_average".to_owned(),
                        "local_average".to_owned(),
                        "static".to_owned(),
                    ],
                },
                default: ParameterDefault::Enum("global_average".to_owned()),
                required: true,
                introduced_version: 1,
                removed_version: None,
                unit: None,
                step: None,
                precision: 0,
                role: ParameterRole::Processing,
                cache_affecting: true,
                animatable: false,
                ui_hint: Some("combo".to_owned()),
                condition: None,
            },
        ],
        flags: OperationFlags::DEPRECATED
            .insert(OperationFlags::HIDDEN)
            .insert(OperationFlags::HISTORY_VISIBLE)
            .insert(OperationFlags::TILEABLE)
            .insert(OperationFlags::DETERMINISTIC_CPU)
            .insert(OperationFlags::COLOR)
            .insert(OperationFlags::MASKS)
            .insert(OperationFlags::BLENDING)
            .insert(OperationFlags::ANALYSIS),
        stage: "display-referred-lab".to_owned(),
        roi: RoiKind::Neighborhood,
        tiling: TilingContract {
            overlap_pixels: 40,
            alignment_pixels: 1,
            minimum_tile_edge: 1,
            preferred_tile_edge: 256,
            temporary_multiplier_milli: 4000,
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
            precision: "f32 scalar fixed-order bounded Lab Gaussian".to_owned(),
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
            analysis: true,
        },
        migration: MigrationContract {
            source_versions: vec![1],
            target_version: 1,
            opaque_unknown_allowed: true,
        },
        ui: Some(UiHint {
            label_key: "operation.defringe".to_owned(),
            group_key: "group.corrective".to_owned(),
            control: "defringe".to_owned(),
        }),
    }
}
