use rusttable_color::ColorEncoding;

use super::{
    AlphaPolicy, CapabilityContract, DescriptorId, ImagePredicate, InputOutputContract,
    MaskBlendContract, MigrationContract, NonFinitePolicy, OperationDescriptor, OperationFlags,
    ParameterDefault, ParameterDescriptor, ParameterKind, ParameterRole, RoiKind, TilingContract,
    UiHint,
};

/// The version-one descriptor shared by the censorize UI seam and #477.
///
/// The descriptor is intentionally published without a processing factory until
/// #477 supplies the qualified CPU implementation. Its parameter identity and
/// bounds are the integration contract for persisted history and GTK controls.
///
/// # Panics
///
/// Panics only if the static descriptor identity is invalid.
#[must_use]
pub fn censorize_descriptor() -> OperationDescriptor {
    OperationDescriptor {
        id: DescriptorId::new("censorize", "rusttable.censorize", 1, 1, 1).expect("static ID"),
        parameters: vec![
            radius("radius_1", "pre-blur radius"),
            radius("pixelate", "pixelization radius"),
            radius("radius_2", "post-blur radius"),
            ParameterDescriptor {
                id: "noise".to_owned(),
                kind: ParameterKind::Scalar {
                    minimum: 0.0,
                    maximum: 1.0,
                },
                default: ParameterDefault::Scalar(0.0),
                required: false,
                introduced_version: 1,
                removed_version: None,
                unit: Some("normalized".to_owned()),
                step: Some(0.01),
                precision: 2,
                role: ParameterRole::Processing,
                cache_affecting: true,
                animatable: true,
                ui_hint: Some("noise".to_owned()),
                condition: None,
            },
        ],
        flags: OperationFlags::FULL_IMAGE
            .insert(OperationFlags::HISTORY_VISIBLE)
            .insert(OperationFlags::MASKS)
            .insert(OperationFlags::BLENDING),
        stage: "display-linear".to_owned(),
        roi: RoiKind::FullImage,
        tiling: TilingContract {
            overlap_pixels: 0,
            alignment_pixels: 1,
            minimum_tile_edge: 1,
            preferred_tile_edge: 256,
            temporary_multiplier_milli: 5000,
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
            precision: "f32".to_owned(),
            modes: vec!["preview".to_owned(), "full".to_owned(), "export".to_owned()],
        },
        io: InputOutputContract {
            input: censorize_image_predicate(),
            output: censorize_image_predicate(),
            derives_output_encoding: false,
        },
        mask_blend: MaskBlendContract {
            consumes_mask: true,
            publishes_mask: false,
            blend_if: true,
            geometry: false,
            analysis: false,
        },
        migration: MigrationContract {
            source_versions: vec![1],
            target_version: 1,
            opaque_unknown_allowed: true,
        },
        ui: Some(UiHint {
            label_key: "operation.censorize".to_owned(),
            group_key: "group.effects".to_owned(),
            control: "censorize".to_owned(),
        }),
    }
}

fn radius(id: &str, control: &str) -> ParameterDescriptor {
    ParameterDescriptor {
        id: id.to_owned(),
        kind: ParameterKind::Scalar {
            minimum: 0.0,
            maximum: 500.0,
        },
        default: ParameterDefault::Scalar(0.0),
        required: false,
        introduced_version: 1,
        removed_version: None,
        unit: Some("px".to_owned()),
        step: Some(1.0),
        precision: 0,
        role: ParameterRole::Processing,
        cache_affecting: true,
        animatable: true,
        ui_hint: Some(control.to_owned()),
        condition: None,
    }
}

fn censorize_image_predicate() -> ImagePredicate {
    ImagePredicate {
        channels: 4,
        alpha: AlphaPolicy::Preserve,
        encodings: vec![ColorEncoding::LinearSrgbD65],
        nonfinite: NonFinitePolicy::Reject,
    }
}

#[cfg(test)]
mod tests {
    use super::censorize_descriptor;
    use crate::descriptor::{ParameterDefault, ParameterKind};

    #[test]
    fn descriptor_matches_the_v1_ui_contract() {
        let descriptor = censorize_descriptor();
        assert_eq!(descriptor.id.compatibility_name, "censorize");
        assert_eq!(descriptor.id.rust_id, "rusttable.censorize");
        assert_eq!(descriptor.id.parameter_version, 1);
        assert_eq!(descriptor.parameters.len(), 4);
        for (parameter, expected_id) in descriptor.parameters[..3]
            .iter()
            .zip(["radius_1", "pixelate", "radius_2"])
        {
            assert_eq!(parameter.id, expected_id);
            assert_eq!(
                parameter.kind,
                ParameterKind::Scalar {
                    minimum: 0.0,
                    maximum: 500.0
                }
            );
            assert_eq!(parameter.default, ParameterDefault::Scalar(0.0));
            assert_eq!(parameter.step, Some(1.0));
        }
        let noise = &descriptor.parameters[3];
        assert_eq!(noise.id, "noise");
        assert_eq!(
            noise.kind,
            ParameterKind::Scalar {
                minimum: 0.0,
                maximum: 1.0
            }
        );
        assert_eq!(noise.default, ParameterDefault::Scalar(0.0));
        assert_eq!(noise.step, Some(0.01));
        assert!(descriptor.validate().is_ok());
    }
}
