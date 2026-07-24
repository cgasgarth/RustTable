use rusttable_color::ColorEncoding;

use super::{
    AlphaPolicy, CapabilityContract, DescriptorId, ImagePredicate, InputOutputContract,
    MaskBlendContract, MigrationContract, NonFinitePolicy, OperationDescriptor, OperationFlags,
    ParameterDefault, ParameterDescriptor, ParameterKind, ParameterRole, RoiKind, TilingContract,
    UiHint,
};

#[must_use]
pub fn bloom_descriptor() -> OperationDescriptor {
    let mut descriptor = effect_descriptor(
        "bloom",
        "rusttable.bloom",
        vec![
            scalar("size", 0.0, 100.0, 20.0, "percent"),
            scalar("threshold", 0.0, 100.0, 90.0, "percent"),
            scalar("strength", 0.0, 100.0, 25.0, "percent"),
        ],
        "operation.bloom",
        "bloom",
        "display-referred-lab",
        lab_effect_io(),
    );
    descriptor.mask_blend.consumes_mask = true;
    descriptor
}

#[must_use]
pub fn soften_descriptor() -> OperationDescriptor {
    effect_descriptor(
        "soften",
        "rusttable.soften",
        vec![
            scalar("size", 0.0, 100.0, 50.0, "percent"),
            scalar("saturation", 0.0, 100.0, 100.0, "percent"),
            scalar_with_unit("brightness", -2.0, 2.0, 0.33, "ev", "ev"),
            scalar("amount", 0.0, 100.0, 50.0, "percent"),
        ],
        "operation.soften",
        "soften",
        "display-linear",
        rgb_effect_io(),
    )
}

fn effect_descriptor(
    compatibility_name: &str,
    rust_id: &str,
    parameters: Vec<ParameterDescriptor>,
    label_key: &str,
    control: &str,
    stage: &str,
    io: InputOutputContract,
) -> OperationDescriptor {
    OperationDescriptor {
        id: DescriptorId::new(compatibility_name, rust_id, 1, 1, 1).expect("static ID"),
        parameters,
        flags: OperationFlags::FULL_IMAGE
            .insert(OperationFlags::DETERMINISTIC_CPU)
            .insert(OperationFlags::ANALYSIS)
            .insert(OperationFlags::HISTORY_VISIBLE)
            .insert(OperationFlags::BLENDING),
        stage: stage.to_owned(),
        roi: RoiKind::FullImage,
        tiling: TilingContract {
            overlap_pixels: 0,
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
            precision: "f32".to_owned(),
            modes: vec!["preview".to_owned(), "full".to_owned(), "export".to_owned()],
        },
        io,
        mask_blend: MaskBlendContract {
            consumes_mask: false,
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
            label_key: label_key.to_owned(),
            group_key: "group.effects".to_owned(),
            control: control.to_owned(),
        }),
    }
}

fn scalar(id: &str, minimum: f64, maximum: f64, default: f64, unit: &str) -> ParameterDescriptor {
    scalar_with_unit(id, minimum, maximum, default, unit, unit)
}

fn scalar_with_unit(
    id: &str,
    minimum: f64,
    maximum: f64,
    default: f64,
    unit: &str,
    control: &str,
) -> ParameterDescriptor {
    ParameterDescriptor {
        id: id.to_owned(),
        kind: ParameterKind::Scalar { minimum, maximum },
        default: ParameterDefault::Scalar(default),
        required: false,
        introduced_version: 1,
        removed_version: None,
        unit: Some(unit.to_owned()),
        step: Some(if unit == "ev" { 0.01 } else { 0.1 }),
        precision: if unit == "ev" { 2 } else { 1 },
        role: ParameterRole::Processing,
        cache_affecting: true,
        animatable: true,
        ui_hint: Some(control.to_owned()),
        condition: None,
    }
}

fn rgb_effect_io() -> InputOutputContract {
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

fn lab_effect_io() -> InputOutputContract {
    let image = ImagePredicate {
        channels: 4,
        alpha: AlphaPolicy::Preserve,
        encodings: vec![ColorEncoding::LabD50],
        nonfinite: NonFinitePolicy::Reject,
    };
    InputOutputContract {
        input: image.clone(),
        output: image,
        derives_output_encoding: false,
    }
}
