#![allow(clippy::missing_panics_doc)]

use rusttable_color::ColorEncoding;

use crate::descriptor::{
    AlphaPolicy, CapabilityContract, DescriptorId, ImagePredicate, InputOutputContract,
    MaskBlendContract, MigrationContract, NonFinitePolicy, OperationDescriptor, OperationFlags,
    RoiKind, TilingContract,
};

/// Hidden, one-instance compatibility node. Its CPU execution is identity;
/// publication and consumption are owned by the typed mask graph boundary.
#[must_use]
pub fn mask_manager_descriptor() -> OperationDescriptor {
    let image = ImagePredicate {
        channels: 4,
        alpha: AlphaPolicy::Preserve,
        encodings: vec![ColorEncoding::LinearSrgbD65],
        nonfinite: NonFinitePolicy::Reject,
    };
    OperationDescriptor {
        id: DescriptorId::new("mask_manager", "rusttable.mask_manager", 2, 2, 1)
            .expect("static ID"),
        parameters: Vec::new(),
        flags: OperationFlags::HIDDEN
            .insert(OperationFlags::MASKS)
            .insert(OperationFlags::DETERMINISTIC_CPU),
        stage: "scene-linear".to_owned(),
        roi: RoiKind::Identity,
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
            precision: "identity RGBA32F with typed mask publication".to_owned(),
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
            publishes_mask: true,
            blend_if: false,
            geometry: true,
            analysis: false,
        },
        migration: MigrationContract {
            source_versions: vec![2],
            target_version: 2,
            opaque_unknown_allowed: true,
        },
        ui: None,
    }
}
