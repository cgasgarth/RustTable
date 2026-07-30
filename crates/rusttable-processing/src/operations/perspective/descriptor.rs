use crate::descriptor::{
    AlphaPolicy, CapabilityContract, DescriptorId, ImagePredicate, InputOutputContract,
    MaskBlendContract, MigrationContract, NonFinitePolicy, OperationDescriptor, OperationFlags,
    ParameterDefault, ParameterDescriptor, ParameterKind, ParameterRole, RoiKind, TilingContract,
    UiHint,
};
use rusttable_color::ColorEncoding;

use super::codec::{
    ASHIFT_COMPATIBILITY_ID, ASHIFT_IMPLEMENTATION_VERSION, ASHIFT_MAX_DIMENSION,
    ASHIFT_MAX_SAVED_LINES, ASHIFT_PARAMETER_VERSION, ASHIFT_RUST_ID, ASHIFT_SCHEMA_VERSION,
};

pub const ASHIFT_DESCRIPTOR_LINE_COMPONENTS: usize = 4;
pub const ASHIFT_DESCRIPTOR_LINE_PARAMETER_COUNT: usize =
    ASHIFT_MAX_SAVED_LINES * ASHIFT_DESCRIPTOR_LINE_COMPONENTS;
pub const ASHIFT_DESCRIPTOR_PARAMETER_COUNT: usize = 17 + ASHIFT_DESCRIPTOR_LINE_PARAMETER_COUNT;

/// Returns the descriptor for the checked `ashift` perspective operation.
///
/// # Panics
///
/// Panics only if the fixed operation identifiers violate the descriptor key contract.
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn perspective_descriptor() -> OperationDescriptor {
    let mut parameters = vec![
        scalar("rotation", -180.0, 180.0, 0.0, "degree", 2),
        scalar("lensshift_v", -2.0, 2.0, 0.0, "ratio", 3),
        scalar("lensshift_h", -2.0, 2.0, 0.0, "ratio", 3),
        scalar("shear", -0.5, 0.5, 0.0, "ratio", 4),
        scalar("focal_length", 1.0, 2000.0, 28.0, "millimeter", 2),
        scalar("crop_factor", 0.5, 10.0, 1.0, "ratio", 3),
        scalar("orthocorr", 0.0, 100.0, 100.0, "percent", 2),
        scalar("aspect", 0.5, 2.0, 1.0, "ratio", 3),
        enum_parameter("lens_model", &["generic", "specific"], "generic"),
        enum_parameter("crop_mode", &["off", "largest", "aspect"], "largest"),
        scalar("crop_left", 0.0, 1.0, 0.0, "ratio", 4),
        scalar("crop_right", 0.0, 1.0, 1.0, "ratio", 4),
        scalar("crop_top", 0.0, 1.0, 0.0, "ratio", 4),
        scalar("crop_bottom", 0.0, 1.0, 1.0, "ratio", 4),
        enum_parameter("method", &["none", "automatic", "quad", "lines"], "none"),
        integer("fit_axis", 0, 63, 63, "bitset"),
        ParameterDescriptor {
            id: "last_quad".to_owned(),
            kind: ParameterKind::Matrix {
                rows: 4,
                columns: 2,
                minimum: -f64::from(ASHIFT_MAX_DIMENSION),
                maximum: f64::from(ASHIFT_MAX_DIMENSION),
            },
            default: ParameterDefault::Matrix(vec![0.0; 8]),
            required: true,
            introduced_version: ASHIFT_PARAMETER_VERSION,
            removed_version: None,
            unit: Some("pixel".to_owned()),
            step: Some(0.01),
            precision: 2,
            role: ParameterRole::Geometry,
            cache_affecting: true,
            animatable: false,
            ui_hint: None,
            condition: None,
        },
    ];
    for index in 0..ASHIFT_MAX_SAVED_LINES {
        for component in 0..ASHIFT_DESCRIPTOR_LINE_COMPONENTS {
            parameters.push(scalar(
                &format!("last_drawn_line_{index}_{component}"),
                -f64::from(ASHIFT_MAX_DIMENSION),
                f64::from(ASHIFT_MAX_DIMENSION),
                0.0,
                "pixel",
                2,
            ));
        }
    }
    debug_assert_eq!(parameters.len(), ASHIFT_DESCRIPTOR_PARAMETER_COUNT);

    let image = ImagePredicate {
        channels: 3,
        alpha: AlphaPolicy::Preserve,
        encodings: vec![ColorEncoding::LinearSrgbD65],
        nonfinite: NonFinitePolicy::Reject,
    };
    OperationDescriptor {
        id: DescriptorId::new(
            ASHIFT_COMPATIBILITY_ID,
            ASHIFT_RUST_ID,
            ASHIFT_SCHEMA_VERSION,
            ASHIFT_PARAMETER_VERSION,
            ASHIFT_IMPLEMENTATION_VERSION,
        )
        .expect("static ashift descriptor ID"),
        parameters,
        flags: OperationFlags::HISTORY_VISIBLE
            .insert(OperationFlags::FULL_IMAGE)
            .insert(OperationFlags::DETERMINISTIC_CPU)
            .insert(OperationFlags::GEOMETRY)
            .insert(OperationFlags::MASKS)
            .insert(OperationFlags::ANALYSIS),
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
            gpu_tier: None,
            required_features: Vec::new(),
            required_formats: Vec::new(),
            deterministic_cpu: true,
            deterministic_gpu: false,
            fallback_to_cpu: true,
            precision: "f64 checked homography planning with f32 scalar samples".to_owned(),
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
            analysis: true,
        },
        migration: MigrationContract {
            source_versions: vec![1, 2, 3, 4, 5],
            target_version: ASHIFT_PARAMETER_VERSION,
            opaque_unknown_allowed: true,
        },
        ui: Some(UiHint {
            label_key: "operation.ashift".to_owned(),
            group_key: "group.corrective".to_owned(),
            control: "method".to_owned(),
        }),
    }
}

fn scalar(
    id: &str,
    minimum: f64,
    maximum: f64,
    default: f64,
    unit: &str,
    precision: u8,
) -> ParameterDescriptor {
    ParameterDescriptor {
        id: id.to_owned(),
        kind: ParameterKind::Scalar { minimum, maximum },
        default: ParameterDefault::Scalar(default),
        required: true,
        introduced_version: ASHIFT_PARAMETER_VERSION,
        removed_version: None,
        unit: Some(unit.to_owned()),
        step: Some(10_f64.powi(-i32::from(precision))),
        precision,
        role: ParameterRole::Geometry,
        cache_affecting: true,
        animatable: false,
        ui_hint: None,
        condition: None,
    }
}

fn integer(id: &str, minimum: i64, maximum: i64, default: i64, unit: &str) -> ParameterDescriptor {
    ParameterDescriptor {
        id: id.to_owned(),
        kind: ParameterKind::Integer { minimum, maximum },
        default: ParameterDefault::Integer(default),
        required: true,
        introduced_version: ASHIFT_PARAMETER_VERSION,
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

fn enum_parameter(id: &str, tags: &[&str], default: &str) -> ParameterDescriptor {
    ParameterDescriptor {
        id: id.to_owned(),
        kind: ParameterKind::Enum {
            tags: tags.iter().map(|tag| (*tag).to_owned()).collect(),
        },
        default: ParameterDefault::Enum(default.to_owned()),
        required: true,
        introduced_version: ASHIFT_PARAMETER_VERSION,
        removed_version: None,
        unit: None,
        step: None,
        precision: 0,
        role: ParameterRole::Geometry,
        cache_affecting: true,
        animatable: false,
        ui_hint: None,
        condition: None,
    }
}
