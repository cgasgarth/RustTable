use crate::descriptor::{
    AlphaPolicy, CapabilityContract, DescriptorId, ImagePredicate, InputOutputContract,
    MaskBlendContract, MigrationContract, NonFinitePolicy, OperationDescriptor, OperationFlags,
    ParameterDefault, ParameterDescriptor, ParameterKind, ParameterRole, RoiKind, TilingContract,
};
use rusttable_color::ColorEncoding;

#[must_use]
pub fn spots_descriptor() -> OperationDescriptor {
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
        role: ParameterRole::Processing,
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
            super::SPOTS_COMPATIBILITY_ID,
            super::SPOTS_RUST_ID,
            2,
            super::SPOTS_SCHEMA_VERSION,
            super::SPOTS_IMPLEMENTATION_VERSION,
        )
        .expect("static spots ID"),
        parameters: vec![payload],
        flags: OperationFlags::HISTORY_VISIBLE
            .insert(OperationFlags::HIDDEN)
            .insert(OperationFlags::DEPRECATED)
            .insert(OperationFlags::FULL_IMAGE)
            .insert(OperationFlags::DETERMINISTIC_CPU)
            .insert(OperationFlags::MASKS)
            .insert(OperationFlags::BLENDING),
        stage: "scene-linear".to_owned(),
        roi: RoiKind::FullImage,
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
            gpu_tier: None,
            required_features: Vec::new(),
            required_formats: Vec::new(),
            deterministic_cpu: true,
            deterministic_gpu: false,
            fallback_to_cpu: true,
            precision: "canonical deterministic f32 clone/heal scalar".to_owned(),
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
            geometry: true,
            analysis: false,
        },
        migration: MigrationContract {
            source_versions: vec![1, 2],
            target_version: super::SPOTS_SCHEMA_VERSION,
            opaque_unknown_allowed: true,
        },
        ui: None,
    }
}
