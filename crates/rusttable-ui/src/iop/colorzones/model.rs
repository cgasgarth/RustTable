//! Pure Color Zones editor state derived from `src/iop/colorzones.c`.
//!
//! This module owns the source-shaped curve mutations independently of GTK.
//! Parameters remain in the canonical native v5 layout so callers can persist
//! each accepted mutation without translating through a second curve model.

#![expect(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    reason = "The typed mutation errors and construction-validated state invariants are documented at the model boundary."
)]

use std::fmt;

use rusttable_processing::{
    COLORZONES_CHANNELS, COLORZONES_MAX_NODES, ColorZonesChannel, ColorZonesCurveType,
    ColorZonesMode, ColorZonesNode, ColorZonesParametersV5, ColorZonesSplinesVersion,
};

/// Native minimum horizontal distance between neighboring curve nodes.
pub const COLORZONES_MIN_X_DISTANCE: f32 = 0.0025;

const NEUTRAL_Y: f32 = 0.5;
const MIN_STRENGTH: f32 = -200.0;
const MAX_STRENGTH: f32 = 200.0;
const ZERO_NODE: ColorZonesNode = ColorZonesNode::new(0.0, 0.0);

/// Result of a source-compatible node deletion request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorZonesDeleteOutcome {
    /// The requested node was removed and the active tail shifted left.
    Deleted,
    /// A spline-v1 boundary node was restored instead of being removed.
    BoundaryNeutralized,
    /// Spline v2 retained its sole node and restored it to `(0.5, 0.5)`.
    SoleNodeReset,
}

/// Invalid input to the pure Color Zones editor model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorZonesEditorError {
    /// The persisted selection-channel tag is unknown.
    InvalidSelectionChannel(i32),
    /// The persisted process-mode tag is unknown.
    InvalidMode(i32),
    /// The persisted spline-version tag is unknown.
    InvalidSplinesVersion(i32),
    /// One curve has an unknown interpolation tag.
    InvalidCurveType { channel: usize, value: i32 },
    /// One curve has a node count outside the native version-specific range.
    InvalidNodeCount { channel: usize, count: i32 },
    /// One active coordinate is not finite.
    NonFiniteCoordinate {
        channel: usize,
        node: usize,
        coordinate: &'static str,
    },
    /// One active coordinate lies outside the editable unit square.
    CoordinateOutOfRange {
        channel: usize,
        node: usize,
        coordinate: &'static str,
    },
    /// Active nodes are not strictly ordered by their x coordinate.
    UnsortedNodes { channel: usize, node: usize },
    /// A requested node index is not active.
    InvalidNodeIndex { node: usize, active: usize },
    /// A coordinate supplied for insertion or movement is not finite.
    NonFiniteInput,
    /// An insertion coordinate lies outside the editable unit square.
    InputOutOfRange,
    /// The curve already contains the native maximum of 20 nodes.
    NodeLimitReached,
    /// The requested x coordinate violates native adjacent-node separation.
    NodesTooClose,
}

impl fmt::Display for ColorZonesEditorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSelectionChannel(value) => {
                write!(
                    formatter,
                    "Color Zones selection channel tag {value} is invalid"
                )
            }
            Self::InvalidMode(value) => {
                write!(formatter, "Color Zones mode tag {value} is invalid")
            }
            Self::InvalidSplinesVersion(value) => {
                write!(
                    formatter,
                    "Color Zones spline version tag {value} is invalid"
                )
            }
            Self::InvalidCurveType { channel, value } => write!(
                formatter,
                "Color Zones curve {channel} interpolation tag {value} is invalid"
            ),
            Self::InvalidNodeCount { channel, count } => write!(
                formatter,
                "Color Zones curve {channel} has invalid active node count {count}"
            ),
            Self::NonFiniteCoordinate {
                channel,
                node,
                coordinate,
            } => write!(
                formatter,
                "Color Zones curve {channel} node {node} {coordinate} is non-finite"
            ),
            Self::CoordinateOutOfRange {
                channel,
                node,
                coordinate,
            } => write!(
                formatter,
                "Color Zones curve {channel} node {node} {coordinate} is outside 0..=1"
            ),
            Self::UnsortedNodes { channel, node } => write!(
                formatter,
                "Color Zones curve {channel} node {node} is not strictly after its predecessor"
            ),
            Self::InvalidNodeIndex { node, active } => write!(
                formatter,
                "Color Zones node index {node} is outside the active prefix of length {active}"
            ),
            Self::NonFiniteInput => formatter.write_str("Color Zones editor input is non-finite"),
            Self::InputOutOfRange => {
                formatter.write_str("Color Zones editor input is outside 0..=1")
            }
            Self::NodeLimitReached => write!(
                formatter,
                "Color Zones curve already contains {COLORZONES_MAX_NODES} nodes"
            ),
            Self::NodesTooClose => write!(
                formatter,
                "Color Zones nodes must remain more than {COLORZONES_MIN_X_DISTANCE} apart"
            ),
        }
    }
}

impl std::error::Error for ColorZonesEditorError {}

/// Source-lineage pure editor state over canonical Color Zones v5 parameters.
///
/// The selection channel is persisted in `parameters.channel`. The output
/// channel is transient GUI state and deliberately independent, matching
/// Darktable's `p->channel` and `g->channel` split.
#[derive(Debug, Clone, PartialEq)]
pub struct ColorZonesEditorState {
    parameters: ColorZonesParametersV5,
    output_channel: ColorZonesChannel,
}

impl Default for ColorZonesEditorState {
    fn default() -> Self {
        Self {
            parameters: ColorZonesParametersV5::defaults(),
            output_channel: ColorZonesChannel::Lightness,
        }
    }
}

impl ColorZonesEditorState {
    /// Builds native v5 defaults with the requested transient output channel.
    #[must_use]
    pub fn with_output_channel(output_channel: ColorZonesChannel) -> Self {
        Self {
            output_channel,
            ..Self::default()
        }
    }

    /// Loads editable v5 parameters and clears every inactive tail slot.
    ///
    /// Active points are preserved exactly. Invalid enum tags, counts,
    /// non-finite coordinates, out-of-range coordinates, and unsorted active
    /// prefixes are rejected rather than silently rewritten.
    pub fn from_parameters(
        mut parameters: ColorZonesParametersV5,
        output_channel: ColorZonesChannel,
    ) -> Result<Self, ColorZonesEditorError> {
        validate_parameters(&parameters)?;
        normalize_inactive_tails(&mut parameters);
        Ok(Self {
            parameters,
            output_channel,
        })
    }

    /// Returns the canonical v5 parameters.
    #[must_use]
    pub const fn parameters(&self) -> &ColorZonesParametersV5 {
        &self.parameters
    }

    /// Returns a copy of the canonical v5 parameters for persistence.
    #[must_use]
    pub const fn parameters_value(&self) -> ColorZonesParametersV5 {
        self.parameters
    }

    /// Consumes the state and returns canonical v5 parameters.
    #[must_use]
    pub const fn into_parameters(self) -> ColorZonesParametersV5 {
        self.parameters
    }

    /// Returns the persisted channel used as the graph abscissa.
    #[must_use]
    pub const fn selection_channel(&self) -> ColorZonesChannel {
        ColorZonesChannel::from_raw(self.parameters.channel)
            .expect("editor construction validates the selection channel")
    }

    /// Returns the independently selected curve edited by the graph ordinate.
    #[must_use]
    pub const fn output_channel(&self) -> ColorZonesChannel {
        self.output_channel
    }

    /// Selects which output curve subsequent curve operations edit.
    pub const fn set_output_channel(&mut self, channel: ColorZonesChannel) {
        self.output_channel = channel;
    }

    /// Changes the graph selection criterion using native `gui_changed` rules.
    ///
    /// Darktable resets all curves, interpolation modes, strength, and process
    /// mode when this parameter changes. The spline version and transient
    /// output channel are retained.
    pub fn set_selection_channel(&mut self, channel: ColorZonesChannel) {
        if channel != self.selection_channel() {
            let version = self.splines_version();
            self.reset_parameters(channel, version, channel != ColorZonesChannel::Hue);
        }
    }

    /// Returns the active nodes for one output curve.
    #[must_use]
    pub fn active_nodes(&self, channel: ColorZonesChannel) -> &[ColorZonesNode] {
        let index = channel.index();
        &self.parameters.curves[index][..self.active_count(channel)]
    }

    /// Returns the number of active nodes for one output curve.
    #[must_use]
    pub fn active_count(&self, channel: ColorZonesChannel) -> usize {
        usize::try_from(self.parameters.curve_num_nodes[channel.index()])
            .expect("editor construction validates positive node counts")
    }

    /// Returns the interpolation mode for one output curve.
    #[must_use]
    pub const fn curve_type(&self, channel: ColorZonesChannel) -> ColorZonesCurveType {
        ColorZonesCurveType::from_raw(self.parameters.curve_type[channel.index()])
            .expect("editor construction validates interpolation tags")
    }

    /// Sets the interpolation mode for one output curve.
    pub const fn set_curve_type(
        &mut self,
        channel: ColorZonesChannel,
        curve_type: ColorZonesCurveType,
    ) {
        self.parameters.curve_type[channel.index()] = curve_type.raw();
    }

    /// Returns the native point-processing mode.
    #[must_use]
    pub const fn mode(&self) -> ColorZonesMode {
        ColorZonesMode::from_raw(self.parameters.mode)
            .expect("editor construction validates the process mode")
    }

    /// Sets the native point-processing mode.
    pub const fn set_mode(&mut self, mode: ColorZonesMode) {
        self.parameters.mode = mode.raw();
    }

    /// Returns the native strength value.
    #[must_use]
    pub const fn strength(&self) -> f32 {
        self.parameters.strength
    }

    /// Sets strength using the native `-200..=200` editable range.
    pub const fn set_strength(&mut self, strength: f32) -> Result<(), ColorZonesEditorError> {
        if !strength.is_finite() {
            return Err(ColorZonesEditorError::NonFiniteInput);
        }
        self.parameters.strength = strength.clamp(MIN_STRENGTH, MAX_STRENGTH);
        Ok(())
    }

    /// Returns the persisted spline-boundary implementation version.
    #[must_use]
    pub const fn splines_version(&self) -> ColorZonesSplinesVersion {
        ColorZonesSplinesVersion::from_raw(self.parameters.splines_version)
            .expect("editor construction validates the spline version")
    }

    /// Changes spline implementation using native `_reset_parameters` rules.
    ///
    /// The native UI does not reinterpret arbitrary nodes between the two
    /// incompatible boundary representations. A transition therefore starts
    /// neutral curves for the existing selection criterion, preserving the
    /// transient output channel.
    pub fn set_splines_version(&mut self, version: ColorZonesSplinesVersion) {
        if version != self.splines_version() {
            let selection_channel = self.selection_channel();
            let touch_edges = version == ColorZonesSplinesVersion::V1
                || selection_channel != ColorZonesChannel::Hue;
            self.reset_parameters(selection_channel, version, touch_edges);
        }
    }

    /// Inserts a point into the currently selected output curve in x order.
    pub fn insert_node(&mut self, x: f32, y: f32) -> Result<usize, ColorZonesEditorError> {
        self.insert_node_on(self.output_channel, x, y)
    }

    /// Inserts a point into one output curve in x order.
    pub fn insert_node_on(
        &mut self,
        channel: ColorZonesChannel,
        x: f32,
        y: f32,
    ) -> Result<usize, ColorZonesEditorError> {
        validate_unit_input(x, y)?;
        let channel_index = channel.index();
        let count = self.active_count(channel);
        if count == COLORZONES_MAX_NODES {
            return Err(ColorZonesEditorError::NodeLimitReached);
        }

        let insertion =
            self.parameters.curves[channel_index][..count].partition_point(|node| node.x < x);
        if !x_is_separated_for_insertion(
            &self.parameters.curves[channel_index],
            count,
            insertion,
            x,
        ) {
            return Err(ColorZonesEditorError::NodesTooClose);
        }

        let curve = &mut self.parameters.curves[channel_index];
        curve.copy_within(insertion..count, insertion + 1);
        curve[insertion] = ColorZonesNode::new(x, y);
        self.parameters.curve_num_nodes[channel_index] += 1;
        debug_assert!(inactive_tail_is_zero(curve, count + 1));
        Ok(insertion)
    }

    /// Moves one active point to an absolute coordinate.
    ///
    /// Coordinates are clamped to the native editable unit square. Spline-v1
    /// boundary x coordinates remain fixed. With hue selection, moving either
    /// spline-v1 boundary links both endpoint values; spline v2 instead keeps
    /// endpoint values independent and enforces separation across the wrap.
    pub fn move_node(
        &mut self,
        node: usize,
        x: f32,
        y: f32,
    ) -> Result<bool, ColorZonesEditorError> {
        self.move_node_on(self.output_channel, node, x, y)
    }

    /// Moves one active point on a specified output curve.
    ///
    /// Returns `false` when native separation checks consume the movement as an
    /// unchanged no-op.
    #[expect(
        clippy::while_float,
        reason = "Source spline separation advances with next_up/next_down until the native float boundary is met."
    )]
    pub fn move_node_on(
        &mut self,
        channel: ColorZonesChannel,
        node: usize,
        x: f32,
        y: f32,
    ) -> Result<bool, ColorZonesEditorError> {
        if !x.is_finite() || !y.is_finite() {
            return Err(ColorZonesEditorError::NonFiniteInput);
        }
        let channel_index = channel.index();
        let count = self.active_count(channel);
        validate_node_index(node, count)?;
        let version = self.splines_version();
        let periodic = self.has_periodic_boundary();
        let boundary = node == 0 || node + 1 == count;
        let curve = &mut self.parameters.curves[channel_index];

        let mut new_x = x.clamp(0.0, 1.0);
        let new_y = y.clamp(0.0, 1.0);
        if version == ColorZonesSplinesVersion::V1 && boundary {
            new_x = curve[node].x;
        }
        if !x_is_separated_for_move(curve, count, node, new_x) {
            return Ok(false);
        }

        if version == ColorZonesSplinesVersion::V2 && periodic && boundary {
            if node == 0 && new_x + 1.0 - curve[count - 1].x < COLORZONES_MIN_X_DISTANCE {
                new_x = curve[count - 1].x + COLORZONES_MIN_X_DISTANCE - 1.0;
                while new_x + 1.0 - curve[count - 1].x < COLORZONES_MIN_X_DISTANCE {
                    new_x = new_x.next_up();
                }
            } else if node + 1 == count && curve[0].x + 1.0 - new_x < COLORZONES_MIN_X_DISTANCE {
                new_x = curve[0].x + 1.0 - COLORZONES_MIN_X_DISTANCE;
                while curve[0].x + 1.0 - new_x < COLORZONES_MIN_X_DISTANCE {
                    new_x = new_x.next_down();
                }
            }
            if !(0.0..=1.0).contains(&new_x) || !x_is_separated_for_move(curve, count, node, new_x)
            {
                return Ok(false);
            }
        }

        let linked = (version == ColorZonesSplinesVersion::V1 && periodic && boundary)
            .then(|| count - 1 - node);
        let previous = curve[node];
        let previous_linked = linked.map(|linked| curve[linked]);
        curve[node] = ColorZonesNode::new(new_x, new_y);
        if let Some(linked) = linked {
            curve[linked] = ColorZonesNode::new(1.0 - new_x, new_y);
        }
        let selected_changed =
            previous.x.to_bits() != new_x.to_bits() || previous.y.to_bits() != new_y.to_bits();
        let linked_changed = linked
            .zip(previous_linked)
            .is_some_and(|(linked, previous)| {
                let current = curve[linked];
                previous.x.to_bits() != current.x.to_bits()
                    || previous.y.to_bits() != current.y.to_bits()
            });
        Ok(selected_changed || linked_changed)
    }

    /// Applies a delta to one point on the current output curve.
    pub fn move_node_by(
        &mut self,
        node: usize,
        dx: f32,
        dy: f32,
    ) -> Result<bool, ColorZonesEditorError> {
        let current = *self
            .active_nodes(self.output_channel)
            .get(node)
            .ok_or_else(|| ColorZonesEditorError::InvalidNodeIndex {
                node,
                active: self.active_count(self.output_channel),
            })?;
        self.move_node(node, current.x + dx, current.y + dy)
    }

    /// Restores one active node to the neutral ordinate.
    ///
    /// Native spline-v1 boundaries also return to x=0 or x=1. With hue
    /// selection they are a linked pair restored together.
    pub fn neutralize_node(&mut self, node: usize) -> Result<(), ColorZonesEditorError> {
        self.neutralize_node_on(self.output_channel, node)
    }

    /// Restores one active node on a specified output curve to neutral.
    pub fn neutralize_node_on(
        &mut self,
        channel: ColorZonesChannel,
        node: usize,
    ) -> Result<(), ColorZonesEditorError> {
        let count = self.active_count(channel);
        validate_node_index(node, count)?;
        let v1_boundary = self.splines_version() == ColorZonesSplinesVersion::V1
            && (node == 0 || node + 1 == count);
        let periodic = self.has_periodic_boundary();
        let curve = &mut self.parameters.curves[channel.index()];
        if v1_boundary && periodic {
            curve[0] = ColorZonesNode::new(0.0, NEUTRAL_Y);
            curve[count - 1] = ColorZonesNode::new(1.0, NEUTRAL_Y);
        } else if v1_boundary {
            curve[node] = ColorZonesNode::new(if node == 0 { 0.0 } else { 1.0 }, NEUTRAL_Y);
        } else {
            curve[node].y = NEUTRAL_Y;
        }
        Ok(())
    }

    /// Deletes one active node from the current output curve.
    pub fn delete_node(
        &mut self,
        node: usize,
    ) -> Result<ColorZonesDeleteOutcome, ColorZonesEditorError> {
        self.delete_node_on(self.output_channel, node)
    }

    /// Deletes one active node from a specified output curve.
    pub fn delete_node_on(
        &mut self,
        channel: ColorZonesChannel,
        node: usize,
    ) -> Result<ColorZonesDeleteOutcome, ColorZonesEditorError> {
        let count = self.active_count(channel);
        validate_node_index(node, count)?;
        let version = self.splines_version();
        let periodic = self.has_periodic_boundary();
        if version == ColorZonesSplinesVersion::V1 && (node == 0 || node + 1 == count) {
            let curve = &mut self.parameters.curves[channel.index()];
            if periodic {
                curve[0] = ColorZonesNode::new(0.0, NEUTRAL_Y);
                curve[count - 1] = ColorZonesNode::new(1.0, NEUTRAL_Y);
            } else {
                curve[node] = ColorZonesNode::new(if node == 0 { 0.0 } else { 1.0 }, NEUTRAL_Y);
            }
            return Ok(ColorZonesDeleteOutcome::BoundaryNeutralized);
        }

        let curve = &mut self.parameters.curves[channel.index()];
        if count > 1 {
            curve.copy_within(node + 1..count, node);
            curve[count - 1] = ZERO_NODE;
            self.parameters.curve_num_nodes[channel.index()] -= 1;
            debug_assert!(inactive_tail_is_zero(curve, count - 1));
            Ok(ColorZonesDeleteOutcome::Deleted)
        } else {
            curve[0] = ColorZonesNode::new(0.5, NEUTRAL_Y);
            Ok(ColorZonesDeleteOutcome::SoleNodeReset)
        }
    }

    /// Resets the current output curve to native neutral nodes and Catmull-Rom.
    pub fn reset_curve(&mut self) {
        self.reset_curve_on(self.output_channel);
    }

    /// Resets one output curve to native neutral nodes and Catmull-Rom.
    pub fn reset_curve_on(&mut self, channel: ColorZonesChannel) {
        let touch_edges = self.splines_version() == ColorZonesSplinesVersion::V1
            || self.selection_channel() != ColorZonesChannel::Hue;
        reset_one_curve(&mut self.parameters, channel, touch_edges);
    }

    /// Restores one curve verbatim from the module's canonical defaults.
    ///
    /// Native picker application uses `default_params`, independent of the
    /// current selection channel and spline version.
    pub const fn reset_curve_to_defaults_on(&mut self, channel: ColorZonesChannel) {
        let defaults = ColorZonesParametersV5::defaults();
        let index = channel.index();
        self.parameters.curves[index] = defaults.curves[index];
        self.parameters.curve_num_nodes[index] = defaults.curve_num_nodes[index];
        self.parameters.curve_type[index] = defaults.curve_type[index];
    }

    /// Restores complete canonical native v5 defaults.
    ///
    /// The transient output channel remains selected, as native GUI channel
    /// state is independent of module parameters.
    pub const fn reset_all(&mut self) {
        self.parameters = ColorZonesParametersV5::defaults();
    }

    fn has_periodic_boundary(&self) -> bool {
        self.selection_channel() == ColorZonesChannel::Hue
    }

    fn reset_parameters(
        &mut self,
        selection_channel: ColorZonesChannel,
        version: ColorZonesSplinesVersion,
        touch_edges: bool,
    ) {
        self.parameters = ColorZonesParametersV5::defaults();
        self.parameters.channel = selection_channel.raw();
        self.parameters.splines_version = version.raw();
        for channel in [
            ColorZonesChannel::Lightness,
            ColorZonesChannel::Chroma,
            ColorZonesChannel::Hue,
        ] {
            reset_one_curve(&mut self.parameters, channel, touch_edges);
        }
    }
}

fn validate_parameters(parameters: &ColorZonesParametersV5) -> Result<(), ColorZonesEditorError> {
    ColorZonesChannel::from_raw(parameters.channel).ok_or(
        ColorZonesEditorError::InvalidSelectionChannel(parameters.channel),
    )?;
    ColorZonesMode::from_raw(parameters.mode)
        .ok_or(ColorZonesEditorError::InvalidMode(parameters.mode))?;
    let version = ColorZonesSplinesVersion::from_raw(parameters.splines_version).ok_or(
        ColorZonesEditorError::InvalidSplinesVersion(parameters.splines_version),
    )?;
    if !parameters.strength.is_finite() {
        return Err(ColorZonesEditorError::NonFiniteInput);
    }

    for channel in 0..COLORZONES_CHANNELS {
        ColorZonesCurveType::from_raw(parameters.curve_type[channel]).ok_or(
            ColorZonesEditorError::InvalidCurveType {
                channel,
                value: parameters.curve_type[channel],
            },
        )?;
        let count = parameters.curve_num_nodes[channel];
        let minimum = if version == ColorZonesSplinesVersion::V1 {
            2
        } else {
            1
        };
        let maximum = i32::try_from(COLORZONES_MAX_NODES).expect("native node limit fits i32");
        if !(minimum..=maximum).contains(&count) {
            return Err(ColorZonesEditorError::InvalidNodeCount { channel, count });
        }
        let active = usize::try_from(count).expect("validated node count is positive");
        for node in 0..active {
            let point = parameters.curves[channel][node];
            validate_stored_coordinate(point.x, channel, node, "x")?;
            validate_stored_coordinate(point.y, channel, node, "y")?;
            if node > 0 && parameters.curves[channel][node - 1].x >= point.x {
                return Err(ColorZonesEditorError::UnsortedNodes { channel, node });
            }
        }
    }
    Ok(())
}

fn validate_stored_coordinate(
    value: f32,
    channel: usize,
    node: usize,
    coordinate: &'static str,
) -> Result<(), ColorZonesEditorError> {
    if !value.is_finite() {
        return Err(ColorZonesEditorError::NonFiniteCoordinate {
            channel,
            node,
            coordinate,
        });
    }
    if !(0.0..=1.0).contains(&value) {
        return Err(ColorZonesEditorError::CoordinateOutOfRange {
            channel,
            node,
            coordinate,
        });
    }
    Ok(())
}

fn validate_unit_input(x: f32, y: f32) -> Result<(), ColorZonesEditorError> {
    if !x.is_finite() || !y.is_finite() {
        return Err(ColorZonesEditorError::NonFiniteInput);
    }
    if !(0.0..=1.0).contains(&x) || !(0.0..=1.0).contains(&y) {
        return Err(ColorZonesEditorError::InputOutOfRange);
    }
    Ok(())
}

const fn validate_node_index(node: usize, active: usize) -> Result<(), ColorZonesEditorError> {
    if node >= active {
        Err(ColorZonesEditorError::InvalidNodeIndex { node, active })
    } else {
        Ok(())
    }
}

fn x_is_separated_for_insertion(
    curve: &[ColorZonesNode; COLORZONES_MAX_NODES],
    count: usize,
    insertion: usize,
    x: f32,
) -> bool {
    (insertion == 0 || x - curve[insertion - 1].x > COLORZONES_MIN_X_DISTANCE)
        && (insertion == count || curve[insertion].x - x > COLORZONES_MIN_X_DISTANCE)
}

fn x_is_separated_for_move(
    curve: &[ColorZonesNode; COLORZONES_MAX_NODES],
    count: usize,
    node: usize,
    x: f32,
) -> bool {
    (node == 0 || x - curve[node - 1].x > COLORZONES_MIN_X_DISTANCE)
        && (node + 1 == count || curve[node + 1].x - x > COLORZONES_MIN_X_DISTANCE)
}

const fn reset_one_curve(
    parameters: &mut ColorZonesParametersV5,
    channel: ColorZonesChannel,
    touch_edges: bool,
) {
    let index = channel.index();
    parameters.curves[index] = [ZERO_NODE; COLORZONES_MAX_NODES];
    parameters.curves[index][0] =
        ColorZonesNode::new(if touch_edges { 0.0 } else { 0.25 }, NEUTRAL_Y);
    parameters.curves[index][1] =
        ColorZonesNode::new(if touch_edges { 1.0 } else { 0.75 }, NEUTRAL_Y);
    parameters.curve_num_nodes[index] = 2;
    parameters.curve_type[index] = ColorZonesCurveType::Catmull.raw();
}

fn normalize_inactive_tails(parameters: &mut ColorZonesParametersV5) {
    for channel in 0..COLORZONES_CHANNELS {
        let active = usize::try_from(parameters.curve_num_nodes[channel])
            .expect("parameter validation precedes tail normalization");
        parameters.curves[channel][active..].fill(ZERO_NODE);
    }
}

fn inactive_tail_is_zero(curve: &[ColorZonesNode; COLORZONES_MAX_NODES], active: usize) -> bool {
    curve[active..]
        .iter()
        .all(|point| point.x.to_bits() == 0 && point.y.to_bits() == 0)
}

#[cfg(test)]
mod tests {
    use super::{
        COLORZONES_MIN_X_DISTANCE, ColorZonesDeleteOutcome, ColorZonesEditorError,
        ColorZonesEditorState, inactive_tail_is_zero,
    };
    use rusttable_processing::{
        COLORZONES_CHANNELS, COLORZONES_MAX_NODES, ColorZonesChannel, ColorZonesCurveType,
        ColorZonesMode, ColorZonesNode, ColorZonesParametersV5, ColorZonesSplinesVersion,
    };

    const CHANNELS: [ColorZonesChannel; COLORZONES_CHANNELS] = [
        ColorZonesChannel::Lightness,
        ColorZonesChannel::Chroma,
        ColorZonesChannel::Hue,
    ];

    #[test]
    fn defaults_preserve_native_v5_parameters_and_independent_output_channel() {
        let state = ColorZonesEditorState::default();
        assert_eq!(state.parameters(), &ColorZonesParametersV5::defaults());
        assert_eq!(state.selection_channel(), ColorZonesChannel::Hue);
        assert_eq!(state.output_channel(), ColorZonesChannel::Lightness);
        assert_eq!(state.splines_version(), ColorZonesSplinesVersion::V2);
        assert_eq!(state.mode(), ColorZonesMode::Smooth);
        assert_eq!(state.strength().to_bits(), 0);
        for channel in CHANNELS {
            assert_eq!(state.curve_type(channel), ColorZonesCurveType::Catmull);
            assert_eq!(
                state.active_nodes(channel),
                [
                    ColorZonesNode::new(0.25, 0.5),
                    ColorZonesNode::new(0.75, 0.5),
                ]
            );
            assert!(inactive_tail_is_zero(
                &state.parameters().curves[channel.index()],
                2
            ));
        }
    }

    #[test]
    fn selection_changes_reset_parameters_but_output_selection_stays_independent() {
        let mut state = ColorZonesEditorState::with_output_channel(ColorZonesChannel::Hue);
        state.set_strength(73.0).unwrap();
        state.set_mode(ColorZonesMode::Strong);
        state
            .insert_node_on(ColorZonesChannel::Chroma, 0.5, 0.8)
            .unwrap();

        state.set_selection_channel(ColorZonesChannel::Chroma);

        assert_eq!(state.selection_channel(), ColorZonesChannel::Chroma);
        assert_eq!(state.output_channel(), ColorZonesChannel::Hue);
        assert_eq!(state.strength().to_bits(), 0);
        assert_eq!(state.mode(), ColorZonesMode::Smooth);
        for channel in CHANNELS {
            assert_eq!(
                state.active_nodes(channel),
                [ColorZonesNode::new(0.0, 0.5), ColorZonesNode::new(1.0, 0.5),]
            );
        }
    }

    #[test]
    fn changing_a_v1_instance_to_hue_restores_default_interior_nodes() {
        let mut state = ColorZonesEditorState::default();
        state.set_splines_version(ColorZonesSplinesVersion::V1);
        state.set_selection_channel(ColorZonesChannel::Chroma);
        state.set_selection_channel(ColorZonesChannel::Hue);

        for channel in CHANNELS {
            assert_eq!(
                state.active_nodes(channel),
                [
                    ColorZonesNode::new(0.25, 0.5),
                    ColorZonesNode::new(0.75, 0.5),
                ]
            );
        }
    }

    #[test]
    fn spline_v1_hue_boundaries_move_and_neutralize_as_a_linked_pair() {
        let mut state = ColorZonesEditorState::default();
        state.set_splines_version(ColorZonesSplinesVersion::V1);
        state.set_output_channel(ColorZonesChannel::Chroma);

        state.move_node(0, 0.4, 0.8).unwrap();
        assert_eq!(
            state.active_nodes(ColorZonesChannel::Chroma),
            [ColorZonesNode::new(0.0, 0.8), ColorZonesNode::new(1.0, 0.8),]
        );

        state.neutralize_node(1).unwrap();
        assert_eq!(
            state.active_nodes(ColorZonesChannel::Chroma),
            [ColorZonesNode::new(0.0, 0.5), ColorZonesNode::new(1.0, 0.5),]
        );
        assert_eq!(
            state.delete_node(0).unwrap(),
            ColorZonesDeleteOutcome::BoundaryNeutralized
        );
        assert_eq!(state.active_count(ColorZonesChannel::Chroma), 2);
    }

    #[test]
    fn spline_v1_boundary_move_reports_repaired_mismatched_loaded_endpoint() {
        let mut parameters = ColorZonesParametersV5::defaults();
        parameters.splines_version = ColorZonesSplinesVersion::V1.raw();
        let channel = ColorZonesChannel::Chroma;
        parameters.curves[channel.index()][0] = ColorZonesNode::new(0.0, 0.2);
        parameters.curves[channel.index()][1] = ColorZonesNode::new(1.0, 0.8);
        let mut state = ColorZonesEditorState::from_parameters(parameters, channel).unwrap();

        assert!(state.move_node_on(channel, 0, 0.0, 0.2).unwrap());
        assert_eq!(
            state.active_nodes(channel),
            [ColorZonesNode::new(0.0, 0.2), ColorZonesNode::new(1.0, 0.2)]
        );
    }

    #[test]
    fn spline_v2_hue_boundaries_are_independent_and_wrap_separated() {
        let mut state = ColorZonesEditorState::default();
        state
            .move_node_on(ColorZonesChannel::Lightness, 1, 0.999, 0.2)
            .unwrap();
        state
            .move_node_on(ColorZonesChannel::Lightness, 0, 0.0, 0.8)
            .unwrap();

        let points = state.active_nodes(ColorZonesChannel::Lightness);
        assert_eq!(points[1].y.to_bits(), 0.2_f32.to_bits());
        assert_eq!(points[0].y.to_bits(), 0.8_f32.to_bits());
        assert!(points[0].x + 1.0 - points[1].x >= COLORZONES_MIN_X_DISTANCE);
        assert!(points[0].x > 0.0);
    }

    #[test]
    fn insertion_is_sorted_and_enforces_only_adjacent_distance() {
        let mut state = ColorZonesEditorState::default();
        let inserted = state
            .insert_node_on(ColorZonesChannel::Hue, 0.5, 0.7)
            .unwrap();
        assert_eq!(inserted, 1);
        assert_eq!(
            state.active_nodes(ColorZonesChannel::Hue),
            [
                ColorZonesNode::new(0.25, 0.5),
                ColorZonesNode::new(0.5, 0.7),
                ColorZonesNode::new(0.75, 0.5),
            ]
        );

        assert_eq!(
            state.insert_node_on(ColorZonesChannel::Hue, 0.5 + COLORZONES_MIN_X_DISTANCE, 0.4,),
            Err(ColorZonesEditorError::NodesTooClose)
        );

        let mut wrapped = single_node_state(0.999, ColorZonesChannel::Hue);
        assert_eq!(
            wrapped
                .insert_node_on(ColorZonesChannel::Lightness, 0.0, 0.5)
                .unwrap(),
            0
        );
        assert_eq!(
            wrapped.active_nodes(ColorZonesChannel::Lightness),
            [
                ColorZonesNode::new(0.0, 0.5),
                ColorZonesNode::new(0.999, 0.5)
            ]
        );
    }

    #[test]
    fn insertion_stops_at_twenty_nodes_and_keeps_tail_normalized() {
        let mut state = single_node_state(0.0, ColorZonesChannel::Hue);
        for node in 1..COLORZONES_MAX_NODES {
            let node = u16::try_from(node).unwrap();
            let x = f32::from(node) / 20.0;
            state
                .insert_node_on(ColorZonesChannel::Lightness, x, 0.5)
                .unwrap();
        }
        assert_eq!(state.active_count(ColorZonesChannel::Lightness), 20);
        assert_eq!(
            state.insert_node_on(ColorZonesChannel::Lightness, 0.975, 0.5),
            Err(ColorZonesEditorError::NodeLimitReached)
        );
        assert!(inactive_tail_is_zero(
            &state.parameters().curves[ColorZonesChannel::Lightness.index()],
            20
        ));
    }

    #[test]
    fn movement_rejects_adjacent_distance_as_noop_and_clamps_the_unit_square() {
        let mut state = ColorZonesEditorState::default();
        let previous = *state.parameters();
        assert_eq!(
            state.move_node_on(
                ColorZonesChannel::Lightness,
                0,
                0.75 - COLORZONES_MIN_X_DISTANCE,
                0.5,
            ),
            Ok(false)
        );
        assert_eq!(state.parameters(), &previous);
        state
            .move_node_on(ColorZonesChannel::Lightness, 0, -5.0, 4.0)
            .unwrap();
        assert_eq!(
            state.active_nodes(ColorZonesChannel::Lightness)[0],
            ColorZonesNode::new(0.0, 1.0)
        );
    }

    #[test]
    fn deletion_shifts_active_nodes_and_v2_retains_one_neutral_node() {
        let mut state = ColorZonesEditorState::default();
        state
            .insert_node_on(ColorZonesChannel::Lightness, 0.5, 0.9)
            .unwrap();
        assert_eq!(
            state
                .delete_node_on(ColorZonesChannel::Lightness, 1)
                .unwrap(),
            ColorZonesDeleteOutcome::Deleted
        );
        assert_eq!(
            state.active_nodes(ColorZonesChannel::Lightness),
            [
                ColorZonesNode::new(0.25, 0.5),
                ColorZonesNode::new(0.75, 0.5),
            ]
        );
        assert!(inactive_tail_is_zero(
            &state.parameters().curves[ColorZonesChannel::Lightness.index()],
            2
        ));

        let mut one = single_node_state(0.8, ColorZonesChannel::Chroma);
        one.move_node_on(ColorZonesChannel::Lightness, 0, 0.8, 0.2)
            .unwrap();
        assert_eq!(
            one.delete_node_on(ColorZonesChannel::Lightness, 0).unwrap(),
            ColorZonesDeleteOutcome::SoleNodeReset
        );
        assert_eq!(
            one.active_nodes(ColorZonesChannel::Lightness),
            [ColorZonesNode::new(0.5, 0.5)]
        );
    }

    #[test]
    fn per_curve_and_all_resets_preserve_their_native_boundaries() {
        let mut state = ColorZonesEditorState::with_output_channel(ColorZonesChannel::Chroma);
        state.insert_node(0.5, 0.9).unwrap();
        state.set_curve_type(ColorZonesChannel::Chroma, ColorZonesCurveType::Monotone);
        state.reset_curve();
        assert_eq!(
            state.active_nodes(ColorZonesChannel::Chroma),
            [
                ColorZonesNode::new(0.25, 0.5),
                ColorZonesNode::new(0.75, 0.5),
            ]
        );
        assert_eq!(
            state.curve_type(ColorZonesChannel::Chroma),
            ColorZonesCurveType::Catmull
        );

        state.set_selection_channel(ColorZonesChannel::Lightness);
        state.set_strength(100.0).unwrap();
        state.reset_all();
        assert_eq!(state.parameters(), &ColorZonesParametersV5::defaults());
        assert_eq!(state.output_channel(), ColorZonesChannel::Chroma);
    }

    #[test]
    fn interpolation_changes_are_per_curve_and_spline_transitions_reset_all() {
        let mut state = ColorZonesEditorState::default();
        state.set_curve_type(ColorZonesChannel::Hue, ColorZonesCurveType::Cubic);
        assert_eq!(
            state.curve_type(ColorZonesChannel::Hue),
            ColorZonesCurveType::Cubic
        );
        assert_eq!(
            state.curve_type(ColorZonesChannel::Lightness),
            ColorZonesCurveType::Catmull
        );

        state.set_strength(10.0).unwrap();
        state.set_splines_version(ColorZonesSplinesVersion::V1);
        assert_eq!(state.splines_version(), ColorZonesSplinesVersion::V1);
        assert_eq!(state.strength().to_bits(), 0);
        for channel in CHANNELS {
            assert_eq!(state.curve_type(channel), ColorZonesCurveType::Catmull);
            assert_eq!(
                state.active_nodes(channel),
                [ColorZonesNode::new(0.0, 0.5), ColorZonesNode::new(1.0, 0.5),]
            );
        }

        state.set_splines_version(ColorZonesSplinesVersion::V2);
        for channel in CHANNELS {
            assert_eq!(
                state.active_nodes(channel),
                [
                    ColorZonesNode::new(0.25, 0.5),
                    ColorZonesNode::new(0.75, 0.5),
                ]
            );
        }
    }

    #[test]
    fn loading_preserves_active_bits_and_exactly_zeroes_every_tail_slot() {
        let mut parameters = ColorZonesParametersV5::defaults();
        let negative_zero = f32::from_bits(0x8000_0000);
        parameters.curves[0][0].y = negative_zero;
        for channel in 0..COLORZONES_CHANNELS {
            for node in 2..COLORZONES_MAX_NODES {
                parameters.curves[channel][node] = ColorZonesNode::new(f32::NAN, f32::INFINITY);
            }
        }

        let state =
            ColorZonesEditorState::from_parameters(parameters, ColorZonesChannel::Hue).unwrap();

        assert_eq!(
            state.parameters().curves[0][0].y.to_bits(),
            negative_zero.to_bits()
        );
        for channel in 0..COLORZONES_CHANNELS {
            assert!(inactive_tail_is_zero(
                &state.parameters().curves[channel],
                2
            ));
        }
    }

    #[test]
    fn loading_rejects_invalid_active_state_without_inspecting_inactive_tail() {
        let mut parameters = ColorZonesParametersV5::defaults();
        parameters.curves[1][0].x = f32::NAN;
        assert!(matches!(
            ColorZonesEditorState::from_parameters(parameters, ColorZonesChannel::Lightness),
            Err(ColorZonesEditorError::NonFiniteCoordinate {
                channel: 1,
                node: 0,
                coordinate: "x"
            })
        ));

        let mut parameters = ColorZonesParametersV5::defaults();
        parameters.curves[2][1].x = parameters.curves[2][0].x;
        assert_eq!(
            ColorZonesEditorState::from_parameters(parameters, ColorZonesChannel::Lightness),
            Err(ColorZonesEditorError::UnsortedNodes {
                channel: 2,
                node: 1
            })
        );
    }

    fn single_node_state(x: f32, selection: ColorZonesChannel) -> ColorZonesEditorState {
        let mut parameters = ColorZonesParametersV5::defaults();
        parameters.channel = selection.raw();
        parameters.curve_num_nodes = [1; COLORZONES_CHANNELS];
        for channel in 0..COLORZONES_CHANNELS {
            parameters.curves[channel] = [ColorZonesNode::new(0.0, 0.0); COLORZONES_MAX_NODES];
            parameters.curves[channel][0] = ColorZonesNode::new(x, 0.5);
        }
        ColorZonesEditorState::from_parameters(parameters, ColorZonesChannel::Lightness).unwrap()
    }
}
