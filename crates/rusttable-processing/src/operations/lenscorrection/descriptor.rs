use crate::descriptor::{
    AlphaPolicy, CapabilityContract, DescriptorId, ImagePredicate, InputOutputContract,
    MaskBlendContract, MigrationContract, NonFinitePolicy, OperationDescriptor, OperationFlags,
    ParameterDefault, ParameterDescriptor, ParameterKind, ParameterRole, RoiKind, TilingContract,
    UiHint,
};
use rusttable_color::ColorEncoding;

use super::parameters::LENS_CORRECTION_PARAMETER_VERSION;
use super::{
    LENS_CORRECTION_COMPATIBILITY_ID, LENS_CORRECTION_IMPLEMENTATION_VERSION,
    LENS_CORRECTION_RUST_ID, LENS_CORRECTION_SCHEMA_VERSION,
};

/// Registry-ready metadata.  Registration remains owned by the orchestrator.
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn lenscorrection_descriptor() -> OperationDescriptor {
    let image = ImagePredicate {
        channels: 4,
        alpha: AlphaPolicy::Preserve,
        encodings: vec![ColorEncoding::LinearSrgbD65],
        nonfinite: NonFinitePolicy::Reject,
    };
    let choice = |id: &str, maximum: i64| ParameterDescriptor {
        id: id.to_owned(),
        kind: ParameterKind::Integer {
            minimum: 0,
            maximum,
        },
        default: ParameterDefault::Integer(0),
        required: true,
        introduced_version: LENS_CORRECTION_PARAMETER_VERSION,
        removed_version: None,
        unit: None,
        step: Some(1.0),
        precision: 0,
        role: ParameterRole::Processing,
        cache_affecting: true,
        animatable: false,
        ui_hint: Some("choice".to_owned()),
        condition: None,
    };
    let scalar = |id: &str, minimum: f64, maximum: f64, default: f64| ParameterDescriptor {
        id: id.to_owned(),
        kind: ParameterKind::Scalar { minimum, maximum },
        default: ParameterDefault::Scalar(default),
        required: true,
        introduced_version: LENS_CORRECTION_PARAMETER_VERSION,
        removed_version: None,
        unit: None,
        step: Some(0.01),
        precision: 4,
        role: ParameterRole::Processing,
        cache_affecting: true,
        animatable: false,
        ui_hint: Some("slider".to_owned()),
        condition: None,
    };
    OperationDescriptor {
        id: DescriptorId::new(
            LENS_CORRECTION_COMPATIBILITY_ID,
            LENS_CORRECTION_RUST_ID,
            LENS_CORRECTION_SCHEMA_VERSION,
            LENS_CORRECTION_PARAMETER_VERSION,
            LENS_CORRECTION_IMPLEMENTATION_VERSION,
        )
        .expect("static lens correction descriptor ID"),
        parameters: vec![
            choice("method", 1),
            choice("modify_flags", 7),
            choice("mode", 1),
            scalar("scale", 0.1, 2.0, 1.0),
            scalar("crop_factor", 0.1, 10.0, 1.0),
            scalar("focal_length", 0.1, 1000.0, 50.0),
            scalar("aperture", 0.1, 128.0, 8.0),
            scalar("distance", 0.0, 100_000.0, 1000.0),
        ],
        flags: OperationFlags::HISTORY_VISIBLE
            .insert(OperationFlags::TILEABLE)
            .insert(OperationFlags::DETERMINISTIC_CPU)
            .insert(OperationFlags::GEOMETRY)
            .insert(OperationFlags::MASKS),
        stage: "geometry".to_owned(),
        roi: RoiKind::Distortion,
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
            gpu_tier: None,
            required_features: Vec::new(),
            required_formats: vec!["rgba32float".to_owned()],
            deterministic_cpu: true,
            deterministic_gpu: false,
            fallback_to_cpu: true,
            precision: "f64 checked geometry with deterministic f32 bilinear sampling".to_owned(),
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
            source_versions: vec![LENS_CORRECTION_PARAMETER_VERSION],
            target_version: LENS_CORRECTION_PARAMETER_VERSION,
            opaque_unknown_allowed: true,
        },
        ui: Some(UiHint {
            label_key: "operation.lenscorrection".to_owned(),
            group_key: "group.technical".to_owned(),
            control: "lenscorrection".to_owned(),
        }),
    }
}
