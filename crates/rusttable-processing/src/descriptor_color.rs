use super::{
    AlphaPolicy, CapabilityContract, DescriptorId, ImagePredicate, InputOutputContract,
    MigrationContract, NonFinitePolicy, OperationDescriptor, OperationFlags, ParameterDefault,
    ParameterDescriptor, ParameterKind, ParameterRole, RoiKind, TilingContract,
};
use rusttable_color::ColorEncoding;

/// Returns the typed colorin descriptor.
///
/// # Panics
///
/// Panics only if the checked-in static descriptor identity is malformed.
#[must_use]
pub fn colorin_descriptor() -> OperationDescriptor {
    OperationDescriptor {
        id: DescriptorId::new("colorin", "rusttable.colorin", 7, 7, 1).expect("static ID"),
        parameters: vec![
            text("input_profile", "builtin:srgb"),
            text("working_profile", "builtin:linear-rec2020"),
            integer("intent", 0, 3, 0),
            integer("normalize", 0, 4, 0),
            ParameterDescriptor {
                id: "blue_mapping".to_owned(),
                kind: ParameterKind::Bool,
                default: ParameterDefault::Bool(true),
                required: true,
                introduced_version: 3,
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
        ],
        flags: OperationFlags::DETERMINISTIC_CPU
            .insert(OperationFlags::DETERMINISTIC_GPU)
            .insert(OperationFlags::TILEABLE)
            .insert(OperationFlags::COLOR),
        stage: "input-color".to_owned(),
        roi: RoiKind::Identity,
        tiling: tiling(),
        capability: capability(&["colorin_matrix", "colorin_transfer"]),
        io: color_io(true),
        mask_blend: mask_blend(),
        migration: MigrationContract {
            source_versions: (1..=7).collect(),
            target_version: 7,
            opaque_unknown_allowed: true,
        },
        ui: None,
    }
}

/// Returns the typed primaries descriptor.
///
/// # Panics
///
/// Panics only if the checked-in static descriptor identity is malformed.
#[must_use]
pub fn primaries_descriptor() -> OperationDescriptor {
    let names = [
        (
            "achromatic_tint_hue",
            -std::f64::consts::PI,
            std::f64::consts::PI,
            0.0,
        ),
        ("achromatic_tint_purity", 0.0, 0.99, 0.0),
        ("red_hue", -std::f64::consts::PI, std::f64::consts::PI, 0.0),
        ("red_purity", 0.01, 5.0, 1.0),
        (
            "green_hue",
            -std::f64::consts::PI,
            std::f64::consts::PI,
            0.0,
        ),
        ("green_purity", 0.01, 5.0, 1.0),
        ("blue_hue", -std::f64::consts::PI, std::f64::consts::PI, 0.0),
        ("blue_purity", 0.01, 5.0, 1.0),
    ];
    OperationDescriptor {
        id: DescriptorId::new("primaries", "rusttable.primaries", 1, 1, 1).expect("static ID"),
        parameters: names
            .into_iter()
            .map(|(id, minimum, maximum, default)| scalar(id, minimum, maximum, default))
            .collect(),
        flags: OperationFlags::DETERMINISTIC_CPU
            .insert(OperationFlags::DETERMINISTIC_GPU)
            .insert(OperationFlags::TILEABLE)
            .insert(OperationFlags::COLOR)
            .insert(OperationFlags::BLENDING),
        stage: "scene-linear".to_owned(),
        roi: RoiKind::Identity,
        tiling: tiling(),
        capability: capability(&["primaries_matrix"]),
        io: color_io(false),
        mask_blend: mask_blend(),
        migration: MigrationContract {
            source_versions: vec![1],
            target_version: 1,
            opaque_unknown_allowed: true,
        },
        ui: None,
    }
}

fn text(id: &str, default: &str) -> ParameterDescriptor {
    ParameterDescriptor {
        id: id.to_owned(),
        kind: ParameterKind::Text { maximum_bytes: 512 },
        default: ParameterDefault::Text(default.to_owned()),
        required: true,
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
    }
}

fn integer(id: &str, minimum: i64, maximum: i64, default: i64) -> ParameterDescriptor {
    ParameterDescriptor {
        id: id.to_owned(),
        kind: ParameterKind::Integer { minimum, maximum },
        default: ParameterDefault::Integer(default),
        required: true,
        introduced_version: 1,
        removed_version: None,
        unit: None,
        step: Some(1.0),
        precision: 0,
        role: ParameterRole::Color,
        cache_affecting: true,
        animatable: false,
        ui_hint: None,
        condition: None,
    }
}

fn scalar(id: &str, minimum: f64, maximum: f64, default: f64) -> ParameterDescriptor {
    ParameterDescriptor {
        id: id.to_owned(),
        kind: ParameterKind::Scalar { minimum, maximum },
        default: ParameterDefault::Scalar(default),
        required: true,
        introduced_version: 1,
        removed_version: None,
        unit: None,
        step: Some(0.001),
        precision: 4,
        role: ParameterRole::Color,
        cache_affecting: true,
        animatable: true,
        ui_hint: None,
        condition: None,
    }
}

fn tiling() -> TilingContract {
    TilingContract {
        overlap_pixels: 0,
        alignment_pixels: 1,
        minimum_tile_edge: 1,
        preferred_tile_edge: 256,
        temporary_multiplier_milli: 1000,
        input_multiplier_milli: 1000,
        output_multiplier_milli: 1000,
    }
}

fn capability(passes: &[&str]) -> CapabilityContract {
    CapabilityContract {
        cpu_supported: true,
        gpu_tier: Some(1),
        required_features: passes.iter().map(|pass| (*pass).to_owned()).collect(),
        required_formats: vec!["rgba32float".to_owned()],
        deterministic_cpu: true,
        deterministic_gpu: true,
        fallback_to_cpu: true,
        precision: "f32".to_owned(),
        modes: vec!["preview".to_owned(), "full".to_owned(), "export".to_owned()],
    }
}

fn color_io(derives_output: bool) -> InputOutputContract {
    let encodings = vec![
        ColorEncoding::SrgbD65,
        ColorEncoding::LinearSrgbD65,
        ColorEncoding::DisplayP3D65,
        ColorEncoding::LinearDisplayP3D65,
        ColorEncoding::Rec2020D65,
        ColorEncoding::LinearRec2020D65,
        ColorEncoding::AcesCgD60,
    ];
    let image = ImagePredicate {
        channels: 3,
        alpha: AlphaPolicy::Preserve,
        encodings,
        nonfinite: NonFinitePolicy::Reject,
    };
    InputOutputContract {
        input: image.clone(),
        output: image,
        derives_output_encoding: derives_output,
    }
}

fn mask_blend() -> super::MaskBlendContract {
    super::MaskBlendContract {
        consumes_mask: false,
        publishes_mask: false,
        blend_if: true,
        geometry: false,
        analysis: false,
    }
}
