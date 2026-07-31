//! Bounded Darktable Color Balance processing leaf.
//!
//! Direct source lineage: `src/iop/colorbalance.c`,
//! `data/kernels/extended.cl::{colorbalance,colorbalance_lgg,colorbalance_cdl}`,
//! `src/common/colorspaces_inline_conversions.h`, and
//! `src/develop/blends/blendif_lab.c`.
//!
//! This leaf deliberately stops before shared operation export, descriptor
//! registration, history dispatch, pixelpipe Lab/alpha integration, GPU
//! binding, and UI mounting.  Its descriptor is operation-local evidence only;
//! production availability remains fail-closed until those seams are owned by
//! the integration lane.

#![forbid(unsafe_code)]
#![allow(
    clippy::missing_errors_doc,
    reason = "the compatibility codec and closed f32 point operation have conventional contracts"
)]
#![allow(
    clippy::needless_range_loop,
    reason = "source-derived channel loops preserve native per-channel arithmetic order"
)]
#![allow(
    dead_code,
    reason = "the bounded leaf is intentionally not exported through shared hubs yet"
)]

pub mod codec;
pub mod execution;
pub mod math;

#[allow(unused_imports)]
pub use codec::{
    CHANNEL_BLUE, CHANNEL_FACTOR, CHANNEL_GREEN, CHANNEL_RED, CHANNEL_SIZE,
    COLORBALANCE_INTROSPECTION_VERSION, COLORBALANCE_V1_PARAMETER_BYTES,
    COLORBALANCE_V2_PARAMETER_BYTES, COLORBALANCE_V3_PARAMETER_BYTES, ColorBalanceCodecError,
    ColorBalanceHistory, ColorBalanceMode, ColorBalanceParametersV1, ColorBalanceParametersV2,
    ColorBalanceParametersV3, migrate_v1_to_v3, migrate_v2_to_v3,
};
#[allow(unused_imports)]
pub use execution::{
    ColorBalanceCoefficients, ColorBalanceCommitted, ColorBalanceConfig,
    ColorBalanceExecutionError, ColorBalanceParameterError, ColorBalancePixel, ColorBalancePlan,
};

use rusttable_color::ColorEncoding;

use crate::descriptor::{
    AlphaPolicy, CapabilityContract, DescriptorId, ImagePredicate, InputOutputContract,
    MaskBlendContract, MigrationContract, NonFinitePolicy, OperationDescriptor, OperationFlags,
    ParameterDefault, ParameterDescriptor, ParameterKind, ParameterRole, RoiKind, TilingContract,
};

pub const COLORBALANCE_COMPATIBILITY_ID: &str = "colorbalance";
pub const COLORBALANCE_RUST_ID: &str = "rusttable.colorbalance";

/// Operation-local descriptor. It is intentionally not exported through the
/// shared descriptor/registry hubs until the complete operation seam is ready.
#[must_use]
pub fn colorbalance_descriptor() -> OperationDescriptor {
    OperationDescriptor {
        id: DescriptorId::new(
            COLORBALANCE_COMPATIBILITY_ID,
            COLORBALANCE_RUST_ID,
            COLORBALANCE_INTROSPECTION_VERSION,
            COLORBALANCE_INTROSPECTION_VERSION,
            1,
        )
        .expect("static Color Balance descriptor identity"),
        parameters: vec![
            mode_parameter(),
            vector_parameter("lift", 1),
            vector_parameter("gamma", 1),
            vector_parameter("gain", 1),
            scalar_parameter("saturation", 0.0, 2.0, 1.0, 2),
            scalar_parameter("contrast", 0.01, 1.99, 1.0, 2),
            scalar_parameter("grey", 0.1, 100.0, 18.0, 2),
            scalar_parameter("saturation_out", 0.0, 2.0, 1.0, 3),
        ],
        // Native `flags()` omits `IOP_FLAGS_ALLOW_TILING`; this is a full-frame
        // scheduler contract even though the bounded CPU leaf chunks privately.
        flags: OperationFlags::MULTI_INSTANCE
            .insert(OperationFlags::STYLE_ELIGIBLE)
            .insert(OperationFlags::HISTORY_VISIBLE)
            .insert(OperationFlags::FULL_IMAGE)
            .insert(OperationFlags::DETERMINISTIC_CPU)
            .insert(OperationFlags::COLOR)
            .insert(OperationFlags::MASKS)
            .insert(OperationFlags::BLENDING),
        stage: "display-referred-lab-d50".to_owned(),
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
            required_formats: vec!["lab-f32x4".to_owned()],
            deterministic_cpu: true,
            deterministic_gpu: false,
            fallback_to_cpu: false,
            precision: "scalar f32 CPU with native approximate exp2(log2(x) * p); GPU unavailable"
                .to_owned(),
            modes: vec!["preview".to_owned(), "full".to_owned(), "export".to_owned()],
        },
        io: lab_io(),
        mask_blend: MaskBlendContract {
            consumes_mask: true,
            publishes_mask: false,
            blend_if: false,
            geometry: false,
            analysis: false,
        },
        migration: MigrationContract {
            source_versions: vec![1, 2, COLORBALANCE_INTROSPECTION_VERSION],
            target_version: COLORBALANCE_INTROSPECTION_VERSION,
            opaque_unknown_allowed: true,
        },
        ui: None,
    }
}

fn mode_parameter() -> ParameterDescriptor {
    ParameterDescriptor {
        id: "mode".to_owned(),
        kind: ParameterKind::Enum {
            tags: vec![
                "lift-gamma-gain".to_owned(),
                "slope-offset-power".to_owned(),
                "legacy".to_owned(),
            ],
        },
        default: ParameterDefault::Enum("slope-offset-power".to_owned()),
        required: false,
        introduced_version: 2,
        removed_version: None,
        unit: None,
        step: None,
        precision: 0,
        role: ParameterRole::Color,
        cache_affecting: true,
        animatable: false,
        ui_hint: None,
        condition: None,
    }
}

fn vector_parameter(id: &str, introduced_version: u16) -> ParameterDescriptor {
    ParameterDescriptor {
        id: id.to_owned(),
        kind: ParameterKind::Vector {
            dimensions: 4,
            minimum: 0.0,
            maximum: 2.0,
        },
        default: ParameterDefault::Vector(vec![1.0; 4]),
        required: false,
        introduced_version,
        removed_version: None,
        unit: Some("factor".to_owned()),
        step: Some(0.01),
        precision: 5,
        role: ParameterRole::Color,
        cache_affecting: true,
        animatable: true,
        ui_hint: None,
        condition: None,
    }
}

fn scalar_parameter(
    id: &str,
    minimum: f64,
    maximum: f64,
    default: f64,
    introduced_version: u16,
) -> ParameterDescriptor {
    ParameterDescriptor {
        id: id.to_owned(),
        kind: ParameterKind::Scalar { minimum, maximum },
        default: ParameterDefault::Scalar(default),
        required: false,
        introduced_version,
        removed_version: None,
        unit: Some("factor".to_owned()),
        step: Some(0.01),
        precision: 4,
        role: ParameterRole::Color,
        cache_affecting: true,
        animatable: true,
        ui_hint: None,
        condition: None,
    }
}

fn lab_io() -> InputOutputContract {
    let image = ImagePredicate {
        channels: 4,
        alpha: AlphaPolicy::Preserve,
        encodings: vec![ColorEncoding::LabD50],
        nonfinite: NonFinitePolicy::Reject,
    };
    InputOutputContract {
        input: image.clone(),
        output: image,
        derives_output_encoding: false,
    }
}
