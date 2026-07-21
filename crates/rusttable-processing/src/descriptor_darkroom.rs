//! Descriptor-only metadata for Darktable operations whose evaluators are not
//! part of this migration slice.

use rusttable_color::ColorEncoding;

use super::{
    AlphaPolicy, CapabilityContract, DescriptorId, ImagePredicate, InputOutputContract,
    MaskBlendContract, MigrationContract, NonFinitePolicy, OperationDescriptor, OperationFlags,
    ParameterDefault, ParameterDescriptor, ParameterKind, ParameterRole, RoiKind, TilingContract,
    UiHint,
};

#[must_use]
pub fn vignette_descriptor() -> OperationDescriptor {
    darkroom_descriptor(
        "vignette",
        "rusttable.vignette",
        vec![
            scalar("scale", 0.0, 200.0, 80.0, "percent", 0.1, 1),
            scalar("falloff_scale", 0.0, 200.0, 50.0, "percent", 0.1, 1),
            scalar("brightness", -1.0, 1.0, -0.5, "strength", 0.001, 3),
            scalar("saturation", -1.0, 1.0, -0.5, "strength", 0.001, 3),
            vector("center", 2, (-1.0, 1.0), 0.0, "position", 0.001, 3),
            boolean("autoratio", false, "toggle"),
            scalar("whratio", 0.0, 2.0, 1.0, "ratio", 0.001, 3),
            scalar("shape", 0.0, 5.0, 1.0, "shape", 0.1, 1),
            enumeration(
                "dithering",
                ["off", "8-bit output", "16-bit output"],
                "off",
                "choice",
            ),
            boolean("unbound", true, "toggle"),
        ],
        "operation.vignette",
        "vignette",
        "display-linear",
        RoiKind::Identity,
    )
}

#[must_use]
pub fn graduatednd_descriptor() -> OperationDescriptor {
    darkroom_descriptor(
        "graduatednd",
        "rusttable.graduatednd",
        vec![
            scalar("density", -8.0, 8.0, 1.0, "ev", 0.01, 2),
            scalar("hardness", 0.0, 100.0, 0.0, "percent", 0.1, 1),
            scalar("rotation", -180.0, 180.0, 0.0, "degrees", 0.1, 1),
            scalar("offset", 0.0, 100.0, 50.0, "percent", 0.1, 1),
            scalar("hue", 0.0, 1.0, 0.0, "hue", 0.001, 3),
            scalar("saturation", 0.0, 1.0, 0.0, "saturation", 0.001, 3),
        ],
        "operation.graduatednd",
        "gradient",
        "scene-linear",
        RoiKind::Identity,
    )
}

fn darkroom_descriptor(
    compatibility_name: &str,
    rust_id: &str,
    parameters: Vec<ParameterDescriptor>,
    label_key: &str,
    control: &str,
    stage: &str,
    roi: RoiKind,
) -> OperationDescriptor {
    OperationDescriptor {
        id: DescriptorId::new(compatibility_name, rust_id, 1, 1, 1).expect("static ID"),
        parameters,
        flags: OperationFlags::HISTORY_VISIBLE.insert(OperationFlags::BLENDING),
        stage: stage.to_owned(),
        roi,
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
            cpu_supported: false,
            gpu_tier: None,
            required_features: Vec::new(),
            required_formats: Vec::new(),
            deterministic_cpu: false,
            deterministic_gpu: false,
            fallback_to_cpu: false,
            precision: "f32".to_owned(),
            modes: vec!["preview".to_owned(), "full".to_owned(), "export".to_owned()],
        },
        io: darkroom_io_contract(),
        mask_blend: darkroom_mask_blend(),
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

fn darkroom_io_contract() -> InputOutputContract {
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

fn darkroom_mask_blend() -> MaskBlendContract {
    MaskBlendContract {
        consumes_mask: false,
        publishes_mask: false,
        blend_if: true,
        geometry: false,
        analysis: false,
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
        required: false,
        introduced_version: 1,
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

fn vector(
    id: &str,
    dimensions: u8,
    range: (f64, f64),
    default: f64,
    unit: &str,
    step: f64,
    precision: u8,
) -> ParameterDescriptor {
    ParameterDescriptor {
        id: id.to_owned(),
        kind: ParameterKind::Vector {
            dimensions,
            minimum: range.0,
            maximum: range.1,
        },
        default: ParameterDefault::Vector(vec![default; usize::from(dimensions)]),
        required: false,
        introduced_version: 1,
        removed_version: None,
        unit: Some(unit.to_owned()),
        step: Some(step),
        precision,
        role: ParameterRole::Geometry,
        cache_affecting: true,
        animatable: true,
        ui_hint: Some("vector".to_owned()),
        condition: None,
    }
}

fn boolean(id: &str, default: bool, control: &str) -> ParameterDescriptor {
    ParameterDescriptor {
        id: id.to_owned(),
        kind: ParameterKind::Bool,
        default: ParameterDefault::Bool(default),
        required: false,
        introduced_version: 1,
        removed_version: None,
        unit: None,
        step: None,
        precision: 0,
        role: ParameterRole::Presentation,
        cache_affecting: true,
        animatable: false,
        ui_hint: Some(control.to_owned()),
        condition: None,
    }
}

fn enumeration(id: &str, tags: [&str; 3], default: &str, control: &str) -> ParameterDescriptor {
    ParameterDescriptor {
        id: id.to_owned(),
        kind: ParameterKind::Enum {
            tags: tags.into_iter().map(str::to_owned).collect(),
        },
        default: ParameterDefault::Enum(default.to_owned()),
        required: false,
        introduced_version: 1,
        removed_version: None,
        unit: None,
        step: None,
        precision: 0,
        role: ParameterRole::Presentation,
        cache_affecting: true,
        animatable: false,
        ui_hint: Some(control.to_owned()),
        condition: None,
    }
}
