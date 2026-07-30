use super::{
    BORDERS_COMPATIBILITY_ID, BORDERS_IMPLEMENTATION_VERSION, BORDERS_PARAMETER_VERSION,
    BORDERS_RUST_ID, BORDERS_SCHEMA_VERSION,
};
use crate::descriptor::{
    AlphaPolicy, CapabilityContract, DescriptorId, ImagePredicate, InputOutputContract,
    MaskBlendContract, MigrationContract, NonFinitePolicy, OperationDescriptor, OperationFlags,
    ParameterDefault, ParameterDescriptor, ParameterKind, ParameterRole, RoiKind, TilingContract,
};
use rusttable_color::ColorEncoding;

#[must_use]
///
/// # Panics
///
/// Panics only if the checked-in descriptor identity is invalid.
pub fn borders_descriptor() -> OperationDescriptor {
    let image = ImagePredicate {
        channels: 4,
        alpha: AlphaPolicy::Preserve,
        encodings: vec![ColorEncoding::LinearSrgbD65],
        nonfinite: NonFinitePolicy::Reject,
    };
    OperationDescriptor {
        id: DescriptorId::new(
            BORDERS_COMPATIBILITY_ID,
            BORDERS_RUST_ID,
            BORDERS_SCHEMA_VERSION,
            BORDERS_PARAMETER_VERSION,
            BORDERS_IMPLEMENTATION_VERSION,
        )
        .expect("static borders descriptor"),
        parameters: border_parameters(),
        flags: OperationFlags::HISTORY_VISIBLE
            .insert(OperationFlags::TILEABLE)
            .insert(OperationFlags::DETERMINISTIC_CPU)
            .insert(OperationFlags::DETERMINISTIC_GPU)
            .insert(OperationFlags::GEOMETRY)
            .insert(OperationFlags::SCALE),
        stage: "display-referred".to_owned(),
        roi: RoiKind::Scale,
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
            required_features: vec!["borders_fill_copy".to_owned()],
            required_formats: vec!["rgba32float".to_owned()],
            deterministic_cpu: true,
            deterministic_gpu: true,
            fallback_to_cpu: true,
            precision: "checked integer geometry and scalar frame precedence".to_owned(),
            modes: vec!["preview".to_owned(), "full".to_owned(), "export".to_owned()],
        },
        io: InputOutputContract {
            input: image.clone(),
            output: image,
            derives_output_encoding: false,
        },
        mask_blend: MaskBlendContract {
            consumes_mask: true,
            publishes_mask: true,
            blend_if: false,
            geometry: true,
            analysis: false,
        },
        migration: MigrationContract {
            source_versions: vec![1, 2, 3, 4],
            target_version: 4,
            opaque_unknown_allowed: true,
        },
        ui: None,
    }
}
fn border_parameters() -> Vec<ParameterDescriptor> {
    vec![
        scalar("aspect", -1.0, 3.0, ParameterRole::Geometry),
        scalar("size", 0.0, 0.5, ParameterRole::Geometry),
        scalar("pos_h", 0.0, 1.0, ParameterRole::Geometry),
        scalar("pos_v", 0.0, 1.0, ParameterRole::Geometry),
        scalar("frame_size", 0.0, 1.0, ParameterRole::Geometry),
        scalar("frame_offset", 0.0, 1.0, ParameterRole::Geometry),
        integer("orientation", 0, 2),
        integer("basis", 0, 4),
    ]
}

fn integer(id: &str, minimum: i64, maximum: i64) -> ParameterDescriptor {
    ParameterDescriptor {
        id: id.to_owned(),
        kind: ParameterKind::Integer { minimum, maximum },
        default: ParameterDefault::Integer(minimum),
        required: true,
        introduced_version: 4,
        removed_version: None,
        unit: None,
        step: Some(1.0),
        precision: 0,
        role: ParameterRole::Geometry,
        cache_affecting: true,
        animatable: false,
        ui_hint: None,
        condition: None,
    }
}

fn scalar(id: &str, min: f64, max: f64, role: ParameterRole) -> ParameterDescriptor {
    ParameterDescriptor {
        id: id.to_owned(),
        kind: ParameterKind::Scalar {
            minimum: min,
            maximum: max,
        },
        default: ParameterDefault::Scalar(min.max(0.0)),
        required: true,
        introduced_version: 4,
        removed_version: None,
        unit: None,
        step: Some(0.001),
        precision: 4,
        role,
        cache_affecting: true,
        animatable: false,
        ui_hint: None,
        condition: None,
    }
}
