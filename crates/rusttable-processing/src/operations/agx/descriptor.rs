use rusttable_color::ColorEncoding;

use crate::descriptor::{
    AlphaPolicy, CapabilityContract, DescriptorId, ImagePredicate, InputOutputContract,
    MaskBlendContract, MigrationContract, NonFinitePolicy, OperationDescriptor, OperationFlags,
    ParameterDefault, ParameterDescriptor, ParameterKind, ParameterRole, RoiKind, TilingContract,
};

use super::{AGX_COMPATIBILITY_ID, AGX_RUST_ID, AGX_SCHEMA_VERSION, AgxParametersV7};

#[allow(
    clippy::approx_constant,
    reason = "the native GTK rotation range is the exact decimal value 0.5236"
)]
const AGX_ROTATION_BOUND: f64 = 0.5236;

/// Source-derived processing contract for Darktable AgX v7.
///
/// Built-in matrix-shaper profiles are executable on CPU. Configured external
/// working/export profile transforms, masks, outer blending, GPU, and GTK remain
/// explicit deferred capability surfaces.
#[must_use]
pub fn agx_descriptor() -> OperationDescriptor {
    let defaults = AgxParametersV7::defaults();
    OperationDescriptor {
        id: DescriptorId::new(
            AGX_COMPATIBILITY_ID,
            AGX_RUST_ID,
            AGX_SCHEMA_VERSION,
            AGX_SCHEMA_VERSION,
            1,
        )
        .expect("static AgX ID"),
        parameters: vec![
            scalar("look_lift", -1.0, 1.0, defaults.look_lift),
            scalar("look_slope", 0.0, 10.0, defaults.look_slope),
            scalar("look_brightness", 0.0, 100.0, defaults.look_brightness),
            scalar("look_saturation", 0.0, 10.0, defaults.look_saturation),
            scalar(
                "look_original_hue_mix_ratio",
                0.0,
                1.0,
                defaults.look_original_hue_mix_ratio,
            ),
            scalar(
                "range_black_relative_ev",
                -20.0,
                -0.1,
                defaults.range_black_relative_ev,
            ),
            scalar(
                "range_white_relative_ev",
                0.1,
                20.0,
                defaults.range_white_relative_ev,
            ),
            scalar(
                "dynamic_range_scaling",
                -0.5,
                2.0,
                defaults.dynamic_range_scaling,
            ),
            scalar("curve_pivot_x", 0.0, 1.0, defaults.curve_pivot_x),
            scalar(
                "curve_pivot_y_linear_output",
                0.0,
                1.0,
                defaults.curve_pivot_y_linear_output,
            ),
            scalar(
                "curve_contrast_around_pivot",
                0.1,
                10.0,
                defaults.curve_contrast_around_pivot,
            ),
            scalar(
                "curve_linear_ratio_below_pivot",
                0.0,
                1.0,
                defaults.curve_linear_ratio_below_pivot,
            ),
            scalar(
                "curve_linear_ratio_above_pivot",
                0.0,
                1.0,
                defaults.curve_linear_ratio_above_pivot,
            ),
            scalar("curve_toe_power", 0.0, 10.0, defaults.curve_toe_power),
            scalar(
                "curve_shoulder_power",
                0.0,
                10.0,
                defaults.curve_shoulder_power,
            ),
            scalar("curve_gamma", 0.01, 100.0, defaults.curve_gamma),
            boolean("auto_gamma", defaults.auto_gamma != 0),
            scalar(
                "curve_target_display_black_ratio",
                0.0,
                0.15,
                defaults.curve_target_display_black_ratio,
            ),
            scalar(
                "curve_target_display_white_ratio",
                0.2,
                1.0,
                defaults.curve_target_display_white_ratio,
            ),
            enumeration(
                "base_primaries",
                &[
                    "export_profile",
                    "working_profile",
                    "rec2020",
                    "display_p3",
                    "adobe_rgb",
                    "srgb",
                ],
                defaults.base_primaries as usize,
            ),
            boolean(
                "disable_primaries_adjustments",
                defaults.disable_primaries_adjustments != 0,
            ),
            scalar("red_inset", 0.0, 0.99, defaults.red_inset),
            scalar(
                "red_rotation",
                -AGX_ROTATION_BOUND,
                AGX_ROTATION_BOUND,
                defaults.red_rotation,
            ),
            scalar("green_inset", 0.0, 0.99, defaults.green_inset),
            scalar(
                "green_rotation",
                -AGX_ROTATION_BOUND,
                AGX_ROTATION_BOUND,
                defaults.green_rotation,
            ),
            scalar("blue_inset", 0.0, 0.99, defaults.blue_inset),
            scalar(
                "blue_rotation",
                -AGX_ROTATION_BOUND,
                AGX_ROTATION_BOUND,
                defaults.blue_rotation,
            ),
            scalar(
                "master_outset_ratio",
                0.0,
                2.0,
                defaults.master_outset_ratio,
            ),
            scalar(
                "master_unrotation_ratio",
                0.0,
                2.0,
                defaults.master_unrotation_ratio,
            ),
            scalar("red_outset", 0.0, 0.99, defaults.red_outset),
            scalar(
                "red_unrotation",
                -AGX_ROTATION_BOUND,
                AGX_ROTATION_BOUND,
                defaults.red_unrotation,
            ),
            scalar("green_outset", 0.0, 0.99, defaults.green_outset),
            scalar(
                "green_unrotation",
                -AGX_ROTATION_BOUND,
                AGX_ROTATION_BOUND,
                defaults.green_unrotation,
            ),
            scalar("blue_outset", 0.0, 0.99, defaults.blue_outset),
            scalar(
                "blue_unrotation",
                -AGX_ROTATION_BOUND,
                AGX_ROTATION_BOUND,
                defaults.blue_unrotation,
            ),
            boolean(
                "completely_reverse_primaries",
                defaults.completely_reverse_primaries != 0,
            ),
        ],
        flags: OperationFlags::MULTI_INSTANCE
            .insert(OperationFlags::STYLE_ELIGIBLE)
            .insert(OperationFlags::HISTORY_VISIBLE)
            .insert(OperationFlags::TILEABLE)
            .insert(OperationFlags::DETERMINISTIC_CPU)
            .insert(OperationFlags::COLOR),
        stage: "scene-to-display-linear-rgb".to_owned(),
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
            required_features: vec!["builtin-d50-matrix-shaper-profile".to_owned()],
            required_formats: vec!["rgba32float".to_owned()],
            deterministic_cpu: true,
            deterministic_gpu: false,
            fallback_to_cpu: true,
            precision: "f32 AgX v7 native scalar equations".to_owned(),
            modes: vec!["preview".to_owned(), "full".to_owned(), "export".to_owned()],
        },
        io: rgb_io(),
        mask_blend: MaskBlendContract {
            consumes_mask: false,
            publishes_mask: false,
            blend_if: false,
            geometry: false,
            analysis: false,
        },
        migration: MigrationContract {
            source_versions: vec![AGX_SCHEMA_VERSION],
            target_version: AGX_SCHEMA_VERSION,
            opaque_unknown_allowed: false,
        },
        ui: None,
    }
}

fn scalar(id: &str, minimum: f64, maximum: f64, default: f32) -> ParameterDescriptor {
    ParameterDescriptor {
        id: id.to_owned(),
        kind: ParameterKind::Scalar { minimum, maximum },
        default: ParameterDefault::Scalar(f64::from(default)),
        required: false,
        introduced_version: AGX_SCHEMA_VERSION,
        removed_version: None,
        unit: None,
        step: Some(0.01),
        precision: 6,
        role: ParameterRole::Processing,
        cache_affecting: true,
        animatable: true,
        ui_hint: Some("slider".to_owned()),
        condition: None,
    }
}

fn boolean(id: &str, default: bool) -> ParameterDescriptor {
    ParameterDescriptor {
        id: id.to_owned(),
        kind: ParameterKind::Bool,
        default: ParameterDefault::Bool(default),
        required: false,
        introduced_version: AGX_SCHEMA_VERSION,
        removed_version: None,
        unit: None,
        step: None,
        precision: 0,
        role: ParameterRole::Processing,
        cache_affecting: true,
        animatable: false,
        ui_hint: Some("toggle".to_owned()),
        condition: None,
    }
}

fn enumeration(id: &str, tags: &[&str], default_index: usize) -> ParameterDescriptor {
    ParameterDescriptor {
        id: id.to_owned(),
        kind: ParameterKind::Enum {
            tags: tags.iter().map(|tag| (*tag).to_owned()).collect(),
        },
        default: ParameterDefault::Enum(tags[default_index].to_owned()),
        required: false,
        introduced_version: AGX_SCHEMA_VERSION,
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

fn rgb_io() -> InputOutputContract {
    let encodings = vec![
        ColorEncoding::LinearSrgbD65,
        ColorEncoding::LinearDisplayP3D65,
        ColorEncoding::LinearRec2020D65,
    ];
    let image = ImagePredicate {
        channels: 4,
        alpha: AlphaPolicy::Preserve,
        encodings,
        nonfinite: NonFinitePolicy::Reject,
    };
    InputOutputContract {
        input: image.clone(),
        output: image,
        derives_output_encoding: false,
    }
}
