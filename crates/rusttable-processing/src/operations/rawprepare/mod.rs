//! Bounded operation-local Rust leaf for retained Darktable
//! `src/iop/rawprepare.c`.
//!
//! The leaf owns the native v1/v2 parameter boundary, source-derived crop and
//! level planning, Bayer/X-Trans CFA phase publication, four-plane CPU
//! normalization, valid embedded Bayer gain-map interpolation, checked tile
//! execution, and cancellation-safe publication. It deliberately does not
//! register with shared processing, import, pixelpipe, GPU, history, or UI
//! hubs. Unsupported camera layouts remain unavailable until those seams own
//! the decoder and metadata contracts.

#![forbid(unsafe_code)]
#![allow(
    clippy::doc_markdown,
    clippy::struct_excessive_bools,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::similar_names,
    unused_imports,
    dead_code,
    reason = "standalone bounded leaf is intentionally not exported through shared hubs"
)]

pub mod codec;
pub mod execution;
pub mod source_map;

pub use codec::{
    RAWPREPARE_COMPATIBILITY_ID, RAWPREPARE_HISTORY_V1_PARAMETER_BYTES,
    RAWPREPARE_HISTORY_V2_PARAMETER_BYTES, RAWPREPARE_INTROSPECTION_VERSION,
    RAWPREPARE_NATIVE_V1_PARAMETER_BYTES, RAWPREPARE_NATIVE_V2_PARAMETER_BYTES,
    RAWPREPARE_PARAMETER_BYTES, RAWPREPARE_V1_PARAMETER_BYTES, RAWPREPARE_V2_PARAMETER_BYTES,
    RawPrepareCodecError, RawPrepareFlatField, RawPrepareHistory, RawPrepareParametersV1,
    RawPrepareParametersV2, migrate_native_v1_to_v2, migrate_v1_to_v2,
};
pub use execution::{
    DT_IMAGE_HDR, DT_IMAGE_RAW, DT_IMAGE_S_RAW, RAWPREPARE_CHANNELS,
    RAWPREPARE_DEFAULT_MEMORY_BUDGET, RAWPREPARE_DEFAULT_TILE_EDGE, RawPrepareAlphaBehavior,
    RawPrepareCfa, RawPrepareCrop, RawPrepareError, RawPrepareGainMap, RawPrepareGainMapSet,
    RawPrepareImageMetadata, RawPrepareInputKind, RawPrepareMemoryBudget, RawPreparePlan,
    RawPrepareSampleFormat, RawPrepareTile, RawPrepareTiling,
};

use rusttable_color::ColorEncoding;
use rusttable_processing::descriptor::{
    AlphaPolicy, CapabilityContract, DescriptorId, ImagePredicate, InputOutputContract,
    MaskBlendContract, MigrationContract, NonFinitePolicy, OperationDescriptor, OperationFlags,
    ParameterDefault, ParameterDescriptor, ParameterKind, ParameterRole, RoiKind, TilingContract,
};

pub const RAWPREPARE_RUST_ID: &str = "rusttable.rawprepare";

/// The descriptor is evidence for the leaf only. Production registration is
/// intentionally deferred to the shared operation and raw-image seams.
#[must_use]
pub fn rawprepare_descriptor() -> OperationDescriptor {
    OperationDescriptor {
        id: DescriptorId::new(
            RAWPREPARE_COMPATIBILITY_ID,
            RAWPREPARE_RUST_ID,
            RAWPREPARE_INTROSPECTION_VERSION,
            RAWPREPARE_INTROSPECTION_VERSION,
            1,
        )
        .expect("static rawprepare descriptor identity"),
        parameters: vec![
            integer("left", 0, "pixels"),
            integer("top", 0, "pixels"),
            integer("right", 0, "pixels"),
            integer("bottom", 0, "pixels"),
            ParameterDescriptor {
                id: "raw_black_level_separate".to_owned(),
                kind: ParameterKind::Vector {
                    dimensions: 4,
                    minimum: 0.0,
                    maximum: f64::from(u16::MAX),
                },
                default: ParameterDefault::Vector(vec![0.0; 4]),
                required: true,
                introduced_version: 1,
                removed_version: None,
                unit: Some("sensor levels".to_owned()),
                step: Some(1.0),
                precision: 0,
                role: ParameterRole::Processing,
                cache_affecting: true,
                animatable: false,
                ui_hint: None,
                condition: None,
            },
            integer("raw_white_point", i64::from(u16::MAX), "sensor levels"),
            ParameterDescriptor {
                id: "flat_field".to_owned(),
                kind: ParameterKind::Enum {
                    tags: vec!["disabled".to_owned(), "embedded GainMap".to_owned()],
                },
                default: ParameterDefault::Enum("disabled".to_owned()),
                required: true,
                introduced_version: 2,
                removed_version: None,
                unit: None,
                step: None,
                precision: 0,
                role: ParameterRole::Processing,
                cache_affecting: true,
                animatable: false,
                ui_hint: None,
                condition: None,
            },
        ],
        flags: OperationFlags::MANDATORY
            .insert(OperationFlags::HISTORY_VISIBLE)
            .insert(OperationFlags::TILEABLE)
            .insert(OperationFlags::DETERMINISTIC_CPU)
            .insert(OperationFlags::GEOMETRY),
        stage: "raw-sensor-linear".to_owned(),
        roi: RoiKind::Crop,
        tiling: TilingContract {
            overlap_pixels: 0,
            alignment_pixels: 1,
            minimum_tile_edge: 1,
            preferred_tile_edge: RAWPREPARE_DEFAULT_TILE_EDGE,
            temporary_multiplier_milli: 1000,
            input_multiplier_milli: 1000,
            output_multiplier_milli: 1000,
        },
        capability: CapabilityContract {
            cpu_supported: true,
            gpu_tier: None,
            required_features: vec!["raw-image-metadata".to_owned()],
            // This scalar descriptor advertises only the fail-closed RAW
            // branch. The local executor also has the native SRAW 4-lane
            // branch, but ImagePredicate cannot express the required 1->1
            // OR 4->4 union without changing shared descriptor hubs.
            required_formats: vec!["raw-u16x1".to_owned(), "raw-f32x1".to_owned()],
            deterministic_cpu: true,
            deterministic_gpu: false,
            fallback_to_cpu: false,
            precision: "source-shaped scalar f32 CPU normalization; GPU unavailable".to_owned(),
            modes: vec!["preview".to_owned(), "full".to_owned(), "export".to_owned()],
        },
        io: InputOutputContract {
            // Native default_input_format/default_output_format has two
            // mutually exclusive shapes: true Bayer/X-Trans RAW is 1->1,
            // while imageio_rawspeed's SRAW materialization is 4->4. The
            // operation-local ImagePredicate has one channel count rather
            // than a disjunction, so advertise the narrowest truthful RAW
            // contract here. SRAW preparation remains an explicit deferred
            // union seam; it must not be approximated as 1->4.
            input: ImagePredicate {
                channels: 1,
                alpha: AlphaPolicy::Ignore,
                encodings: vec![ColorEncoding::Unspecified],
                nonfinite: NonFinitePolicy::Reject,
            },
            output: ImagePredicate {
                channels: 1,
                alpha: AlphaPolicy::Ignore,
                encodings: vec![ColorEncoding::Unspecified],
                nonfinite: NonFinitePolicy::Reject,
            },
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
            source_versions: vec![1, 2],
            target_version: 2,
            opaque_unknown_allowed: true,
        },
        ui: None,
    }
}

fn integer(id: &str, default: i64, unit: &str) -> ParameterDescriptor {
    ParameterDescriptor {
        id: id.to_owned(),
        kind: ParameterKind::Integer {
            minimum: 0,
            maximum: i64::from(u16::MAX),
        },
        default: ParameterDefault::Integer(default),
        required: false,
        introduced_version: 1,
        removed_version: None,
        unit: Some(unit.to_owned()),
        step: Some(1.0),
        precision: 0,
        role: ParameterRole::Geometry,
        cache_affecting: true,
        animatable: false,
        ui_hint: None,
        condition: None,
    }
}

/// Capability facts are kept separate so a caller cannot mistake a CPU leaf
/// for production registration or a decoder/GPU/UI implementation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawPrepareCapabilities {
    pub cpu: bool,
    pub gpu: bool,
    pub import_materialization: bool,
    pub production_routing: bool,
    pub ui: bool,
}

#[must_use]
pub const fn capabilities() -> RawPrepareCapabilities {
    RawPrepareCapabilities {
        cpu: true,
        gpu: false,
        import_materialization: false,
        production_routing: false,
        ui: false,
    }
}
