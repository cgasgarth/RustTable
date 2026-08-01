use rusttable_color::ColorEncoding;

use crate::descriptor::{
    AlphaPolicy, CapabilityContract, DescriptorId, ImagePredicate, InputOutputContract,
    MaskBlendContract, MigrationContract, NonFinitePolicy, OperationDescriptor, OperationFlags,
    ParameterDefault, ParameterDescriptor, ParameterKind, ParameterRole, RoiKind, TilingContract,
};

use super::{
    LEVELS_BLACK_DEFAULT, LEVELS_BLACK_MAXIMUM, LEVELS_BLACK_MINIMUM, LEVELS_COMPATIBILITY_ID,
    LEVELS_DEFAULT_GRAY, LEVELS_DEFAULT_LEVELS, LEVELS_GRAY_MAXIMUM, LEVELS_GRAY_MINIMUM,
    LEVELS_RUST_ID, LEVELS_SCHEMA_VERSION, LEVELS_WHITE_DEFAULT, LEVELS_WHITE_MAXIMUM,
    LEVELS_WHITE_MINIMUM,
};

/// Source-derived processing contract for Darktable Levels.
///
/// The native blend flag is retained in architecture evidence, while the Rust
/// descriptor deliberately leaves masks and outer blending unavailable until
/// imported blend payloads can be materialized without approximation.
#[must_use]
pub fn levels_descriptor() -> OperationDescriptor {
    OperationDescriptor {
        id: DescriptorId::new(
            LEVELS_COMPATIBILITY_ID,
            LEVELS_RUST_ID,
            LEVELS_SCHEMA_VERSION,
            LEVELS_SCHEMA_VERSION,
            1,
        )
        .expect("static Levels ID"),
        parameters: vec![
            enum_parameter("mode", &["manual", "automatic"], "manual"),
            scalar_parameter(
                "black",
                LEVELS_BLACK_MINIMUM,
                LEVELS_BLACK_MAXIMUM,
                LEVELS_BLACK_DEFAULT,
                Some("percentile"),
            ),
            scalar_parameter(
                "gray",
                LEVELS_GRAY_MINIMUM,
                LEVELS_GRAY_MAXIMUM,
                LEVELS_DEFAULT_GRAY,
                Some("percentile"),
            ),
            scalar_parameter(
                "white",
                LEVELS_WHITE_MINIMUM,
                LEVELS_WHITE_MAXIMUM,
                LEVELS_WHITE_DEFAULT,
                Some("percentile"),
            ),
            ParameterDescriptor {
                id: "levels".to_owned(),
                kind: ParameterKind::Vector {
                    dimensions: 3,
                    minimum: 0.0,
                    maximum: 1.0,
                },
                default: ParameterDefault::Vector(
                    LEVELS_DEFAULT_LEVELS.into_iter().map(f64::from).collect(),
                ),
                required: false,
                introduced_version: 1,
                removed_version: None,
                unit: None,
                step: Some(0.001),
                precision: 6,
                role: ParameterRole::Processing,
                cache_affecting: true,
                animatable: true,
                ui_hint: Some("levels-triplet".to_owned()),
                condition: None,
            },
        ],
        flags: OperationFlags::DEPRECATED
            .insert(OperationFlags::MULTI_INSTANCE)
            .insert(OperationFlags::HISTORY_VISIBLE)
            .insert(OperationFlags::TILEABLE)
            .insert(OperationFlags::DETERMINISTIC_CPU)
            .insert(OperationFlags::COLOR),
        stage: "display-referred-lab-d50".to_owned(),
        roi: RoiKind::Identity,
        tiling: point_tiling(),
        capability: cpu_capability("f32 Lab D50 65536-entry lightness LUT"),
        io: lab_io(),
        mask_blend: MaskBlendContract {
            consumes_mask: false,
            publishes_mask: false,
            blend_if: false,
            geometry: false,
            analysis: false,
        },
        migration: MigrationContract {
            source_versions: vec![1, LEVELS_SCHEMA_VERSION],
            target_version: LEVELS_SCHEMA_VERSION,
            opaque_unknown_allowed: true,
        },
        ui: None,
    }
}

fn scalar_parameter(
    id: &str,
    minimum: f32,
    maximum: f32,
    default: f32,
    unit: Option<&str>,
) -> ParameterDescriptor {
    ParameterDescriptor {
        id: id.to_owned(),
        kind: ParameterKind::Scalar {
            minimum: f64::from(minimum),
            maximum: f64::from(maximum),
        },
        default: ParameterDefault::Scalar(f64::from(default)),
        required: false,
        introduced_version: LEVELS_SCHEMA_VERSION,
        removed_version: None,
        unit: unit.map(str::to_owned),
        step: Some(0.1),
        precision: 2,
        role: ParameterRole::Processing,
        cache_affecting: true,
        animatable: true,
        ui_hint: Some("slider".to_owned()),
        condition: None,
    }
}

fn enum_parameter(id: &str, tags: &[&str], default: &str) -> ParameterDescriptor {
    ParameterDescriptor {
        id: id.to_owned(),
        kind: ParameterKind::Enum {
            tags: tags.iter().map(|tag| (*tag).to_owned()).collect(),
        },
        default: ParameterDefault::Enum(default.to_owned()),
        required: false,
        introduced_version: LEVELS_SCHEMA_VERSION,
        removed_version: None,
        unit: None,
        step: None,
        precision: 0,
        role: ParameterRole::Processing,
        cache_affecting: true,
        animatable: false,
        ui_hint: Some("combo".to_owned()),
        condition: None,
    }
}

fn point_tiling() -> TilingContract {
    TilingContract {
        overlap_pixels: 0,
        alignment_pixels: 1,
        minimum_tile_edge: 1,
        preferred_tile_edge: 256,
        temporary_multiplier_milli: 1000,
        input_multiplier_milli: 1000,
        output_multiplier_milli: 1000,
    }
}

fn cpu_capability(precision: &str) -> CapabilityContract {
    CapabilityContract {
        cpu_supported: true,
        gpu_tier: None,
        required_features: vec!["deterministic-row-major".to_owned()],
        required_formats: vec!["rgba32float".to_owned()],
        deterministic_cpu: true,
        deterministic_gpu: false,
        fallback_to_cpu: true,
        precision: precision.to_owned(),
        modes: vec!["preview".to_owned(), "full".to_owned(), "export".to_owned()],
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
