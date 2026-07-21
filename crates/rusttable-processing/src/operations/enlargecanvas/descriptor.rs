use crate::descriptor::{
    AlphaPolicy, CapabilityContract, DescriptorId, ImagePredicate, InputOutputContract,
    MaskBlendContract, MigrationContract, NonFinitePolicy, OperationDescriptor, OperationFlags,
    ParameterDefault, ParameterDescriptor, ParameterKind, ParameterRole, RoiKind, TilingContract,
    UiHint,
};
use rusttable_color::ColorEncoding;

use super::{
    ENLARGECANVAS_COMPATIBILITY_ID, ENLARGECANVAS_PARAMETER_VERSION, ENLARGECANVAS_RUST_ID,
    ENLARGECANVAS_SCHEMA_VERSION,
};

#[must_use]
#[allow(clippy::too_many_lines)]
pub fn enlargecanvas_descriptor() -> OperationDescriptor {
    let percent = |id: &str| ParameterDescriptor {
        id: id.to_owned(),
        kind: ParameterKind::Scalar {
            minimum: 0.0,
            maximum: 100.0,
        },
        default: ParameterDefault::Scalar(0.0),
        required: true,
        introduced_version: ENLARGECANVAS_PARAMETER_VERSION,
        removed_version: None,
        unit: Some("percent".to_owned()),
        step: Some(0.1),
        precision: 2,
        role: ParameterRole::Geometry,
        cache_affecting: true,
        animatable: false,
        ui_hint: Some("slider".to_owned()),
        condition: None,
    };
    let image = ImagePredicate {
        channels: 4,
        alpha: AlphaPolicy::Preserve,
        encodings: vec![ColorEncoding::LinearSrgbD65],
        nonfinite: NonFinitePolicy::Reject,
    };
    OperationDescriptor {
        id: DescriptorId::new(
            ENLARGECANVAS_COMPATIBILITY_ID,
            ENLARGECANVAS_RUST_ID,
            ENLARGECANVAS_SCHEMA_VERSION,
            ENLARGECANVAS_PARAMETER_VERSION,
            1,
        )
        .expect("static enlargecanvas descriptor ID"),
        parameters: vec![
            percent("percent_left"),
            percent("percent_right"),
            percent("percent_top"),
            percent("percent_bottom"),
            ParameterDescriptor {
                id: "color".to_owned(),
                kind: ParameterKind::Integer {
                    minimum: 0,
                    maximum: 4,
                },
                default: ParameterDefault::Integer(0),
                required: true,
                introduced_version: ENLARGECANVAS_PARAMETER_VERSION,
                removed_version: None,
                unit: None,
                step: Some(1.0),
                precision: 0,
                role: ParameterRole::Color,
                cache_affecting: true,
                animatable: false,
                ui_hint: Some("choice".to_owned()),
                condition: None,
            },
        ],
        flags: OperationFlags::HISTORY_VISIBLE
            .insert(OperationFlags::TILEABLE)
            .insert(OperationFlags::DETERMINISTIC_CPU)
            .insert(OperationFlags::DETERMINISTIC_GPU)
            .insert(OperationFlags::GEOMETRY)
            .insert(OperationFlags::SCALE)
            .insert(OperationFlags::MASKS),
        stage: "geometry".to_owned(),
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
            required_features: vec!["enlargecanvas_fill_copy".to_owned()],
            required_formats: vec!["rgba32float".to_owned()],
            deterministic_cpu: true,
            deterministic_gpu: true,
            fallback_to_cpu: true,
            precision: "f32 source and fill with checked integer geometry".to_owned(),
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
            source_versions: vec![ENLARGECANVAS_PARAMETER_VERSION],
            target_version: ENLARGECANVAS_PARAMETER_VERSION,
            opaque_unknown_allowed: true,
        },
        ui: Some(UiHint {
            label_key: "operation.enlargecanvas".to_owned(),
            group_key: "group.geometry".to_owned(),
            control: "enlargecanvas".to_owned(),
        }),
    }
}
