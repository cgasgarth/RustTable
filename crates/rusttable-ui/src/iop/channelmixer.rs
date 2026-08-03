//! Source-shaped GTK4 editor leaf for Darktable `src/iop/channelmixer.c`.
//!
//! Darktable's deprecated Channel Mixer editor owns one transient destination
//! selector and three sliders over a persisted 3x7 matrix. This leaf keeps the
//! complete matrix and algorithm in its action payload, while the selector is
//! deliberately GUI-only. It is not registered in the `RustTable` module
//! registry yet; callers must keep the operation fail-closed until processing,
//! import, and shared routing are qualified.

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use gtk4::prelude::*;
use rusttable_core::{OperationId, Revision};

use crate::bauhaus::{
    combobox::{BauhausComboBox, BauhausComboBoxSpec},
    slider_input::{BauhausSlider, FullWidthSliderSpec, SliderInputSpec, full_width_slider},
};

/// Stable Darktable operation name from `src/iop/channelmixer.c::name`.
pub const CHANNEL_MIXER_MODULE_ID: &str = "channelmixer";
/// Native module title.
pub const CHANNEL_MIXER_TITLE: &str = "channel mixer";
/// Native deprecation notice. This leaf does not replace the operation with
/// Color Calibration.
pub const CHANNEL_MIXER_DEPRECATED_MESSAGE: &str =
    "this module is deprecated. please use the color calibration module instead.";
/// Native description from `src/iop/channelmixer.c::description`.
pub const CHANNEL_MIXER_DESCRIPTION: &str = "perform color space corrections\nsuch as white balance, channels mixing\nand conversions to monochrome emulating film";
/// Native group ordering from `default_group()`.
pub const CHANNEL_MIXER_GROUP_KEYS: [&str; 2] = ["group.color", "group.grading"];
/// The shared module registry intentionally does not expose this leaf yet.
pub const CHANNEL_MIXER_PRODUCTION_ROUTING_INTEGRATED: bool = false;

/// Persisted destination indices from the native parameter arrays.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(usize)]
pub enum ChannelMixerDestination {
    Hue = 0,
    Saturation = 1,
    Lightness = 2,
    Red = 3,
    Green = 4,
    Blue = 5,
    Gray = 6,
}

impl ChannelMixerDestination {
    /// All seven native destinations in persisted index order.
    pub const ALL: [Self; 7] = [
        Self::Hue,
        Self::Saturation,
        Self::Lightness,
        Self::Red,
        Self::Green,
        Self::Blue,
        Self::Gray,
    ];

    #[must_use]
    pub const fn index(self) -> usize {
        self as usize
    }

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Hue => "hue",
            Self::Saturation => "saturation",
            Self::Lightness => "lightness",
            Self::Red => "red",
            Self::Green => "green",
            Self::Blue => "blue",
            Self::Gray => "gray",
        }
    }

    #[must_use]
    pub const fn from_index(index: usize) -> Option<Self> {
        match index {
            0 => Some(Self::Hue),
            1 => Some(Self::Saturation),
            2 => Some(Self::Lightness),
            3 => Some(Self::Red),
            4 => Some(Self::Green),
            5 => Some(Self::Blue),
            6 => Some(Self::Gray),
            _ => None,
        }
    }
}

/// The three source-channel sliders in native order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(usize)]
pub enum ChannelMixerInput {
    Red = 0,
    Green = 1,
    Blue = 2,
}

impl ChannelMixerInput {
    /// All source channels in the native slider order.
    pub const ALL: [Self; 3] = [Self::Red, Self::Green, Self::Blue];

    #[must_use]
    pub const fn index(self) -> usize {
        self as usize
    }

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Red => "red",
            Self::Green => "green",
            Self::Blue => "blue",
        }
    }
}

/// Native algorithm enum retained in the complete persisted payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum ChannelMixerAlgorithm {
    V1 = 0,
    V2 = 1,
}

/// The exact native 21-float matrix plus algorithm field.
///
/// The three rows are laid out as native `red[7]`, `green[7]`, and `blue[7]`.
/// This UI-owned type deliberately keeps all seven destinations even though
/// the editor shows one destination at a time.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChannelMixerParameters {
    red: [f32; 7],
    green: [f32; 7],
    blue: [f32; 7],
    algorithm: ChannelMixerAlgorithm,
}

impl ChannelMixerParameters {
    const fn from_parts(
        red: [f32; 7],
        green: [f32; 7],
        blue: [f32; 7],
        algorithm: ChannelMixerAlgorithm,
    ) -> Self {
        Self {
            red,
            green,
            blue,
            algorithm,
        }
    }

    /// Native v2 RGB identity defaults.
    #[must_use]
    pub const fn defaults() -> Self {
        Self::from_parts(
            [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0],
            [0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0],
            ChannelMixerAlgorithm::V2,
        )
    }

    /// Builds parameters while rejecting non-finite persisted values.
    ///
    /// Native commit code does not clamp finite values. This constructor only
    /// rejects values that cannot safely participate in a typed editor state.
    ///
    /// # Errors
    ///
    /// Returns [`ChannelMixerEditorError::NonFiniteValue`] when any matrix
    /// entry is not finite.
    pub fn try_from_rows(
        red: [f32; 7],
        green: [f32; 7],
        blue: [f32; 7],
        algorithm: ChannelMixerAlgorithm,
    ) -> Result<Self, ChannelMixerEditorError> {
        let parameters = Self::from_parts(red, green, blue, algorithm);
        parameters.validate()?;
        Ok(parameters)
    }

    /// Returns the complete native red row.
    #[must_use]
    pub const fn red(self) -> [f32; 7] {
        self.red
    }

    /// Returns the complete native green row.
    #[must_use]
    pub const fn green(self) -> [f32; 7] {
        self.green
    }

    /// Returns the complete native blue row.
    #[must_use]
    pub const fn blue(self) -> [f32; 7] {
        self.blue
    }

    /// Returns the persisted algorithm without exposing a GUI control.
    #[must_use]
    pub const fn algorithm(self) -> ChannelMixerAlgorithm {
        self.algorithm
    }

    /// Returns one complete source-channel row.
    #[must_use]
    pub const fn row(self, input: ChannelMixerInput) -> [f32; 7] {
        match input {
            ChannelMixerInput::Red => self.red,
            ChannelMixerInput::Green => self.green,
            ChannelMixerInput::Blue => self.blue,
        }
    }

    /// Returns one matrix value at a native destination index.
    #[must_use]
    pub const fn value(
        self,
        input: ChannelMixerInput,
        destination: ChannelMixerDestination,
    ) -> f32 {
        self.row(input)[destination.index()]
    }

    fn validate(self) -> Result<(), ChannelMixerEditorError> {
        for input in ChannelMixerInput::ALL {
            for destination in ChannelMixerDestination::ALL {
                if !self.value(input, destination).is_finite() {
                    return Err(ChannelMixerEditorError::NonFiniteValue { input, destination });
                }
            }
        }
        Ok(())
    }

    const fn with_value(
        mut self,
        input: ChannelMixerInput,
        destination: ChannelMixerDestination,
        value: f32,
    ) -> Self {
        match input {
            ChannelMixerInput::Red => self.red[destination.index()] = value,
            ChannelMixerInput::Green => self.green[destination.index()] = value,
            ChannelMixerInput::Blue => self.blue[destination.index()] = value,
        }
        self
    }
}

/// Invalid input to the source-shaped Channel Mixer editor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelMixerEditorError {
    /// A matrix value cannot be represented as a finite typed UI value.
    NonFiniteValue {
        input: ChannelMixerInput,
        destination: ChannelMixerDestination,
    },
}

/// Complete native presentation for one Channel Mixer slider.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChannelMixerSliderSpec {
    input: ChannelMixerInput,
    label: &'static str,
    minimum: f64,
    maximum: f64,
    digits: i32,
    gtk_step: f64,
    automatic_step: bool,
    tooltip: &'static str,
}

impl ChannelMixerSliderSpec {
    #[must_use]
    pub const fn input(self) -> ChannelMixerInput {
        self.input
    }

    #[must_use]
    pub const fn parameter_id(self) -> &'static str {
        self.input.label()
    }

    #[must_use]
    pub const fn label(self) -> &'static str {
        self.label
    }

    #[must_use]
    pub const fn range(self) -> (f64, f64) {
        (self.minimum, self.maximum)
    }

    #[must_use]
    pub const fn digits(self) -> i32 {
        self.digits
    }

    #[must_use]
    pub const fn gtk_step(self) -> f64 {
        self.gtk_step
    }

    #[must_use]
    pub const fn automatic_step(self) -> bool {
        self.automatic_step
    }

    #[must_use]
    pub const fn tooltip(self) -> &'static str {
        self.tooltip
    }

    /// Returns the native reset default for the currently selected output.
    #[must_use]
    pub const fn reset_default(self, destination: ChannelMixerDestination) -> f64 {
        match (self.input, destination) {
            (ChannelMixerInput::Red, ChannelMixerDestination::Red)
            | (ChannelMixerInput::Green, ChannelMixerDestination::Green)
            | (ChannelMixerInput::Blue, ChannelMixerDestination::Blue) => 1.0,
            _ => 0.0,
        }
    }

    /// Returns a stable widget identity under a module-instance prefix.
    #[must_use]
    pub fn widget_name(self, widget_id: &str) -> String {
        format!("{widget_id}-{}", self.parameter_id())
    }
}

/// Native output selector metadata and ordering.
pub const CHANNEL_MIXER_DESTINATION_LABEL: &str = "destination";
pub const CHANNEL_MIXER_DESTINATION_OPTIONS: [&str; 7] = [
    "hue",
    "saturation",
    "lightness",
    "red",
    "green",
    "blue",
    "gray",
];

/// Native three-slider presentation: `-2..2`, automatic step, precision 3.
pub const CHANNEL_MIXER_SLIDERS: [ChannelMixerSliderSpec; 3] = [
    ChannelMixerSliderSpec {
        input: ChannelMixerInput::Red,
        label: "red",
        minimum: -2.0,
        maximum: 2.0,
        digits: 3,
        gtk_step: 1.0,
        automatic_step: true,
        tooltip: "amount of red channel in the output channel",
    },
    ChannelMixerSliderSpec {
        input: ChannelMixerInput::Green,
        label: "green",
        minimum: -2.0,
        maximum: 2.0,
        digits: 3,
        gtk_step: 1.0,
        automatic_step: true,
        tooltip: "amount of green channel in the output channel",
    },
    ChannelMixerSliderSpec {
        input: ChannelMixerInput::Blue,
        label: "blue",
        minimum: -2.0,
        maximum: 2.0,
        digits: 3,
        gtk_step: 1.0,
        automatic_step: true,
        tooltip: "amount of blue channel in the output channel",
    },
];

/// Validated, GTK-independent state for the native three-slider editor.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChannelMixerEditorState {
    parameters: ChannelMixerParameters,
    destination: ChannelMixerDestination,
}

impl Default for ChannelMixerEditorState {
    fn default() -> Self {
        Self {
            parameters: ChannelMixerParameters::defaults(),
            destination: ChannelMixerDestination::Red,
        }
    }
}

impl ChannelMixerEditorState {
    /// Loads a complete matrix and keeps the destination as transient GUI state.
    ///
    /// # Errors
    ///
    /// Returns [`ChannelMixerEditorError::NonFiniteValue`] when any matrix
    /// entry is not finite.
    pub fn from_parameters(
        parameters: ChannelMixerParameters,
        destination: ChannelMixerDestination,
    ) -> Result<Self, ChannelMixerEditorError> {
        parameters.validate()?;
        Ok(Self {
            parameters,
            destination,
        })
    }

    #[must_use]
    pub const fn parameters(self) -> ChannelMixerParameters {
        self.parameters
    }

    #[must_use]
    pub const fn destination(self) -> ChannelMixerDestination {
        self.destination
    }

    /// Selects the transient output destination without changing parameters.
    pub const fn set_destination(&mut self, destination: ChannelMixerDestination) {
        self.destination = destination;
    }

    #[must_use]
    pub const fn value(self, input: ChannelMixerInput) -> f32 {
        self.parameters.value(input, self.destination)
    }

    #[must_use]
    pub const fn selected_values(self) -> [f32; 3] {
        [
            self.value(ChannelMixerInput::Red),
            self.value(ChannelMixerInput::Green),
            self.value(ChannelMixerInput::Blue),
        ]
    }

    /// Returns reset defaults for the three visible sliders. These defaults are
    /// presentation state only and never become persisted matrix parameters.
    #[must_use]
    pub const fn reset_defaults(self) -> [f32; 3] {
        [
            match self.destination {
                ChannelMixerDestination::Red => 1.0,
                _ => 0.0,
            },
            match self.destination {
                ChannelMixerDestination::Green => 1.0,
                _ => 0.0,
            },
            match self.destination {
                ChannelMixerDestination::Blue => 1.0,
                _ => 0.0,
            },
        ]
    }

    /// Changes only the selected matrix row entry, preserving all other 20
    /// values and the persisted algorithm exactly.
    ///
    /// # Errors
    ///
    /// Returns [`ChannelMixerEditorError::NonFiniteValue`] for a non-finite
    /// slider value and leaves the complete matrix unchanged.
    pub fn set(
        &mut self,
        input: ChannelMixerInput,
        value: f32,
    ) -> Result<bool, ChannelMixerEditorError> {
        if !value.is_finite() {
            return Err(ChannelMixerEditorError::NonFiniteValue {
                input,
                destination: self.destination,
            });
        }
        let candidate = self.parameters.with_value(input, self.destination, value);
        let changed = candidate != self.parameters;
        self.parameters = candidate;
        Ok(changed)
    }
}

/// Native style-eligible blend colorspace for all built-in presets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelMixerBlendColorspace {
    RgbDisplay,
}

/// One exact native `init_presets()` registration.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChannelMixerPreset {
    label: &'static str,
    parameters: ChannelMixerParameters,
    style_eligible: bool,
    blend_colorspace: ChannelMixerBlendColorspace,
}

impl ChannelMixerPreset {
    const fn new(label: &'static str, parameters: ChannelMixerParameters) -> Self {
        Self {
            label,
            parameters,
            style_eligible: true,
            blend_colorspace: ChannelMixerBlendColorspace::RgbDisplay,
        }
    }

    #[must_use]
    pub const fn label(self) -> &'static str {
        self.label
    }

    #[must_use]
    pub const fn parameters(self) -> ChannelMixerParameters {
        self.parameters
    }

    #[must_use]
    pub const fn style_eligible(self) -> bool {
        self.style_eligible
    }

    #[must_use]
    pub const fn blend_colorspace(self) -> ChannelMixerBlendColorspace {
        self.blend_colorspace
    }
}

const fn preset(
    label: &'static str,
    red: [f32; 7],
    green: [f32; 7],
    blue: [f32; 7],
) -> ChannelMixerPreset {
    ChannelMixerPreset::new(
        label,
        ChannelMixerParameters::from_parts(red, green, blue, ChannelMixerAlgorithm::V2),
    )
}

/// All 17 source `init_presets()` matrices in registration order.
pub const CHANNEL_MIXER_PRESETS: [ChannelMixerPreset; 17] = [
    preset(
        "swap R and B",
        [0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0],
    ),
    preset(
        "swap G and B",
        [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0],
        [0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0],
    ),
    preset(
        "color contrast boost",
        [0.0, 0.0, 0.8, 1.0, 0.0, 0.0, 0.0],
        [0.0, 0.0, 0.1, 0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 0.1, 0.0, 0.0, 1.0, 0.0],
    ),
    preset(
        "color details boost",
        [0.0, 0.0, 0.1, 1.0, 0.0, 0.0, 0.0],
        [0.0, 0.0, 0.8, 0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 0.1, 0.0, 0.0, 1.0, 0.0],
    ),
    preset(
        "color artifacts boost",
        [0.0, 0.0, 0.1, 1.0, 0.0, 0.0, 0.0],
        [0.0, 0.0, 0.1, 0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 0.8, 0.0, 0.0, 1.0, 0.0],
    ),
    preset(
        "B/W luminance-based",
        [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.21],
        [0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.72],
        [0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.07],
    ),
    preset(
        "B/W artifacts boost",
        [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, -0.275],
        [0.0, 0.0, 0.0, 0.0, 1.0, 0.0, -0.275],
        [0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.275],
    ),
    preset(
        "B/W smooth skin",
        [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0],
        [0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.325],
        [0.0, 0.0, 0.0, 0.0, 0.0, 1.0, -0.4],
    ),
    preset(
        "B/W blue artifacts reduce",
        [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.4],
        [0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.750],
        [0.0, 0.0, 0.0, 0.0, 0.0, 1.0, -0.15],
    ),
    preset(
        "B/W Ilford Delta 100-400",
        [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.21],
        [0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.42],
        [0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.37],
    ),
    preset(
        "B/W Ilford Delta 3200",
        [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.31],
        [0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.36],
        [0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.33],
    ),
    preset(
        "B/W Ilford FP4",
        [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.28],
        [0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.41],
        [0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.31],
    ),
    preset(
        "B/W Ilford HP5",
        [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.23],
        [0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.37],
        [0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.40],
    ),
    preset(
        "B/W Ilford SFX",
        [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.36],
        [0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.31],
        [0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.33],
    ),
    preset(
        "B/W Kodak T-Max 100",
        [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.24],
        [0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.37],
        [0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.39],
    ),
    preset(
        "B/W Kodak T-max 400",
        [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.27],
        [0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.36],
        [0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.37],
    ),
    preset(
        "B/W Kodak Tri-X 400",
        [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.25],
        [0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.35],
        [0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.40],
    ),
];

/// Complete UI-owned snapshot for one Channel Mixer operation projection.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChannelMixerGtkState {
    operation_id: OperationId,
    revision: Revision,
    editor: ChannelMixerEditorState,
    enabled: bool,
    sensitive: bool,
    materialization_required: bool,
}

impl ChannelMixerGtkState {
    #[must_use]
    pub const fn new(
        operation_id: OperationId,
        revision: Revision,
        editor: ChannelMixerEditorState,
        enabled: bool,
        sensitive: bool,
        materialization_required: bool,
    ) -> Self {
        Self {
            operation_id,
            revision,
            editor,
            enabled,
            sensitive,
            materialization_required,
        }
    }

    #[must_use]
    pub const fn operation_id(self) -> OperationId {
        self.operation_id
    }

    #[must_use]
    pub const fn revision(self) -> Revision {
        self.revision
    }

    #[must_use]
    pub const fn editor(self) -> ChannelMixerEditorState {
        self.editor
    }

    #[must_use]
    pub const fn enabled(self) -> bool {
        self.enabled
    }

    #[must_use]
    pub const fn sensitive(self) -> bool {
        self.sensitive
    }

    #[must_use]
    pub const fn materialization_required(self) -> bool {
        self.materialization_required
    }
}

/// One settled action carrying the complete 21-value matrix and algorithm.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChannelMixerSettledAction {
    target: OperationId,
    expected_revision: Revision,
    parameters: ChannelMixerParameters,
    enable_required: bool,
    materialization_required: bool,
}

impl ChannelMixerSettledAction {
    #[must_use]
    pub const fn target(self) -> OperationId {
        self.target
    }

    #[must_use]
    pub const fn expected_revision(self) -> Revision {
        self.expected_revision
    }

    #[must_use]
    pub const fn parameters(self) -> ChannelMixerParameters {
        self.parameters
    }

    #[must_use]
    pub const fn enable_required(self) -> bool {
        self.enable_required
    }

    #[must_use]
    pub const fn materialization_required(self) -> bool {
        self.materialization_required
    }
}

/// Synchronous persistence result consumed without rebuilding the module rail.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ChannelMixerGtkHandlerOutcome {
    /// Keep the candidate matrix and advance to the committed edit revision.
    Commit { revision: Revision },
    /// Restore the exact state from before the attempted action.
    Rollback,
    /// Replace the complete leaf snapshot in place.
    Reconcile(ChannelMixerGtkState),
}

pub type ChannelMixerGtkActionHandler =
    Rc<dyn Fn(ChannelMixerSettledAction) -> ChannelMixerGtkHandlerOutcome>;

/// Mounted Channel Mixer leaf with stable widget identities and reconciliation.
#[derive(Clone)]
pub struct ChannelMixerGtkLeaf {
    root: gtk4::Box,
    shared: Rc<Shared>,
}

impl ChannelMixerGtkLeaf {
    #[must_use]
    pub const fn widget(&self) -> &gtk4::Box {
        &self.root
    }

    #[must_use]
    pub fn state(&self) -> ChannelMixerGtkState {
        *self.shared.snapshot.borrow()
    }

    /// Replaces persisted state and synchronizes existing widgets in place.
    pub fn reconcile(&self, state: ChannelMixerGtkState) {
        *self.shared.snapshot.borrow_mut() = state;
        self.shared.sync_widgets();
    }

    /// Returns the destination selector's GTK control for boundary inspection.
    #[must_use]
    pub fn destination(&self) -> gtk4::DropDown {
        self.shared.widgets.destination.dropdown().clone()
    }

    /// Returns one mounted slider for boundary inspection.
    #[must_use]
    pub fn slider(&self, input: ChannelMixerInput) -> gtk4::Scale {
        self.shared.widgets.slider(input).scale().clone()
    }

    /// Returns the source reset default currently installed for one slider.
    ///
    /// The shared Bauhaus adapter currently exposes reset defaults only at
    /// construction. The leaf keeps the native dynamic defaults here and the
    /// reset gesture below consumes this state until that adapter seam is
    /// integrated centrally.
    #[must_use]
    pub fn reset_default(&self, input: ChannelMixerInput) -> f64 {
        self.shared.reset_defaults.get()[input.index()]
    }
}

/// Builds the source-ordered destination selector and three sliders.
///
/// This constructor is intentionally a leaf seam. Because
/// `CHANNEL_MIXER_PRODUCTION_ROUTING_INTEGRATED` is false, no registry or app
/// caller should mount it as an available production operation yet.
///
/// # Panics
///
/// Panics only if the closed [`ChannelMixerDestination`] index cannot fit the
/// GTK unsigned selection type, which is impossible for its seven variants.
#[must_use]
pub fn build_channelmixer_gtk(
    widget_id: &str,
    state: ChannelMixerGtkState,
    handler: Option<ChannelMixerGtkActionHandler>,
) -> ChannelMixerGtkLeaf {
    let root = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    root.set_widget_name(&format!("{widget_id}-channelmixer-editor"));
    root.set_hexpand(true);

    let destination = BauhausComboBox::new(
        &format!("{widget_id}-destination"),
        BauhausComboBoxSpec::new(
            CHANNEL_MIXER_DESTINATION_LABEL,
            "",
            &CHANNEL_MIXER_DESTINATION_OPTIONS,
        ),
    );
    destination.set_selected(
        u32::try_from(state.editor().destination().index()).expect("destination index fits GTK"),
    );
    root.append(destination.widget());

    let sliders = CHANNEL_MIXER_SLIDERS.map(|spec| build_slider(widget_id, spec, state.editor()));
    for slider in &sliders {
        root.append(slider.widget());
    }

    let shared = Rc::new(Shared {
        snapshot: RefCell::new(state),
        handler,
        reset_defaults: Cell::new(state.editor().reset_defaults().map(f64::from)),
        suppress_updates: Cell::new(false),
        widgets: Widgets {
            root: root.downgrade(),
            destination,
            sliders,
        },
    });

    shared.widgets.destination.connect_selection_changed({
        let weak = Rc::downgrade(&shared);
        move |destination| {
            let Some(shared) = weak.upgrade() else {
                return;
            };
            if shared.suppress_updates.get() {
                return;
            }
            let Some(destination) = usize::try_from(destination.selected())
                .ok()
                .and_then(ChannelMixerDestination::from_index)
            else {
                shared.sync_widgets();
                return;
            };
            shared.select_destination(destination);
        }
    });

    for input in ChannelMixerInput::ALL {
        connect_slider(input, &shared);
        install_dynamic_reset(input, &shared);
    }
    shared.sync_widgets();

    ChannelMixerGtkLeaf { root, shared }
}

fn build_slider(
    widget_id: &str,
    spec: ChannelMixerSliderSpec,
    editor: ChannelMixerEditorState,
) -> BauhausSlider {
    let (minimum, maximum) = spec.range();
    let slider = full_width_slider(FullWidthSliderSpec {
        widget_name: &spec.widget_name(widget_id),
        label: spec.label(),
        tooltip: spec.tooltip(),
        minimum,
        maximum,
        gtk_step: spec.gtk_step(),
        value: f64::from(editor.value(spec.input())),
        input: SliderInputSpec::IDENTITY
            .with_soft_range(minimum, maximum)
            .with_default_value(spec.reset_default(editor.destination()))
            .with_digits(spec.digits())
            .with_automatic_step(),
    });
    slider
        .scale()
        .update_property(&[gtk4::accessible::Property::Description(spec.tooltip())]);
    slider
}

fn connect_slider(input: ChannelMixerInput, shared: &Rc<Shared>) {
    let weak = Rc::downgrade(shared);
    shared
        .widgets
        .slider(input)
        .connect_value_settled(move |value| {
            let Some(shared) = weak.upgrade() else {
                return;
            };
            #[allow(
                clippy::cast_possible_truncation,
                reason = "the GTK slider range is finite and bounded to native f32 values"
            )]
            shared.settle(input, value as f32);
        });
}

fn install_dynamic_reset(input: ChannelMixerInput, shared: &Rc<Shared>) {
    let root = shared.widgets.slider(input).widget().clone();
    let scale = shared.widgets.slider(input).scale().clone();
    let weak = Rc::downgrade(shared);
    let gesture = gtk4::GestureClick::new();
    gesture.set_name(Some("dt-channelmixer-reset"));
    gesture.set_button(1);
    gesture.set_propagation_phase(gtk4::PropagationPhase::Capture);
    gesture.connect_pressed(move |gesture, press_count, _, _| {
        if press_count != 2 {
            return;
        }
        let Some(shared) = weak.upgrade() else {
            return;
        };
        let default = shared.reset_defaults.get()[input.index()];
        let previous = scale.value();
        let _ = gesture.set_state(gtk4::EventSequenceState::Claimed);
        if previous.to_bits() != default.to_bits() {
            scale.set_value(default);
            #[allow(
                clippy::cast_possible_truncation,
                reason = "the source reset default is always in the finite GTK range"
            )]
            shared.settle(input, default as f32);
        }
    });
    root.add_controller(gesture);
}

struct Shared {
    snapshot: RefCell<ChannelMixerGtkState>,
    handler: Option<ChannelMixerGtkActionHandler>,
    reset_defaults: Cell<[f64; 3]>,
    suppress_updates: Cell<bool>,
    widgets: Widgets,
}

struct Widgets {
    root: gtk4::glib::WeakRef<gtk4::Box>,
    destination: BauhausComboBox,
    sliders: [BauhausSlider; 3],
}

impl Widgets {
    const fn slider(&self, input: ChannelMixerInput) -> &BauhausSlider {
        &self.sliders[input.index()]
    }
}

impl Shared {
    fn select_destination(&self, destination: ChannelMixerDestination) {
        if self.suppress_updates.get() {
            return;
        }
        self.snapshot
            .borrow_mut()
            .editor
            .set_destination(destination);
        self.sync_widgets();
    }

    fn settle(&self, input: ChannelMixerInput, value: f32) {
        let before = *self.snapshot.borrow();
        let mut candidate = before.editor();
        let Ok(changed) = candidate.set(input, value) else {
            self.sync_widgets();
            return;
        };
        if !changed {
            self.sync_widgets();
            return;
        }

        let action = ChannelMixerSettledAction {
            target: before.operation_id(),
            expected_revision: before.revision(),
            parameters: candidate.parameters(),
            enable_required: !before.enabled(),
            materialization_required: before.materialization_required(),
        };
        let outcome = self
            .handler
            .as_ref()
            .map_or(ChannelMixerGtkHandlerOutcome::Rollback, |handler| {
                handler(action)
            });
        match outcome {
            ChannelMixerGtkHandlerOutcome::Commit { revision } => {
                *self.snapshot.borrow_mut() = ChannelMixerGtkState::new(
                    before.operation_id(),
                    revision,
                    candidate,
                    before.enabled() || action.enable_required(),
                    before.sensitive(),
                    false,
                );
            }
            ChannelMixerGtkHandlerOutcome::Rollback => {}
            ChannelMixerGtkHandlerOutcome::Reconcile(state) => {
                *self.snapshot.borrow_mut() = state;
            }
        }
        self.sync_widgets();
    }

    fn sync_widgets(&self) {
        let snapshot = *self.snapshot.borrow();
        let previous = self.suppress_updates.replace(true);
        self.widgets.destination.set_selected(
            u32::try_from(snapshot.editor().destination().index())
                .expect("destination index fits GTK"),
        );
        self.reset_defaults
            .set(snapshot.editor().reset_defaults().map(f64::from));
        for spec in CHANNEL_MIXER_SLIDERS {
            self.widgets
                .slider(spec.input())
                .set_value(f64::from(snapshot.editor().value(spec.input())));
        }
        self.suppress_updates.set(previous);
        if let Some(root) = self.widgets.root.upgrade() {
            root.set_sensitive(snapshot.sensitive());
        }
    }
}

#[cfg(test)]
#[expect(
    clippy::assertions_on_constants,
    clippy::float_cmp,
    reason = "The test locks exact channel-mixer metadata and IEEE slider values to the source contract."
)]
mod tests {
    use super::*;

    #[test]
    fn source_metadata_and_destination_order_are_exact() {
        assert_eq!(CHANNEL_MIXER_MODULE_ID, "channelmixer");
        assert_eq!(CHANNEL_MIXER_TITLE, "channel mixer");
        assert_eq!(
            CHANNEL_MIXER_DEPRECATED_MESSAGE,
            "this module is deprecated. please use the color calibration module instead."
        );
        assert_eq!(CHANNEL_MIXER_GROUP_KEYS, ["group.color", "group.grading"]);
        assert_eq!(
            CHANNEL_MIXER_DESTINATION_OPTIONS,
            [
                "hue",
                "saturation",
                "lightness",
                "red",
                "green",
                "blue",
                "gray"
            ]
        );
        assert_eq!(
            ChannelMixerDestination::ALL.map(ChannelMixerDestination::index),
            [0, 1, 2, 3, 4, 5, 6]
        );
        assert!(!CHANNEL_MIXER_PRODUCTION_ROUTING_INTEGRATED);
    }

    #[test]
    fn slider_presentation_preserves_source_ranges_precision_step_and_tooltips() {
        let expected = [
            (
                ChannelMixerInput::Red,
                "red",
                "amount of red channel in the output channel",
            ),
            (
                ChannelMixerInput::Green,
                "green",
                "amount of green channel in the output channel",
            ),
            (
                ChannelMixerInput::Blue,
                "blue",
                "amount of blue channel in the output channel",
            ),
        ];
        for ((input, label, tooltip), spec) in expected.into_iter().zip(CHANNEL_MIXER_SLIDERS) {
            assert_eq!(spec.input(), input);
            assert_eq!(spec.parameter_id(), label);
            assert_eq!(spec.label(), label);
            assert_eq!(spec.range(), (-2.0, 2.0));
            assert_eq!(spec.digits(), 3);
            assert_eq!(spec.gtk_step().to_bits(), 1.0_f64.to_bits());
            assert!(spec.automatic_step());
            assert_eq!(spec.tooltip(), tooltip);
            assert_eq!(
                spec.widget_name("channelmixer"),
                format!("channelmixer-{label}")
            );
        }
    }

    #[test]
    fn defaults_match_native_identity_and_red_destination() {
        let state = ChannelMixerEditorState::default();
        assert_eq!(state.destination(), ChannelMixerDestination::Red);
        assert_eq!(state.parameters().algorithm(), ChannelMixerAlgorithm::V2);
        assert_eq!(
            state.parameters().red(),
            [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0]
        );
        assert_eq!(
            state.parameters().green(),
            [0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0]
        );
        assert_eq!(
            state.parameters().blue(),
            [0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0]
        );
        assert_eq!(state.selected_values(), [1.0, 0.0, 0.0]);
        assert_eq!(state.reset_defaults(), [1.0, 0.0, 0.0]);
    }

    #[test]
    fn destination_is_transient_and_updates_only_reset_defaults() {
        let mut state = ChannelMixerEditorState::default();
        let parameters = state.parameters();
        state.set_destination(ChannelMixerDestination::Green);
        assert_eq!(state.parameters(), parameters);
        assert_eq!(state.selected_values(), [0.0, 1.0, 0.0]);
        assert_eq!(state.reset_defaults(), [0.0, 1.0, 0.0]);
        state.set_destination(ChannelMixerDestination::Gray);
        assert_eq!(state.reset_defaults(), [0.0, 0.0, 0.0]);
    }

    #[test]
    fn one_settled_edit_changes_only_the_selected_matrix_row_entry() {
        let mut state = ChannelMixerEditorState::default();
        state.set_destination(ChannelMixerDestination::Gray);
        let before = state.parameters();
        assert!(state.set(ChannelMixerInput::Green, 0.75).unwrap());
        let after = state.parameters();
        assert_eq!(after.red(), before.red());
        assert_eq!(after.blue(), before.blue());
        assert_eq!(after.green()[6].to_bits(), 0.75_f32.to_bits());
        for index in 0..6 {
            assert_eq!(
                after.green()[index].to_bits(),
                before.green()[index].to_bits()
            );
        }
        assert_eq!(after.algorithm(), before.algorithm());
        assert!(!state.set(ChannelMixerInput::Green, 0.75).unwrap());
    }

    #[test]
    fn finite_values_outside_presentation_range_are_preserved_without_clamping() {
        let parameters = ChannelMixerParameters::try_from_rows(
            [4.5, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0],
            [0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0],
            ChannelMixerAlgorithm::V1,
        )
        .unwrap();
        let state =
            ChannelMixerEditorState::from_parameters(parameters, ChannelMixerDestination::Hue)
                .unwrap();
        assert_eq!(state.parameters().red()[0].to_bits(), 4.5_f32.to_bits());
        assert_eq!(state.parameters().algorithm(), ChannelMixerAlgorithm::V1);
    }

    #[test]
    fn nonfinite_values_are_rejected_without_mutating_editor_state() {
        let mut state = ChannelMixerEditorState::default();
        let before = state;
        assert!(state.set(ChannelMixerInput::Red, f32::NAN).is_err());
        assert_eq!(state, before);
        assert!(state.set(ChannelMixerInput::Blue, f32::INFINITY).is_err());
        assert_eq!(state, before);
    }

    #[test]
    fn reset_defaults_follow_only_rgb_destination_identity() {
        for destination in ChannelMixerDestination::ALL {
            let state = ChannelMixerEditorState::from_parameters(
                ChannelMixerParameters::defaults(),
                destination,
            )
            .unwrap();
            let expected = match destination {
                ChannelMixerDestination::Red => [1.0, 0.0, 0.0],
                ChannelMixerDestination::Green => [0.0, 1.0, 0.0],
                ChannelMixerDestination::Blue => [0.0, 0.0, 1.0],
                _ => [0.0, 0.0, 0.0],
            };
            assert_eq!(state.reset_defaults(), expected);
        }
    }

    #[test]
    fn all_seventeen_presets_keep_exact_native_metadata_and_v2_algorithm() {
        assert_eq!(CHANNEL_MIXER_PRESETS.len(), 17);
        let labels = CHANNEL_MIXER_PRESETS.map(ChannelMixerPreset::label);
        assert_eq!(
            labels,
            [
                "swap R and B",
                "swap G and B",
                "color contrast boost",
                "color details boost",
                "color artifacts boost",
                "B/W luminance-based",
                "B/W artifacts boost",
                "B/W smooth skin",
                "B/W blue artifacts reduce",
                "B/W Ilford Delta 100-400",
                "B/W Ilford Delta 3200",
                "B/W Ilford FP4",
                "B/W Ilford HP5",
                "B/W Ilford SFX",
                "B/W Kodak T-Max 100",
                "B/W Kodak T-max 400",
                "B/W Kodak Tri-X 400",
            ]
        );
        for preset in CHANNEL_MIXER_PRESETS {
            assert_eq!(preset.parameters().algorithm(), ChannelMixerAlgorithm::V2);
            assert!(preset.style_eligible());
            assert_eq!(
                preset.blend_colorspace(),
                ChannelMixerBlendColorspace::RgbDisplay
            );
        }
        assert_eq!(
            CHANNEL_MIXER_PRESETS[0].parameters().red()[5].to_bits(),
            1.0_f32.to_bits()
        );
        assert_eq!(
            CHANNEL_MIXER_PRESETS[5].parameters().blue()[6].to_bits(),
            0.07_f32.to_bits()
        );
        assert_eq!(
            CHANNEL_MIXER_PRESETS[6].parameters().blue()[6].to_bits(),
            1.275_f32.to_bits()
        );
    }
}
