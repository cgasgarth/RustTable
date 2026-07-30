use super::{
    MAX_PIXEL_ASPECT_RATIO, MIN_PIXEL_ASPECT_RATIO, SCALEPIXELS_COMPATIBILITY_ID,
    SCALEPIXELS_RUST_ID, SCALEPIXELS_SCHEMA_VERSION,
};
use crate::descriptor::{
    AlphaPolicy, CapabilityContract, DescriptorId, ImagePredicate, InputOutputContract,
    MaskBlendContract, MigrationContract, NonFinitePolicy, OperationDescriptor, OperationFlags,
    ParameterDefault, ParameterDescriptor, ParameterKind, ParameterRole, RoiKind, TilingContract,
    UiHint,
};
use rusttable_color::ColorEncoding;

/// Typed descriptor for the single-instance Darktable-compatible operation.
#[must_use]
pub fn scalepixels_descriptor() -> OperationDescriptor {
    let image = ImagePredicate {
        channels: 3,
        alpha: AlphaPolicy::Preserve,
        encodings: vec![ColorEncoding::LinearSrgbD65],
        nonfinite: NonFinitePolicy::Reject,
    };
    OperationDescriptor {
        id: DescriptorId::new(
            SCALEPIXELS_COMPATIBILITY_ID,
            SCALEPIXELS_RUST_ID,
            1,
            SCALEPIXELS_SCHEMA_VERSION,
            1,
        )
        .expect("static scalepixels ID"),
        parameters: vec![ParameterDescriptor {
            id: "pixel_aspect_ratio".to_owned(),
            kind: ParameterKind::Scalar {
                minimum: f64::from(MIN_PIXEL_ASPECT_RATIO),
                maximum: f64::from(MAX_PIXEL_ASPECT_RATIO),
            },
            default: ParameterDefault::Scalar(1.0),
            required: true,
            introduced_version: 1,
            removed_version: None,
            unit: Some("ratio".to_owned()),
            step: Some(0.01),
            precision: 2,
            role: ParameterRole::Geometry,
            cache_affecting: true,
            animatable: false,
            ui_hint: Some("slider".to_owned()),
            condition: None,
        }],
        flags: OperationFlags::DETERMINISTIC_CPU
            .insert(OperationFlags::DETERMINISTIC_GPU)
            .insert(OperationFlags::TILEABLE)
            .insert(OperationFlags::GEOMETRY)
            .insert(OperationFlags::SCALE)
            .insert(OperationFlags::MASKS)
            .insert(OperationFlags::HISTORY_VISIBLE),
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
            required_features: vec![
                "scalepixels_image_resampler".to_owned(),
                "scalepixels_mask_resampler".to_owned(),
            ],
            required_formats: vec!["rgba32float".to_owned()],
            deterministic_cpu: true,
            deterministic_gpu: true,
            fallback_to_cpu: true,
            precision: "f32".to_owned(),
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
            source_versions: vec![1],
            target_version: SCALEPIXELS_SCHEMA_VERSION,
            opaque_unknown_allowed: true,
        },
        ui: Some(UiHint {
            label_key: "operation.scalepixels".to_owned(),
            group_key: "group.corrective".to_owned(),
            control: "pixel_aspect_ratio".to_owned(),
        }),
    }
}
