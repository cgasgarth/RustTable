//! Bounded AgX CPU leaf ported from `src/iop/agx.c`.
//!
//! This module owns the native v7 parameter ABI and its pre-v7 migration,
//! source-shaped AgX tone curve and look equations, custom-primaries matrix
//! path, input sanitisation, alpha handling, finite publication checks, and a
//! cancellation-aware CPU raster boundary. Registry, typed history import,
//! built-in D50 working-profile selection, evaluator, and pixelpipe CPU routing
//! are integrated. Configured external profile transforms, presets, GTK,
//! masks/outer blending, and GPU/OpenCL remain explicitly deferred.

#![forbid(unsafe_code)]
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::doc_markdown,
    clippy::assign_op_pattern,
    clippy::excessive_precision,
    clippy::float_cmp,
    clippy::if_same_then_else,
    clippy::manual_clamp,
    clippy::manual_range_contains,
    clippy::many_single_char_names,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::needless_range_loop,
    clippy::similar_names,
    clippy::struct_excessive_bools,
    clippy::too_many_arguments,
    clippy::too_many_lines,
    clippy::unreadable_literal,
    dead_code,
    reason = "native ABI and scalar raster expressions retain source-shaped f32 boundaries"
)]

use std::fmt;
use std::hash::{Hash, Hasher};
use std::mem::size_of;

use crate::RasterDimensions;

mod descriptor;
pub mod source_map;

pub use descriptor::agx_descriptor;

pub const AGX_COMPATIBILITY_ID: &str = "agx";
pub const AGX_RUST_ID: &str = "rusttable.agx";
pub const AGX_SCHEMA_VERSION: u16 = 7;
pub const AGX_PARAMETER_BYTES_V7: usize = 144;
pub const AGX_PARAMETER_LAYOUT_HASH: &str =
    "3aff44c0fabc743303ac96a0b818ba65c964555a41c89b9cfbf164748905443a";
pub const AGX_DEFAULT_COLORSPACE: &str = "RGB";
pub const AGX_DEFAULT_GROUPS: [&str; 2] = ["tone", "technical"];
pub const AGX_SUPPORTS_BLENDING: bool = true;
pub const AGX_GPU_PROGRAM: u32 = 39;
pub const AGX_GPU_KERNELS: [&str; 1] = ["kernel_agx"];
pub const AGX_GPU_EXECUTABLE: bool = false;

/// There are no released parameter revisions before v7.  Native `legacy_params`
/// intentionally maps every version below v7 directly to the current
/// scene-referred defaults.
pub const AGX_MIGRATION_EDGES: &[(u16, u16)] = &[];

const EPSILON: f32 = 1e-6;
const DEFAULT_GAMMA: f32 = 2.2;
const MATRIX_EPSILON: f32 = 1e-7;
const INPUT_LIMIT: f32 = 1e6;
const MIDDLE_GREY: f32 = 0.18;

pub const AGX_PARAMETER_FIELD_ORDER: &str = "look_lift,look_slope,look_brightness,look_saturation,look_original_hue_mix_ratio,range_black_relative_ev,range_white_relative_ev,dynamic_range_scaling,curve_pivot_x,curve_pivot_y_linear_output,curve_contrast_around_pivot,curve_linear_ratio_below_pivot,curve_linear_ratio_above_pivot,curve_toe_power,curve_shoulder_power,curve_gamma,auto_gamma,curve_target_display_black_ratio,curve_target_display_white_ratio,base_primaries,disable_primaries_adjustments,red_inset,red_rotation,green_inset,green_rotation,blue_inset,blue_rotation,master_outset_ratio,master_unrotation_ratio,red_outset,red_unrotation,green_outset,green_unrotation,blue_outset,blue_unrotation,completely_reverse_primaries";

/// Native `dt_iop_agx_base_primaries_t` values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum AgxBasePrimaries {
    ExportProfile = 0,
    WorkingProfile = 1,
    Rec2020 = 2,
    DisplayP3 = 3,
    AdobeRgb = 4,
    Srgb = 5,
}

impl TryFrom<i32> for AgxBasePrimaries {
    type Error = AgxCodecError;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::ExportProfile),
            1 => Ok(Self::WorkingProfile),
            2 => Ok(Self::Rec2020),
            3 => Ok(Self::DisplayP3),
            4 => Ok(Self::AdobeRgb),
            5 => Ok(Self::Srgb),
            other => Err(AgxCodecError::InvalidBasePrimaries(other)),
        }
    }
}

/// Current native v7 `dt_iop_agx_params_t`, in declaration order.
///
/// `gboolean` is an ABI-sized C `int` here rather than Rust `bool`; native
/// history payloads therefore remain exactly 144 little-endian bytes.
#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(C)]
pub struct AgxParametersV7 {
    pub look_lift: f32,
    pub look_slope: f32,
    pub look_brightness: f32,
    pub look_saturation: f32,
    pub look_original_hue_mix_ratio: f32,
    pub range_black_relative_ev: f32,
    pub range_white_relative_ev: f32,
    pub dynamic_range_scaling: f32,
    pub curve_pivot_x: f32,
    pub curve_pivot_y_linear_output: f32,
    pub curve_contrast_around_pivot: f32,
    pub curve_linear_ratio_below_pivot: f32,
    pub curve_linear_ratio_above_pivot: f32,
    pub curve_toe_power: f32,
    pub curve_shoulder_power: f32,
    pub curve_gamma: f32,
    pub auto_gamma: i32,
    pub curve_target_display_black_ratio: f32,
    pub curve_target_display_white_ratio: f32,
    pub base_primaries: AgxBasePrimaries,
    pub disable_primaries_adjustments: i32,
    pub red_inset: f32,
    pub red_rotation: f32,
    pub green_inset: f32,
    pub green_rotation: f32,
    pub blue_inset: f32,
    pub blue_rotation: f32,
    pub master_outset_ratio: f32,
    pub master_unrotation_ratio: f32,
    pub red_outset: f32,
    pub red_unrotation: f32,
    pub green_outset: f32,
    pub green_unrotation: f32,
    pub blue_outset: f32,
    pub blue_unrotation: f32,
    pub completely_reverse_primaries: i32,
}

const _: () = assert!(size_of::<AgxParametersV7>() == AGX_PARAMETER_BYTES_V7);

impl AgxParametersV7 {
    #[must_use]
    pub const fn zeroed() -> Self {
        Self {
            look_lift: 0.0,
            look_slope: 0.0,
            look_brightness: 0.0,
            look_saturation: 0.0,
            look_original_hue_mix_ratio: 0.0,
            range_black_relative_ev: 0.0,
            range_white_relative_ev: 0.0,
            dynamic_range_scaling: 0.0,
            curve_pivot_x: 0.0,
            curve_pivot_y_linear_output: 0.0,
            curve_contrast_around_pivot: 0.0,
            curve_linear_ratio_below_pivot: 0.0,
            curve_linear_ratio_above_pivot: 0.0,
            curve_toe_power: 0.0,
            curve_shoulder_power: 0.0,
            curve_gamma: 0.0,
            auto_gamma: 0,
            curve_target_display_black_ratio: 0.0,
            curve_target_display_white_ratio: 0.0,
            base_primaries: AgxBasePrimaries::ExportProfile,
            disable_primaries_adjustments: 0,
            red_inset: 0.0,
            red_rotation: 0.0,
            green_inset: 0.0,
            green_rotation: 0.0,
            blue_inset: 0.0,
            blue_rotation: 0.0,
            master_outset_ratio: 0.0,
            master_unrotation_ratio: 0.0,
            red_outset: 0.0,
            red_unrotation: 0.0,
            green_outset: 0.0,
            green_unrotation: 0.0,
            blue_outset: 0.0,
            blue_unrotation: 0.0,
            completely_reverse_primaries: 0,
        }
    }

    /// Native parameter defaults before workflow-specific preset application.
    #[must_use]
    pub fn defaults() -> Self {
        let mut parameters = Self::zeroed();
        set_default_curve_and_look_params(&mut parameters);
        set_unmodified_primaries(&mut parameters);
        parameters
    }

    /// Native scene-referred AgX default preset.
    #[must_use]
    pub fn scene_referred_defaults() -> Self {
        let mut parameters = Self::zeroed();
        set_default_curve_and_look_params(&mut parameters);
        set_blenderlike_primaries(&mut parameters);
        parameters
    }

    /// Native Blender-like base preset.
    #[must_use]
    pub fn blenderlike_defaults() -> Self {
        let mut parameters = Self::zeroed();
        set_default_curve_and_look_params(&mut parameters);
        set_blenderlike_primaries(&mut parameters);
        parameters.curve_shoulder_power = 1.5;
        parameters.curve_toe_power = 1.5;
        parameters.curve_gamma = 2.4;
        let compensation_factor = calculate_slope_gamma_compensation(
            parameters.curve_gamma,
            parameters
                .curve_pivot_y_linear_output
                .powf(1.0 / parameters.curve_gamma),
            &parameters,
        );
        parameters.curve_contrast_around_pivot = 2.4 * compensation_factor;
        parameters
    }

    /// Native Blender-like punchy preset.
    #[must_use]
    pub fn blenderlike_punchy_defaults() -> Self {
        let mut parameters = Self::blenderlike_defaults();
        parameters.look_brightness = 1.0 / (1.35 * 1.35);
        parameters.look_lift = 0.0;
        parameters.look_saturation = 1.4;
        parameters
    }

    /// Native smooth preset.
    #[must_use]
    pub fn smooth_defaults() -> Self {
        let mut parameters = Self::zeroed();
        set_default_curve_and_look_params(&mut parameters);
        set_smooth_primaries(&mut parameters);
        parameters
    }

    /// Serializes the native v7 field sequence as little-endian bytes.
    #[must_use]
    pub fn to_bytes(self) -> [u8; AGX_PARAMETER_BYTES_V7] {
        let mut bytes = [0_u8; AGX_PARAMETER_BYTES_V7];
        let mut offset = 0;
        for value in [
            self.look_lift,
            self.look_slope,
            self.look_brightness,
            self.look_saturation,
            self.look_original_hue_mix_ratio,
            self.range_black_relative_ev,
            self.range_white_relative_ev,
            self.dynamic_range_scaling,
            self.curve_pivot_x,
            self.curve_pivot_y_linear_output,
            self.curve_contrast_around_pivot,
            self.curve_linear_ratio_below_pivot,
            self.curve_linear_ratio_above_pivot,
            self.curve_toe_power,
            self.curve_shoulder_power,
            self.curve_gamma,
        ] {
            write_f32(&mut bytes, &mut offset, value);
        }
        write_i32(&mut bytes, &mut offset, self.auto_gamma);
        for value in [
            self.curve_target_display_black_ratio,
            self.curve_target_display_white_ratio,
        ] {
            write_f32(&mut bytes, &mut offset, value);
        }
        write_i32(&mut bytes, &mut offset, self.base_primaries as i32);
        write_i32(&mut bytes, &mut offset, self.disable_primaries_adjustments);
        for value in [
            self.red_inset,
            self.red_rotation,
            self.green_inset,
            self.green_rotation,
            self.blue_inset,
            self.blue_rotation,
            self.master_outset_ratio,
            self.master_unrotation_ratio,
            self.red_outset,
            self.red_unrotation,
            self.green_outset,
            self.green_unrotation,
            self.blue_outset,
            self.blue_unrotation,
        ] {
            write_f32(&mut bytes, &mut offset, value);
        }
        write_i32(&mut bytes, &mut offset, self.completely_reverse_primaries);
        debug_assert_eq!(offset, bytes.len());
        bytes
    }

    /// Decodes exactly one native v7 payload.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, AgxCodecError> {
        if bytes.len() != AGX_PARAMETER_BYTES_V7 {
            return Err(AgxCodecError::InvalidLength {
                expected: AGX_PARAMETER_BYTES_V7,
                actual: bytes.len(),
            });
        }
        let mut offset = 0;
        let look_lift = read_f32(bytes, &mut offset);
        let look_slope = read_f32(bytes, &mut offset);
        let look_brightness = read_f32(bytes, &mut offset);
        let look_saturation = read_f32(bytes, &mut offset);
        let look_original_hue_mix_ratio = read_f32(bytes, &mut offset);
        let range_black_relative_ev = read_f32(bytes, &mut offset);
        let range_white_relative_ev = read_f32(bytes, &mut offset);
        let dynamic_range_scaling = read_f32(bytes, &mut offset);
        let curve_pivot_x = read_f32(bytes, &mut offset);
        let curve_pivot_y_linear_output = read_f32(bytes, &mut offset);
        let curve_contrast_around_pivot = read_f32(bytes, &mut offset);
        let curve_linear_ratio_below_pivot = read_f32(bytes, &mut offset);
        let curve_linear_ratio_above_pivot = read_f32(bytes, &mut offset);
        let curve_toe_power = read_f32(bytes, &mut offset);
        let curve_shoulder_power = read_f32(bytes, &mut offset);
        let curve_gamma = read_f32(bytes, &mut offset);
        let auto_gamma = read_i32(bytes, &mut offset);
        let curve_target_display_black_ratio = read_f32(bytes, &mut offset);
        let curve_target_display_white_ratio = read_f32(bytes, &mut offset);
        let base_primaries = AgxBasePrimaries::try_from(read_i32(bytes, &mut offset))?;
        let disable_primaries_adjustments = read_i32(bytes, &mut offset);
        let red_inset = read_f32(bytes, &mut offset);
        let red_rotation = read_f32(bytes, &mut offset);
        let green_inset = read_f32(bytes, &mut offset);
        let green_rotation = read_f32(bytes, &mut offset);
        let blue_inset = read_f32(bytes, &mut offset);
        let blue_rotation = read_f32(bytes, &mut offset);
        let master_outset_ratio = read_f32(bytes, &mut offset);
        let master_unrotation_ratio = read_f32(bytes, &mut offset);
        let red_outset = read_f32(bytes, &mut offset);
        let red_unrotation = read_f32(bytes, &mut offset);
        let green_outset = read_f32(bytes, &mut offset);
        let green_unrotation = read_f32(bytes, &mut offset);
        let blue_outset = read_f32(bytes, &mut offset);
        let blue_unrotation = read_f32(bytes, &mut offset);
        let completely_reverse_primaries = read_i32(bytes, &mut offset);
        debug_assert_eq!(offset, bytes.len());
        Ok(Self {
            look_lift,
            look_slope,
            look_brightness,
            look_saturation,
            look_original_hue_mix_ratio,
            range_black_relative_ev,
            range_white_relative_ev,
            dynamic_range_scaling,
            curve_pivot_x,
            curve_pivot_y_linear_output,
            curve_contrast_around_pivot,
            curve_linear_ratio_below_pivot,
            curve_linear_ratio_above_pivot,
            curve_toe_power,
            curve_shoulder_power,
            curve_gamma,
            auto_gamma,
            curve_target_display_black_ratio,
            curve_target_display_white_ratio,
            base_primaries,
            disable_primaries_adjustments,
            red_inset,
            red_rotation,
            green_inset,
            green_rotation,
            blue_inset,
            blue_rotation,
            master_outset_ratio,
            master_unrotation_ratio,
            red_outset,
            red_unrotation,
            green_outset,
            green_unrotation,
            blue_outset,
            blue_unrotation,
            completely_reverse_primaries,
        })
    }
}

impl Default for AgxParametersV7 {
    fn default() -> Self {
        Self::defaults()
    }
}

/// History values accepted by the native AgX loader.
#[derive(Debug, Clone, PartialEq)]
pub enum AgxHistory {
    /// Native versions below v7 were unreleased test versions and are replaced
    /// wholesale by the current scene-referred defaults.
    LegacySceneReferred {
        source_version: u16,
        parameters: AgxParametersV7,
    },
    V7(AgxParametersV7),
}

impl AgxHistory {
    pub fn decode(version: u16, bytes: &[u8]) -> Result<Self, AgxCodecError> {
        if version < AGX_SCHEMA_VERSION {
            return Ok(Self::LegacySceneReferred {
                source_version: version,
                parameters: AgxParametersV7::scene_referred_defaults(),
            });
        }
        if version == AGX_SCHEMA_VERSION {
            return Ok(Self::V7(AgxParametersV7::from_bytes(bytes)?));
        }
        Err(AgxCodecError::UnsupportedVersion(version))
    }

    /// Serialization always emits the current native module version, including
    /// after a pre-v7 history value has been migrated to scene-referred defaults.
    #[must_use]
    pub const fn version(&self) -> u16 {
        match self {
            Self::LegacySceneReferred { .. } | Self::V7(_) => AGX_SCHEMA_VERSION,
        }
    }

    /// Native migration publishes the current v7 payload after replacement.
    #[must_use]
    pub fn payload(&self) -> Vec<u8> {
        self.current().to_bytes().to_vec()
    }

    #[must_use]
    pub const fn current(&self) -> AgxParametersV7 {
        match self {
            Self::LegacySceneReferred { parameters, .. } | Self::V7(parameters) => *parameters,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgxCodecError {
    InvalidLength { expected: usize, actual: usize },
    InvalidBasePrimaries(i32),
    UnsupportedVersion(u16),
}

impl fmt::Display for AgxCodecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLength { expected, actual } => {
                write!(
                    formatter,
                    "AgX payload has {actual} bytes; expected {expected}"
                )
            }
            Self::InvalidBasePrimaries(value) => {
                write!(formatter, "AgX base primaries value {value} is unknown")
            }
            Self::UnsupportedVersion(version) => {
                write!(formatter, "AgX version {version} is unsupported")
            }
        }
    }
}

impl std::error::Error for AgxCodecError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgxParameterError {
    NonFinite(&'static str),
}

impl fmt::Display for AgxParameterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFinite(name) => write!(formatter, "AgX {name} is non-finite"),
        }
    }
}

impl std::error::Error for AgxParameterError {}

/// Finite committed parameters. Native UI ranges are not execution clamps.
#[derive(Debug, Clone, Copy)]
pub struct AgxConfig {
    parameters: AgxParametersV7,
}

impl PartialEq for AgxConfig {
    fn eq(&self, other: &Self) -> bool {
        self.parameters.to_bytes() == other.parameters.to_bytes()
    }
}

impl Eq for AgxConfig {}

impl Hash for AgxConfig {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.parameters.to_bytes().hash(state);
    }
}

impl TryFrom<AgxParametersV7> for AgxConfig {
    type Error = AgxParameterError;

    fn try_from(parameters: AgxParametersV7) -> Result<Self, Self::Error> {
        let fields = [
            ("look_lift", parameters.look_lift),
            ("look_slope", parameters.look_slope),
            ("look_brightness", parameters.look_brightness),
            ("look_saturation", parameters.look_saturation),
            (
                "look_original_hue_mix_ratio",
                parameters.look_original_hue_mix_ratio,
            ),
            (
                "range_black_relative_ev",
                parameters.range_black_relative_ev,
            ),
            (
                "range_white_relative_ev",
                parameters.range_white_relative_ev,
            ),
            ("dynamic_range_scaling", parameters.dynamic_range_scaling),
            ("curve_pivot_x", parameters.curve_pivot_x),
            (
                "curve_pivot_y_linear_output",
                parameters.curve_pivot_y_linear_output,
            ),
            (
                "curve_contrast_around_pivot",
                parameters.curve_contrast_around_pivot,
            ),
            (
                "curve_linear_ratio_below_pivot",
                parameters.curve_linear_ratio_below_pivot,
            ),
            (
                "curve_linear_ratio_above_pivot",
                parameters.curve_linear_ratio_above_pivot,
            ),
            ("curve_toe_power", parameters.curve_toe_power),
            ("curve_shoulder_power", parameters.curve_shoulder_power),
            ("curve_gamma", parameters.curve_gamma),
            (
                "curve_target_display_black_ratio",
                parameters.curve_target_display_black_ratio,
            ),
            (
                "curve_target_display_white_ratio",
                parameters.curve_target_display_white_ratio,
            ),
            ("red_inset", parameters.red_inset),
            ("red_rotation", parameters.red_rotation),
            ("green_inset", parameters.green_inset),
            ("green_rotation", parameters.green_rotation),
            ("blue_inset", parameters.blue_inset),
            ("blue_rotation", parameters.blue_rotation),
            ("master_outset_ratio", parameters.master_outset_ratio),
            (
                "master_unrotation_ratio",
                parameters.master_unrotation_ratio,
            ),
            ("red_outset", parameters.red_outset),
            ("red_unrotation", parameters.red_unrotation),
            ("green_outset", parameters.green_outset),
            ("green_unrotation", parameters.green_unrotation),
            ("blue_outset", parameters.blue_outset),
            ("blue_unrotation", parameters.blue_unrotation),
        ];
        for (name, value) in fields {
            if !value.is_finite() {
                return Err(AgxParameterError::NonFinite(name));
            }
        }
        Ok(Self { parameters })
    }
}

impl AgxConfig {
    pub fn new(parameters: AgxParametersV7) -> Result<Self, AgxParameterError> {
        parameters.try_into()
    }

    #[must_use]
    pub fn defaults() -> Self {
        Self::new(AgxParametersV7::defaults()).expect("AgX defaults are finite")
    }

    #[must_use]
    pub const fn parameters(self) -> AgxParametersV7 {
        self.parameters
    }
}

pub type AgxMatrix = [[f32; 3]; 3];

// The built-in profiles in `src/common/colorspaces.c` are created by
// `cmsCreateRGBProfile`. `src/common/iop_profile.c` then consumes their
// matrix-shaper colorant tags in the ICC D50 PCS and stores the transposed
// input/output matrices below. AgX also derives its tuning chromaticities from
// those D50 colorants; the original D65 profile-construction chromaticities
// are therefore not valid substitutes here.
const NATIVE_D50_MEDIA_WHITE_XYZ: [f32; 3] = [0.9642000198364258, 1.0, 0.8248999714851379];

const SRGB_D50_MATRIX_IN_TRANSPOSED: AgxMatrix = [
    [
        0.4360368549823761,
        0.22248178720474243,
        0.013922344893217087,
    ],
    [0.38512367010116577, 0.7169127464294434, 0.09707818925380707],
    [0.14303947985172272, 0.06060545891523361, 0.7138994932174683],
];
const SRGB_D50_MATRIX_OUT_TRANSPOSED: AgxMatrix = [
    [3.13423490524292, -0.9787409901618958, 0.07196881622076035],
    [-1.6172577142715454, 1.9161189794540405, -0.2290201336145401],
    [-0.4906919002532959, 0.03343794122338295, 1.4057797193527222],
];

const REC2020_D50_MATRIX_IN_TRANSPOSED: AgxMatrix = [
    [
        0.6734747886657715,
        0.27904054522514343,
        -0.0019327097106724977,
    ],
    [
        0.16567546129226685,
        0.6753473281860352,
        0.029981441795825958,
    ],
    [0.12504972517490387, 0.04561210051178932, 0.7968512773513794],
];
const REC2020_D50_MATRIX_OUT_TRANSPOSED: AgxMatrix = [
    [1.6472502946853638, -0.682616651058197, 0.029678674414753914],
    [
        -0.3936258554458618,
        1.6476095914840698,
        -0.06294584274291992,
    ],
    [-0.2359713762998581, 0.01281304843723774, 1.2538849115371704],
];

const DISPLAY_P3_D50_MATRIX_IN_TRANSPOSED: AgxMatrix = [
    [
        0.5151157975196838,
        0.24118758738040924,
        -0.0010501055512577295,
    ],
    [0.2919851541519165, 0.6922499537467957, 0.04188039153814316],
    [0.15709903836250305, 0.0665624588727951, 0.7840697169303894],
];
const DISPLAY_P3_D50_MATRIX_OUT_TRANSPOSED: AgxMatrix = [
    [2.4040069580078125, -0.8422179818153381, 0.04820602014660835],
    [
        -0.9899331331253052,
        1.7988348007202148,
        -0.09740899503231049,
    ],
    [
        -0.3976365923881531,
        0.016040362417697906,
        1.2740074396133423,
    ],
];

const ADOBE_RGB_D50_MATRIX_IN_TRANSPOSED: AgxMatrix = [
    [
        0.6097398996353149,
        0.31111136078834534,
        0.019468558952212334,
    ],
    [0.2052789330482483, 0.6256809234619141, 0.06087923422455788],
    [0.14918117225170135, 0.06320767849683762, 0.7445521950721741],
];
const ADOBE_RGB_D50_MATRIX_OUT_TRANSPOSED: AgxMatrix = [
    [1.962528109550476, -0.9787409901618958, 0.028711769729852676],
    [-0.6106672883033752, 1.9161192178726196, -0.1407061368227005],
    [
        -0.3413775563240051,
        0.033437930047512054,
        1.3492814302444458,
    ],
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgxProfileError {
    NonFinite,
    SingularMatrix,
}

impl fmt::Display for AgxProfileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFinite => formatter.write_str("AgX profile contains a non-finite value"),
            Self::SingularMatrix => formatter.write_str("AgX profile matrix is singular"),
        }
    }
}

impl std::error::Error for AgxProfileError {}

/// Matrix-shaper profile data used by the native AgX profile path.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AgxProfile {
    primaries: [[f32; 2]; 3],
    whitepoint: [f32; 2],
    matrix_in_transposed: AgxMatrix,
    matrix_out_transposed: AgxMatrix,
}

impl AgxProfile {
    /// Builds a synthetic profile from chromaticities already expressed in the
    /// intended connection space. Native ICC profiles must instead enter
    /// through [`Self::from_matrices`] with their D50 PCS matrix-shaper data.
    pub fn from_primaries(
        primaries: [[f32; 2]; 3],
        whitepoint: [f32; 2],
    ) -> Result<Self, AgxProfileError> {
        let matrix_in_transposed = matrix_from_primaries(primaries, whitepoint)?;
        let matrix_out_transposed = invert_profile_input_matrix(matrix_in_transposed)
            .ok_or(AgxProfileError::SingularMatrix)?;
        Self::from_matrices(
            primaries,
            whitepoint,
            matrix_in_transposed,
            matrix_out_transposed,
        )
    }

    /// Accepts the profile chromaticities and transposed RGB↔D50 PCS matrices
    /// populated by native `dt_iop_order_iccprofile_info_t`.
    pub fn from_matrices(
        primaries: [[f32; 2]; 3],
        whitepoint: [f32; 2],
        matrix_in_transposed: AgxMatrix,
        matrix_out_transposed: AgxMatrix,
    ) -> Result<Self, AgxProfileError> {
        if !primaries
            .into_iter()
            .flatten()
            .chain(whitepoint)
            .chain(matrix_in_transposed.into_iter().flatten())
            .chain(matrix_out_transposed.into_iter().flatten())
            .all(f32::is_finite)
        {
            return Err(AgxProfileError::NonFinite);
        }
        Ok(Self {
            primaries,
            whitepoint,
            matrix_in_transposed,
            matrix_out_transposed,
        })
    }

    #[must_use]
    pub fn srgb() -> Self {
        Self::from_standard_icc_matrices(
            SRGB_D50_MATRIX_IN_TRANSPOSED,
            SRGB_D50_MATRIX_OUT_TRANSPOSED,
        )
    }

    #[must_use]
    pub fn rec2020() -> Self {
        Self::from_standard_icc_matrices(
            REC2020_D50_MATRIX_IN_TRANSPOSED,
            REC2020_D50_MATRIX_OUT_TRANSPOSED,
        )
    }

    #[must_use]
    pub fn display_p3() -> Self {
        Self::from_standard_icc_matrices(
            DISPLAY_P3_D50_MATRIX_IN_TRANSPOSED,
            DISPLAY_P3_D50_MATRIX_OUT_TRANSPOSED,
        )
    }

    #[must_use]
    pub fn adobe_rgb() -> Self {
        Self::from_standard_icc_matrices(
            ADOBE_RGB_D50_MATRIX_IN_TRANSPOSED,
            ADOBE_RGB_D50_MATRIX_OUT_TRANSPOSED,
        )
    }

    fn from_standard_icc_matrices(
        matrix_in_transposed: AgxMatrix,
        matrix_out_transposed: AgxMatrix,
    ) -> Self {
        let primaries = matrix_in_transposed.map(xyz_colorant_to_xy);
        let whitepoint = xyz_colorant_to_xy(NATIVE_D50_MEDIA_WHITE_XYZ);
        debug_assert_eq!(
            invert_profile_input_matrix(matrix_in_transposed),
            Some(matrix_out_transposed)
        );
        Self::from_matrices(
            primaries,
            whitepoint,
            matrix_in_transposed,
            matrix_out_transposed,
        )
        .expect("native standard ICC matrix-shaper profile is finite")
    }

    #[must_use]
    pub const fn primaries(self) -> [[f32; 2]; 3] {
        self.primaries
    }

    #[must_use]
    pub const fn whitepoint(self) -> [f32; 2] {
        self.whitepoint
    }

    #[must_use]
    pub const fn matrix_in_transposed(self) -> AgxMatrix {
        self.matrix_in_transposed
    }

    #[must_use]
    pub const fn matrix_out_transposed(self) -> AgxMatrix {
        self.matrix_out_transposed
    }

    /// Returns native `matrix_in` in conventional XYZ-row/RGB-column order.
    #[must_use]
    pub const fn matrix_in_row_major(self) -> AgxMatrix {
        [
            [
                self.matrix_in_transposed[0][0],
                self.matrix_in_transposed[1][0],
                self.matrix_in_transposed[2][0],
            ],
            [
                self.matrix_in_transposed[0][1],
                self.matrix_in_transposed[1][1],
                self.matrix_in_transposed[2][1],
            ],
            [
                self.matrix_in_transposed[0][2],
                self.matrix_in_transposed[1][2],
                self.matrix_in_transposed[2][2],
            ],
        ]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgxProfileResolutionError {
    ExternalProfileUnsupported,
    UnsupportedWorkingEncoding(rusttable_color::ColorEncoding),
}

impl fmt::Display for AgxProfileResolutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ExternalProfileUnsupported => {
                formatter.write_str("configured external working-profile resolution is unavailable")
            }
            Self::UnsupportedWorkingEncoding(encoding) => write!(
                formatter,
                "working encoding {encoding:?} is unsupported by the built-in D50 matrix-shaper resolver"
            ),
        }
    }
}

impl std::error::Error for AgxProfileResolutionError {}

/// Resolves the exact built-in D50 matrix-shaper evidence accepted by the
/// production CPU route. Configured external profiles remain fail-closed.
pub fn resolve_builtin_working_profile(
    frame: crate::WorkingFrameDescriptor,
) -> Result<AgxProfile, AgxProfileResolutionError> {
    if frame.profile_id().is_some() {
        return Err(AgxProfileResolutionError::ExternalProfileUnsupported);
    }
    match frame.encoding() {
        rusttable_color::ColorEncoding::LinearSrgbD65 => Ok(AgxProfile::srgb()),
        rusttable_color::ColorEncoding::LinearDisplayP3D65 => Ok(AgxProfile::display_p3()),
        rusttable_color::ColorEncoding::LinearRec2020D65 => Ok(AgxProfile::rec2020()),
        encoding => Err(AgxProfileResolutionError::UnsupportedWorkingEncoding(
            encoding,
        )),
    }
}

/// Derived curve and look state matching native `tone_mapping_params_t`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AgxToneMappingParameters {
    pub black_relative_ev: f32,
    pub white_relative_ev: f32,
    pub range_in_ev: f32,
    pub curve_gamma: f32,
    pub pivot_x: f32,
    pub pivot_y: f32,
    pub target_black: f32,
    pub toe_power: f32,
    pub toe_transition_x: f32,
    pub toe_transition_y: f32,
    pub toe_scale: f32,
    pub need_convex_toe: bool,
    pub toe_fallback_coefficient: f32,
    pub toe_fallback_power: f32,
    pub slope: f32,
    pub intercept: f32,
    pub target_white: f32,
    pub shoulder_power: f32,
    pub shoulder_transition_x: f32,
    pub shoulder_transition_y: f32,
    pub shoulder_scale: f32,
    pub need_concave_shoulder: bool,
    pub shoulder_fallback_coefficient: f32,
    pub shoulder_fallback_power: f32,
    pub look_lift: f32,
    pub look_slope: f32,
    pub look_power: f32,
    pub look_saturation: f32,
    pub look_original_hue_mix_ratio: f32,
    pub look_tuned: bool,
    pub restore_hue: bool,
}

impl AgxToneMappingParameters {
    #[must_use]
    pub fn apply_curve(self, x: f32) -> f32 {
        apply_curve(x, &self)
    }

    #[must_use]
    pub fn apply_log_encoding(self, x: f32) -> f32 {
        apply_log_encoding(x, self.range_in_ev, self.black_relative_ev)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgxPlanError {
    NonFiniteDerived(&'static str),
    SingularMatrix,
    InvalidProfile(AgxProfileError),
}

impl fmt::Display for AgxPlanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFiniteDerived(name) => write!(formatter, "AgX derived {name} is non-finite"),
            Self::SingularMatrix => formatter.write_str("AgX adjusted profile matrix is singular"),
            Self::InvalidProfile(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for AgxPlanError {}

impl From<AgxProfileError> for AgxPlanError {
    fn from(error: AgxProfileError) -> Self {
        Self::InvalidProfile(error)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgxExecutionError {
    DimensionsMismatch { expected: usize, actual: usize },
    NonFiniteOutput { pixel: usize },
    AllocationFailed { required_bytes: usize },
    SizeOverflow,
    Cancelled,
}

impl fmt::Display for AgxExecutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DimensionsMismatch { expected, actual } => {
                write!(formatter, "AgX expected {expected} pixels, got {actual}")
            }
            Self::NonFiniteOutput { pixel } => {
                write!(formatter, "AgX produced non-finite output at pixel {pixel}")
            }
            Self::AllocationFailed { required_bytes } => {
                write!(
                    formatter,
                    "AgX allocation failed for {required_bytes} bytes"
                )
            }
            Self::SizeOverflow => formatter.write_str("AgX execution size overflowed"),
            Self::Cancelled => formatter.write_str("AgX execution was cancelled"),
        }
    }
}

impl std::error::Error for AgxExecutionError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgxCapabilityError {
    GpuUnavailable,
    GtkUnavailable,
    ProductionRoutingDeferred,
}

impl fmt::Display for AgxCapabilityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::GpuUnavailable => formatter.write_str("AgX GPU execution is unavailable"),
            Self::GtkUnavailable => formatter.write_str("AgX GTK controls are unavailable"),
            Self::ProductionRoutingDeferred => {
                formatter.write_str("AgX production routing is deferred")
            }
        }
    }
}

impl std::error::Error for AgxCapabilityError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgxCapabilities {
    pub cpu_supported: bool,
    pub gpu_supported: bool,
    pub gtk_supported: bool,
    pub profile_transforms_supported: bool,
    pub masks_consumed: bool,
    pub outer_blending_deferred: bool,
    pub production_routing_deferred: bool,
    pub alpha_preserved: bool,
}

impl AgxCapabilities {
    #[must_use]
    pub const fn bounded_cpu_leaf() -> Self {
        Self {
            cpu_supported: true,
            gpu_supported: AGX_GPU_EXECUTABLE,
            gtk_supported: false,
            // The operation-local transform math and standard profiles are
            // available, but the required working/export profile resolution
            // still belongs to the deferred shared profile hub.
            profile_transforms_supported: false,
            masks_consumed: false,
            outer_blending_deferred: true,
            production_routing_deferred: false,
            alpha_preserved: true,
        }
    }

    pub const fn require_gpu(self) -> Result<(), AgxCapabilityError> {
        if self.gpu_supported {
            Ok(())
        } else {
            Err(AgxCapabilityError::GpuUnavailable)
        }
    }

    pub const fn require_gtk(self) -> Result<(), AgxCapabilityError> {
        if self.gtk_supported {
            Ok(())
        } else {
            Err(AgxCapabilityError::GtkUnavailable)
        }
    }

    pub const fn require_production_routing(self) -> Result<(), AgxCapabilityError> {
        if self.production_routing_deferred {
            Err(AgxCapabilityError::ProductionRoutingDeferred)
        } else {
            Ok(())
        }
    }
}

#[must_use]
pub const fn capabilities() -> AgxCapabilities {
    AgxCapabilities::bounded_cpu_leaf()
}

/// Four-channel native RGB sample. The fourth channel is copied after input
/// sanitisation, exactly as `process()` does.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AgxPixel {
    channels: [f32; 4],
}

impl AgxPixel {
    #[must_use]
    pub const fn new(red: f32, green: f32, blue: f32, alpha: f32) -> Self {
        Self {
            channels: [red, green, blue, alpha],
        }
    }

    #[must_use]
    pub const fn from_channels(channels: [f32; 4]) -> Self {
        Self { channels }
    }

    #[must_use]
    pub const fn channels(self) -> [f32; 4] {
        self.channels
    }

    #[must_use]
    pub const fn alpha(self) -> f32 {
        self.channels[3]
    }
}

/// Immutable CPU plan. Matrix fields retain Darktable's transposed storage
/// order, where each row is an input channel and each column is an output
/// channel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AgxPlan {
    config: AgxConfig,
    dimensions: RasterDimensions,
    tone_mapping: AgxToneMappingParameters,
    rendering_to_xyz: AgxMatrix,
    pipe_to_base: AgxMatrix,
    base_to_rendering: AgxMatrix,
    rendering_to_pipe: AgxMatrix,
    base_working_same_profile: bool,
}

impl AgxPlan {
    pub fn new(config: AgxConfig, dimensions: RasterDimensions) -> Result<Self, AgxPlanError> {
        Self::new_with_profiles(config, dimensions, AgxProfile::srgb(), None)
    }

    pub fn new_with_profile(
        config: AgxConfig,
        dimensions: RasterDimensions,
        working_profile: AgxProfile,
    ) -> Result<Self, AgxPlanError> {
        Self::new_with_profiles(config, dimensions, working_profile, None)
    }

    pub fn new_with_profiles(
        config: AgxConfig,
        dimensions: RasterDimensions,
        working_profile: AgxProfile,
        export_profile: Option<AgxProfile>,
    ) -> Result<Self, AgxPlanError> {
        let tone_mapping = calculate_tone_mapping_parameters(config.parameters())?;
        let primaries = get_primaries_params(config.parameters());
        let base_profile = match config.parameters().base_primaries {
            AgxBasePrimaries::ExportProfile => export_profile
                .filter(|profile| profile_is_usable(*profile))
                .unwrap_or_else(AgxProfile::rec2020),
            AgxBasePrimaries::WorkingProfile => working_profile,
            AgxBasePrimaries::Rec2020 => AgxProfile::rec2020(),
            AgxBasePrimaries::DisplayP3 => AgxProfile::display_p3(),
            AgxBasePrimaries::AdobeRgb => AgxProfile::adobe_rgb(),
            AgxBasePrimaries::Srgb => AgxProfile::srgb(),
        };
        // Native compares profile pointers. The operation-local profile value
        // is immutable and copied, so equality is the faithful local identity
        // approximation and also covers a standard profile selected as the
        // working profile.
        let base_working_same_profile = base_profile == working_profile;
        let matrices = create_matrices(&primaries, working_profile, base_profile)?;
        let plan = Self {
            config,
            dimensions,
            tone_mapping,
            rendering_to_xyz: matrices.rendering_to_xyz,
            pipe_to_base: matrices.pipe_to_base,
            base_to_rendering: matrices.base_to_rendering,
            rendering_to_pipe: matrices.rendering_to_pipe,
            base_working_same_profile,
        };
        ensure_plan_finite(&plan)?;
        Ok(plan)
    }

    #[must_use]
    pub const fn config(self) -> AgxConfig {
        self.config
    }

    #[must_use]
    pub const fn dimensions(self) -> RasterDimensions {
        self.dimensions
    }

    #[must_use]
    pub const fn tone_mapping_parameters(self) -> AgxToneMappingParameters {
        self.tone_mapping
    }

    #[must_use]
    pub const fn matrices(self) -> (AgxMatrix, AgxMatrix, AgxMatrix, AgxMatrix) {
        (
            self.rendering_to_xyz,
            self.pipe_to_base,
            self.base_to_rendering,
            self.rendering_to_pipe,
        )
    }

    pub fn execute(&self, input: &[AgxPixel]) -> Result<Vec<AgxPixel>, AgxExecutionError> {
        self.execute_with_cancel(input, || false)
    }

    /// Polls cancellation at row boundaries and publishes only a complete
    /// output vector. The source CPU loop has no cancellation primitive, so
    /// this is an additive Rust publication boundary.
    pub fn execute_with_cancel<F: FnMut() -> bool>(
        &self,
        input: &[AgxPixel],
        mut cancelled: F,
    ) -> Result<Vec<AgxPixel>, AgxExecutionError> {
        let expected = usize::try_from(self.dimensions.pixel_count())
            .map_err(|_| AgxExecutionError::SizeOverflow)?;
        if input.len() != expected {
            return Err(AgxExecutionError::DimensionsMismatch {
                expected,
                actual: input.len(),
            });
        }
        if cancelled() {
            return Err(AgxExecutionError::Cancelled);
        }
        let required_bytes = expected
            .checked_mul(size_of::<AgxPixel>())
            .ok_or(AgxExecutionError::SizeOverflow)?;
        let mut output = Vec::new();
        output
            .try_reserve_exact(expected)
            .map_err(|_| AgxExecutionError::AllocationFailed { required_bytes })?;

        let width = usize::try_from(self.dimensions.width()).expect("u32 dimensions fit usize");
        for (index, pixel) in input.iter().copied().enumerate() {
            if index % width == 0 && cancelled() {
                return Err(AgxExecutionError::Cancelled);
            }
            let result = self.process_pixel(pixel);
            if !result.channels.into_iter().all(f32::is_finite) {
                return Err(AgxExecutionError::NonFiniteOutput { pixel: index });
            }
            output.push(result);
        }
        if cancelled() {
            return Err(AgxExecutionError::Cancelled);
        }
        Ok(output)
    }

    fn process_pixel(&self, pixel: AgxPixel) -> AgxPixel {
        let sanitised = sanitise_pixel(pixel);
        let input = sanitised.channels;
        let input_rgb = [input[0], input[1], input[2]];

        let mut base_rgb = if self.base_working_same_profile {
            input_rgb
        } else {
            apply_matrix(input_rgb, &self.pipe_to_base)
        };
        compress_into_gamut(&mut base_rgb);
        let mut rendering_rgb = apply_matrix(base_rgb, &self.base_to_rendering);
        tone_mapping(
            &mut rendering_rgb,
            &self.tone_mapping,
            &self.rendering_to_xyz,
        );
        let output_rgb = apply_matrix(rendering_rgb, &self.rendering_to_pipe);
        AgxPixel::new(output_rgb[0], output_rgb[1], output_rgb[2], input[3])
    }
}

#[derive(Debug, Clone, Copy)]
struct MatrixBundle {
    rendering_to_xyz: AgxMatrix,
    pipe_to_base: AgxMatrix,
    base_to_rendering: AgxMatrix,
    rendering_to_pipe: AgxMatrix,
}

fn create_matrices(
    parameters: &PrimariesParameters,
    pipe_work_profile: AgxProfile,
    base_profile: AgxProfile,
) -> Result<MatrixBundle, AgxPlanError> {
    let pipe_to_base = multiply_matrices(
        pipe_work_profile.matrix_in_transposed,
        base_profile.matrix_out_transposed,
    );
    let base_to_pipe = invert_matrix(pipe_to_base).ok_or(AgxPlanError::SingularMatrix)?;

    let mut inset_and_rotated_primaries = [[0.0_f32; 2]; 3];
    for index in 0..3 {
        inset_and_rotated_primaries[index] = rotate_and_scale_primary(
            base_profile,
            1.0 - parameters.inset[index],
            parameters.rotation[index],
            index,
        );
    }
    let rendering_to_xyz =
        matrix_from_primaries(inset_and_rotated_primaries, base_profile.whitepoint)?;
    let base_to_rendering = multiply_matrices(rendering_to_xyz, base_profile.matrix_out_transposed);

    let mut outset_and_unrotated_primaries = [[0.0_f32; 2]; 3];
    for index in 0..3 {
        let scaling = 1.0 - parameters.master_outset_ratio * parameters.outset[index];
        outset_and_unrotated_primaries[index] = rotate_and_scale_primary(
            base_profile,
            scaling,
            parameters.master_unrotation_ratio * parameters.unrotation[index],
            index,
        );
    }
    let outset_and_unrotated_to_xyz =
        matrix_from_primaries(outset_and_unrotated_primaries, base_profile.whitepoint)?;
    let temporary = multiply_matrices(
        outset_and_unrotated_to_xyz,
        base_profile.matrix_out_transposed,
    );
    let rendering_to_base = invert_matrix(temporary).ok_or(AgxPlanError::SingularMatrix)?;
    let rendering_to_pipe = multiply_matrices(rendering_to_base, base_to_pipe);

    Ok(MatrixBundle {
        rendering_to_xyz,
        pipe_to_base,
        base_to_rendering,
        rendering_to_pipe,
    })
}

#[derive(Debug, Clone, Copy)]
struct PrimariesParameters {
    base_primaries: AgxBasePrimaries,
    inset: [f32; 3],
    rotation: [f32; 3],
    master_outset_ratio: f32,
    master_unrotation_ratio: f32,
    outset: [f32; 3],
    unrotation: [f32; 3],
}

fn get_primaries_params(parameters: AgxParametersV7) -> PrimariesParameters {
    let mut result = PrimariesParameters {
        base_primaries: parameters.base_primaries,
        inset: [
            parameters.red_inset,
            parameters.green_inset,
            parameters.blue_inset,
        ],
        rotation: [
            parameters.red_rotation,
            parameters.green_rotation,
            parameters.blue_rotation,
        ],
        master_outset_ratio: parameters.master_outset_ratio,
        master_unrotation_ratio: parameters.master_unrotation_ratio,
        outset: [
            parameters.red_outset,
            parameters.green_outset,
            parameters.blue_outset,
        ],
        unrotation: [
            parameters.red_unrotation,
            parameters.green_unrotation,
            parameters.blue_unrotation,
        ],
    };

    if parameters.disable_primaries_adjustments != 0 {
        result.inset = [0.0; 3];
        result.rotation = [0.0; 3];
        result.outset = [0.0; 3];
        result.unrotation = [0.0; 3];
    } else if parameters.completely_reverse_primaries != 0 {
        result.outset = result.inset;
        result.unrotation = result.rotation;
        result.master_outset_ratio = 1.0;
        result.master_unrotation_ratio = 1.0;
    }
    result
}

fn calculate_tone_mapping_parameters(
    parameters: AgxParametersV7,
) -> Result<AgxToneMappingParameters, AgxPlanError> {
    let brightness = parameters.look_brightness;
    let look_power = if brightness < 1.0 {
        1.0 / native_max(brightness, EPSILON).sqrt()
    } else {
        1.0 / brightness
    };
    let pivot_x = native_clamp(parameters.curve_pivot_x, EPSILON, 1.0 - EPSILON);
    let curve_gamma = if parameters.auto_gamma != 0
        && pivot_x > 0.0
        && parameters.curve_pivot_y_linear_output > 0.0
    {
        parameters.curve_pivot_y_linear_output.log2() / pivot_x.log2()
    } else {
        parameters.curve_gamma
    };
    let pivot_y = calculate_pivot_y_at_gamma(&parameters, curve_gamma);
    let range_in_ev = parameters.range_white_relative_ev - parameters.range_black_relative_ev;
    let range_adjusted_slope = parameters.curve_contrast_around_pivot * (range_in_ev / 16.5);
    let compensation_factor = calculate_slope_gamma_compensation(curve_gamma, pivot_y, &parameters);
    let slope = range_adjusted_slope / compensation_factor;

    let target_black = parameters
        .curve_target_display_black_ratio
        .powf(1.0 / curve_gamma);
    let toe_power = native_max(0.01, parameters.curve_toe_power);
    let remaining_y_below_pivot = pivot_y - target_black;
    let toe_length_y = remaining_y_below_pivot * parameters.curve_linear_ratio_below_pivot;
    let mut dx_linear_below_pivot = toe_length_y / slope;
    let toe_transition_x = native_max(EPSILON, pivot_x - dx_linear_below_pivot);
    dx_linear_below_pivot = pivot_x - toe_transition_x;
    let toe_dy_below_pivot = slope * dx_linear_below_pivot;
    let toe_transition_y = pivot_y - toe_dy_below_pivot;

    let inverse_toe_limit_x = 1.0;
    let inverse_toe_limit_y = 1.0 - target_black;
    let inverse_toe_transition_x = 1.0 - toe_transition_x;
    let inverse_toe_transition_y = 1.0 - toe_transition_y;
    let toe_scale = -scale(
        inverse_toe_limit_x,
        inverse_toe_limit_y,
        inverse_toe_transition_x,
        inverse_toe_transition_y,
        slope,
        toe_power,
    );
    let toe_length_x = toe_transition_x;
    let toe_dy_transition_to_limit = native_max(EPSILON, toe_transition_y - target_black);
    let toe_slope_transition_to_limit = toe_dy_transition_to_limit / toe_length_x;
    let need_convex_toe = toe_slope_transition_to_limit > slope;
    let toe_fallback_power =
        calculate_slope_matching_power(slope, toe_length_x, toe_dy_transition_to_limit);
    let toe_fallback_coefficient = calculate_fallback_curve_coefficient(
        toe_length_x,
        toe_dy_transition_to_limit,
        toe_fallback_power,
    );
    let intercept = toe_transition_y - (slope * toe_transition_x);

    let target_white = parameters
        .curve_target_display_white_ratio
        .powf(1.0 / curve_gamma);
    let remaining_y_above_pivot = target_white - pivot_y;
    let shoulder_length_y = remaining_y_above_pivot * parameters.curve_linear_ratio_above_pivot;
    let mut dx_linear_above_pivot = shoulder_length_y / slope;
    let shoulder_transition_x = native_min(1.0 - EPSILON, pivot_x + dx_linear_above_pivot);
    dx_linear_above_pivot = shoulder_transition_x - pivot_x;
    let shoulder_dy_above_pivot = slope * dx_linear_above_pivot;
    let shoulder_transition_y = pivot_y + shoulder_dy_above_pivot;
    let shoulder_power = native_max(0.01, parameters.curve_shoulder_power);
    let shoulder_scale = scale(
        1.0,
        target_white,
        shoulder_transition_x,
        shoulder_transition_y,
        slope,
        shoulder_power,
    );
    let shoulder_length_x = 1.0 - shoulder_transition_x;
    let shoulder_dy_transition_to_limit = native_max(EPSILON, target_white - shoulder_transition_y);
    let shoulder_slope_transition_to_limit = shoulder_dy_transition_to_limit / shoulder_length_x;
    let need_concave_shoulder = shoulder_slope_transition_to_limit > slope;
    let shoulder_fallback_power =
        calculate_slope_matching_power(slope, shoulder_length_x, shoulder_dy_transition_to_limit);
    let shoulder_fallback_coefficient = calculate_fallback_curve_coefficient(
        shoulder_length_x,
        shoulder_dy_transition_to_limit,
        shoulder_fallback_power,
    );

    let tone_mapping = AgxToneMappingParameters {
        black_relative_ev: parameters.range_black_relative_ev,
        white_relative_ev: parameters.range_white_relative_ev,
        range_in_ev,
        curve_gamma,
        pivot_x,
        pivot_y,
        target_black,
        toe_power,
        toe_transition_x,
        toe_transition_y,
        toe_scale,
        need_convex_toe,
        toe_fallback_coefficient,
        toe_fallback_power,
        slope,
        intercept,
        target_white,
        shoulder_power,
        shoulder_transition_x,
        shoulder_transition_y,
        shoulder_scale,
        need_concave_shoulder,
        shoulder_fallback_coefficient,
        shoulder_fallback_power,
        look_lift: parameters.look_lift,
        look_slope: parameters.look_slope,
        look_power,
        look_saturation: parameters.look_saturation,
        look_original_hue_mix_ratio: parameters.look_original_hue_mix_ratio,
        look_tuned: parameters.look_slope != 1.0
            || parameters.look_brightness != 1.0
            || parameters.look_lift != 0.0
            || parameters.look_saturation != 1.0,
        restore_hue: parameters.look_original_hue_mix_ratio != 0.0,
    };
    for (name, value) in tone_mapping_floats(tone_mapping) {
        if !value.is_finite() {
            return Err(AgxPlanError::NonFiniteDerived(name));
        }
    }
    Ok(tone_mapping)
}

fn tone_mapping_floats(parameters: AgxToneMappingParameters) -> [(&'static str, f32); 24] {
    [
        ("black_relative_ev", parameters.black_relative_ev),
        ("white_relative_ev", parameters.white_relative_ev),
        ("range_in_ev", parameters.range_in_ev),
        ("curve_gamma", parameters.curve_gamma),
        ("pivot_x", parameters.pivot_x),
        ("pivot_y", parameters.pivot_y),
        ("target_black", parameters.target_black),
        ("toe_power", parameters.toe_power),
        ("toe_transition_x", parameters.toe_transition_x),
        ("toe_transition_y", parameters.toe_transition_y),
        ("toe_scale", parameters.toe_scale),
        (
            "toe_fallback_coefficient",
            parameters.toe_fallback_coefficient,
        ),
        ("toe_fallback_power", parameters.toe_fallback_power),
        ("slope", parameters.slope),
        ("intercept", parameters.intercept),
        ("target_white", parameters.target_white),
        ("shoulder_power", parameters.shoulder_power),
        ("shoulder_transition_x", parameters.shoulder_transition_x),
        ("shoulder_transition_y", parameters.shoulder_transition_y),
        ("shoulder_scale", parameters.shoulder_scale),
        (
            "shoulder_fallback_coefficient",
            parameters.shoulder_fallback_coefficient,
        ),
        (
            "shoulder_fallback_power",
            parameters.shoulder_fallback_power,
        ),
        ("look_power", parameters.look_power),
        (
            "look_original_hue_mix_ratio",
            parameters.look_original_hue_mix_ratio,
        ),
    ]
}

fn calculate_pivot_y_at_gamma(parameters: &AgxParametersV7, gamma: f32) -> f32 {
    native_clamp(
        parameters.curve_pivot_y_linear_output,
        parameters.curve_target_display_black_ratio,
        parameters.curve_target_display_white_ratio,
    )
    .powf(1.0 / gamma)
}

fn calculate_slope_gamma_compensation(
    gamma: f32,
    pivot_y: f32,
    parameters: &AgxParametersV7,
) -> f32 {
    let pivot_y_at_default_gamma = calculate_pivot_y_at_gamma(parameters, DEFAULT_GAMMA);
    let derivative_at_current_gamma = gamma * native_max(EPSILON, pivot_y).powf(gamma - 1.0);
    let derivative_at_default_gamma =
        DEFAULT_GAMMA * native_max(EPSILON, pivot_y_at_default_gamma).powf(DEFAULT_GAMMA - 1.0);
    derivative_at_current_gamma / derivative_at_default_gamma
}

fn scale(
    limit_x: f32,
    limit_y: f32,
    transition_x: f32,
    transition_y: f32,
    slope: f32,
    power: f32,
) -> f32 {
    let projected_rise = slope * native_max(EPSILON, limit_x - transition_x);
    let actual_rise = native_max(EPSILON, limit_y - transition_y);
    let transformed_projected_rise = projected_rise.powf(-power);
    let transformed_actual_rise = actual_rise.powf(-power);
    let base = native_max(
        EPSILON,
        transformed_actual_rise - transformed_projected_rise,
    );
    let scale_value = base.powf(-1.0 / power);
    native_min(1e9, scale_value)
}

fn sigmoid(x: f32, power: f32) -> f32 {
    x / (1.0 + x.powf(power)).powf(1.0 / power)
}

fn scaled_sigmoid(
    x: f32,
    scale_value: f32,
    slope: f32,
    power: f32,
    transition_x: f32,
    transition_y: f32,
) -> f32 {
    scale_value * sigmoid(slope * (x - transition_x) / scale_value, power) + transition_y
}

fn fallback_toe(x: f32, parameters: &AgxToneMappingParameters) -> f32 {
    if x < 0.0 {
        parameters.target_black
    } else {
        parameters.target_black
            + native_max(
                0.0,
                parameters.toe_fallback_coefficient * x.powf(parameters.toe_fallback_power),
            )
    }
}

fn fallback_shoulder(x: f32, parameters: &AgxToneMappingParameters) -> f32 {
    if x >= 1.0 {
        parameters.target_white
    } else {
        parameters.target_white
            - native_max(
                0.0,
                parameters.shoulder_fallback_coefficient
                    * (1.0 - x).powf(parameters.shoulder_fallback_power),
            )
    }
}

fn apply_curve(x: f32, parameters: &AgxToneMappingParameters) -> f32 {
    let result = if x < parameters.toe_transition_x {
        if parameters.need_convex_toe {
            fallback_toe(x, parameters)
        } else {
            scaled_sigmoid(
                x,
                parameters.toe_scale,
                parameters.slope,
                parameters.toe_power,
                parameters.toe_transition_x,
                parameters.toe_transition_y,
            )
        }
    } else if x <= parameters.shoulder_transition_x {
        parameters.slope * x + parameters.intercept
    } else if parameters.need_concave_shoulder {
        fallback_shoulder(x, parameters)
    } else {
        scaled_sigmoid(
            x,
            parameters.shoulder_scale,
            parameters.slope,
            parameters.shoulder_power,
            parameters.shoulder_transition_x,
            parameters.shoulder_transition_y,
        )
    };
    native_clamp(result, parameters.target_black, parameters.target_white)
}

fn apply_log_encoding(x: f32, range_in_ev: f32, black_relative_ev: f32) -> f32 {
    let x_relative = native_max(EPSILON, x / MIDDLE_GREY);
    let mapped = (native_max(x_relative, 0.0).log2() - black_relative_ev) / range_in_ev;
    native_clamp(mapped, 0.0, 1.0)
}

fn calculate_slope_matching_power(
    slope: f32,
    dx_transition_to_limit: f32,
    dy_transition_to_limit: f32,
) -> f32 {
    slope * dx_transition_to_limit / dy_transition_to_limit
}

fn calculate_fallback_curve_coefficient(
    dx_transition_to_limit: f32,
    dy_transition_to_limit: f32,
    exponent: f32,
) -> f32 {
    dy_transition_to_limit / dx_transition_to_limit.powf(exponent)
}

fn tone_mapping(
    rgb: &mut [f32; 3],
    parameters: &AgxToneMappingParameters,
    rendering_to_xyz: &AgxMatrix,
) {
    let h_before = if parameters.restore_hue {
        rgb_to_hsv(*rgb)[0]
    } else {
        0.0
    };
    let mut transformed = [0.0_f32; 3];
    for channel in 0..3 {
        let log_value = apply_log_encoding(
            rgb[channel],
            parameters.range_in_ev,
            parameters.black_relative_ev,
        );
        transformed[channel] = apply_curve(log_value, parameters);
    }
    if parameters.look_tuned {
        agx_look(&mut transformed, parameters, rendering_to_xyz);
    }
    for value in &mut transformed {
        *value = native_max(0.0, *value).powf(parameters.curve_gamma);
    }
    if parameters.restore_hue {
        let mut hsv = rgb_to_hsv(transformed);
        hsv[0] = lerp_hue(h_before, hsv[0], parameters.look_original_hue_mix_ratio);
        *rgb = hsv_to_rgb(hsv);
    } else {
        *rgb = transformed;
    }
}

fn agx_look(
    pixel: &mut [f32; 3],
    parameters: &AgxToneMappingParameters,
    rendering_to_xyz: &AgxMatrix,
) {
    let slope = parameters.look_slope;
    let lift = parameters.look_lift;
    let power = parameters.look_power;
    let saturation = parameters.look_saturation;
    let m = slope / (1.0 + lift);
    let b = lift * m;
    for value in pixel.iter_mut() {
        let value_with_slope_and_lift = m.mul_add(*value, b);
        *value = if value_with_slope_and_lift > 0.0 {
            value_with_slope_and_lift.powf(power)
        } else {
            value_with_slope_and_lift
        };
    }
    let luma = luminance_from_matrix(*pixel, rendering_to_xyz);
    for value in pixel.iter_mut() {
        *value = luma + saturation * (*value - luma);
    }
}

fn compress_into_gamut(pixel: &mut [f32; 3]) {
    let luminance_coeffs = [
        0.2658180370250449_f32,
        0.59846986045365_f32,
        0.1357121025213052_f32,
    ];
    let input_y = pixel[0] * luminance_coeffs[0]
        + pixel[1] * luminance_coeffs[1]
        + pixel[2] * luminance_coeffs[2];
    let max_rgb = max3(*pixel);
    let opponent_rgb = [max_rgb - pixel[0], max_rgb - pixel[1], max_rgb - pixel[2]];
    let opponent_y = opponent_rgb[0] * luminance_coeffs[0]
        + opponent_rgb[1] * luminance_coeffs[1]
        + opponent_rgb[2] * luminance_coeffs[2];
    let max_opponent = max3(opponent_rgb);
    let y_compensate_negative = max_opponent - opponent_y + input_y;

    let min_rgb = min3(*pixel);
    let offset = native_max(-min_rgb, 0.0);
    let rgb_offset = [pixel[0] + offset, pixel[1] + offset, pixel[2] + offset];
    let max_of_rgb_offset = max3(rgb_offset);
    let opponent_rgb_offset = [
        max_of_rgb_offset - rgb_offset[0],
        max_of_rgb_offset - rgb_offset[1],
        max_of_rgb_offset - rgb_offset[2],
    ];
    let max_inverse_rgb_offset = max3(opponent_rgb_offset);
    let y_inverse_rgb_offset = opponent_rgb_offset[0] * luminance_coeffs[0]
        + opponent_rgb_offset[1] * luminance_coeffs[1]
        + opponent_rgb_offset[2] * luminance_coeffs[2];
    let mut y_new = rgb_offset[0] * luminance_coeffs[0]
        + rgb_offset[1] * luminance_coeffs[1]
        + rgb_offset[2] * luminance_coeffs[2];
    y_new = max_inverse_rgb_offset - y_inverse_rgb_offset + y_new;
    let luminance_ratio = if y_new > y_compensate_negative && y_new > EPSILON {
        y_compensate_negative / y_new
    } else {
        1.0
    };
    for channel in 0..3 {
        pixel[channel] = luminance_ratio * rgb_offset[channel];
    }
}

fn luminance_from_matrix(pixel: [f32; 3], matrix: &AgxMatrix) -> f32 {
    apply_matrix(pixel, matrix)[1]
}

fn rgb_to_hsv(rgb: [f32; 3]) -> [f32; 3] {
    let min = min3(rgb);
    let max = max3(rgb);
    let delta = max - min;
    let value = max;
    let (saturation, hue) = if max.abs() > EPSILON && delta.abs() > EPSILON {
        (delta / max, rgb_to_hue(rgb, max, delta))
    } else {
        (0.0, 0.0)
    };
    [hue, saturation, value]
}

fn rgb_to_hue(rgb: [f32; 3], max: f32, delta: f32) -> f32 {
    let hue = (if rgb[0] == max {
        (rgb[1] - rgb[2]) / delta
    } else if rgb[1] == max {
        2.0 + (rgb[2] - rgb[0]) / delta
    } else {
        4.0 + (rgb[0] - rgb[1]) / delta
    }) / 6.0;
    hue - hue.floor()
}

fn hsv_to_rgb(hsv: [f32; 3]) -> [f32; 3] {
    let chroma = hsv[1] * hsv[2];
    let min = hsv[2] - chroma;
    let h = hsv[0] * 6.0;
    let i = h.floor();
    let f = h - i;
    let fc = f * chroma;
    let top = chroma + min;
    let inc = fc + min;
    let dec = top - fc;
    match i as usize {
        0 => [top, inc, min],
        1 => [dec, top, min],
        2 => [min, top, inc],
        3 => [min, dec, top],
        4 => [inc, min, top],
        _ => [top, min, dec],
    }
}

fn lerp_hue(original_hue: f32, processed_hue: f32, mix: f32) -> f32 {
    let shortest_distance = remainderf(processed_hue - original_hue, 1.0);
    let mixed_hue = (1.0 - mix).mul_add(shortest_distance, original_hue);
    mixed_hue - mixed_hue.floor()
}

/// `remainderf(x, 1)` with the C nearest-even quotient rule.
fn remainderf(x: f32, divisor: f32) -> f32 {
    let quotient = x / divisor;
    let trunc = quotient.trunc();
    let fraction = quotient - trunc;
    let odd = trunc.rem_euclid(2.0) == 1.0;
    let nearest = if fraction > 0.5 || (fraction == 0.5 && odd) {
        trunc + 1.0
    } else if fraction < -0.5 || (fraction == -0.5 && odd) {
        trunc - 1.0
    } else {
        trunc
    };
    x - nearest * divisor
}

fn sanitise_pixel(pixel: AgxPixel) -> AgxPixel {
    AgxPixel::from_channels(pixel.channels.map(sanitise_component))
}

fn sanitise_component(component: f32) -> f32 {
    if component.is_nan() {
        0.0
    } else {
        native_clamp(component, -INPUT_LIMIT, INPUT_LIMIT)
    }
}

fn profile_is_usable(profile: AgxProfile) -> bool {
    matrix_is_finite(profile.matrix_in_transposed)
        && matrix_is_finite(profile.matrix_out_transposed)
        && invert_matrix(profile.matrix_in_transposed).is_some()
        && invert_matrix(profile.matrix_out_transposed).is_some()
}

fn ensure_plan_finite(plan: &AgxPlan) -> Result<(), AgxPlanError> {
    for matrix in [
        plan.rendering_to_xyz,
        plan.pipe_to_base,
        plan.base_to_rendering,
        plan.rendering_to_pipe,
    ] {
        if !matrix_is_finite(matrix) {
            return Err(AgxPlanError::NonFiniteDerived("profile_matrix"));
        }
    }
    Ok(())
}

fn rotate_and_scale_primary(
    profile: AgxProfile,
    scaling: f32,
    rotation: f32,
    primary_index: usize,
) -> [f32; 2] {
    let dx = profile.primaries[primary_index][0] - profile.whitepoint[0];
    let dy = profile.primaries[primary_index][1] - profile.whitepoint[1];
    let angle = dy.atan2(dx) + rotation;
    let cos_angle = angle.cos();
    let sin_angle = angle.sin();
    let distance_to_edge = find_distance_to_edge(profile, cos_angle, sin_angle);
    [
        scaling * distance_to_edge * cos_angle + profile.whitepoint[0],
        scaling * distance_to_edge * sin_angle + profile.whitepoint[1],
    ]
}

fn find_distance_to_edge(profile: AgxProfile, cos_angle: f32, sin_angle: f32) -> f32 {
    let x1 = profile.whitepoint[0];
    let y1 = profile.whitepoint[1];
    let x2 = x1 + cos_angle;
    let y2 = y1 + sin_angle;
    let mut distance_to_edge = f32::MAX;
    for index in 0..3 {
        let next = if index == 2 { 0 } else { index + 1 };
        let distance = intersect_line_segments(
            x1,
            y1,
            x2,
            y2,
            profile.primaries[index][0],
            profile.primaries[index][1],
            profile.primaries[next][0],
            profile.primaries[next][1],
        );
        if distance < distance_to_edge {
            distance_to_edge = distance;
        }
    }
    distance_to_edge
}

fn determinant(a: f32, b: f32, c: f32, d: f32) -> f32 {
    a * d - b * c
}

fn intersect_line_segments(
    x1: f32,
    y1: f32,
    x2: f32,
    y2: f32,
    x3: f32,
    y3: f32,
    x4: f32,
    y4: f32,
) -> f32 {
    let denominator = determinant(x1 - x2, x3 - x4, y1 - y2, y3 - y4);
    if denominator == 0.0 {
        return f32::MAX;
    }
    let t = determinant(x1 - x3, x3 - x4, y1 - y3, y3 - y4) / denominator;
    if t >= 0.0 { t } else { f32::MAX }
}

fn xyz_colorant_to_xy(xyz: [f32; 3]) -> [f32; 2] {
    let xyz = xyz.map(|component| native_max(component, 0.0));
    let sum = xyz[0] + xyz[1] + xyz[2];
    if sum > 0.0 {
        [xyz[0] / sum, xyz[1] / sum]
    } else {
        // Native `dt_D50_XYZ_to_xyY` uses the source D50 chromaticity for
        // a zero colorant. Standard matrix-shaper profiles never take this
        // branch, but retaining it keeps the extraction helper source-shaped.
        [0.34567, 0.35850]
    }
}

fn matrix_from_primaries(
    primaries: [[f32; 2]; 3],
    whitepoint: [f32; 2],
) -> Result<AgxMatrix, AgxProfileError> {
    let mut primaries_matrix = [[0.0_f32; 3]; 3];
    for index in 0..3 {
        let y = sanitise_y(primaries[index][1]);
        primaries_matrix[index][0] = primaries[index][0] / y;
        primaries_matrix[index][1] = 1.0;
        primaries_matrix[index][2] = (1.0 - primaries[index][0] - y) / y;
    }
    let primaries_inverse =
        invert_matrix(primaries_matrix).ok_or(AgxProfileError::SingularMatrix)?;
    let y = sanitise_y(whitepoint[1]);
    let xyz_white = [whitepoint[0] / y, 1.0, (1.0 - whitepoint[0] - y) / y];
    let scale = apply_matrix(xyz_white, &primaries_inverse);
    let mut output = [[0.0_f32; 3]; 3];
    for index in 0..3 {
        for channel in 0..3 {
            output[index][channel] = scale[index] * primaries_matrix[index][channel];
        }
    }
    if matrix_is_finite(output) {
        Ok(output)
    } else {
        Err(AgxProfileError::NonFinite)
    }
}

fn sanitise_y(y: f32) -> f32 {
    if y < f32::EPSILON && y >= 0.0 {
        f32::EPSILON
    } else if y < 0.0 && y > -f32::EPSILON {
        -f32::EPSILON
    } else {
        y
    }
}

fn matrix_is_finite(matrix: AgxMatrix) -> bool {
    matrix.into_iter().flatten().all(f32::is_finite)
}

fn apply_matrix(input: [f32; 3], matrix: &AgxMatrix) -> [f32; 3] {
    [
        matrix[0][0] * input[0] + matrix[1][0] * input[1] + matrix[2][0] * input[2],
        matrix[0][1] * input[0] + matrix[1][1] * input[1] + matrix[2][1] * input[2],
        matrix[0][2] * input[0] + matrix[1][2] * input[1] + matrix[2][2] * input[2],
    ]
}

fn multiply_matrices(m1: AgxMatrix, m2: AgxMatrix) -> AgxMatrix {
    let mut output = [[0.0_f32; 3]; 3];
    for row in 0..3 {
        for column in 0..3 {
            let mut sum = 0.0_f32;
            for index in 0..3 {
                sum += m1[row][index] * m2[index][column];
            }
            output[row][column] = sum;
        }
    }
    output
}

fn transpose_matrix(source: AgxMatrix) -> AgxMatrix {
    let mut transposed = [[0.0_f32; 3]; 3];
    for row in 0..3 {
        for column in 0..3 {
            transposed[row][column] = source[column][row];
        }
    }
    transposed
}

fn invert_profile_input_matrix(matrix_in_transposed: AgxMatrix) -> Option<AgxMatrix> {
    // Native profile generation inverts the row-major RGB→XYZ matrix and
    // transposes both profile matrices afterwards. Preserve that operation
    // order rather than directly inverting the already-transposed input.
    invert_matrix(transpose_matrix(matrix_in_transposed)).map(transpose_matrix)
}

fn invert_matrix(source: AgxMatrix) -> Option<AgxMatrix> {
    let det = source[0][0] * (source[2][2] * source[1][1] - source[2][1] * source[1][2])
        - source[1][0] * (source[2][2] * source[0][1] - source[2][1] * source[0][2])
        + source[2][0] * (source[1][2] * source[0][1] - source[1][1] * source[0][2]);
    if det.abs() < MATRIX_EPSILON {
        return None;
    }
    let inverse_det = 1.0 / det;
    Some([
        [
            inverse_det * (source[2][2] * source[1][1] - source[2][1] * source[1][2]),
            -inverse_det * (source[2][2] * source[0][1] - source[2][1] * source[0][2]),
            inverse_det * (source[1][2] * source[0][1] - source[1][1] * source[0][2]),
        ],
        [
            -inverse_det * (source[2][2] * source[1][0] - source[2][0] * source[1][2]),
            inverse_det * (source[2][2] * source[0][0] - source[2][0] * source[0][2]),
            -inverse_det * (source[1][2] * source[0][0] - source[1][0] * source[0][2]),
        ],
        [
            inverse_det * (source[2][1] * source[1][0] - source[2][0] * source[1][1]),
            -inverse_det * (source[2][1] * source[0][0] - source[2][0] * source[0][1]),
            inverse_det * (source[1][1] * source[0][0] - source[1][0] * source[0][1]),
        ],
    ])
}

fn max3(values: [f32; 3]) -> f32 {
    native_max(native_max(values[0], values[1]), values[2])
}

fn min3(values: [f32; 3]) -> f32 {
    native_min(native_min(values[0], values[1]), values[2])
}

fn native_max(left: f32, right: f32) -> f32 {
    if left.is_nan() {
        right
    } else if right.is_nan() {
        left
    } else if left > right {
        left
    } else if right > left {
        right
    } else if left == 0.0 && right == 0.0 {
        if left.is_sign_negative() && right.is_sign_negative() {
            left
        } else {
            0.0
        }
    } else {
        left
    }
}

fn native_min(left: f32, right: f32) -> f32 {
    if left.is_nan() {
        right
    } else if right.is_nan() {
        left
    } else if left < right {
        left
    } else if right < left {
        right
    } else if left == 0.0 && right == 0.0 {
        if left.is_sign_negative() || right.is_sign_negative() {
            -0.0
        } else {
            left
        }
    } else {
        left
    }
}

fn native_clamp(value: f32, minimum: f32, maximum: f32) -> f32 {
    if value >= minimum {
        if value <= maximum { value } else { maximum }
    } else {
        minimum
    }
}

fn set_default_curve_and_look_params(parameters: &mut AgxParametersV7) {
    parameters.look_slope = 1.0;
    parameters.look_brightness = 1.0;
    parameters.look_lift = 0.0;
    parameters.look_saturation = 1.0;
    parameters.look_original_hue_mix_ratio = 0.6;
    parameters.range_black_relative_ev = -10.0;
    parameters.range_white_relative_ev = 6.5;
    parameters.dynamic_range_scaling = 0.1;
    parameters.curve_contrast_around_pivot = 3.0;
    parameters.curve_linear_ratio_below_pivot = 0.0;
    parameters.curve_linear_ratio_above_pivot = 0.0;
    parameters.curve_toe_power = 1.5;
    parameters.curve_shoulder_power = 3.3;
    parameters.curve_target_display_black_ratio = 0.0;
    parameters.curve_target_display_white_ratio = 1.0;
    parameters.auto_gamma = 0;
    parameters.curve_gamma = DEFAULT_GAMMA;
    parameters.curve_pivot_x = -parameters.range_black_relative_ev
        / (parameters.range_white_relative_ev - parameters.range_black_relative_ev);
    parameters.curve_pivot_y_linear_output = 0.18;
}

fn set_unmodified_primaries(parameters: &mut AgxParametersV7) {
    parameters.disable_primaries_adjustments = 0;
    parameters.completely_reverse_primaries = 0;
    parameters.base_primaries = AgxBasePrimaries::Rec2020;
    parameters.red_inset = 0.0;
    parameters.red_rotation = 0.0;
    parameters.green_inset = 0.0;
    parameters.green_rotation = 0.0;
    parameters.blue_inset = 0.0;
    parameters.blue_rotation = 0.0;
    parameters.master_outset_ratio = 1.0;
    parameters.master_unrotation_ratio = 1.0;
    parameters.red_outset = 0.0;
    parameters.red_unrotation = 0.0;
    parameters.green_outset = 0.0;
    parameters.green_unrotation = 0.0;
    parameters.blue_outset = 0.0;
    parameters.blue_unrotation = 0.0;
}

fn set_blenderlike_primaries(parameters: &mut AgxParametersV7) {
    parameters.disable_primaries_adjustments = 0;
    parameters.completely_reverse_primaries = 0;
    parameters.base_primaries = AgxBasePrimaries::Rec2020;
    parameters.red_inset = 0.29462451;
    parameters.green_inset = 0.25861925;
    parameters.blue_inset = 0.14641371;
    parameters.red_rotation = 0.03540329;
    parameters.green_rotation = -0.02108586;
    parameters.blue_rotation = -0.06305724;
    parameters.master_outset_ratio = 1.0;
    parameters.master_unrotation_ratio = 0.0;
    parameters.red_outset = 0.290776401758;
    parameters.green_outset = 0.263155400753;
    parameters.blue_outset = 0.045810721815;
    parameters.red_unrotation = parameters.red_rotation;
    parameters.green_unrotation = parameters.green_rotation;
    parameters.blue_unrotation = parameters.blue_rotation;
}

fn deg2radf(degrees: f32) -> f32 {
    degrees * std::f32::consts::PI / 180.0
}

fn set_smooth_primaries(parameters: &mut AgxParametersV7) {
    parameters.disable_primaries_adjustments = 0;
    parameters.completely_reverse_primaries = 0;
    parameters.base_primaries = AgxBasePrimaries::WorkingProfile;
    parameters.red_inset = 0.1;
    parameters.green_inset = 0.1;
    parameters.blue_inset = 0.15;
    parameters.red_rotation = deg2radf(2.0);
    parameters.green_rotation = deg2radf(-1.0);
    parameters.blue_rotation = deg2radf(-3.0);
    parameters.master_outset_ratio = 0.0;
    parameters.red_outset = parameters.red_inset;
    parameters.green_outset = parameters.green_inset;
    parameters.blue_outset = parameters.blue_inset;
    parameters.master_unrotation_ratio = 1.0;
    parameters.red_unrotation = parameters.red_rotation;
    parameters.green_unrotation = parameters.green_rotation;
    parameters.blue_unrotation = parameters.blue_rotation;
}

fn write_f32<const N: usize>(bytes: &mut [u8; N], offset: &mut usize, value: f32) {
    bytes[*offset..*offset + 4].copy_from_slice(&value.to_le_bytes());
    *offset += 4;
}

fn write_i32<const N: usize>(bytes: &mut [u8; N], offset: &mut usize, value: i32) {
    bytes[*offset..*offset + 4].copy_from_slice(&value.to_le_bytes());
    *offset += 4;
}

fn read_f32(bytes: &[u8], offset: &mut usize) -> f32 {
    let value = f32::from_le_bytes(
        bytes[*offset..*offset + 4]
            .try_into()
            .expect("payload length was checked"),
    );
    *offset += 4;
    value
}

fn read_i32(bytes: &[u8], offset: &mut usize) -> i32 {
    let value = i32::from_le_bytes(
        bytes[*offset..*offset + 4]
            .try_into()
            .expect("payload length was checked"),
    );
    *offset += 4;
    value
}
