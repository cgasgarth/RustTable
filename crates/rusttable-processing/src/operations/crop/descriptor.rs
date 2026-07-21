use super::{CROP_COMPATIBILITY_ID, CROP_RUST_ID, CROP_SCHEMA_VERSION, MIN_OUTPUT_EDGE};
use crate::descriptor::{
    AlphaPolicy, CapabilityContract, DescriptorId, ImagePredicate, InputOutputContract,
    MaskBlendContract, MigrationContract, NonFinitePolicy, OperationDescriptor, OperationFlags,
    ParameterDefault, ParameterDescriptor, ParameterKind, ParameterRole, RoiKind, TilingContract,
    UiHint,
};
use rusttable_color::ColorEncoding;

#[must_use]
#[allow(clippy::too_many_lines)]
pub fn crop_descriptor() -> OperationDescriptor {
    let scalar = |id: &str, default: f64| ParameterDescriptor {
        id: id.to_owned(),
        kind: ParameterKind::Scalar {
            minimum: 0.0,
            maximum: 1.0,
        },
        default: ParameterDefault::Scalar(default),
        required: true,
        introduced_version: 1,
        removed_version: None,
        unit: None,
        step: Some(0.001),
        precision: 3,
        role: ParameterRole::Geometry,
        cache_affecting: true,
        animatable: true,
        ui_hint: Some("slider".to_owned()),
        condition: None,
    };
    let integer = |id: &str, default: i64| ParameterDescriptor {
        id: id.to_owned(),
        kind: ParameterKind::Integer {
            minimum: i64::from(i32::MIN),
            maximum: i64::from(i32::MAX),
        },
        default: ParameterDefault::Integer(default),
        required: true,
        introduced_version: 1,
        removed_version: None,
        unit: None,
        step: Some(1.0),
        precision: 0,
        role: ParameterRole::Geometry,
        cache_affecting: true,
        animatable: false,
        ui_hint: None,
        condition: None,
    };
    let image = ImagePredicate {
        channels: 3,
        alpha: AlphaPolicy::Preserve,
        encodings: vec![ColorEncoding::LinearSrgbD65],
        nonfinite: NonFinitePolicy::Reject,
    };
    OperationDescriptor {
        id: DescriptorId::new(
            CROP_COMPATIBILITY_ID,
            CROP_RUST_ID,
            1,
            CROP_SCHEMA_VERSION,
            1,
        )
        .expect("static crop ID"),
        parameters: vec![
            scalar("cx", 0.0),
            scalar("cy", 0.0),
            scalar("cw", 1.0),
            scalar("ch", 1.0),
            integer("ratio_n", -1),
            integer("ratio_d", -1),
        ],
        flags: OperationFlags::DETERMINISTIC_CPU
            .insert(OperationFlags::TILEABLE)
            .insert(OperationFlags::GEOMETRY)
            .insert(OperationFlags::HISTORY_VISIBLE),
        stage: "geometry".to_owned(),
        roi: RoiKind::Crop,
        tiling: TilingContract {
            overlap_pixels: 0,
            alignment_pixels: 1,
            minimum_tile_edge: MIN_OUTPUT_EDGE,
            preferred_tile_edge: 256,
            temporary_multiplier_milli: 1000,
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
            modes: vec!["preview".to_owned(), "export".to_owned()],
        },
        io: InputOutputContract {
            input: image.clone(),
            output: image,
            derives_output_encoding: false,
        },
        mask_blend: MaskBlendContract {
            consumes_mask: false,
            publishes_mask: false,
            blend_if: false,
            geometry: true,
            analysis: false,
        },
        migration: MigrationContract {
            source_versions: vec![1, 2, 3],
            target_version: 3,
            opaque_unknown_allowed: true,
        },
        ui: Some(UiHint {
            label_key: "operation.crop".to_owned(),
            group_key: "group.basic".to_owned(),
            control: "crop".to_owned(),
        }),
    }
}
