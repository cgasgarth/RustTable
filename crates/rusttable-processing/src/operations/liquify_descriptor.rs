use super::{LIQUIFY_COMPATIBILITY_ID, LIQUIFY_RUST_ID, LIQUIFY_SCHEMA_VERSION, wgpu_passes};
use crate::descriptor::{
    AlphaPolicy, CapabilityContract, DescriptorId, ImagePredicate, InputOutputContract,
    MaskBlendContract, MigrationContract, NonFinitePolicy, OperationDescriptor, OperationFlags,
    ParameterDefault, ParameterDescriptor, ParameterKind, ParameterRole, RoiKind, TilingContract,
    UiHint,
};
use rusttable_color::ColorEncoding;

/// # Panics
///
/// Panics only if the compile-time liquify descriptor identity is invalid.
pub fn liquify_descriptor() -> OperationDescriptor {
    let payload = ParameterDescriptor {
        id: "payload".to_owned(),
        kind: ParameterKind::Text {
            maximum_bytes: 4096,
        },
        default: ParameterDefault::Text(String::new()),
        required: true,
        introduced_version: 1,
        removed_version: None,
        unit: None,
        step: None,
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
            LIQUIFY_COMPATIBILITY_ID,
            LIQUIFY_RUST_ID,
            1,
            LIQUIFY_SCHEMA_VERSION,
            1,
        )
        .expect("static liquify ID"),
        parameters: vec![payload],
        flags: OperationFlags::HISTORY_VISIBLE
            .insert(OperationFlags::STYLE_ELIGIBLE)
            .insert(OperationFlags::FULL_IMAGE)
            .insert(OperationFlags::DETERMINISTIC_CPU)
            .insert(OperationFlags::GEOMETRY)
            .insert(OperationFlags::MASKS),
        stage: "geometry".to_owned(),
        roi: RoiKind::FullImage,
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
            gpu_tier: None,
            required_features: wgpu_passes().into_iter().map(str::to_owned).collect(),
            required_formats: vec!["rgba32float".to_owned()],
            deterministic_cpu: true,
            deterministic_gpu: false,
            fallback_to_cpu: true,
            precision: "f32-inverse-field".to_owned(),
            modes: vec!["preview".to_owned(), "export".to_owned()],
        },
        io: InputOutputContract {
            input: image.clone(),
            output: image,
            derives_output_encoding: false,
        },
        mask_blend: MaskBlendContract {
            consumes_mask: false,
            publishes_mask: true,
            blend_if: true,
            geometry: true,
            analysis: false,
        },
        migration: MigrationContract {
            source_versions: vec![1],
            target_version: 1,
            opaque_unknown_allowed: true,
        },
        ui: Some(UiHint {
            label_key: "operation.liquify".to_owned(),
            group_key: "group.geometry".to_owned(),
            control: "liquify".to_owned(),
        }),
    }
}
