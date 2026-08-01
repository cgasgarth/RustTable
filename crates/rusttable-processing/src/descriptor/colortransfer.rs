use rusttable_color::ColorEncoding;

use crate::descriptor::{
    AlphaPolicy, CapabilityContract, DescriptorId, ImagePredicate, InputOutputContract,
    MaskBlendContract, MigrationContract, NonFinitePolicy, OperationDescriptor, OperationFlags,
    ParameterDefault, ParameterDescriptor, ParameterKind, ParameterRole, RoiKind, TilingContract,
};
use crate::operations::common::encode_native_payload_chunks;

use crate::operations::colortransfer::{
    COLORTRANSFER_COMPATIBILITY_ID, COLORTRANSFER_NATIVE_PARAMETER_BYTES, COLORTRANSFER_RUST_ID,
    COLORTRANSFER_SCHEMA_VERSION, ColorTransferParameters,
};

/// Source-derived descriptor for deprecated Color Transfer's full-frame CPU route.
///
/// # Panics
///
/// Panics only if the compile-time descriptor identity or default payload is invalid.
#[must_use]
pub fn colortransfer_descriptor() -> OperationDescriptor {
    let parameters = encode_native_payload_chunks(&ColorTransferParameters::default().to_bytes())
        .into_iter()
        .enumerate()
        .map(|(index, default)| ParameterDescriptor {
            id: format!("payload_{index}"),
            kind: ParameterKind::Text {
                maximum_bytes: 4_096,
            },
            default: ParameterDefault::Text(default),
            required: false,
            introduced_version: COLORTRANSFER_SCHEMA_VERSION,
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
        COLORTRANSFER_NATIVE_PARAMETER_BYTES.div_ceil(2_048)
    );
    let image = ImagePredicate {
        channels: 4,
        alpha: AlphaPolicy::Preserve,
        encodings: vec![ColorEncoding::LabD50],
        nonfinite: NonFinitePolicy::Reject,
    };
    OperationDescriptor {
        id: DescriptorId::new(
            COLORTRANSFER_COMPATIBILITY_ID,
            COLORTRANSFER_RUST_ID,
            COLORTRANSFER_SCHEMA_VERSION,
            COLORTRANSFER_SCHEMA_VERSION,
            1,
        )
        .expect("static Color Transfer ID"),
        parameters,
        flags: OperationFlags::DEPRECATED
            .insert(OperationFlags::HISTORY_VISIBLE)
            .insert(OperationFlags::FULL_IMAGE)
            .insert(OperationFlags::ANALYSIS)
            .insert(OperationFlags::COLOR),
        stage: "display-referred-lab-d50".to_owned(),
        roi: RoiKind::FullImage,
        tiling: TilingContract {
            overlap_pixels: 0,
            alignment_pixels: 1,
            minimum_tile_edge: 1,
            preferred_tile_edge: 256,
            temporary_multiplier_milli: 2_000,
            input_multiplier_milli: 1_000,
            output_multiplier_milli: 1_000,
        },
        capability: CapabilityContract {
            cpu_supported: true,
            gpu_tier: None,
            required_features: vec!["full-native-v1-state".to_owned()],
            required_formats: vec!["rgba32float".to_owned()],
            deterministic_cpu: false,
            deterministic_gpu: false,
            fallback_to_cpu: true,
            precision: "native scalar f32 Lab D50 stochastic cluster transfer".to_owned(),
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
            analysis: true,
        },
        migration: MigrationContract {
            source_versions: vec![COLORTRANSFER_SCHEMA_VERSION],
            target_version: COLORTRANSFER_SCHEMA_VERSION,
            opaque_unknown_allowed: true,
        },
        ui: None,
    }
}
