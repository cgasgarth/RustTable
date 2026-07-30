use super::{
    OVERLAY_COMPATIBILITY_ID, OVERLAY_IMPLEMENTATION_VERSION, OVERLAY_PARAMETER_VERSION,
    OVERLAY_RUST_ID, OVERLAY_SCHEMA_VERSION,
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
pub fn overlay_descriptor() -> OperationDescriptor {
    let image = ImagePredicate {
        channels: 4,
        alpha: AlphaPolicy::Preserve,
        encodings: vec![ColorEncoding::LinearSrgbD65],
        nonfinite: NonFinitePolicy::Reject,
    };
    OperationDescriptor {
        id: DescriptorId::new(
            OVERLAY_COMPATIBILITY_ID,
            OVERLAY_RUST_ID,
            OVERLAY_SCHEMA_VERSION,
            OVERLAY_PARAMETER_VERSION,
            OVERLAY_IMPLEMENTATION_VERSION,
        )
        .expect("static overlay descriptor"),
        parameters: vec![
            scalar("opacity", 0.0, 1.0),
            scalar("scale", 0.01, 5.0),
            scalar("xoffset", -1.0, 1.0),
            scalar("yoffset", -1.0, 1.0),
            ParameterDescriptor {
                id: "asset".to_owned(),
                kind: ParameterKind::FileRef,
                default: ParameterDefault::FileRef(String::new()),
                required: true,
                introduced_version: 1,
                removed_version: None,
                unit: None,
                step: None,
                precision: 0,
                role: ParameterRole::Processing,
                cache_affecting: true,
                animatable: false,
                ui_hint: None,
                condition: None,
            },
        ],
        flags: OperationFlags::HISTORY_VISIBLE
            .insert(OperationFlags::DETERMINISTIC_CPU)
            .insert(OperationFlags::DETERMINISTIC_GPU)
            .insert(OperationFlags::TILEABLE),
        stage: "scene-linear".to_owned(),
        roi: RoiKind::Identity,
        tiling: TilingContract {
            overlap_pixels: 2,
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
            required_features: vec!["overlay_inverse_sample".to_owned()],
            required_formats: vec!["rgba32float".to_owned()],
            deterministic_cpu: true,
            deterministic_gpu: true,
            fallback_to_cpu: true,
            precision: "canonical inverse sampling and linear-light alpha".to_owned(),
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
        ui: None,
    }
}
fn scalar(id: &str, min: f64, max: f64) -> ParameterDescriptor {
    ParameterDescriptor {
        id: id.to_owned(),
        kind: ParameterKind::Scalar {
            minimum: min,
            maximum: max,
        },
        default: ParameterDefault::Scalar(min.max(0.0)),
        required: true,
        introduced_version: 1,
        removed_version: None,
        unit: None,
        step: Some(0.001),
        precision: 4,
        role: ParameterRole::Geometry,
        cache_affecting: true,
        animatable: false,
        ui_hint: None,
        condition: None,
    }
}
