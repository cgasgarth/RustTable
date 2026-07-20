use crate::descriptor::{
    AlphaPolicy, CapabilityContract, DescriptorId, ImagePredicate, InputOutputContract,
    MaskBlendContract, MigrationContract, NonFinitePolicy, OperationDescriptor, OperationFlags,
    ParameterDefault, ParameterDescriptor, ParameterKind, ParameterRole, RoiKind, TilingContract,
};
use rusttable_color::ColorEncoding;

use super::codec::{
    ROTATEPIXELS_COMPATIBILITY_ID, ROTATEPIXELS_IMPLEMENTATION_VERSION, ROTATEPIXELS_MAX_DIMENSION,
    ROTATEPIXELS_PARAMETER_VERSION, ROTATEPIXELS_RUST_ID, ROTATEPIXELS_SCHEMA_VERSION,
};

/// Explicit history and UI policy for this technical operation.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RotatePixelsHistoryPolicy {
    pub hidden: bool,
    pub one_instance: bool,
    pub unsafe_copy: bool,
    pub copy_to_user_stack: bool,
    pub ordinary_controls: bool,
}

#[must_use]
pub const fn history_policy() -> RotatePixelsHistoryPolicy {
    RotatePixelsHistoryPolicy {
        hidden: true,
        one_instance: true,
        unsafe_copy: true,
        copy_to_user_stack: false,
        ordinary_controls: false,
    }
}

/// Descriptor metadata used by the registry owner.
///
/// # Panics
///
/// Panics only if the compile-time operation identifier violates the descriptor key contract.
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn rotatepixels_descriptor() -> OperationDescriptor {
    let integer = |id: &str| ParameterDescriptor {
        id: id.to_owned(),
        kind: ParameterKind::Integer {
            minimum: 0,
            maximum: i64::from(ROTATEPIXELS_MAX_DIMENSION),
        },
        default: ParameterDefault::Integer(0),
        required: true,
        introduced_version: ROTATEPIXELS_PARAMETER_VERSION,
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
    let image = ImagePredicate {
        channels: 4,
        alpha: AlphaPolicy::Preserve,
        encodings: vec![ColorEncoding::LinearSrgbD65],
        nonfinite: NonFinitePolicy::Reject,
    };
    OperationDescriptor {
        id: DescriptorId::new(
            ROTATEPIXELS_COMPATIBILITY_ID,
            ROTATEPIXELS_RUST_ID,
            ROTATEPIXELS_SCHEMA_VERSION,
            ROTATEPIXELS_PARAMETER_VERSION,
            ROTATEPIXELS_IMPLEMENTATION_VERSION,
        )
        .expect("static rotatepixels descriptor ID"),
        parameters: vec![
            integer("rx"),
            integer("ry"),
            ParameterDescriptor {
                id: "angle".to_owned(),
                kind: ParameterKind::Scalar {
                    minimum: -360.0,
                    maximum: 360.0,
                },
                default: ParameterDefault::Scalar(0.0),
                required: true,
                introduced_version: ROTATEPIXELS_PARAMETER_VERSION,
                removed_version: None,
                unit: Some("degree".to_owned()),
                step: Some(0.01),
                precision: 2,
                role: ParameterRole::Geometry,
                cache_affecting: true,
                animatable: false,
                ui_hint: None,
                condition: None,
            },
        ],
        flags: OperationFlags::HIDDEN
            .insert(OperationFlags::HISTORY_VISIBLE)
            .insert(OperationFlags::TILEABLE)
            .insert(OperationFlags::DETERMINISTIC_CPU)
            .insert(OperationFlags::DETERMINISTIC_GPU)
            .insert(OperationFlags::GEOMETRY)
            .insert(OperationFlags::MASKS),
        stage: "geometry".to_owned(),
        roi: RoiKind::Distortion,
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
            required_features: Vec::new(),
            required_formats: vec!["rgba32float".to_owned()],
            deterministic_cpu: true,
            deterministic_gpu: true,
            fallback_to_cpu: true,
            precision: "f32 coefficients with f64 planning".to_owned(),
            modes: vec!["preview".to_owned(), "full".to_owned(), "export".to_owned()],
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
            source_versions: vec![ROTATEPIXELS_PARAMETER_VERSION],
            target_version: ROTATEPIXELS_PARAMETER_VERSION,
            opaque_unknown_allowed: true,
        },
        ui: None,
    }
}
