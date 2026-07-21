use crate::descriptor::{
    AlphaPolicy, CapabilityContract, DescriptorId, ImagePredicate, InputOutputContract,
    MaskBlendContract, MigrationContract, NonFinitePolicy, OperationDescriptor, OperationFlags,
    ParameterDefault, ParameterDescriptor, ParameterKind, ParameterRole, RoiKind, TilingContract,
};
use rusttable_color::ColorEncoding;

use super::{
    FINALSCALE_COMPATIBILITY_ID, FINALSCALE_PARAMETER_VERSION, FINALSCALE_RUST_ID,
    FINALSCALE_SCHEMA_VERSION,
};

#[must_use]
#[allow(clippy::too_many_lines)]
pub fn finalscale_descriptor() -> OperationDescriptor {
    let integer = |id: &str, maximum: i64| ParameterDescriptor {
        id: id.to_owned(),
        kind: ParameterKind::Integer {
            minimum: 0,
            maximum,
        },
        default: ParameterDefault::Integer(0),
        required: false,
        introduced_version: FINALSCALE_PARAMETER_VERSION,
        removed_version: None,
        unit: Some("pixel".to_owned()),
        step: Some(1.0),
        precision: 0,
        role: ParameterRole::Geometry,
        cache_affecting: true,
        animatable: false,
        ui_hint: None,
        condition: None,
    };
    let scalar = |id: &str, minimum: f64, maximum: f64, default: f64| ParameterDescriptor {
        id: id.to_owned(),
        kind: ParameterKind::Scalar { minimum, maximum },
        default: ParameterDefault::Scalar(default),
        required: false,
        introduced_version: FINALSCALE_PARAMETER_VERSION,
        removed_version: None,
        unit: None,
        step: Some(0.01),
        precision: 3,
        role: ParameterRole::Geometry,
        cache_affecting: true,
        animatable: false,
        ui_hint: None,
        condition: None,
    };
    let image = image_contract();
    OperationDescriptor {
        id: DescriptorId::new(
            FINALSCALE_COMPATIBILITY_ID,
            FINALSCALE_RUST_ID,
            FINALSCALE_SCHEMA_VERSION,
            FINALSCALE_PARAMETER_VERSION,
            1,
        )
        .expect("static finalscale descriptor ID"),
        parameters: vec![
            integer("mode", 7),
            integer("width", i64::from(super::FINALSCALE_MAX_DIMENSION)),
            integer("height", i64::from(super::FINALSCALE_MAX_DIMENSION)),
            integer("edge", i64::from(super::FINALSCALE_MAX_DIMENSION)),
            scalar("megapixels", 0.000_001, 1_000_000.0, 1.0),
            scalar("width_mm", 0.001, 10_000.0, 210.0),
            scalar("height_mm", 0.001, 10_000.0, 297.0),
            scalar("dpi", 1.0, 100_000.0, 300.0),
            scalar("pipeline_scale", 0.000_001, 1_000_000.0, 1.0),
            integer("allow_upscale", 1),
            integer("kernel", 3),
            integer("quality", 4),
        ],
        flags: OperationFlags::HIDDEN
            .insert(OperationFlags::HISTORY_VISIBLE)
            .insert(OperationFlags::TILEABLE)
            .insert(OperationFlags::DETERMINISTIC_CPU)
            .insert(OperationFlags::DETERMINISTIC_GPU)
            .insert(OperationFlags::GEOMETRY)
            .insert(OperationFlags::SCALE)
            .insert(OperationFlags::MASKS),
        stage: "geometry".to_owned(),
        roi: RoiKind::Scale,
        tiling: TilingContract {
            overlap_pixels: 3,
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
            required_features: vec!["finalscale_resampler".to_owned()],
            required_formats: vec!["rgba32float".to_owned()],
            deterministic_cpu: true,
            deterministic_gpu: true,
            fallback_to_cpu: true,
            precision: "f32 coefficients with f64 planning".to_owned(),
            modes: vec![
                "preview".to_owned(),
                "thumbnail".to_owned(),
                "image-final".to_owned(),
                "export".to_owned(),
                "print".to_owned(),
            ],
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
            source_versions: vec![FINALSCALE_PARAMETER_VERSION],
            target_version: FINALSCALE_PARAMETER_VERSION,
            opaque_unknown_allowed: true,
        },
        ui: None,
    }
}

fn image_contract() -> ImagePredicate {
    ImagePredicate {
        channels: 4,
        alpha: AlphaPolicy::Preserve,
        encodings: vec![ColorEncoding::LinearSrgbD65],
        nonfinite: NonFinitePolicy::Reject,
    }
}
