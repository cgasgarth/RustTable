//! Descriptor for the deprecated, imported-history-only defringe operation.

use crate::defringe_compatibility::{
    DEFRINGE_COMPATIBILITY_ID, DEFRINGE_PARAMETER_VERSION, DEFRINGE_RADIUS_DEFAULT,
    DEFRINGE_RADIUS_MAXIMUM, DEFRINGE_RADIUS_MINIMUM, DEFRINGE_SCHEMA_VERSION,
    DEFRINGE_THRESHOLD_DEFAULT, DEFRINGE_THRESHOLD_MAXIMUM, DEFRINGE_THRESHOLD_MINIMUM,
    DefringeMode,
};
use crate::descriptor::{
    AlphaPolicy, CapabilityContract, DescriptorId, ImagePredicate, InputOutputContract,
    MaskBlendContract, MigrationContract, NonFinitePolicy, OperationDescriptor, OperationFlags,
    ParameterDefault, ParameterDescriptor, ParameterKind, ParameterRole, RoiKind, TilingContract,
    UiHint,
};
use rusttable_color::ColorEncoding;

pub const DEFRINGE_RUST_ID: &str = "rusttable.defringe";

#[must_use]
#[allow(clippy::missing_panics_doc)]
pub fn defringe_descriptor() -> OperationDescriptor {
    OperationDescriptor {
        id: DescriptorId::new(
            DEFRINGE_COMPATIBILITY_ID,
            DEFRINGE_RUST_ID,
            DEFRINGE_SCHEMA_VERSION,
            DEFRINGE_PARAMETER_VERSION,
            1,
        )
        .expect("static defringe descriptor ID"),
        parameters: vec![
            scalar(
                "radius",
                DEFRINGE_RADIUS_MINIMUM,
                DEFRINGE_RADIUS_MAXIMUM,
                DEFRINGE_RADIUS_DEFAULT,
            ),
            scalar(
                "threshold",
                DEFRINGE_THRESHOLD_MINIMUM,
                DEFRINGE_THRESHOLD_MAXIMUM,
                DEFRINGE_THRESHOLD_DEFAULT,
            ),
            ParameterDescriptor {
                id: "mode".to_owned(),
                kind: ParameterKind::Enum {
                    tags: vec![
                        DefringeMode::GlobalAverage.tag().to_owned(),
                        DefringeMode::LocalAverage.tag().to_owned(),
                        DefringeMode::Static.tag().to_owned(),
                    ],
                },
                default: ParameterDefault::Enum(DefringeMode::GlobalAverage.tag().to_owned()),
                required: false,
                introduced_version: DEFRINGE_PARAMETER_VERSION,
                removed_version: None,
                unit: None,
                step: Some(1.0),
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
            .insert(OperationFlags::BLENDING)
            .insert(OperationFlags::ANALYSIS),
        stage: "display-linear".to_owned(),
        roi: RoiKind::Neighborhood,
        tiling: TilingContract {
            overlap_pixels: 40,
            alignment_pixels: 1,
            minimum_tile_edge: 1,
            preferred_tile_edge: 256,
            temporary_multiplier_milli: 1000,
            input_multiplier_milli: 1000,
            output_multiplier_milli: 1000,
        },
        capability: CapabilityContract {
            // This is the intended #475 contract, not a claim that the current
            // registry can execute it. DefinitionAvailability below is the gate.
            cpu_supported: true,
            gpu_tier: None,
            required_features: Vec::new(),
            required_formats: Vec::new(),
            deterministic_cpu: true,
            deterministic_gpu: false,
            fallback_to_cpu: false,
            precision: "f32 Lab compatibility; qualification pending #475".to_owned(),
            modes: vec!["preview".to_owned(), "full".to_owned(), "export".to_owned()],
        },
        io: InputOutputContract {
            input: lab_contract(),
            output: lab_contract(),
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
            source_versions: vec![DEFRINGE_PARAMETER_VERSION],
            target_version: DEFRINGE_PARAMETER_VERSION,
            opaque_unknown_allowed: true,
        },
        ui: Some(UiHint {
            label_key: "operation.defringe".to_owned(),
            group_key: "group.corrective".to_owned(),
            control: "deprecated-compatibility".to_owned(),
        }),
    }
}

fn scalar(id: &str, minimum: f32, maximum: f32, default: f32) -> ParameterDescriptor {
    ParameterDescriptor {
        id: id.to_owned(),
        kind: ParameterKind::Scalar {
            minimum: f64::from(minimum),
            maximum: f64::from(maximum),
        },
        default: ParameterDefault::Scalar(f64::from(default)),
        required: false,
        introduced_version: DEFRINGE_PARAMETER_VERSION,
        removed_version: None,
        unit: None,
        step: Some(0.1),
        precision: 1,
        role: ParameterRole::Processing,
        cache_affecting: true,
        animatable: false,
        ui_hint: Some("slider".to_owned()),
        condition: None,
    }
}

fn lab_contract() -> ImagePredicate {
    ImagePredicate {
        channels: 4,
        alpha: AlphaPolicy::Preserve,
        encodings: vec![ColorEncoding::LabD50],
        nonfinite: NonFinitePolicy::Reject,
    }
}
