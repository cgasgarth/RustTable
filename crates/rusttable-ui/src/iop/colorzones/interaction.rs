//! Pure Color Zones graph interaction derived from `src/iop/colorzones.c`.
//!
//! This module translates source-shaped graph gestures into mutations on
//! [`ColorZonesEditorState`]. It intentionally does not wire GTK controllers,
//! query live keyboard state, sample an interpolated display curve, or convert
//! live picker colors into `LCh`. Callers supply normalized curve samples and
//! picker coordinates, then translate the returned outcomes at those boundaries.

#![allow(
    clippy::missing_errors_doc,
    reason = "interaction failures are represented by the documented typed error"
)]

use std::fmt;

use rusttable_processing::{COLORZONES_MAX_NODES, ColorZonesChannel, ColorZonesSplinesVersion};

use super::model::{ColorZonesDeleteOutcome, ColorZonesEditorError, ColorZonesEditorState};

/// Native number of named hue-band shortcut elements.
pub const COLORZONES_BANDS: usize = 8;
/// Native graph movement step for wheel and arrow-key gestures.
pub const COLORZONES_DEFAULT_STEP: f32 = 0.001;
/// Native picker feather on each side of the sampled range.
pub const COLORZONES_PICKER_FEATHER: f32 = 0.02;
/// Native names and ordering of the eight graph shortcut elements.
pub const COLORZONES_BAND_NAMES: [&str; COLORZONES_BANDS] = [
    "red", "orange", "yellow", "green", "aqua", "blue", "purple", "magenta",
];

const NODE_SELECTION_RADIUS_SQUARED: f32 = 0.04 * 0.04;
const BAND_NODE_DISTANCE: f32 = 1.0 / 16.0;
const PICKER_INCREMENT: f32 = 0.1;
const INITIAL_AREA_RADIUS: f32 = 1.0 / 8.0;

/// Relevant modifier state after platform-specific key normalization.
///
/// Darktable compares exactly among Shift, Control, and Alt/Meta while ignoring
/// lock and button-state bits. A GTK adapter should construct this value from
/// only those three logical modifiers.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ColorZonesModifiers {
    shift: bool,
    control: bool,
    alt_or_meta: bool,
}

impl ColorZonesModifiers {
    /// No relevant modifier keys.
    pub const NONE: Self = Self::new(false, false, false);
    /// Shift and no other relevant modifier.
    pub const SHIFT: Self = Self::new(true, false, false);
    /// Control (Command after a platform adapter remaps it) and nothing else.
    pub const CONTROL: Self = Self::new(false, true, false);
    /// Alt/Meta and no other relevant modifier.
    pub const ALT: Self = Self::new(false, false, true);

    /// Builds a normalized modifier state.
    #[must_use]
    pub const fn new(shift: bool, control: bool, alt_or_meta: bool) -> Self {
        Self {
            shift,
            control,
            alt_or_meta,
        }
    }

    /// Whether the relevant keys exactly equal `desired`.
    #[must_use]
    pub const fn is_exact(self, desired: Self) -> bool {
        self.shift == desired.shift
            && self.control == desired.control
            && self.alt_or_meta == desired.alt_or_meta
    }
}

/// Primary-button click multiplicity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorZonesClick {
    Single,
    Double,
}

/// Arrow keys accepted by the graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorZonesArrowKey {
    Up,
    Down,
    Left,
    Right,
}

/// Source shortcut effects supported for every named band.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorZonesBandEffect {
    Reset,
    Bottom,
    Top,
    Down,
    Up,
}

/// Normalized picker values along the current selection channel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorZonesPickerRange {
    pub minimum: f32,
    pub mean: f32,
    pub maximum: f32,
}

/// Current node-selection state, including Darktable's post-mutation sentinel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorZonesSelection {
    None,
    Node(usize),
    /// Prevents the following motion event from immediately reinserting a node.
    Suppressed,
}

/// Result of a primary-button press.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorZonesPrimaryOutcome {
    Ignored,
    AreaDragStarted,
    NodeInserted(usize),
    CurveReset,
    /// Ctrl insertion was consumed but its sampled ordinate was outside 0..=1.
    SampleOutsideViewport,
}

/// Result of a secondary-button press.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorZonesSecondaryOutcome {
    Ignored,
    Deleted(ColorZonesDeleteOutcome),
    Neutralized,
}

/// Result of a graph scroll gesture.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ColorZonesScrollOutcome {
    /// Alt-only scroll belongs to the output-channel tabs.
    ForwardToChannelTabs {
        delta_y: f32,
    },
    Zoomed,
    RadiusChanged,
    NodeMoved,
    Consumed,
}

/// Result returned by a named band action.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorZonesBandOutcome {
    /// Native ordinate value, before reset or after movement.
    pub value: f32,
    pub changed: bool,
    pub node: Option<usize>,
}

/// Invalid input to the pure interaction controller.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ColorZonesInteractionError {
    Editor(ColorZonesEditorError),
    NonFiniteInput,
    MissingCurveSample,
    InvalidBand(usize),
}

impl fmt::Display for ColorZonesInteractionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Editor(error) => write!(formatter, "{error}"),
            Self::NonFiniteInput => {
                formatter.write_str("Color Zones interaction input is non-finite")
            }
            Self::MissingCurveSample => formatter.write_str(
                "Color Zones Ctrl insertion requires a normalized interpolated curve sample",
            ),
            Self::InvalidBand(band) => write!(
                formatter,
                "Color Zones band {band} is outside 0..{COLORZONES_BANDS}"
            ),
        }
    }
}

impl std::error::Error for ColorZonesInteractionError {}

impl From<ColorZonesEditorError> for ColorZonesInteractionError {
    fn from(error: ColorZonesEditorError) -> Self {
        Self::Editor(error)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ColorZonesPrimaryState {
    Released,
    NodeDrag,
    AreaDrag,
}

/// Pure editor plus transient graph interaction state.
#[derive(Debug, Clone, PartialEq)]
pub struct ColorZonesInteraction {
    editor: ColorZonesEditorState,
    mouse_x: f32,
    mouse_y: f32,
    pointer_inside: bool,
    mouse_radius: f32,
    selection: ColorZonesSelection,
    primary_state: ColorZonesPrimaryState,
    x_move: Option<usize>,
    edit_by_area: bool,
    zoom_factor: f32,
    offset_x: f32,
    offset_y: f32,
}

impl Default for ColorZonesInteraction {
    fn default() -> Self {
        Self::new(ColorZonesEditorState::default())
    }
}

impl ColorZonesInteraction {
    /// Builds native transient defaults around the canonical editor model.
    #[must_use]
    pub const fn new(editor: ColorZonesEditorState) -> Self {
        Self {
            editor,
            mouse_x: -1.0,
            mouse_y: -1.0,
            pointer_inside: false,
            mouse_radius: INITIAL_AREA_RADIUS,
            selection: ColorZonesSelection::None,
            primary_state: ColorZonesPrimaryState::Released,
            x_move: None,
            edit_by_area: false,
            zoom_factor: 1.0,
            offset_x: 0.0,
            offset_y: 0.0,
        }
    }

    /// Canonical editor model used by every parameter mutation.
    #[must_use]
    pub const fn editor(&self) -> &ColorZonesEditorState {
        &self.editor
    }

    /// Mutable model access for non-graph controls such as channel tabs.
    pub const fn editor_mut(&mut self) -> &mut ColorZonesEditorState {
        &mut self.editor
    }

    /// Consumes the controller and returns its canonical editor state.
    #[must_use]
    pub fn into_editor(self) -> ColorZonesEditorState {
        self.editor
    }

    #[must_use]
    pub const fn selection(&self) -> ColorZonesSelection {
        self.selection
    }

    pub const fn set_selection(&mut self, selection: ColorZonesSelection) {
        self.selection = selection;
    }

    #[must_use]
    pub const fn edit_by_area(&self) -> bool {
        self.edit_by_area
    }

    pub const fn set_edit_by_area(&mut self, enabled: bool) {
        self.edit_by_area = enabled;
    }

    #[must_use]
    pub const fn area_radius(&self) -> f32 {
        self.mouse_radius
    }

    /// Returns the current normalized pointer while it remains over the graph.
    #[must_use]
    pub const fn pointer(&self) -> Option<(f32, f32)> {
        if self.pointer_inside {
            Some((self.mouse_x, self.mouse_y))
        } else {
            None
        }
    }

    /// Returns the node whose bottom marker is active during area editing.
    #[must_use]
    pub const fn area_marker(&self) -> Option<usize> {
        self.x_move
    }

    /// Whether source area feedback remains visible during hover or a drag.
    #[must_use]
    pub const fn area_feedback_visible(&self) -> bool {
        self.edit_by_area
            && (self.pointer_inside
                || matches!(self.primary_state, ColorZonesPrimaryState::AreaDrag))
    }

    #[must_use]
    pub const fn zoom_factor(&self) -> f32 {
        self.zoom_factor
    }

    #[must_use]
    pub const fn offsets(&self) -> (f32, f32) {
        (self.offset_x, self.offset_y)
    }

    /// Converts a curve coordinate into the visible graph coordinate.
    #[must_use]
    pub fn curve_to_view(&self, value: f32, offset: f32) -> f32 {
        (value - offset) * self.zoom_factor
    }

    /// Converts a visible graph coordinate into a curve coordinate.
    #[must_use]
    pub fn view_to_curve(&self, value: f32, offset: f32) -> f32 {
        value / self.zoom_factor + offset
    }

    /// Updates the clamped normalized pointer position.
    pub fn set_pointer(&mut self, x: f32, y: f32) -> Result<(), ColorZonesInteractionError> {
        validate_finite_pair(x, y)?;
        self.mouse_x = x.clamp(0.0, 1.0);
        self.mouse_y = y.clamp(0.0, 1.0);
        self.pointer_inside = true;
        Ok(())
    }

    /// Selects the nearest node within the native 0.04 graph-space radius.
    pub fn update_hover_selection(&mut self) -> ColorZonesSelection {
        let channel = self.editor.output_channel();
        let mut minimum = NODE_SELECTION_RADIUS_SQUARED;
        let mut nearest = None;
        for (index, node) in self.editor.active_nodes(channel).iter().enumerate() {
            let x = self.curve_to_view(node.x, self.offset_x);
            let y = self.curve_to_view(node.y, self.offset_y);
            let distance = (self.mouse_x - x)
                .mul_add(self.mouse_x - x, (self.mouse_y - y) * (self.mouse_y - y));
            if distance < minimum {
                minimum = distance;
                nearest = Some(index);
            }
        }
        self.selection = nearest.map_or(ColorZonesSelection::None, ColorZonesSelection::Node);
        self.selection
    }

    /// Tracks the closest x marker while area editing below the graph.
    pub fn update_area_x_marker(&mut self, below_graph: bool) -> Option<usize> {
        if !self.edit_by_area || !below_graph {
            self.x_move = None;
            return None;
        }
        let channel = self.editor.output_channel();
        let mouse_x = self.view_to_curve(self.mouse_x, self.offset_x);
        self.x_move = self
            .editor
            .active_nodes(channel)
            .iter()
            .enumerate()
            .min_by(|(_, left), (_, right)| {
                (left.x - mouse_x)
                    .abs()
                    .total_cmp(&(right.x - mouse_x).abs())
            })
            .map(|(index, _)| index);
        self.x_move
    }

    /// Applies native primary-press precedence.
    ///
    /// `sampled_curve_y` is required only for exact-Ctrl insertion and must be
    /// obtained from the displayed interpolation implementation.
    pub fn primary_press(
        &mut self,
        click: ColorZonesClick,
        modifiers: ColorZonesModifiers,
        sampled_curve_y: Option<f32>,
    ) -> Result<ColorZonesPrimaryOutcome, ColorZonesInteractionError> {
        self.primary_state = ColorZonesPrimaryState::NodeDrag;
        if self.edit_by_area
            && click != ColorZonesClick::Double
            && !modifiers.is_exact(ColorZonesModifiers::CONTROL)
        {
            self.primary_state = ColorZonesPrimaryState::AreaDrag;
            return Ok(ColorZonesPrimaryOutcome::AreaDragStarted);
        }

        let channel = self.editor.output_channel();
        if click == ColorZonesClick::Single
            && modifiers.is_exact(ColorZonesModifiers::CONTROL)
            && self.editor.active_count(channel) < COLORZONES_MAX_NODES
            && (self.selection == ColorZonesSelection::None || self.edit_by_area)
        {
            let y = sampled_curve_y.ok_or(ColorZonesInteractionError::MissingCurveSample)?;
            if !y.is_finite() {
                return Err(ColorZonesInteractionError::NonFiniteInput);
            }
            if !(0.0..=1.0).contains(&y) {
                return Ok(ColorZonesPrimaryOutcome::SampleOutsideViewport);
            }
            let x = self.view_to_curve(self.mouse_x, self.offset_x);
            let old_nodes = self.editor.active_nodes(channel).to_vec();
            let inserted = self.editor.insert_node(x, y)?;
            if old_nodes.iter().any(|node| {
                let other_y = self.curve_to_view(node.y, self.offset_y);
                (y - other_y) * (y - other_y) < NODE_SELECTION_RADIUS_SQUARED
            }) {
                self.selection = ColorZonesSelection::Node(inserted);
            }
            return Ok(ColorZonesPrimaryOutcome::NodeInserted(inserted));
        }

        if click == ColorZonesClick::Double {
            self.editor.reset_curve();
            self.selection = ColorZonesSelection::Suppressed;
            return Ok(ColorZonesPrimaryOutcome::CurveReset);
        }

        Ok(ColorZonesPrimaryOutcome::Ignored)
    }

    /// Applies right-click boundary precedence and exact-Ctrl neutralization.
    pub fn secondary_press(
        &mut self,
        modifiers: ColorZonesModifiers,
    ) -> Result<ColorZonesSecondaryOutcome, ColorZonesInteractionError> {
        let ColorZonesSelection::Node(node) = self.selection else {
            return Ok(ColorZonesSecondaryOutcome::Ignored);
        };
        let channel = self.editor.output_channel();
        let count = self.editor.active_count(channel);
        let v1_boundary = self.editor.splines_version() == ColorZonesSplinesVersion::V1
            && (node == 0 || node + 1 == count);
        if v1_boundary {
            let outcome = self.editor.delete_node(node)?;
            return Ok(ColorZonesSecondaryOutcome::Deleted(outcome));
        }
        if modifiers.is_exact(ColorZonesModifiers::CONTROL) {
            self.editor.neutralize_node(node)?;
            self.selection = ColorZonesSelection::Suppressed;
            Ok(ColorZonesSecondaryOutcome::Neutralized)
        } else {
            let outcome = self.editor.delete_node(node)?;
            self.selection = ColorZonesSelection::Suppressed;
            Ok(ColorZonesSecondaryOutcome::Deleted(outcome))
        }
    }

    /// Ends a primary drag and drops a selection retained across pointer leave.
    pub const fn primary_release(&mut self) {
        self.primary_state = ColorZonesPrimaryState::Released;
        if !self.pointer_inside {
            self.selection = ColorZonesSelection::None;
        }
    }

    /// Applies a primary drag with the default movement speed.
    pub fn primary_drag_to(&mut self, x: f32, y: f32) -> Result<bool, ColorZonesInteractionError> {
        self.primary_drag_to_with_speed(x, y, 1.0)
    }

    /// Applies a primary drag using the normalized native accelerator speed.
    pub fn primary_drag_to_with_speed(
        &mut self,
        x: f32,
        y: f32,
        speed_multiplier: f32,
    ) -> Result<bool, ColorZonesInteractionError> {
        validate_speed_multiplier(speed_multiplier)?;
        let old_x = self.mouse_x;
        let old_y = self.mouse_y.abs();
        self.set_pointer(x, y)?;

        if self.edit_by_area {
            if self.primary_state != ColorZonesPrimaryState::AreaDrag {
                return Ok(false);
            }
            if let Some(node) = self.x_move {
                self.selection = ColorZonesSelection::Node(node);
                let dx = (self.mouse_x - old_x) / self.zoom_factor;
                let dy = (self.mouse_y - old_y) / self.zoom_factor;
                return self.move_selected_by(dx, dy, speed_multiplier);
            }
            self.apply_gaussian_area_edit()?;
            return Ok(true);
        }

        match self.selection {
            ColorZonesSelection::Node(node) => {
                let dx = (self.mouse_x - old_x) / self.zoom_factor;
                let dy = (self.mouse_y - old_y) / self.zoom_factor;
                self.editor
                    .move_node_by(node, dx * speed_multiplier, dy * speed_multiplier)
                    .map_err(Into::into)
            }
            ColorZonesSelection::None
                if self.editor.active_count(self.editor.output_channel())
                    < COLORZONES_MAX_NODES =>
            {
                let curve_x = self.view_to_curve(self.mouse_x, self.offset_x);
                let curve_y = self.view_to_curve(self.mouse_y, self.offset_y);
                let node = self.editor.insert_node(curve_x, curve_y)?;
                self.selection = ColorZonesSelection::Node(node);
                Ok(true)
            }
            ColorZonesSelection::None | ColorZonesSelection::Suppressed => Ok(false),
        }
    }

    /// Applies the native Gaussian area edit at the current pointer position.
    pub fn apply_gaussian_area_edit(&mut self) -> Result<(), ColorZonesInteractionError> {
        let channel = self.editor.output_channel();
        let points = self.editor.active_nodes(channel).to_vec();
        let linear_x = self.view_to_curve(self.mouse_x, self.offset_x);
        let linear_y = self.view_to_curve(self.mouse_y, self.offset_y);
        let radius = self.mouse_radius / self.zoom_factor;
        let radius_squared = radius * radius;
        let periodic_v1 = self.editor.selection_channel() == ColorZonesChannel::Hue
            && self.editor.splines_version() == ColorZonesSplinesVersion::V1;

        if periodic_v1 {
            for (index, point) in points
                .iter()
                .enumerate()
                .take(points.len().saturating_sub(1))
                .skip(1)
            {
                let weight = gaussian_weight(linear_x - point.x, radius_squared);
                self.editor.move_node_on(
                    channel,
                    index,
                    point.x,
                    (1.0 - weight).mul_add(point.y, weight * linear_y),
                )?;
            }
            let last = points.len() - 1;
            let first_distance = linear_x - points[0].x;
            let last_distance = linear_x - points[last].x;
            let minimum_squared =
                (first_distance * first_distance).min(last_distance * last_distance);
            let weight = (-minimum_squared / radius_squared).exp();
            self.editor.move_node_on(
                channel,
                0,
                points[0].x,
                (1.0 - weight).mul_add(points[0].y, weight * linear_y),
            )?;
        } else {
            for (index, point) in points.iter().enumerate() {
                let weight = gaussian_weight(linear_x - point.x, radius_squared);
                self.editor.move_node_on(
                    channel,
                    index,
                    point.x,
                    (1.0 - weight).mul_add(point.y, weight * linear_y),
                )?;
            }
        }
        Ok(())
    }

    /// Handles scrolling with the default movement speed.
    pub fn scroll(
        &mut self,
        delta_y: f32,
        modifiers: ColorZonesModifiers,
        zoom_pan_mode: bool,
    ) -> Result<ColorZonesScrollOutcome, ColorZonesInteractionError> {
        self.scroll_with_speed(delta_y, modifiers, zoom_pan_mode, 1.0)
    }

    /// Handles scrolling using the normalized native accelerator speed.
    pub fn scroll_with_speed(
        &mut self,
        delta_y: f32,
        modifiers: ColorZonesModifiers,
        zoom_pan_mode: bool,
        speed_multiplier: f32,
    ) -> Result<ColorZonesScrollOutcome, ColorZonesInteractionError> {
        validate_speed_multiplier(speed_multiplier)?;
        if !delta_y.is_finite() {
            return Err(ColorZonesInteractionError::NonFiniteInput);
        }
        if modifiers.is_exact(ColorZonesModifiers::ALT) {
            return Ok(ColorZonesScrollOutcome::ForwardToChannelTabs { delta_y });
        }
        if zoom_pan_mode {
            self.zoom_at_pointer(delta_y)?;
            return Ok(ColorZonesScrollOutcome::Zoomed);
        }
        if self.edit_by_area {
            self.adjust_area_radius(delta_y);
            return Ok(ColorZonesScrollOutcome::RadiusChanged);
        }
        if matches!(self.selection, ColorZonesSelection::Node(_)) {
            let moved =
                self.move_selected_by(0.0, -delta_y * COLORZONES_DEFAULT_STEP, speed_multiplier)?;
            return Ok(if moved {
                ColorZonesScrollOutcome::NodeMoved
            } else {
                ColorZonesScrollOutcome::Consumed
            });
        }
        Ok(ColorZonesScrollOutcome::Consumed)
    }

    /// Moves the selected node by one native 0.001 arrow-key step.
    pub fn key_press(
        &mut self,
        key: ColorZonesArrowKey,
    ) -> Result<bool, ColorZonesInteractionError> {
        self.key_press_with_speed(key, 1.0)
    }

    /// Moves the selected node using the normalized native accelerator speed.
    pub fn key_press_with_speed(
        &mut self,
        key: ColorZonesArrowKey,
        speed_multiplier: f32,
    ) -> Result<bool, ColorZonesInteractionError> {
        validate_speed_multiplier(speed_multiplier)?;
        let (dx, dy) = match key {
            ColorZonesArrowKey::Up => (0.0, COLORZONES_DEFAULT_STEP),
            ColorZonesArrowKey::Down => (0.0, -COLORZONES_DEFAULT_STEP),
            ColorZonesArrowKey::Left => (-COLORZONES_DEFAULT_STEP, 0.0),
            ColorZonesArrowKey::Right => (COLORZONES_DEFAULT_STEP, 0.0),
        };
        self.move_selected_by(dx, dy, speed_multiplier)
    }

    /// Zooms around the current pointer by the native ten-percent wheel factor.
    pub fn zoom_at_pointer(&mut self, delta_y: f32) -> Result<(), ColorZonesInteractionError> {
        if !delta_y.is_finite() {
            return Err(ColorZonesInteractionError::NonFiniteInput);
        }
        let linear_x = self.view_to_curve(self.mouse_x, self.offset_x);
        let linear_y = self.view_to_curve(self.mouse_y, self.offset_y);
        self.zoom_factor = (self.zoom_factor * (1.0 - 0.1 * delta_y)).max(1.0);
        self.offset_x = linear_x - self.mouse_x / self.zoom_factor;
        self.offset_y = linear_y - self.mouse_y / self.zoom_factor;
        self.clamp_offsets();
        Ok(())
    }

    /// Pans by the source's `(previous - current) / zoom` view-space delta.
    pub fn pan_by_view_delta(
        &mut self,
        previous_x_minus_current_x: f32,
        previous_y_minus_current_y: f32,
    ) -> Result<(), ColorZonesInteractionError> {
        validate_finite_pair(previous_x_minus_current_x, previous_y_minus_current_y)?;
        self.offset_x += previous_x_minus_current_x / self.zoom_factor;
        self.offset_y += previous_y_minus_current_y / self.zoom_factor;
        self.clamp_offsets();
        Ok(())
    }

    /// Resets zoom and both offsets, as a bottom-bar double click does.
    pub const fn reset_zoom(&mut self) {
        self.zoom_factor = 1.0;
        self.offset_x = 0.0;
        self.offset_y = 0.0;
    }

    /// Marks the pointer outside and clears selection unless a drag is active.
    pub const fn leave(&mut self) {
        self.pointer_inside = false;
        self.mouse_y = -self.mouse_y.abs();
        if matches!(self.primary_state, ColorZonesPrimaryState::Released) {
            self.selection = ColorZonesSelection::None;
        }
    }

    /// Resets the current curve, then adds the native five picker nodes.
    ///
    /// Exact Ctrl creates a positive curve, exact Shift a negative curve, and
    /// every other modifier combination creates a flat curve.
    pub fn apply_picker_curve(
        &mut self,
        picker: ColorZonesPickerRange,
        modifiers: ColorZonesModifiers,
    ) -> Result<usize, ColorZonesInteractionError> {
        if !picker.minimum.is_finite() || !picker.mean.is_finite() || !picker.maximum.is_finite() {
            return Err(ColorZonesInteractionError::NonFiniteInput);
        }
        let polarity = if modifiers.is_exact(ColorZonesModifiers::CONTROL) {
            1.0
        } else if modifiers.is_exact(ColorZonesModifiers::SHIFT) {
            -1.0
        } else {
            0.0
        };
        let increment = PICKER_INCREMENT * polarity;
        let nodes = [
            (picker.minimum - COLORZONES_PICKER_FEATHER, 0.5),
            (picker.minimum, 0.5 + increment),
            (picker.mean, 0.5 + 2.0 * increment),
            (picker.maximum, 0.5 + increment),
            (picker.maximum + COLORZONES_PICKER_FEATHER, 0.5),
        ];

        self.editor
            .reset_curve_to_defaults_on(self.editor.output_channel());
        let mut inserted = 0;
        for (x, y) in nodes {
            if x > 0.0 && x < 1.0 {
                match self.editor.insert_node(x, y) {
                    Ok(_) => inserted += 1,
                    Err(ColorZonesEditorError::NodesTooClose) => {}
                    Err(error) => return Err(error.into()),
                }
            }
        }
        Ok(inserted)
    }

    /// Returns the source-selected value for one of the eight named bands.
    pub fn band_value(
        &self,
        band: usize,
        sampled_curve_y: f32,
    ) -> Result<ColorZonesBandOutcome, ColorZonesInteractionError> {
        let (node, value) = self.band_node_and_value(band, sampled_curve_y)?;
        Ok(ColorZonesBandOutcome {
            value,
            changed: false,
            node,
        })
    }

    /// Applies one native shortcut effect using the default movement speed.
    pub fn apply_band_action(
        &mut self,
        band: usize,
        effect: ColorZonesBandEffect,
        move_size: f32,
        sampled_curve_y: f32,
    ) -> Result<ColorZonesBandOutcome, ColorZonesInteractionError> {
        self.apply_band_action_with_speed(band, effect, move_size, sampled_curve_y, 1.0)
    }

    /// Applies one native shortcut effect using the normalized accelerator speed.
    pub fn apply_band_action_with_speed(
        &mut self,
        band: usize,
        effect: ColorZonesBandEffect,
        move_size: f32,
        sampled_curve_y: f32,
        speed_multiplier: f32,
    ) -> Result<ColorZonesBandOutcome, ColorZonesInteractionError> {
        validate_speed_multiplier(speed_multiplier)?;
        if !move_size.is_finite() {
            return Err(ColorZonesInteractionError::NonFiniteInput);
        }
        let (existing, original_value) = self.band_node_and_value(band, sampled_curve_y)?;
        if effect == ColorZonesBandEffect::Reset {
            if let Some(node) = existing {
                self.editor.delete_node(node)?;
                return Ok(ColorZonesBandOutcome {
                    value: original_value,
                    changed: true,
                    node: None,
                });
            }
            return Ok(ColorZonesBandOutcome {
                value: original_value,
                changed: false,
                node: None,
            });
        }

        let x = band_x(band)?;
        let (node, inserted) = if let Some(node) = existing {
            (node, false)
        } else {
            (self.editor.insert_node(x, original_value)?, true)
        };
        let dy = match effect {
            ColorZonesBandEffect::Bottom => -10_000.0,
            ColorZonesBandEffect::Top => 10_000.0,
            ColorZonesBandEffect::Down => -move_size / 100.0,
            ColorZonesBandEffect::Up => move_size / 100.0,
            ColorZonesBandEffect::Reset => unreachable!("reset returned above"),
        };
        let moved = self.editor.move_node_by(node, 0.0, dy * speed_multiplier)?;
        let value = self.editor.active_nodes(self.editor.output_channel())[node].y;
        Ok(ColorZonesBandOutcome {
            value,
            changed: inserted || moved,
            node: Some(node),
        })
    }

    fn move_selected_by(
        &mut self,
        dx: f32,
        dy: f32,
        speed_multiplier: f32,
    ) -> Result<bool, ColorZonesInteractionError> {
        let ColorZonesSelection::Node(node) = self.selection else {
            return Ok(false);
        };
        self.editor
            .move_node_by(node, dx * speed_multiplier, dy * speed_multiplier)
            .map_err(Into::into)
    }

    fn adjust_area_radius(&mut self, delta_y: f32) {
        let bands = self.editor.active_count(self.editor.output_channel());
        let bands_u16 = u16::try_from(bands).expect("native Color Zones node count fits u16");
        let minimum = 0.2 / f32::from(bands_u16);
        self.mouse_radius = (self.mouse_radius * (1.0 + 0.1 * delta_y)).clamp(minimum, 1.0);
    }

    fn clamp_offsets(&mut self) {
        let maximum = (self.zoom_factor - 1.0) / self.zoom_factor;
        self.offset_x = self.offset_x.clamp(0.0, maximum);
        self.offset_y = self.offset_y.clamp(0.0, maximum);
    }

    fn band_node_and_value(
        &self,
        band: usize,
        sampled_curve_y: f32,
    ) -> Result<(Option<usize>, f32), ColorZonesInteractionError> {
        let x = band_x(band)?;
        let channel = self.editor.output_channel();
        let node = self
            .editor
            .active_nodes(channel)
            .iter()
            .position(|point| (point.x - x).abs() <= BAND_NODE_DISTANCE);
        if let Some(node) = node {
            return Ok((Some(node), self.editor.active_nodes(channel)[node].y));
        }
        if !sampled_curve_y.is_finite() {
            return Err(ColorZonesInteractionError::NonFiniteInput);
        }
        Ok((None, sampled_curve_y))
    }
}

fn validate_finite_pair(x: f32, y: f32) -> Result<(), ColorZonesInteractionError> {
    if x.is_finite() && y.is_finite() {
        Ok(())
    } else {
        Err(ColorZonesInteractionError::NonFiniteInput)
    }
}

fn validate_speed_multiplier(speed_multiplier: f32) -> Result<(), ColorZonesInteractionError> {
    if speed_multiplier.is_finite() {
        Ok(())
    } else {
        Err(ColorZonesInteractionError::NonFiniteInput)
    }
}

fn gaussian_weight(distance: f32, radius_squared: f32) -> f32 {
    (-(distance * distance) / radius_squared).exp()
}

fn band_x(band: usize) -> Result<f32, ColorZonesInteractionError> {
    if band >= COLORZONES_BANDS {
        return Err(ColorZonesInteractionError::InvalidBand(band));
    }
    let band_u16 = u16::try_from(band).expect("validated Color Zones band fits u16");
    Ok(f32::from(band_u16) / 8.0)
}

#[cfg(test)]
mod tests {
    use super::{
        COLORZONES_BAND_NAMES, COLORZONES_DEFAULT_STEP, COLORZONES_PICKER_FEATHER,
        ColorZonesArrowKey, ColorZonesBandEffect, ColorZonesClick, ColorZonesInteraction,
        ColorZonesInteractionError, ColorZonesModifiers, ColorZonesPickerRange,
        ColorZonesPrimaryOutcome, ColorZonesScrollOutcome, ColorZonesSecondaryOutcome,
        ColorZonesSelection,
    };
    use crate::iop::colorzones::model::{COLORZONES_MIN_X_DISTANCE, ColorZonesEditorState};
    use rusttable_processing::{
        COLORZONES_MAX_NODES, ColorZonesChannel, ColorZonesCurveType, ColorZonesNode,
        ColorZonesSplinesVersion,
    };

    #[test]
    fn modifier_matching_is_exact_across_the_three_relevant_keys() {
        assert!(ColorZonesModifiers::CONTROL.is_exact(ColorZonesModifiers::CONTROL));
        assert!(
            !ColorZonesModifiers::new(true, true, false).is_exact(ColorZonesModifiers::CONTROL)
        );
        assert!(!ColorZonesModifiers::new(false, false, true).is_exact(ColorZonesModifiers::NONE));
    }

    #[test]
    fn ctrl_inserts_but_ctrl_shift_starts_an_area_drag() {
        let mut interaction = ColorZonesInteraction::default();
        interaction.set_pointer(0.5, 0.8).unwrap();
        assert_eq!(
            interaction
                .primary_press(
                    ColorZonesClick::Single,
                    ColorZonesModifiers::CONTROL,
                    Some(0.5),
                )
                .unwrap(),
            ColorZonesPrimaryOutcome::NodeInserted(1)
        );
        assert_eq!(
            interaction
                .editor()
                .active_nodes(ColorZonesChannel::Lightness)[1],
            ColorZonesNode::new(0.5, 0.5)
        );

        interaction.set_edit_by_area(true);
        assert_eq!(
            interaction
                .primary_press(
                    ColorZonesClick::Single,
                    ColorZonesModifiers::new(true, true, false),
                    None,
                )
                .unwrap(),
            ColorZonesPrimaryOutcome::AreaDragStarted
        );
    }

    #[test]
    fn right_click_gives_v1_boundaries_precedence_over_ctrl_neutralization() {
        let mut editor = ColorZonesEditorState::default();
        editor.set_splines_version(ColorZonesSplinesVersion::V1);
        editor.set_output_channel(ColorZonesChannel::Chroma);
        editor.move_node(0, 0.0, 0.9).unwrap();
        let mut interaction = ColorZonesInteraction::new(editor);
        interaction.set_selection(ColorZonesSelection::Node(0));

        assert_eq!(
            interaction
                .secondary_press(ColorZonesModifiers::CONTROL)
                .unwrap(),
            ColorZonesSecondaryOutcome::Deleted(
                super::ColorZonesDeleteOutcome::BoundaryNeutralized
            )
        );
        assert_eq!(interaction.selection(), ColorZonesSelection::Node(0));
        assert_eq!(
            interaction.editor().active_nodes(ColorZonesChannel::Chroma),
            [ColorZonesNode::new(0.0, 0.5), ColorZonesNode::new(1.0, 0.5)]
        );
    }

    #[test]
    fn ctrl_right_neutralizes_interior_but_extra_modifiers_delete() {
        let mut interaction = ColorZonesInteraction::default();
        interaction.editor_mut().insert_node(0.5, 0.8).unwrap();
        interaction.set_selection(ColorZonesSelection::Node(1));
        assert_eq!(
            interaction
                .secondary_press(ColorZonesModifiers::CONTROL)
                .unwrap(),
            ColorZonesSecondaryOutcome::Neutralized
        );
        assert_eq!(
            interaction
                .editor()
                .active_nodes(ColorZonesChannel::Lightness)[1]
                .y
                .to_bits(),
            0.5_f32.to_bits()
        );

        interaction.set_selection(ColorZonesSelection::Node(1));
        assert!(matches!(
            interaction
                .secondary_press(ColorZonesModifiers::new(true, true, false))
                .unwrap(),
            ColorZonesSecondaryOutcome::Deleted(super::ColorZonesDeleteOutcome::Deleted)
        ));
    }

    #[test]
    fn double_click_resets_curve_and_suppresses_immediate_reinsertion() {
        let mut interaction = ColorZonesInteraction::default();
        interaction.editor_mut().insert_node(0.5, 0.9).unwrap();
        assert_eq!(
            interaction
                .primary_press(ColorZonesClick::Double, ColorZonesModifiers::CONTROL, None,)
                .unwrap(),
            ColorZonesPrimaryOutcome::CurveReset
        );
        assert_eq!(interaction.selection(), ColorZonesSelection::Suppressed);
        assert_eq!(
            interaction
                .editor()
                .active_nodes(ColorZonesChannel::Lightness),
            [
                ColorZonesNode::new(0.25, 0.5),
                ColorZonesNode::new(0.75, 0.5)
            ]
        );
    }

    #[test]
    fn wheel_and_arrow_keys_use_the_native_point_zero_zero_one_step() {
        let mut interaction = ColorZonesInteraction::default();
        interaction.set_selection(ColorZonesSelection::Node(0));
        interaction
            .scroll(1.0, ColorZonesModifiers::NONE, false)
            .unwrap();
        interaction.key_press(ColorZonesArrowKey::Right).unwrap();
        let point = interaction
            .editor()
            .active_nodes(ColorZonesChannel::Lightness)[0];
        assert_eq!(
            point.x.to_bits(),
            (0.25 + COLORZONES_DEFAULT_STEP).to_bits()
        );
        assert_eq!(point.y.to_bits(), (0.5 - COLORZONES_DEFAULT_STEP).to_bits());
    }

    #[test]
    fn wheel_at_node_boundary_is_consumed_without_reporting_a_move() {
        let mut interaction = ColorZonesInteraction::default();
        interaction.editor_mut().move_node(0, 0.25, 0.0).unwrap();
        interaction.set_selection(ColorZonesSelection::Node(0));
        let previous = *interaction.editor().parameters();

        assert_eq!(
            interaction
                .scroll(1.0, ColorZonesModifiers::NONE, false)
                .unwrap(),
            ColorZonesScrollOutcome::Consumed
        );
        assert_eq!(interaction.editor().parameters(), &previous);
    }

    #[test]
    fn accelerator_speed_scales_drag_wheel_key_and_band_movements() {
        let mut drag = ColorZonesInteraction::default();
        drag.set_pointer(0.25, 0.5).unwrap();
        drag.set_selection(ColorZonesSelection::Node(0));
        assert!(drag.primary_drag_to_with_speed(0.251, 0.501, 10.0).unwrap());
        let dragged = drag.editor().active_nodes(ColorZonesChannel::Lightness)[0];
        assert_eq!(
            dragged.x.to_bits(),
            (0.25 + (0.251_f32 - 0.25) * 10.0).to_bits()
        );
        assert_eq!(
            dragged.y.to_bits(),
            (0.5 + (0.501_f32 - 0.5) * 10.0).to_bits()
        );

        let mut interaction = ColorZonesInteraction::default();
        interaction.set_selection(ColorZonesSelection::Node(0));
        interaction
            .key_press_with_speed(ColorZonesArrowKey::Right, 10.0)
            .unwrap();
        interaction
            .scroll_with_speed(1.0, ColorZonesModifiers::NONE, false, 10.0)
            .unwrap();
        let point = interaction
            .editor()
            .active_nodes(ColorZonesChannel::Lightness)[0];
        assert_eq!(
            point.x.to_bits(),
            (0.25 + 10.0 * COLORZONES_DEFAULT_STEP).to_bits()
        );
        assert_eq!(
            point.y.to_bits(),
            (0.5 - 10.0 * COLORZONES_DEFAULT_STEP).to_bits()
        );

        let outcome = interaction
            .apply_band_action_with_speed(4, ColorZonesBandEffect::Up, 1.0, 0.5, 10.0)
            .unwrap();
        assert_eq!(outcome.value.to_bits(), 0.6_f32.to_bits());
    }

    #[test]
    fn too_close_drag_is_consumed_without_changing_the_editor() {
        let mut interaction = ColorZonesInteraction::default();
        interaction.set_pointer(0.25, 0.5).unwrap();
        interaction.set_selection(ColorZonesSelection::Node(0));
        let previous = *interaction.editor().parameters();

        assert!(
            !interaction
                .primary_drag_to(0.75 - COLORZONES_MIN_X_DISTANCE, 0.5)
                .unwrap()
        );
        assert_eq!(interaction.editor().parameters(), &previous);
    }

    #[test]
    fn ordinary_drag_selection_survives_leave_until_primary_release() {
        let mut interaction = ColorZonesInteraction::default();
        interaction.set_selection(ColorZonesSelection::Node(0));
        interaction
            .primary_press(ColorZonesClick::Single, ColorZonesModifiers::NONE, None)
            .unwrap();
        interaction.leave();
        assert_eq!(interaction.selection(), ColorZonesSelection::Node(0));
        interaction.primary_release();
        assert_eq!(interaction.selection(), ColorZonesSelection::None);
    }

    #[test]
    fn only_alt_alone_forwards_scroll_to_channel_tabs() {
        let mut interaction = ColorZonesInteraction::default();
        assert_eq!(
            interaction
                .scroll(-1.0, ColorZonesModifiers::ALT, true)
                .unwrap(),
            ColorZonesScrollOutcome::ForwardToChannelTabs { delta_y: -1.0 }
        );
        assert_eq!(interaction.zoom_factor().to_bits(), 1.0_f32.to_bits());
        assert_eq!(
            interaction
                .scroll(-1.0, ColorZonesModifiers::new(true, false, true), true,)
                .unwrap(),
            ColorZonesScrollOutcome::Zoomed
        );
        assert!(interaction.zoom_factor() > 1.0);
    }

    #[test]
    fn area_radius_obeys_node_count_lower_bound_and_one_upper_bound() {
        let mut interaction = ColorZonesInteraction::default();
        interaction.set_edit_by_area(true);
        interaction
            .scroll(-100.0, ColorZonesModifiers::NONE, false)
            .unwrap();
        assert_eq!(interaction.area_radius().to_bits(), 0.1_f32.to_bits());
        interaction
            .scroll(100.0, ColorZonesModifiers::NONE, false)
            .unwrap();
        assert_eq!(interaction.area_radius().to_bits(), 1.0_f32.to_bits());
    }

    #[test]
    fn gaussian_area_edit_couples_v1_hue_endpoints() {
        let mut editor = ColorZonesEditorState::default();
        editor.set_splines_version(ColorZonesSplinesVersion::V1);
        editor.set_output_channel(ColorZonesChannel::Hue);
        let mut interaction = ColorZonesInteraction::new(editor);
        interaction.set_pointer(0.0, 1.0).unwrap();
        interaction.apply_gaussian_area_edit().unwrap();
        let points = interaction.editor().active_nodes(ColorZonesChannel::Hue);
        assert_eq!(points[0].y.to_bits(), points[1].y.to_bits());
        assert!(points[0].y > 0.99);
    }

    #[test]
    fn picker_builds_five_feathered_nodes_with_exact_polarity() {
        let picker = ColorZonesPickerRange {
            minimum: 0.4,
            mean: 0.5,
            maximum: 0.6,
        };
        let mut positive = ColorZonesInteraction::default();
        assert_eq!(
            positive
                .apply_picker_curve(picker, ColorZonesModifiers::CONTROL)
                .unwrap(),
            5
        );
        let points = positive.editor().active_nodes(ColorZonesChannel::Lightness);
        assert!(points.contains(&ColorZonesNode::new(0.4 - COLORZONES_PICKER_FEATHER, 0.5)));
        assert!(points.contains(&ColorZonesNode::new(0.5, 0.7)));

        let mut negative = ColorZonesInteraction::default();
        negative
            .apply_picker_curve(picker, ColorZonesModifiers::SHIFT)
            .unwrap();
        assert!(
            negative
                .editor()
                .active_nodes(ColorZonesChannel::Lightness)
                .contains(&ColorZonesNode::new(0.5, 0.3))
        );

        let mut flat = ColorZonesInteraction::default();
        flat.apply_picker_curve(picker, ColorZonesModifiers::new(true, true, false))
            .unwrap();
        assert!(
            flat.editor()
                .active_nodes(ColorZonesChannel::Lightness)
                .iter()
                .all(|node| node.y.to_bits() == 0.5_f32.to_bits())
        );

        let mut editor = ColorZonesEditorState::default();
        editor.set_splines_version(ColorZonesSplinesVersion::V1);
        editor.set_selection_channel(ColorZonesChannel::Chroma);
        editor.set_curve_type(ColorZonesChannel::Lightness, ColorZonesCurveType::Monotone);
        editor
            .insert_node_on(ColorZonesChannel::Lightness, 0.5, 0.9)
            .unwrap();
        let mut non_hue_v1 = ColorZonesInteraction::new(editor);
        non_hue_v1
            .apply_picker_curve(picker, ColorZonesModifiers::NONE)
            .unwrap();
        let editor = non_hue_v1.editor();
        let points = editor.active_nodes(ColorZonesChannel::Lightness);
        assert_eq!(points.len(), 7);
        assert_eq!(
            editor.curve_type(ColorZonesChannel::Lightness),
            ColorZonesCurveType::Catmull
        );
        assert!(points.contains(&ColorZonesNode::new(0.25, 0.5)));
        assert!(points.contains(&ColorZonesNode::new(0.75, 0.5)));
        assert!(
            editor.parameters().curves[ColorZonesChannel::Lightness.index()]
                [points.len()..COLORZONES_MAX_NODES]
                .iter()
                .all(|node| *node == ColorZonesNode::new(0.0, 0.0))
        );
    }

    #[test]
    fn zoom_anchors_pointer_pan_clamps_and_reset_restores_identity() {
        let mut interaction = ColorZonesInteraction::default();
        interaction.set_pointer(0.75, 0.25).unwrap();
        interaction.zoom_at_pointer(-1.0).unwrap();
        let anchored_x = interaction.view_to_curve(0.75, interaction.offsets().0);
        assert!((anchored_x - 0.75).abs() < 1.0e-6);
        interaction.pan_by_view_delta(10.0, -10.0).unwrap();
        let maximum = (interaction.zoom_factor() - 1.0) / interaction.zoom_factor();
        assert_eq!(interaction.offsets(), (maximum, 0.0));
        interaction.reset_zoom();
        assert_eq!(interaction.zoom_factor().to_bits(), 1.0_f32.to_bits());
        assert_eq!(interaction.offsets(), (0.0, 0.0));
    }

    #[test]
    fn all_eight_band_actions_use_native_names_positions_and_effects() {
        assert_eq!(
            COLORZONES_BAND_NAMES,
            [
                "red", "orange", "yellow", "green", "aqua", "blue", "purple", "magenta"
            ]
        );
        let mut interaction = ColorZonesInteraction::default();
        for band in 0..8 {
            let outcome = interaction
                .apply_band_action(band, ColorZonesBandEffect::Up, 1.0, 0.5)
                .unwrap();
            assert!(outcome.changed);
            assert!(outcome.value >= 0.5);
        }
        let top = interaction
            .apply_band_action(3, ColorZonesBandEffect::Top, 0.0, 0.5)
            .unwrap();
        assert_eq!(top.value.to_bits(), 1.0_f32.to_bits());
        let bottom = interaction
            .apply_band_action(3, ColorZonesBandEffect::Bottom, 0.0, 0.5)
            .unwrap();
        assert_eq!(bottom.value.to_bits(), 0.0_f32.to_bits());
        let reset = interaction
            .apply_band_action(3, ColorZonesBandEffect::Reset, 0.0, 0.5)
            .unwrap();
        assert!(reset.changed);
        assert_eq!(
            interaction.band_value(8, 0.5),
            Err(ColorZonesInteractionError::InvalidBand(8))
        );
    }
}
