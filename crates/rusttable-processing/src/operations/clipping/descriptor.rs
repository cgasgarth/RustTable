#![allow(clippy::missing_panics_doc)]

use crate::descriptor::{
    AlphaPolicy, CapabilityContract, DescriptorId, ImagePredicate, InputOutputContract,
    MaskBlendContract, MigrationContract, NonFinitePolicy, OperationDescriptor, OperationFlags,
    ParameterDefault, ParameterDescriptor, ParameterKind, ParameterRole, RoiKind, TilingContract,
};
use rusttable_color::ColorEncoding;

use super::codec::{
    CLIPPING_COMPATIBILITY_ID, CLIPPING_IMPLEMENTATION_VERSION, CLIPPING_PARAMETER_VERSION,
    CLIPPING_RUST_ID, CLIPPING_SCHEMA_VERSION,
};

pub const CLIPPING_DESCRIPTOR_PARAMETER_COUNT: usize = 21;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClippingHistoryPolicy {
    pub hidden: bool,
    pub one_instance: bool,
    pub deprecated: bool,
}

#[must_use]
pub const fn history_policy() -> ClippingHistoryPolicy {
    ClippingHistoryPolicy {
        hidden: true,
        one_instance: true,
        deprecated: true,
    }
}

#[must_use]
#[allow(clippy::too_many_lines)]
pub fn clipping_descriptor() -> OperationDescriptor {
    let scalar = |id: &str, min: f64, max: f64, default: f64, unit: &str| ParameterDescriptor {
        id: id.to_owned(),
        kind: ParameterKind::Scalar {
            minimum: min,
            maximum: max,
        },
        default: ParameterDefault::Scalar(default),
        required: true,
        introduced_version: CLIPPING_PARAMETER_VERSION,
        removed_version: None,
        unit: Some(unit.to_owned()),
        step: Some(0.001),
        precision: 3,
        role: ParameterRole::Geometry,
        cache_affecting: true,
        animatable: false,
        ui_hint: None,
        condition: None,
    };
    let integer = |id: &str, min: i64, max: i64, default: i64| ParameterDescriptor {
        id: id.to_owned(),
        kind: ParameterKind::Integer {
            minimum: min,
            maximum: max,
        },
        default: ParameterDefault::Integer(default),
        required: true,
        introduced_version: CLIPPING_PARAMETER_VERSION,
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
        channels: 4,
        alpha: AlphaPolicy::Preserve,
        encodings: vec![ColorEncoding::LinearSrgbD65],
        nonfinite: NonFinitePolicy::Reject,
    };
    let mut parameters = vec![
        scalar("angle", -180.0, 180.0, 0.0, "degree"),
        scalar("cx", 0.0, 1.0, 0.0, "ratio"),
        scalar("cy", 0.0, 1.0, 0.0, "ratio"),
        scalar("cw", 0.0, 1.0, 1.0, "ratio"),
        scalar("ch", 0.0, 1.0, 1.0, "ratio"),
        scalar("k_h", -2.0, 2.0, 0.0, "ratio"),
        scalar("k_v", -2.0, 2.0, 0.0, "ratio"),
    ];
    for id in ["kxa", "kya", "kxb", "kyb", "kxc", "kyc", "kxd", "kyd"] {
        parameters.push(scalar(id, 0.0, 1.0, 0.5, "ratio"));
    }
    parameters.extend([
        integer("k_type", 0, 4, 0),
        integer("k_sym", 0, 3, 0),
        integer("k_apply", 0, 1, 0),
        ParameterDescriptor {
            id: "crop_auto".to_owned(),
            kind: ParameterKind::Bool,
            default: ParameterDefault::Bool(true),
            required: true,
            introduced_version: CLIPPING_PARAMETER_VERSION,
            removed_version: None,
            unit: None,
            step: None,
            precision: 0,
            role: ParameterRole::Geometry,
            cache_affecting: true,
            animatable: false,
            ui_hint: None,
            condition: None,
        },
        integer("ratio_n", -32, 32, -1),
        integer("ratio_d", -32, 32, -1),
    ]);
    debug_assert_eq!(parameters.len(), CLIPPING_DESCRIPTOR_PARAMETER_COUNT);
    OperationDescriptor {
        id: DescriptorId::new(
            CLIPPING_COMPATIBILITY_ID,
            CLIPPING_RUST_ID,
            CLIPPING_SCHEMA_VERSION,
            CLIPPING_PARAMETER_VERSION,
            CLIPPING_IMPLEMENTATION_VERSION,
        )
        .expect("static clipping descriptor ID"),
        parameters,
        flags: OperationFlags::HIDDEN
            .insert(OperationFlags::HISTORY_VISIBLE)
            .insert(OperationFlags::DEPRECATED)
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
            precision: "f64 checked homography planning with f32 sampling".to_owned(),
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
            source_versions: vec![2, 3, 4, 5],
            target_version: CLIPPING_PARAMETER_VERSION,
            opaque_unknown_allowed: true,
        },
        ui: None,
    }
}
