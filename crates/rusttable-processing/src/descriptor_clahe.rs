//! Descriptor for deprecated, imported-history-only CLAHE compatibility.

use crate::clahe_compatibility::{
    CLAHE_COMPATIBILITY_ID, CLAHE_PARAMETER_VERSION, CLAHE_RADIUS_DEFAULT, CLAHE_RADIUS_MAXIMUM,
    CLAHE_RADIUS_MINIMUM, CLAHE_SCHEMA_VERSION, CLAHE_SLOPE_DEFAULT, CLAHE_SLOPE_MAXIMUM,
    CLAHE_SLOPE_MINIMUM,
};
use crate::descriptor::{
    AlphaPolicy, CapabilityContract, DescriptorId, ImagePredicate, InputOutputContract,
    MaskBlendContract, MigrationContract, NonFinitePolicy, OperationDescriptor, OperationFlags,
    ParameterDefault, ParameterDescriptor, ParameterKind, ParameterRole, RoiKind, TilingContract,
    UiHint,
};
use rusttable_color::ColorEncoding;

#[must_use]
#[allow(clippy::missing_panics_doc)]
pub fn clahe_descriptor() -> OperationDescriptor {
    OperationDescriptor {
        id: DescriptorId::new(
            CLAHE_COMPATIBILITY_ID,
            "rusttable.clahe",
            CLAHE_SCHEMA_VERSION,
            CLAHE_PARAMETER_VERSION,
            1,
        )
        .expect("static CLAHE descriptor ID"),
        parameters: vec![
            scalar(
                "radius",
                CLAHE_RADIUS_MINIMUM,
                CLAHE_RADIUS_MAXIMUM,
                CLAHE_RADIUS_DEFAULT,
                "pixels",
                1.0,
                0,
            ),
            scalar(
                "slope",
                CLAHE_SLOPE_MINIMUM,
                CLAHE_SLOPE_MAXIMUM,
                CLAHE_SLOPE_DEFAULT,
                "amount",
                0.01,
                2,
            ),
        ],
        flags: OperationFlags::DEPRECATED
            .insert(OperationFlags::HIDDEN)
            .insert(OperationFlags::STYLE_ELIGIBLE)
            .insert(OperationFlags::HISTORY_VISIBLE)
            .insert(OperationFlags::FULL_IMAGE)
            .insert(OperationFlags::DETERMINISTIC_CPU)
            .insert(OperationFlags::COLOR)
            .insert(OperationFlags::ANALYSIS),
        stage: "scene-linear-rgb".to_owned(),
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
            // This is the intended #473 contract, not a claim that the current
            // registry can execute it. The registry availability gate below is
            // the source of truth until the CPU backend is qualified.
            cpu_supported: true,
            gpu_tier: None,
            required_features: Vec::new(),
            required_formats: Vec::new(),
            deterministic_cpu: true,
            deterministic_gpu: false,
            fallback_to_cpu: false,
            precision: "f64 scalar compatibility; qualification pending #473".to_owned(),
            modes: vec!["preview".to_owned(), "full".to_owned(), "export".to_owned()],
        },
        io: InputOutputContract {
            input: rgb_contract(),
            output: rgb_contract(),
            derives_output_encoding: false,
        },
        mask_blend: MaskBlendContract {
            consumes_mask: false,
            publishes_mask: false,
            blend_if: false,
            geometry: false,
            analysis: true,
        },
        migration: MigrationContract {
            source_versions: vec![CLAHE_PARAMETER_VERSION],
            target_version: CLAHE_PARAMETER_VERSION,
            opaque_unknown_allowed: true,
        },
        ui: Some(UiHint {
            label_key: "operation.old_local_contrast".to_owned(),
            group_key: "group.effects".to_owned(),
            control: "deprecated-compatibility".to_owned(),
        }),
    }
}

fn scalar(
    id: &str,
    minimum: f64,
    maximum: f64,
    default: f64,
    unit: &str,
    step: f64,
    precision: u8,
) -> ParameterDescriptor {
    ParameterDescriptor {
        id: id.to_owned(),
        kind: ParameterKind::Scalar { minimum, maximum },
        default: ParameterDefault::Scalar(default),
        required: true,
        introduced_version: CLAHE_PARAMETER_VERSION,
        removed_version: None,
        unit: Some(unit.to_owned()),
        step: Some(step),
        precision,
        role: ParameterRole::Processing,
        cache_affecting: true,
        animatable: true,
        ui_hint: Some("slider".to_owned()),
        condition: None,
    }
}

fn rgb_contract() -> ImagePredicate {
    ImagePredicate {
        channels: 4,
        alpha: AlphaPolicy::Preserve,
        encodings: vec![ColorEncoding::LinearSrgbD65],
        nonfinite: NonFinitePolicy::Reject,
    }
}
