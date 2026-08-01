use rusttable_color::ColorEncoding;

use crate::descriptor::{
    AlphaPolicy, CapabilityContract, DescriptorId, ImagePredicate, InputOutputContract,
    MaskBlendContract, MigrationContract, NonFinitePolicy, OperationDescriptor, OperationFlags,
    ParameterDefault, ParameterDescriptor, ParameterKind, ParameterRole, RoiKind, TilingContract,
};
use crate::operations::common::encode_native_payload_chunks;

use crate::operations::colormapping::{
    COLOR_MAPPING_COMPATIBILITY_ID, COLOR_MAPPING_PARAMETER_BYTES, COLOR_MAPPING_RUST_ID,
    COLOR_MAPPING_SCHEMA_VERSION, ColorMappingParametersV1,
};

/// Source-derived descriptor for the integrated Color Mapping CPU route.
///
/// # Panics
///
/// Panics only if the compile-time descriptor identity or default payload is invalid.
#[must_use]
pub fn colormapping_descriptor() -> OperationDescriptor {
    let parameters = encode_native_payload_chunks(&ColorMappingParametersV1::defaults().to_bytes())
        .into_iter()
        .enumerate()
        .map(|(index, default)| ParameterDescriptor {
            id: format!("payload_{index}"),
            kind: ParameterKind::Text {
                maximum_bytes: 4_096,
            },
            default: ParameterDefault::Text(default),
            required: false,
            introduced_version: COLOR_MAPPING_SCHEMA_VERSION,
            removed_version: None,
            unit: None,
            step: None,
            precision: 0,
            role: ParameterRole::Processing,
            cache_affecting: true,
            animatable: false,
            ui_hint: None,
            condition: None,
        })
        .collect::<Vec<_>>();
    debug_assert_eq!(
        parameters.len(),
        COLOR_MAPPING_PARAMETER_BYTES.div_ceil(2_048)
    );
    let image = ImagePredicate {
        channels: 4,
        alpha: AlphaPolicy::Preserve,
        encodings: vec![ColorEncoding::LabD50],
        nonfinite: NonFinitePolicy::Reject,
    };
    OperationDescriptor {
        id: DescriptorId::new(
            COLOR_MAPPING_COMPATIBILITY_ID,
            COLOR_MAPPING_RUST_ID,
            COLOR_MAPPING_SCHEMA_VERSION,
            COLOR_MAPPING_SCHEMA_VERSION,
            1,
        )
        .expect("static Color Mapping ID"),
        parameters,
        flags: OperationFlags::HISTORY_VISIBLE
            .insert(OperationFlags::TILEABLE)
            .insert(OperationFlags::DETERMINISTIC_CPU)
            .insert(OperationFlags::COLOR),
        stage: "display-referred-lab-d50".to_owned(),
        roi: RoiKind::Neighborhood,
        tiling: TilingContract {
            overlap_pixels: 0,
            alignment_pixels: 1,
            minimum_tile_edge: 1,
            preferred_tile_edge: 256,
            temporary_multiplier_milli: 4_000,
            input_multiplier_milli: 1_000,
            output_multiplier_milli: 1_000,
        },
        capability: CapabilityContract {
            cpu_supported: true,
            gpu_tier: None,
            required_features: vec!["full-native-v1-state".to_owned()],
            required_formats: vec!["rgba32float".to_owned()],
            deterministic_cpu: true,
            deterministic_gpu: false,
            fallback_to_cpu: true,
            precision: "native scalar f32 Lab D50 histogram and bilateral mapping".to_owned(),
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
            geometry: false,
            analysis: false,
        },
        migration: MigrationContract {
            source_versions: vec![COLOR_MAPPING_SCHEMA_VERSION],
            target_version: COLOR_MAPPING_SCHEMA_VERSION,
            opaque_unknown_allowed: true,
        },
        ui: None,
    }
}
