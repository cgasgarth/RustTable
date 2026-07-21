#![allow(clippy::missing_panics_doc)]

use rusttable_color::ColorEncoding;

use crate::descriptor::{
    AlphaPolicy, CapabilityContract, DescriptorId, ImagePredicate, InputOutputContract,
    MaskBlendContract, MigrationContract, NonFinitePolicy, OperationDescriptor, OperationFlags,
    ParameterDefault, ParameterDescriptor, ParameterKind, ParameterRole, RoiKind, TilingContract,
};

#[must_use]
pub fn retouch_descriptor() -> OperationDescriptor {
    let scalar = |id: &str, minimum: f64, maximum: f64, default: f64| ParameterDescriptor {
        id: id.to_owned(),
        kind: ParameterKind::Scalar { minimum, maximum },
        default: ParameterDefault::Scalar(default),
        required: true,
        introduced_version: 1,
        removed_version: None,
        unit: None,
        step: Some(1.0),
        precision: 3,
        role: ParameterRole::Processing,
        cache_affecting: true,
        animatable: false,
        ui_hint: None,
        condition: None,
    };
    let image = ImagePredicate {
        channels: 4,
        alpha: AlphaPolicy::Preserve,
        encodings: vec![ColorEncoding::LinearSrgbD65],
        nonfinite: NonFinitePolicy::Reject,
    };
    OperationDescriptor {
        id: DescriptorId::new("retouch", "rusttable.retouch", 1, 1, 1).expect("static ID"),
        parameters: vec![
            scalar("scales", 0.0, 8.0, 0.0),
            scalar("tile_edge", 1.0, 4096.0, 256.0),
            scalar("memory_budget", 1.0, 4_294_967_296.0, 536_870_912.0),
        ],
        flags: OperationFlags::HISTORY_VISIBLE
            .insert(OperationFlags::HIDDEN)
            .insert(OperationFlags::STYLE_ELIGIBLE)
            .insert(OperationFlags::FULL_IMAGE)
            .insert(OperationFlags::SCALE)
            .insert(OperationFlags::DETERMINISTIC_CPU)
            .insert(OperationFlags::MASKS),
        stage: "scene-linear".to_owned(),
        roi: RoiKind::FullImage,
        tiling: TilingContract {
            overlap_pixels: 4,
            alignment_pixels: 1,
            minimum_tile_edge: 16,
            preferred_tile_edge: 256,
            temporary_multiplier_milli: 2500,
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
            precision: "canonical f32 a-trous CPU; GPU capability is explicit unsupported"
                .to_owned(),
            modes: vec![
                "preview".to_owned(),
                "full".to_owned(),
                "thumbnail".to_owned(),
                "export".to_owned(),
            ],
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
            source_versions: vec![1],
            target_version: 1,
            opaque_unknown_allowed: true,
        },
        ui: None,
    }
}
