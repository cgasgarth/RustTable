use crate::descriptor::{
    AlphaPolicy, CapabilityContract, DescriptorId, ImagePredicate, InputOutputContract,
    MaskBlendContract, MigrationContract, NonFinitePolicy, OperationDescriptor, OperationFlags,
    ParameterDefault, ParameterDescriptor, ParameterKind, ParameterRole, RoiKind, TilingContract,
};
use rusttable_color::ColorEncoding;

#[must_use]
pub fn rasterfile_descriptor() -> OperationDescriptor {
    let image = ImagePredicate {
        channels: 4,
        alpha: AlphaPolicy::Preserve,
        encodings: vec![ColorEncoding::LinearSrgbD65],
        nonfinite: NonFinitePolicy::Reject,
    };
    OperationDescriptor {
        id: DescriptorId::new(
            super::RASTERFILE_COMPATIBILITY_ID,
            super::RASTERFILE_RUST_ID,
            super::RASTERFILE_SCHEMA_VERSION,
            super::RASTERFILE_PARAMETER_VERSION,
            super::RASTERFILE_IMPLEMENTATION_VERSION,
        )
        .expect("static rasterfile descriptor ID"),
        parameters: vec![
            ParameterDescriptor {
                id: "mode".to_owned(),
                kind: ParameterKind::Integer {
                    minimum: 1,
                    maximum: 7,
                },
                default: ParameterDefault::Integer(7),
                required: true,
                introduced_version: 1,
                removed_version: None,
                unit: None,
                step: Some(1.0),
                precision: 0,
                role: ParameterRole::Mask,
                cache_affecting: true,
                animatable: false,
                ui_hint: None,
                condition: None,
            },
            file_parameter("filename"),
            file_parameter("filename2"),
        ],
        flags: OperationFlags::HIDDEN
            .insert(OperationFlags::DEPRECATED)
            .insert(OperationFlags::HISTORY_VISIBLE)
            .insert(OperationFlags::FULL_IMAGE)
            .insert(OperationFlags::DETERMINISTIC_CPU)
            .insert(OperationFlags::DETERMINISTIC_GPU)
            .insert(OperationFlags::MASKS),
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
            gpu_tier: Some(1),
            required_features: vec!["raster-mask-upload".to_owned()],
            required_formats: vec!["r32float".to_owned()],
            deterministic_cpu: true,
            deterministic_gpu: true,
            fallback_to_cpu: true,
            precision: "canonical scalar f32 mask publication".to_owned(),
            modes: vec!["preview".to_owned(), "full".to_owned(), "export".to_owned()],
        },
        io: InputOutputContract {
            input: image.clone(),
            output: image,
            derives_output_encoding: false,
        },
        mask_blend: MaskBlendContract {
            consumes_mask: false,
            publishes_mask: true,
            blend_if: false,
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

fn file_parameter(id: &str) -> ParameterDescriptor {
    ParameterDescriptor {
        id: id.to_owned(),
        kind: ParameterKind::FileRef,
        default: ParameterDefault::FileRef(String::new()),
        required: false,
        introduced_version: 1,
        removed_version: None,
        unit: None,
        step: None,
        precision: 0,
        role: ParameterRole::Mask,
        cache_affecting: false,
        animatable: false,
        ui_hint: None,
        condition: None,
    }
}
