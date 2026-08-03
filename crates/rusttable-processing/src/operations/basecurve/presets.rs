//! Native Basecurve preset tables and matching from `src/iop/basecurve.c`.

#![allow(
    clippy::unreadable_literal,
    clippy::too_many_arguments,
    clippy::struct_excessive_bools,
    reason = "camera preset coordinates and native metadata are transcribed source constants"
)]

use super::{
    BasecurveNode, BasecurveParameters, CUBIC_SPLINE, DT_RGB_NORM_LUMINANCE, MAX_NODES,
    MONOTONE_HERMITE,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BasecurveBlendColorspace {
    RgbDisplay,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BasecurvePreset {
    pub name: &'static str,
    pub maker: &'static str,
    pub model: &'static str,
    pub iso_min: i32,
    pub iso_max: f32,
    pub parameters: BasecurveParameters,
    pub filter: bool,
    pub camera: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BasecurvePresetRegistration {
    pub name: String,
    pub source_name: &'static str,
    pub parameters: Option<BasecurveParameters>,
    pub maker: &'static str,
    pub model: &'static str,
    pub iso_min: i32,
    pub iso_max: f32,
    pub camera: bool,
    pub filtered: bool,
    pub raw_only: bool,
    pub auto_apply: bool,
    pub blend_colorspace: BasecurveBlendColorspace,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BasecurveCameraMetadata<'a> {
    pub exif_maker: &'a str,
    pub exif_model: &'a str,
    pub camera_maker: &'a str,
    pub camera_alias: &'a str,
}

fn parameters(points: &[(f32, f32)], curve_type: i32) -> BasecurveParameters {
    let mut value = BasecurveParameters::defaults();
    value.basecurve[0] = [BasecurveNode::new(0.0, 0.0); MAX_NODES];
    for (index, &(x, y)) in points.iter().enumerate() {
        value.basecurve[0][index] = BasecurveNode::new(x, y);
    }
    value.basecurve_nodes = [
        i32::try_from(points.len()).expect("native preset fits i32"),
        0,
        0,
    ];
    value.basecurve_type = [curve_type, 0, 0];
    value.preserve_colors = DT_RGB_NORM_LUMINANCE;
    // Native preset declarations leave the exposure fields zero and
    // set_presets() supplies these compatibility defaults.
    value.exposure_stops = 0.0;
    value.exposure_bias = 0.0;
    value
}

fn preset(
    name: &'static str,
    maker: &'static str,
    model: &'static str,
    points: &[(f32, f32)],
    curve_type: i32,
    filter: bool,
) -> BasecurvePreset {
    BasecurvePreset {
        name,
        maker,
        model,
        iso_min: 0,
        iso_max: f32::MAX,
        parameters: parameters(points, curve_type),
        filter,
        camera: false,
    }
}

fn camera_preset(
    name: &'static str,
    maker: &'static str,
    model: &'static str,
    points: &[(f32, f32)],
) -> BasecurvePreset {
    let mut value = preset(name, maker, model, points, MONOTONE_HERMITE, true);
    value.camera = true;
    value
}

/// The 18 native generic built-in entries, in source order.
#[expect(
    clippy::too_many_lines,
    reason = "Native Basecurve preset declarations remain in source order as one auditable table."
)]
pub fn basecurve_presets() -> Vec<BasecurvePreset> {
    vec![
        preset(
            "cubic spline",
            "",
            "",
            &[(0.0, 0.0), (1.0, 1.0)],
            CUBIC_SPLINE,
            false,
        ),
        preset(
            "neutral",
            "",
            "",
            &[
                (0.0, 0.0),
                (0.005, 0.0025),
                (0.15, 0.3),
                (0.4, 0.7),
                (0.75, 0.95),
                (1.0, 1.0),
            ],
            MONOTONE_HERMITE,
            true,
        ),
        preset(
            "canon eos like",
            "Canon",
            "",
            &[
                (0.0, 0.0),
                (0.028226, 0.029677),
                (0.120968, 0.232258),
                (0.459677, 0.747581),
                (0.858871, 0.967742),
                (1.0, 1.0),
            ],
            MONOTONE_HERMITE,
            false,
        ),
        preset(
            "canon eos like alternate",
            "Canon",
            "EOS 5D Mark%",
            &[
                (0.0, 0.0),
                (0.026210, 0.029677),
                (0.108871, 0.232258),
                (0.350806, 0.747581),
                (0.669355, 0.967742),
                (1.0, 1.0),
            ],
            MONOTONE_HERMITE,
            false,
        ),
        preset(
            "nikon like",
            "NIKON",
            "",
            &[
                (0.0, 0.0),
                (0.036290, 0.036532),
                (0.120968, 0.228226),
                (0.459677, 0.759678),
                (0.858871, 0.983468),
                (1.0, 1.0),
            ],
            MONOTONE_HERMITE,
            false,
        ),
        preset(
            "nikon like alternate",
            "NIKON",
            "%D____%",
            &[
                (0.0, 0.0),
                (0.012097, 0.007322),
                (0.072581, 0.130742),
                (0.310484, 0.729291),
                (0.611321, 0.951613),
                (1.0, 1.0),
            ],
            MONOTONE_HERMITE,
            false,
        ),
        preset(
            "sony alpha like",
            "SONY",
            "",
            &[
                (0.0, 0.0),
                (0.031949, 0.036532),
                (0.105431, 0.228226),
                (0.434505, 0.759678),
                (0.855738, 0.983468),
                (1.0, 1.0),
            ],
            MONOTONE_HERMITE,
            false,
        ),
        preset(
            "pentax like",
            "PENTAX",
            "",
            &[
                (0.0, 0.0),
                (0.032258, 0.024596),
                (0.120968, 0.166419),
                (0.205645, 0.328527),
                (0.604839, 0.790171),
                (1.0, 1.0),
            ],
            MONOTONE_HERMITE,
            false,
        ),
        preset(
            "ricoh like",
            "RICOH",
            "",
            &[
                (0.0, 0.0),
                (0.032259, 0.024596),
                (0.120968, 0.166419),
                (0.205645, 0.328527),
                (0.604839, 0.790171),
                (1.0, 1.0),
            ],
            MONOTONE_HERMITE,
            false,
        ),
        preset(
            "olympus like",
            "OLYMPUS",
            "",
            &[
                (0.0, 0.0),
                (0.033962, 0.028226),
                (0.249057, 0.439516),
                (0.501887, 0.798387),
                (0.750943, 0.955645),
                (1.0, 1.0),
            ],
            MONOTONE_HERMITE,
            false,
        ),
        preset(
            "olympus like alternate",
            "OLYMPUS",
            "E-M%",
            &[
                (0.0, 0.0),
                (0.012097, 0.010322),
                (0.072581, 0.167742),
                (0.310484, 0.711291),
                (0.645161, 0.956855),
                (1.0, 1.0),
            ],
            MONOTONE_HERMITE,
            false,
        ),
        preset(
            "panasonic like",
            "Panasonic",
            "",
            &[
                (0.0, 0.0),
                (0.036290, 0.024596),
                (0.120968, 0.166419),
                (0.205645, 0.328527),
                (0.604839, 0.790171),
                (1.0, 1.0),
            ],
            MONOTONE_HERMITE,
            false,
        ),
        preset(
            "leica like",
            "Leica",
            "",
            &[
                (0.0, 0.0),
                (0.036291, 0.024596),
                (0.120968, 0.166419),
                (0.205645, 0.328527),
                (0.604839, 0.790171),
                (1.0, 1.0),
            ],
            MONOTONE_HERMITE,
            false,
        ),
        preset(
            "kodak easyshare like",
            "EASTMAN KODAK COMPANY",
            "",
            &[
                (0.0, 0.0),
                (0.044355, 0.020967),
                (0.133065, 0.154322),
                (0.209677, 0.300301),
                (0.572581, 0.753477),
                (1.0, 1.0),
            ],
            MONOTONE_HERMITE,
            false,
        ),
        preset(
            "konica minolta like",
            "MINOLTA",
            "",
            &[
                (0.0, 0.0),
                (0.020161, 0.010322),
                (0.112903, 0.167742),
                (0.5, 0.711291),
                (0.899194, 0.956855),
                (1.0, 1.0),
            ],
            MONOTONE_HERMITE,
            false,
        ),
        preset(
            "samsung like",
            "SAMSUNG",
            "",
            &[
                (0.0, 0.0),
                (0.040323, 0.029677),
                (0.133065, 0.232258),
                (0.447581, 0.747581),
                (0.842742, 0.967742),
                (1.0, 1.0),
            ],
            MONOTONE_HERMITE,
            false,
        ),
        preset(
            "fujifilm like",
            "FUJIFILM",
            "",
            &[
                (0.0, 0.0),
                (0.028226, 0.029677),
                (0.104839, 0.232258),
                (0.387097, 0.747581),
                (0.754032, 0.967742),
                (1.0, 1.0),
            ],
            MONOTONE_HERMITE,
            false,
        ),
        preset(
            "nokia like",
            "Nokia",
            "",
            &[
                (0.0, 0.0),
                (0.041825, 0.020161),
                (0.117871, 0.153226),
                (0.319392, 0.5),
                (0.638783, 0.842742),
                (1.0, 1.0),
            ],
            MONOTONE_HERMITE,
            false,
        ),
    ]
}

/// The 14 native camera entries, in source order.
#[expect(
    clippy::too_many_lines,
    reason = "Native Basecurve camera preset declarations remain in source order as one auditable table."
)]
pub fn basecurve_camera_presets() -> Vec<BasecurvePreset> {
    vec![
        camera_preset(
            "Nikon D750",
            "NIKON CORPORATION",
            "NIKON D750",
            &[
                (0.0, 0.0),
                (0.018124, 0.026126),
                (0.143357, 0.370145),
                (0.330116, 0.730507),
                (0.457952, 0.853462),
                (0.734950, 0.965061),
                (0.904758, 0.985699),
                (1.0, 1.0),
            ],
        ),
        camera_preset(
            "Nikon D5100",
            "NIKON CORPORATION",
            "NIKON D5100",
            &[
                (0.0, 0.0),
                (0.001113, 0.000506),
                (0.002842, 0.001338),
                (0.005461, 0.002470),
                (0.011381, 0.006099),
                (0.013303, 0.007758),
                (0.034638, 0.041119),
                (0.044441, 0.063882),
                (0.070338, 0.139639),
                (0.096068, 0.210915),
                (0.137693, 0.310295),
                (0.206041, 0.432674),
                (0.255508, 0.504447),
                (0.302770, 0.569576),
                (0.425625, 0.726755),
                (0.554526, 0.839541),
                (0.621216, 0.882839),
                (0.702662, 0.927072),
                (0.897426, 0.990984),
                (1.0, 1.0),
            ],
        ),
        camera_preset(
            "Nikon D7000",
            "NIKON CORPORATION",
            "NIKON D7000",
            &[
                (0.0, 0.0),
                (0.001943, 0.003040),
                (0.019814, 0.028810),
                (0.080784, 0.210476),
                (0.145700, 0.383873),
                (0.295961, 0.654041),
                (0.651915, 0.952819),
                (1.0, 1.0),
            ],
        ),
        camera_preset(
            "Nikon D7200",
            "NIKON CORPORATION",
            "NIKON D7200",
            &[
                (0.0, 0.0),
                (0.001604, 0.001334),
                (0.007401, 0.005237),
                (0.009474, 0.006890),
                (0.017348, 0.017176),
                (0.032782, 0.044336),
                (0.048033, 0.086548),
                (0.075803, 0.168331),
                (0.109539, 0.273539),
                (0.137373, 0.364645),
                (0.231651, 0.597511),
                (0.323797, 0.736475),
                (0.383796, 0.805797),
                (0.462284, 0.872247),
                (0.549844, 0.918328),
                (0.678855, 0.962361),
                (0.817445, 0.990406),
                (1.0, 1.0),
            ],
        ),
        camera_preset(
            "NIKON D7500",
            "NIKON CORPORATION",
            "NIKON D7500",
            &[
                (0.0, 0.0),
                (0.000892, 0.001062),
                (0.002280, 0.001768),
                (0.013983, 0.011368),
                (0.032597, 0.044700),
                (0.050065, 0.097131),
                (0.084129, 0.219954),
                (0.120975, 0.336806),
                (0.170730, 0.473752),
                (0.258677, 0.647113),
                (0.409997, 0.827417),
                (0.499979, 0.889468),
                (0.615564, 0.941960),
                (0.665272, 0.957736),
                (0.832126, 0.991968),
                (1.0, 1.0),
            ],
        ),
        camera_preset(
            "Sony DSC-RX100M2",
            "SONY",
            "DSC-RX100M2",
            &[
                (0.0, 0.0),
                (0.015106, 0.008116),
                (0.070077, 0.093725),
                (0.107484, 0.170723),
                (0.191528, 0.341093),
                (0.257996, 0.458453),
                (0.305381, 0.537267),
                (0.326367, 0.569257),
                (0.448067, 0.723742),
                (0.509627, 0.777966),
                (0.676751, 0.898797),
                (1.0, 1.0),
            ],
        ),
        camera_preset(
            "Canon EOS 6D",
            "Canon",
            "Canon EOS 6D",
            &[
                (0.0, 0.002917),
                (0.000751, 0.001716),
                (0.006011, 0.004438),
                (0.020286, 0.021725),
                (0.048084, 0.085918),
                (0.093914, 0.233804),
                (0.162284, 0.431375),
                (0.257701, 0.629218),
                (0.384673, 0.800332),
                (0.547709, 0.917761),
                (0.751315, 0.988132),
                (1.0, 0.999943),
            ],
        ),
        camera_preset(
            "Fujifilm X100S",
            "Fujifilm",
            "X100S",
            &[
                (0.0, 0.0),
                (0.009145, 0.007905),
                (0.026570, 0.032201),
                (0.131526, 0.289717),
                (0.175858, 0.395263),
                (0.350981, 0.696899),
                (0.614997, 0.959451),
                (1.0, 1.0),
            ],
        ),
        camera_preset(
            "Fujifilm X100T",
            "Fujifilm",
            "X100T",
            &[
                (0.0, 0.0),
                (0.009145, 0.007905),
                (0.026570, 0.032201),
                (0.131526, 0.289717),
                (0.175858, 0.395263),
                (0.350981, 0.696899),
                (0.614997, 0.959451),
                (1.0, 1.0),
            ],
        ),
        camera_preset(
            "Canon EOS 5D Mark II",
            "Canon",
            "Canon EOS 5D Mark II",
            &[
                (0.0, 0.000366),
                (0.006560, 0.003504),
                (0.027310, 0.029834),
                (0.045915, 0.070230),
                (0.206554, 0.539895),
                (0.442337, 0.872409),
                (0.673263, 0.971703),
                (1.0, 0.999832),
            ],
        ),
        camera_preset(
            "Pentax K-5",
            "Pentax",
            "Pentax K-5",
            &[
                (0.0, 0.0),
                (0.004754, 0.002208),
                (0.009529, 0.004214),
                (0.023713, 0.013508),
                (0.031866, 0.020352),
                (0.046734, 0.034063),
                (0.059989, 0.052413),
                (0.088415, 0.096030),
                (0.136610, 0.190629),
                (0.174480, 0.256484),
                (0.205192, 0.307430),
                (0.228896, 0.348447),
                (0.286411, 0.428680),
                (0.355314, 0.513527),
                (0.440014, 0.607651),
                (0.567096, 0.732791),
                (0.620597, 0.775968),
                (0.760355, 0.881828),
                (0.875139, 0.960682),
                (1.0, 1.0),
            ],
        ),
        camera_preset(
            "Nikon D90",
            "NIKON CORPORATION",
            "NIKON D90",
            &[
                (0.0, 0.0),
                (0.011702, 0.012659),
                (0.122918, 0.289973),
                (0.153642, 0.342731),
                (0.246855, 0.510114),
                (0.448958, 0.733820),
                (0.666759, 0.894290),
                (1.0, 1.0),
            ],
        ),
        camera_preset(
            "Nikon D800",
            "NIKON",
            "NIKON D800",
            &[
                (0.0, 0.0),
                (0.001773, 0.001936),
                (0.009671, 0.009693),
                (0.016754, 0.020617),
                (0.024884, 0.037309),
                (0.048174, 0.107768),
                (0.056932, 0.139532),
                (0.085504, 0.233303),
                (0.130378, 0.349747),
                (0.155476, 0.405445),
                (0.175245, 0.445918),
                (0.217657, 0.516873),
                (0.308475, 0.668608),
                (0.375381, 0.754058),
                (0.459858, 0.839909),
                (0.509567, 0.881543),
                (0.654394, 0.960877),
                (0.783380, 0.999161),
                (0.859310, 1.0),
                (1.0, 1.0),
            ],
        ),
        camera_preset(
            "Olympus OM-D E-M10 II",
            "OLYMPUS CORPORATION    ",
            "E-M10MarkII     ",
            &[
                (0.0, 0.0),
                (0.005707, 0.004764),
                (0.018944, 0.024456),
                (0.054501, 0.129992),
                (0.075665, 0.211873),
                (0.119641, 0.365771),
                (0.173148, 0.532024),
                (0.247979, 0.668989),
                (0.357597, 0.780138),
                (0.459003, 0.839829),
                (0.626844, 0.904426),
                (0.769425, 0.948541),
                (0.820429, 0.964715),
                (1.0, 1.0),
            ],
        ),
    ]
}

fn normalized(mut value: BasecurveParameters) -> BasecurveParameters {
    if value.exposure_fusion == 0 && value.exposure_stops == 0.0 {
        value.exposure_stops = 1.0;
        value.exposure_bias = 1.0;
    }
    value
}

/// Source `_match`: case-insensitive regular-expression matching anchored at
/// the beginning after translating SQL `%` to regex `*` and `_` to `.`.
///
/// `G_REGEX_MATCH_ANCHORED` does not add an end anchor. A successful match may
/// therefore consume only a prefix, and an empty pattern matches every value.
pub fn match_pattern(value: &str, pattern: &str) -> bool {
    let value: Vec<char> = value.chars().flat_map(char::to_lowercase).collect();
    let pattern: Vec<char> = pattern
        .chars()
        .map(|character| match character {
            '%' => '*',
            '_' => '.',
            other => other,
        })
        .flat_map(char::to_lowercase)
        .collect();
    let Some(pattern) = RegexPrefix::new(&pattern) else {
        return false;
    };
    pattern.matches_prefix(&value)
}

#[derive(Debug, Clone, Copy)]
enum RegexAtom {
    Literal(char),
    Any,
}

#[derive(Debug, Clone, Copy)]
struct RegexToken {
    atom: RegexAtom,
    repeated: bool,
}

#[derive(Debug)]
struct RegexPrefix {
    tokens: Vec<RegexToken>,
}

impl RegexPrefix {
    fn new(pattern: &[char]) -> Option<Self> {
        let mut tokens: Vec<RegexToken> = Vec::new();
        for &character in pattern {
            if character == '*' {
                let token = tokens.last_mut()?;
                if token.repeated {
                    return None;
                }
                token.repeated = true;
            } else {
                tokens.push(RegexToken {
                    atom: if character == '.' {
                        RegexAtom::Any
                    } else {
                        RegexAtom::Literal(character)
                    },
                    repeated: false,
                });
            }
        }
        Some(Self { tokens })
    }

    fn matches_prefix(&self, value: &[char]) -> bool {
        let mut states = vec![false; self.tokens.len() + 1];
        states[0] = true;
        close_repeated_states(&mut states, &self.tokens);
        if states[self.tokens.len()] {
            return true;
        }

        for &character in value {
            let mut next = vec![false; self.tokens.len() + 1];
            for (index, (&active, token)) in states.iter().zip(&self.tokens).enumerate() {
                if !active || !atom_matches(token.atom, character) {
                    continue;
                }
                if token.repeated {
                    next[index] = true;
                } else {
                    next[index + 1] = true;
                }
            }
            close_repeated_states(&mut next, &self.tokens);
            if next[self.tokens.len()] {
                return true;
            }
            if !next.iter().any(|active| *active) {
                return false;
            }
            states = next;
        }
        false
    }
}

fn close_repeated_states(states: &mut [bool], tokens: &[RegexToken]) {
    for index in 0..tokens.len() {
        if states[index] && tokens[index].repeated {
            states[index + 1] = true;
        }
    }
}

const fn atom_matches(atom: RegexAtom, character: char) -> bool {
    match atom {
        RegexAtom::Literal(expected) => expected == character,
        RegexAtom::Any => true,
    }
}

/// Native reverse-specificity selection. The source's `k > 0` intentionally
/// means the first entry is never selected by this helper.
pub fn check_camera(
    image: BasecurveCameraMetadata<'_>,
    presets: &[BasecurvePreset],
) -> Option<BasecurveParameters> {
    for index in (1..presets.len()).rev() {
        let preset = &presets[index];
        let exif_match = match_pattern(image.exif_maker, preset.maker)
            && match_pattern(image.exif_model, preset.model);
        let camera_match = match_pattern(image.camera_maker, preset.maker)
            && match_pattern(image.camera_alias, preset.model);
        if exif_match || camera_match {
            return Some(normalized(preset.parameters));
        }
    }
    None
}

/// Native `reload_defaults()` ordering and state transitions.
///
/// The native function mutates the module's existing `default_params`. When
/// no matching preset is found it deliberately leaves that value unchanged;
/// the `current` argument carries that state across this pure transition.
pub fn reload_defaults(
    current: BasecurveParameters,
    multi_priority: i32,
    image: Option<BasecurveCameraMetadata<'_>>,
    autoapply_percamera: bool,
) -> BasecurveParameters {
    if multi_priority != 0 {
        return normalized(basecurve_presets()[0].parameters);
    }
    let Some(image) = image else {
        return current;
    };
    if autoapply_percamera
        && let Some(parameters) = check_camera(image, &basecurve_camera_presets())
    {
        return parameters;
    }
    if let Some(parameters) = check_camera(image, &basecurve_presets()) {
        return parameters;
    }
    current
}

/// Source `init_presets()` registrations, including the conditional default.
pub fn init_presets(display_referred: bool) -> Vec<BasecurvePresetRegistration> {
    let mut registrations = Vec::new();
    for preset in basecurve_presets()
        .into_iter()
        .chain(basecurve_camera_presets())
    {
        registrations.push(BasecurvePresetRegistration {
            name: if preset.camera {
                preset.name.to_owned()
            } else {
                format!("_builtin_{}", preset.name)
            },
            source_name: preset.name,
            parameters: Some(normalized(preset.parameters)),
            maker: preset.maker,
            model: preset.model,
            iso_min: preset.iso_min,
            iso_max: preset.iso_max,
            camera: preset.camera,
            filtered: preset.camera || preset.filter,
            raw_only: true,
            auto_apply: false,
            blend_colorspace: BasecurveBlendColorspace::RgbDisplay,
        });
    }
    if display_referred {
        registrations.push(BasecurvePresetRegistration {
            name: "_builtin_display-referred default".to_owned(),
            source_name: "display-referred default",
            parameters: None,
            maker: "",
            model: "",
            iso_min: 0,
            iso_max: f32::MAX,
            camera: false,
            filtered: false,
            raw_only: true,
            auto_apply: true,
            blend_colorspace: BasecurveBlendColorspace::RgbDisplay,
        });
    }
    registrations
}
