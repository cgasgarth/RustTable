//! Built-in operation descriptors kept separate from the descriptor schema core.

use rusttable_color::ColorEncoding;

use super::{
    AlphaPolicy, CapabilityContract, DescriptorId, ImagePredicate, InputOutputContract,
    MaskBlendContract, MigrationContract, NonFinitePolicy, OperationDescriptor, OperationFlags,
    ParameterDefault, ParameterDescriptor, ParameterKind, ParameterRole, RoiKind, TilingContract,
    UiHint,
};
#[must_use]
///
/// # Panics
///
/// This function cannot panic because its fixed descriptor identity is valid.
fn base_exposure_descriptor() -> OperationDescriptor {
    OperationDescriptor {
        id: DescriptorId::new("exposure", "rusttable.exposure", 1, 1, 1).expect("static ID"),
        parameters: vec![
            ParameterDescriptor {
                id: "stops".to_owned(),
                kind: ParameterKind::Scalar {
                    minimum: -18.0,
                    maximum: 18.0,
                },
                default: ParameterDefault::Scalar(0.0),
                required: true,
                introduced_version: 1,
                removed_version: None,
                unit: Some("ev".to_owned()),
                step: Some(0.01),
                precision: 2,
                role: ParameterRole::Processing,
                cache_affecting: true,
                animatable: true,
                ui_hint: Some("slider".to_owned()),
                condition: None,
            },
            ParameterDescriptor {
                id: "black".to_owned(),
                kind: ParameterKind::Scalar {
                    minimum: -1.0,
                    maximum: 1.0,
                },
                default: ParameterDefault::Scalar(0.0),
                required: false,
                introduced_version: 1,
                removed_version: None,
                unit: None,
                step: Some(0.000_001),
                precision: 6,
                role: ParameterRole::Processing,
                cache_affecting: true,
                animatable: true,
                ui_hint: Some("slider".to_owned()),
                condition: None,
            },
        ],
        flags: OperationFlags::DETERMINISTIC_CPU.insert(OperationFlags::TILEABLE),
        stage: "scene-linear".to_owned(),
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
            required_formats: Vec::new(),
            deterministic_cpu: true,
            deterministic_gpu: false,
            fallback_to_cpu: true,
            precision: "f32".to_owned(),
            modes: Vec::new(),
        },
        io: default_io_contract(),
        mask_blend: default_mask_blend(),
        migration: MigrationContract {
            source_versions: vec![1],
            target_version: 1,
            opaque_unknown_allowed: true,
        },
        ui: Some(UiHint {
            label_key: "operation.exposure".to_owned(),
            group_key: "group.basic".to_owned(),
            control: "slider".to_owned(),
        }),
    }
}

#[must_use]
///
/// # Panics
///
/// This function cannot panic because its fixed descriptor identity is valid.
pub fn exposure_descriptor() -> OperationDescriptor {
    let mut descriptor = base_exposure_descriptor();
    descriptor.flags = descriptor.flags.insert(OperationFlags::DETERMINISTIC_GPU);
    descriptor.capability.gpu_tier = Some(1);
    descriptor.capability.required_features = vec![
        "f32-storage".to_owned(),
        "deterministic-row-major".to_owned(),
    ];
    descriptor.capability.required_formats = vec!["rgba32float".to_owned()];
    descriptor.capability.deterministic_gpu = true;
    descriptor
}

#[must_use]
///
/// # Panics
///
/// This function cannot panic because its fixed descriptor identity is valid.
pub fn rgb_gain_descriptor() -> OperationDescriptor {
    let mut descriptor = base_exposure_descriptor();
    descriptor.id = DescriptorId::new("rgbgain", "rusttable.rgb_gain", 1, 1, 1).expect("static ID");
    descriptor.parameters = ["red", "green", "blue"]
        .into_iter()
        .map(|id| ParameterDescriptor {
            id: id.to_owned(),
            kind: ParameterKind::Scalar {
                minimum: 0.0,
                maximum: 64.0,
            },
            default: ParameterDefault::Scalar(1.0),
            required: true,
            introduced_version: 1,
            removed_version: None,
            unit: None,
            step: Some(0.001),
            precision: 3,
            role: ParameterRole::Color,
            cache_affecting: true,
            animatable: true,
            ui_hint: Some("slider".to_owned()),
            condition: None,
        })
        .collect();
    descriptor.ui = Some(UiHint {
        label_key: "operation.rgb_gain".to_owned(),
        group_key: "group.color".to_owned(),
        control: "triplet".to_owned(),
    });
    descriptor.flags = descriptor.flags.insert(OperationFlags::COLOR);
    descriptor
}

#[must_use]
///
/// # Panics
///
/// This function cannot panic because its fixed descriptor identity is valid.
#[expect(
    clippy::too_many_lines,
    reason = "the source-ordered temperature parameters and migration metadata stay together"
)]
pub fn temperature_descriptor() -> OperationDescriptor {
    let mut descriptor = base_exposure_descriptor();
    descriptor.id =
        DescriptorId::new("temperature", "rusttable.temperature", 1, 4, 1).expect("static ID");
    descriptor.parameters = vec![
        temperature_scalar_parameter("red", 0.0, 8.0, 1.0, "slider", ParameterRole::Color, 1),
        temperature_scalar_parameter("green", 0.0, 8.0, 1.0, "slider", ParameterRole::Color, 1),
        temperature_scalar_parameter("blue", 0.0, 8.0, 1.0, "slider", ParameterRole::Color, 1),
        temperature_scalar_parameter("various", 0.0, 8.0, 1.0, "slider", ParameterRole::Color, 1),
        ParameterDescriptor {
            id: "preset".to_owned(),
            kind: ParameterKind::Integer {
                minimum: -1,
                maximum: 1024,
            },
            default: ParameterDefault::Integer(0),
            required: true,
            introduced_version: 4,
            removed_version: None,
            unit: None,
            step: Some(1.0),
            precision: 0,
            role: ParameterRole::Color,
            cache_affecting: true,
            animatable: false,
            ui_hint: Some("preset".to_owned()),
            condition: None,
        },
        ParameterDescriptor {
            id: "source".to_owned(),
            kind: ParameterKind::Enum {
                tags: vec![
                    "camera_reference".to_owned(),
                    "as_shot".to_owned(),
                    "daylight_reference".to_owned(),
                    "preset".to_owned(),
                    "temperature_tint".to_owned(),
                    "spot".to_owned(),
                    "custom".to_owned(),
                ],
            },
            default: ParameterDefault::Enum("as_shot".to_owned()),
            required: false,
            introduced_version: 4,
            removed_version: None,
            unit: None,
            step: None,
            precision: 0,
            role: ParameterRole::Color,
            cache_affecting: true,
            animatable: false,
            ui_hint: Some("combo".to_owned()),
            condition: None,
        },
        temperature_scalar_parameter(
            "temperature",
            1901.0,
            25000.0,
            4000.0,
            "slider",
            ParameterRole::Presentation,
            4,
        ),
        temperature_scalar_parameter(
            "tint",
            0.135,
            2.326,
            1.0,
            "slider",
            ParameterRole::Presentation,
            4,
        ),
        ParameterDescriptor {
            id: "stage".to_owned(),
            kind: ParameterKind::Enum {
                tags: vec!["pre_demosaic".to_owned(), "post_demosaic".to_owned()],
            },
            default: ParameterDefault::Enum("pre_demosaic".to_owned()),
            required: false,
            introduced_version: 4,
            removed_version: None,
            unit: None,
            step: None,
            precision: 0,
            role: ParameterRole::Processing,
            cache_affecting: true,
            animatable: false,
            ui_hint: Some("combo".to_owned()),
            condition: None,
        },
        text_parameter("camera_alias", 512, 4),
        text_parameter("preset_id", 256, 4),
        integer_parameter("tuning", -32768, 32767, 0, 4),
        integer_parameter("source_table_revision", 0, i64::MAX, 0, 4),
    ];
    descriptor.flags = OperationFlags::DETERMINISTIC_CPU
        .insert(OperationFlags::TILEABLE)
        .insert(OperationFlags::COLOR)
        .insert(OperationFlags::HISTORY_VISIBLE);
    "raw-scene-linear".clone_into(&mut descriptor.stage);
    descriptor.ui = Some(UiHint {
        label_key: "operation.temperature".to_owned(),
        group_key: "group.basic".to_owned(),
        control: "white-balance".to_owned(),
    });
    descriptor.migration = MigrationContract {
        source_versions: vec![1, 2, 3, 4],
        target_version: 4,
        opaque_unknown_allowed: true,
    };
    descriptor
}

fn temperature_scalar_parameter(
    id: &str,
    minimum: f64,
    maximum: f64,
    default: f64,
    ui_hint: &str,
    role: ParameterRole,
    introduced_version: u16,
) -> ParameterDescriptor {
    ParameterDescriptor {
        id: id.to_owned(),
        kind: ParameterKind::Scalar { minimum, maximum },
        default: ParameterDefault::Scalar(default),
        required: true,
        introduced_version,
        removed_version: None,
        unit: None,
        step: Some(0.001),
        precision: 3,
        role,
        cache_affecting: true,
        animatable: false,
        ui_hint: Some(ui_hint.to_owned()),
        condition: None,
    }
}

fn integer_parameter(
    id: &str,
    minimum: i64,
    maximum: i64,
    default: i64,
    introduced_version: u16,
) -> ParameterDescriptor {
    ParameterDescriptor {
        id: id.to_owned(),
        kind: ParameterKind::Integer { minimum, maximum },
        default: ParameterDefault::Integer(default),
        required: false,
        introduced_version,
        removed_version: None,
        unit: None,
        step: Some(1.0),
        precision: 0,
        role: ParameterRole::Color,
        cache_affecting: true,
        animatable: false,
        ui_hint: None,
        condition: None,
    }
}

fn text_parameter(id: &str, maximum_bytes: u16, introduced_version: u16) -> ParameterDescriptor {
    ParameterDescriptor {
        id: id.to_owned(),
        kind: ParameterKind::Text { maximum_bytes },
        default: ParameterDefault::Text(String::new()),
        required: false,
        introduced_version,
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

#[must_use]
///
/// # Panics
///
/// This function cannot panic because its fixed descriptor identity is valid.
pub fn linear_offset_descriptor() -> OperationDescriptor {
    let mut descriptor = base_exposure_descriptor();
    descriptor.id =
        DescriptorId::new("linear-offset", "rusttable.linear_offset", 1, 1, 1).expect("static ID");
    descriptor.parameters = vec![ParameterDescriptor {
        id: "value".to_owned(),
        kind: ParameterKind::Scalar {
            minimum: -64.0,
            maximum: 64.0,
        },
        default: ParameterDefault::Scalar(0.0),
        required: true,
        introduced_version: 1,
        removed_version: None,
        unit: None,
        step: Some(0.001),
        precision: 3,
        role: ParameterRole::Processing,
        cache_affecting: true,
        animatable: true,
        ui_hint: Some("slider".to_owned()),
        condition: None,
    }];
    descriptor.ui = Some(UiHint {
        label_key: "operation.linear_offset".to_owned(),
        group_key: "group.basic".to_owned(),
        control: "slider".to_owned(),
    });
    descriptor
}

/// Builds the source-ordered Highlights descriptor.
///
/// # Panics
///
/// Panics only if the fixed native-compatible descriptor identity is invalid.
#[must_use]
pub fn highlights_descriptor() -> OperationDescriptor {
    let mut descriptor = base_exposure_descriptor();
    descriptor.id =
        DescriptorId::new("highlights", "rusttable.highlights", 4, 4, 1).expect("static ID");
    descriptor.parameters = vec![
        scalar_parameter("method", 0.0, 5.0, 5.0, ParameterRole::Processing),
        scalar_parameter("blend_l", 0.0, 2.0, 1.0, ParameterRole::Color),
        scalar_parameter("blend_c", 0.0, 2.0, 0.0, ParameterRole::Color),
        scalar_parameter("strength", 0.0, 1.0, 0.0, ParameterRole::Processing),
        scalar_parameter("clip", 0.0, 2.0, 1.0, ParameterRole::Processing),
        scalar_parameter("noise_level", 0.0, 0.5, 0.0, ParameterRole::Processing),
        scalar_parameter("iterations", 1.0, 256.0, 30.0, ParameterRole::Processing),
        scalar_parameter("scales", 0.0, 11.0, 6.0, ParameterRole::Geometry),
        scalar_parameter("candidating", 0.0, 1.0, 0.4, ParameterRole::Mask),
        scalar_parameter("combine", 0.0, 8.0, 2.0, ParameterRole::Mask),
        scalar_parameter("recovery", 0.0, 6.0, 0.0, ParameterRole::Processing),
        scalar_parameter("solid_color", 0.0, 1.0, 0.0, ParameterRole::Color),
    ];
    descriptor.id.schema_version = 4;
    descriptor.id.parameter_version = 4;
    descriptor.flags = OperationFlags::DETERMINISTIC_CPU
        .insert(OperationFlags::DETERMINISTIC_GPU)
        .insert(OperationFlags::FULL_IMAGE)
        .insert(OperationFlags::COLOR)
        .insert(OperationFlags::MASKS)
        .insert(OperationFlags::BLENDING)
        .insert(OperationFlags::ANALYSIS);
    "raw-highlight-reconstruction".clone_into(&mut descriptor.stage);
    descriptor.roi = RoiKind::FullImage;
    descriptor.tiling.overlap_pixels = 2048;
    descriptor.tiling.preferred_tile_edge = 1024;
    descriptor.capability = reconstruction_capability();
    descriptor.io = default_io_contract();
    descriptor.mask_blend = MaskBlendContract {
        consumes_mask: false,
        publishes_mask: true,
        blend_if: true,
        geometry: false,
        analysis: true,
    };
    descriptor.migration = MigrationContract {
        source_versions: vec![1, 2, 3, 4],
        target_version: 4,
        opaque_unknown_allowed: true,
    };
    descriptor.ui = Some(UiHint {
        label_key: "operation.highlights".to_owned(),
        group_key: "group.basic".to_owned(),
        control: "highlights-reconstruction".to_owned(),
    });
    descriptor
}

/// Builds the source-ordered Color Reconstruction descriptor.
///
/// # Panics
///
/// Panics only if the fixed native-compatible descriptor identity is invalid.
#[must_use]
pub fn color_reconstruction_descriptor() -> OperationDescriptor {
    let mut descriptor = base_exposure_descriptor();
    descriptor.id = DescriptorId::new("colorreconstruct", "rusttable.colorreconstruct", 3, 3, 1)
        .expect("static ID");
    descriptor.parameters = vec![
        scalar_parameter("threshold", 50.0, 150.0, 100.0, ParameterRole::Mask),
        scalar_parameter("spatial", 0.0, 1000.0, 400.0, ParameterRole::Geometry),
        scalar_parameter("range", 0.0, 50.0, 10.0, ParameterRole::Color),
        scalar_parameter("hue", 0.0, 1.0, 0.66, ParameterRole::Color),
        scalar_parameter("precedence", 0.0, 2.0, 0.0, ParameterRole::Color),
    ];
    // The native module opts into the shared blend UI, but this Rust slice does
    // not yet own that shared payload or drawn-mask execution. Keep those
    // capabilities out of the descriptor so generic projections cannot expose
    // unsupported controls. The reconstruction's own full-image analysis is
    // still represented independently below.
    descriptor.flags = OperationFlags::MULTI_INSTANCE
        .insert(OperationFlags::STYLE_ELIGIBLE)
        .insert(OperationFlags::DETERMINISTIC_CPU)
        .insert(OperationFlags::FULL_IMAGE)
        .insert(OperationFlags::COLOR)
        .insert(OperationFlags::ANALYSIS);
    "post-demosaic-color-reconstruction".clone_into(&mut descriptor.stage);
    descriptor.roi = RoiKind::FullImage;
    descriptor.tiling.overlap_pixels = 1000;
    descriptor.tiling.preferred_tile_edge = 1024;
    descriptor.capability = reconstruction_capability();
    descriptor.io = color_reconstruction_io();
    descriptor.mask_blend = MaskBlendContract {
        // Shared blend payloads and drawn masks remain deferred for this slice;
        // the operation only consumes its own full-image analysis state.
        consumes_mask: false,
        publishes_mask: false,
        blend_if: false,
        geometry: false,
        analysis: true,
    };
    descriptor.migration = MigrationContract {
        source_versions: vec![1, 2, 3],
        target_version: 3,
        opaque_unknown_allowed: true,
    };
    descriptor.ui = Some(UiHint {
        label_key: "operation.colorreconstruct".to_owned(),
        group_key: "group.basic".to_owned(),
        control: "color-reconstruction".to_owned(),
    });
    descriptor
}

fn scalar_parameter(
    id: &str,
    minimum: f64,
    maximum: f64,
    default: f64,
    role: ParameterRole,
) -> ParameterDescriptor {
    ParameterDescriptor {
        id: id.to_owned(),
        kind: ParameterKind::Scalar { minimum, maximum },
        default: ParameterDefault::Scalar(default),
        required: false,
        introduced_version: 1,
        removed_version: None,
        unit: None,
        step: Some(0.001),
        precision: 3,
        role,
        cache_affecting: true,
        animatable: true,
        ui_hint: Some("slider".to_owned()),
        condition: None,
    }
}

fn reconstruction_capability() -> CapabilityContract {
    CapabilityContract {
        cpu_supported: true,
        gpu_tier: Some(1),
        required_features: vec!["f32-storage".to_owned()],
        required_formats: vec!["rgba32float".to_owned()],
        deterministic_cpu: true,
        deterministic_gpu: false,
        fallback_to_cpu: true,
        precision: "f32".to_owned(),
        modes: vec!["preview".to_owned(), "full".to_owned(), "export".to_owned()],
    }
}

fn color_reconstruction_io() -> InputOutputContract {
    let image = ImagePredicate {
        channels: 3,
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

/// Descriptor for the Lab D50 Highpass neighborhood path.
#[must_use]
#[expect(
    clippy::missing_panics_doc,
    reason = "the static descriptor identity is validated at construction"
)]
pub fn highpass_descriptor() -> OperationDescriptor {
    let image = lab_image_predicate();
    OperationDescriptor {
        id: DescriptorId::new("highpass", "rusttable.highpass", 1, 1, 1).expect("static ID"),
        parameters: vec![
            scalar_processing_parameter("sharpness", 0.0, 100.0, 50.0, "percent"),
            scalar_processing_parameter("contrast", 0.0, 100.0, 50.0, "percent"),
        ],
        flags: OperationFlags::HISTORY_VISIBLE
            .insert(OperationFlags::STYLE_ELIGIBLE)
            .insert(OperationFlags::TILEABLE)
            .insert(OperationFlags::DETERMINISTIC_CPU)
            .insert(OperationFlags::COLOR)
            .insert(OperationFlags::BLENDING),
        stage: "frequential-lab-d50".to_owned(),
        roi: RoiKind::Neighborhood,
        tiling: TilingContract {
            overlap_pixels: 0,
            alignment_pixels: 1,
            minimum_tile_edge: 1,
            preferred_tile_edge: 256,
            temporary_multiplier_milli: 2_100,
            input_multiplier_milli: 1_000,
            output_multiplier_milli: 1_000,
        },
        capability: CapabilityContract {
            cpu_supported: true,
            gpu_tier: None,
            required_features: Vec::new(),
            required_formats: vec!["rgba32float".to_owned()],
            deterministic_cpu: true,
            deterministic_gpu: false,
            fallback_to_cpu: true,
            precision: "native scalar f32 Lab D50 box-filter".to_owned(),
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
            source_versions: vec![1],
            target_version: 1,
            opaque_unknown_allowed: true,
        },
        ui: None,
    }
}

/// Descriptor for the Lab D50 Tone Curve path.
#[must_use]
#[expect(
    clippy::missing_panics_doc,
    reason = "the static descriptor identity is validated at construction"
)]
pub fn tonecurve_descriptor() -> OperationDescriptor {
    let defaults = crate::operations::common::encode_native_payload_chunks(
        &crate::operations::tonecurve::ToneCurveParametersV5::default().to_bytes(),
    );
    let parameters = defaults
        .into_iter()
        .enumerate()
        .map(|(index, default)| native_payload_parameter(index, default, 5))
        .collect::<Vec<_>>();
    let image = lab_image_predicate();
    OperationDescriptor {
        id: DescriptorId::new("tonecurve", "rusttable.tonecurve", 5, 5, 1).expect("static ID"),
        parameters,
        flags: OperationFlags::HISTORY_VISIBLE
            .insert(OperationFlags::STYLE_ELIGIBLE)
            .insert(OperationFlags::TILEABLE)
            .insert(OperationFlags::DETERMINISTIC_CPU)
            .insert(OperationFlags::COLOR)
            .insert(OperationFlags::BLENDING),
        stage: "frequential-lab-d50".to_owned(),
        roi: RoiKind::Identity,
        tiling: TilingContract {
            overlap_pixels: 0,
            alignment_pixels: 1,
            minimum_tile_edge: 1,
            preferred_tile_edge: 256,
            temporary_multiplier_milli: 1_000,
            input_multiplier_milli: 1_000,
            output_multiplier_milli: 1_000,
        },
        capability: CapabilityContract {
            cpu_supported: true,
            gpu_tier: None,
            required_features: Vec::new(),
            required_formats: vec!["rgba32float".to_owned()],
            deterministic_cpu: true,
            deterministic_gpu: false,
            fallback_to_cpu: true,
            precision: "native scalar f32 Lab D50 LUT".to_owned(),
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
            source_versions: vec![1, 3, 4, 5],
            target_version: 5,
            opaque_unknown_allowed: true,
        },
        ui: None,
    }
}

/// Descriptor for Darktable's deprecated Lab D50 Colisa path.
#[must_use]
#[expect(
    clippy::missing_panics_doc,
    reason = "the static descriptor identity is validated at construction"
)]
pub fn colisa_descriptor() -> OperationDescriptor {
    let image = lab_image_predicate();
    OperationDescriptor {
        id: DescriptorId::new("colisa", "rusttable.colisa", 1, 1, 1).expect("static ID"),
        parameters: vec![
            scalar_processing_parameter("contrast", -1.0, 1.0, 0.0, "normalized"),
            scalar_processing_parameter("brightness", -1.0, 1.0, 0.0, "normalized"),
            scalar_processing_parameter("saturation", -1.0, 1.0, 0.0, "normalized"),
        ],
        flags: OperationFlags::DEPRECATED
            .insert(OperationFlags::HISTORY_VISIBLE)
            .insert(OperationFlags::STYLE_ELIGIBLE)
            .insert(OperationFlags::TILEABLE)
            .insert(OperationFlags::DETERMINISTIC_CPU)
            .insert(OperationFlags::COLOR)
            .insert(OperationFlags::BLENDING),
        stage: "frequential-lab-d50".to_owned(),
        roi: RoiKind::Identity,
        tiling: TilingContract {
            overlap_pixels: 0,
            alignment_pixels: 1,
            minimum_tile_edge: 1,
            preferred_tile_edge: 256,
            temporary_multiplier_milli: 1_000,
            input_multiplier_milli: 1_000,
            output_multiplier_milli: 1_000,
        },
        capability: CapabilityContract {
            cpu_supported: true,
            gpu_tier: None,
            required_features: Vec::new(),
            required_formats: vec!["rgba32float".to_owned()],
            deterministic_cpu: true,
            deterministic_gpu: false,
            fallback_to_cpu: true,
            precision: "native scalar f32 Lab D50 LUT".to_owned(),
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
            source_versions: vec![1],
            target_version: 1,
            opaque_unknown_allowed: true,
        },
        ui: None,
    }
}

/// Descriptor for the bounded tileable linear-RGB Base Curve path.
#[must_use]
#[expect(
    clippy::missing_panics_doc,
    reason = "the static descriptor identity is validated at construction"
)]
pub fn basecurve_descriptor() -> OperationDescriptor {
    let default = crate::operations::common::encode_native_payload_chunks(
        &crate::operations::basecurve::BasecurveParameters::defaults().to_bytes(),
    );
    let parameters = default
        .into_iter()
        .enumerate()
        .map(|(index, value)| native_payload_parameter(index, value, 6))
        .collect::<Vec<_>>();
    let image = linear_rgb_image_predicate();
    OperationDescriptor {
        id: DescriptorId::new("basecurve", "rusttable.basecurve", 6, 6, 1).expect("static ID"),
        parameters,
        flags: OperationFlags::HISTORY_VISIBLE
            .insert(OperationFlags::STYLE_ELIGIBLE)
            .insert(OperationFlags::TILEABLE)
            .insert(OperationFlags::DETERMINISTIC_CPU)
            .insert(OperationFlags::COLOR),
        stage: "scene-linear-rgb".to_owned(),
        roi: RoiKind::Identity,
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
            required_features: Vec::new(),
            required_formats: vec!["rgba32float".to_owned()],
            deterministic_cpu: true,
            deterministic_gpu: false,
            fallback_to_cpu: true,
            precision: "native scalar f32 linear RGB LUT".to_owned(),
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
            source_versions: vec![1, 2, 3, 4, 5, 6],
            target_version: 6,
            opaque_unknown_allowed: true,
        },
        ui: None,
    }
}

fn scalar_processing_parameter(
    id: &str,
    minimum: f64,
    maximum: f64,
    default: f64,
    unit: &str,
) -> ParameterDescriptor {
    ParameterDescriptor {
        id: id.to_owned(),
        kind: ParameterKind::Scalar { minimum, maximum },
        default: ParameterDefault::Scalar(default),
        required: false,
        introduced_version: 1,
        removed_version: None,
        unit: Some(unit.to_owned()),
        step: Some(0.1),
        precision: 1,
        role: ParameterRole::Processing,
        cache_affecting: true,
        animatable: true,
        ui_hint: Some("slider".to_owned()),
        condition: None,
    }
}

fn native_payload_parameter(index: usize, default: String, version: u16) -> ParameterDescriptor {
    ParameterDescriptor {
        id: format!("payload_{index}"),
        kind: ParameterKind::Text {
            maximum_bytes: 4_096,
        },
        default: ParameterDefault::Text(default),
        required: false,
        introduced_version: version,
        removed_version: None,
        unit: None,
        step: None,
        precision: 0,
        role: ParameterRole::Processing,
        cache_affecting: true,
        animatable: false,
        ui_hint: None,
        condition: None,
    }
}

fn lab_image_predicate() -> ImagePredicate {
    ImagePredicate {
        channels: 4,
        alpha: AlphaPolicy::Preserve,
        encodings: vec![ColorEncoding::LabD50],
        nonfinite: NonFinitePolicy::Reject,
    }
}

fn linear_rgb_image_predicate() -> ImagePredicate {
    ImagePredicate {
        channels: 4,
        alpha: AlphaPolicy::Preserve,
        encodings: vec![ColorEncoding::LinearSrgbD65],
        nonfinite: NonFinitePolicy::Reject,
    }
}

pub fn default_io_contract() -> InputOutputContract {
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

pub const fn default_mask_blend() -> MaskBlendContract {
    MaskBlendContract {
        consumes_mask: false,
        publishes_mask: false,
        blend_if: false,
        geometry: false,
        analysis: false,
    }
}
