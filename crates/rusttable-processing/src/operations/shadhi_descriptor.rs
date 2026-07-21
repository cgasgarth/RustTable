//! Descriptor metadata for the legacy shadows/highlights operation.

use rusttable_color::ColorEncoding;

use super::{
    SHADHI_DEFAULT_HIGHLIGHTS, SHADHI_DEFAULT_LOW_APPROXIMATION, SHADHI_DEFAULT_RADIUS,
    SHADHI_DEFAULT_SHADOWS,
};
use crate::descriptor::{
    AlphaPolicy, CapabilityContract, DescriptorId, ImagePredicate, InputOutputContract,
    MaskBlendContract, MigrationContract, NonFinitePolicy, OperationDescriptor, OperationFlags,
    ParameterDefault, ParameterDescriptor, ParameterKind, ParameterRole, RoiKind, TilingContract,
    UiHint,
};

#[must_use]
#[allow(clippy::too_many_lines)]
pub fn shadhi_descriptor() -> OperationDescriptor {
    OperationDescriptor {
        id: DescriptorId::new("shadhi", "rusttable.shadhi", 5, 5, 1).expect("static ID"),
        parameters: vec![
            scalar("order", 0.0, 3.0, 0.0, "order", ParameterRole::Processing),
            scalar(
                "radius",
                0.1,
                500.0,
                f64::from(SHADHI_DEFAULT_RADIUS),
                "pixels",
                ParameterRole::Geometry,
            ),
            scalar(
                "shadows",
                -100.0,
                100.0,
                f64::from(SHADHI_DEFAULT_SHADOWS),
                "percent",
                ParameterRole::Processing,
            ),
            scalar(
                "whitepoint",
                -10.0,
                10.0,
                0.0,
                "percent",
                ParameterRole::Color,
            ),
            scalar(
                "highlights",
                -100.0,
                100.0,
                f64::from(SHADHI_DEFAULT_HIGHLIGHTS),
                "percent",
                ParameterRole::Processing,
            ),
            scalar(
                "reserved2",
                -f64::MAX,
                f64::MAX,
                0.0,
                "reserved",
                ParameterRole::Processing,
            ),
            scalar(
                "compress",
                0.0,
                100.0,
                50.0,
                "percent",
                ParameterRole::Processing,
            ),
            scalar(
                "shadows_ccorrect",
                0.0,
                100.0,
                100.0,
                "percent",
                ParameterRole::Color,
            ),
            scalar(
                "highlights_ccorrect",
                0.0,
                100.0,
                50.0,
                "percent",
                ParameterRole::Color,
            ),
            scalar(
                "flags",
                0.0,
                f64::from(u32::MAX),
                127.0,
                "flags",
                ParameterRole::Processing,
            ),
            scalar(
                "low_approximation",
                0.000_000_001,
                1.0,
                f64::from(SHADHI_DEFAULT_LOW_APPROXIMATION),
                "epsilon",
                ParameterRole::Processing,
            ),
            scalar(
                "shadhi_algo",
                0.0,
                1.0,
                1.0,
                "algorithm",
                ParameterRole::Processing,
            ),
        ],
        flags: OperationFlags::FULL_IMAGE
            .insert(OperationFlags::DETERMINISTIC_CPU)
            .insert(OperationFlags::HISTORY_VISIBLE)
            .insert(OperationFlags::COLOR)
            .insert(OperationFlags::BLENDING)
            .insert(OperationFlags::ANALYSIS),
        stage: "scene-linear-rgb-compat".to_owned(),
        roi: RoiKind::FullImage,
        tiling: TilingContract {
            overlap_pixels: 256,
            alignment_pixels: 1,
            minimum_tile_edge: 1,
            preferred_tile_edge: 1024,
            temporary_multiplier_milli: 3000,
            input_multiplier_milli: 1000,
            output_multiplier_milli: 1000,
        },
        capability: CapabilityContract {
            cpu_supported: true,
            gpu_tier: None,
            required_features: vec!["linear-rgb".to_owned(), "gaussian-blur".to_owned()],
            required_formats: vec!["rgb-f32".to_owned()],
            deterministic_cpu: true,
            deterministic_gpu: false,
            fallback_to_cpu: false,
            precision: "f32".to_owned(),
            modes: vec!["preview".to_owned(), "full".to_owned(), "export".to_owned()],
        },
        io: rgb_io(),
        mask_blend: MaskBlendContract {
            consumes_mask: false,
            publishes_mask: false,
            blend_if: true,
            geometry: false,
            analysis: true,
        },
        migration: MigrationContract {
            source_versions: vec![1, 2, 3, 4, 5],
            target_version: 5,
            opaque_unknown_allowed: true,
        },
        ui: Some(UiHint {
            label_key: "operation.shadhi".to_owned(),
            group_key: "group.basic".to_owned(),
            control: "deprecated-shadows-highlights".to_owned(),
        }),
    }
}

fn scalar(
    id: &str,
    minimum: f64,
    maximum: f64,
    default: f64,
    unit: &str,
    role: ParameterRole,
) -> ParameterDescriptor {
    ParameterDescriptor {
        id: id.to_owned(),
        kind: ParameterKind::Scalar { minimum, maximum },
        default: ParameterDefault::Scalar(default),
        required: false,
        introduced_version: 1,
        removed_version: None,
        unit: Some(unit.to_owned()),
        step: Some(0.01),
        precision: 3,
        role,
        cache_affecting: true,
        animatable: true,
        ui_hint: Some("slider".to_owned()),
        condition: None,
    }
}

fn rgb_io() -> InputOutputContract {
    let image = ImagePredicate {
        channels: 3,
        alpha: AlphaPolicy::Preserve,
        encodings: vec![ColorEncoding::LinearSrgbD65],
        nonfinite: NonFinitePolicy::Reject,
    };
    InputOutputContract {
        input: image.clone(),
        output: image,
        derives_output_encoding: false,
    }
}
