//! Pure editor state ported from the GTK callbacks in `src/iop/rgbcurve.c`.
//!
//! This module deliberately models state and source constraints only. GTK
//! widget composition, actions, allocations, drawing, and picker plumbing are
//! unavailable until the UI owner ports them.

#![forbid(unsafe_code)]
#![allow(
    clippy::cast_possible_truncation,
    clippy::float_cmp,
    reason = "editor state follows native f32 coordinates and exact comparisons"
)]

use std::fmt;

use super::curve::CurveCompileError;
use super::parameters::{
    CHANNELS, MAX_NODES, MIN_X_DISTANCE, RgbCurveAutoscale, RgbCurveChannel, RgbCurveNode,
    RgbCurveParametersV1, RgbCurveType,
};
#[cfg(not(test))]
use crate::common::curve_tools::{CurveAnchor, interpolate_value_v1};
#[cfg(test)]
use rusttable_processing::common::curve_tools::{CurveAnchor, interpolate_value_v1};

pub const NODE_HIT_RADIUS_SQUARED: f32 = 0.04 * 0.04;

/// Editor-only view state from `dt_iop_rgbcurve_gui_data_t`.
#[derive(Debug, Clone, PartialEq)]
pub struct RgbCurveEditorState {
    parameters: RgbCurveParametersV1,
    defaults: RgbCurveParametersV1,
    channel: RgbCurveChannel,
    selected: i32,
    zoom_factor: f32,
    offset_x: f32,
    offset_y: f32,
}

impl RgbCurveEditorState {
    #[must_use]
    pub fn new(defaults: RgbCurveParametersV1) -> Self {
        Self {
            parameters: defaults.clone(),
            defaults,
            channel: RgbCurveChannel::Red,
            selected: -1,
            zoom_factor: 1.0,
            offset_x: 0.0,
            offset_y: 0.0,
        }
    }

    #[must_use]
    pub const fn parameters(&self) -> &RgbCurveParametersV1 {
        &self.parameters
    }

    #[must_use]
    pub const fn channel(&self) -> RgbCurveChannel {
        self.channel
    }

    #[must_use]
    pub const fn selected(&self) -> i32 {
        self.selected
    }

    #[must_use]
    pub const fn zoom_factor(&self) -> f32 {
        self.zoom_factor
    }

    #[must_use]
    pub const fn offsets(&self) -> (f32, f32) {
        (self.offset_x, self.offset_y)
    }

    #[must_use]
    pub const fn channel_tabs_visible(&self) -> bool {
        matches!(
            self.parameters.curve_autoscale,
            RgbCurveAutoscale::ManualRgb
        )
    }

    #[must_use]
    pub const fn preserve_colors_visible(&self) -> bool {
        matches!(
            self.parameters.curve_autoscale,
            RgbCurveAutoscale::AutomaticRgb
        )
    }

    #[must_use]
    pub fn channel_editable(&self) -> bool {
        self.channel_tabs_visible() || self.channel == RgbCurveChannel::Red
    }

    /// Native autoscale callback, including first manual-mode R-to-G/B copy.
    pub fn set_autoscale(&mut self, autoscale: RgbCurveAutoscale) {
        self.channel = RgbCurveChannel::Red;
        if matches!(autoscale, RgbCurveAutoscale::ManualRgb)
            && is_identity(&self.parameters, RgbCurveChannel::Green)
            && is_identity(&self.parameters, RgbCurveChannel::Blue)
        {
            self.parameters.curve_nodes[RgbCurveChannel::Green.index()] =
                self.parameters.curve_nodes[RgbCurveChannel::Red.index()];
            self.parameters.curve_nodes[RgbCurveChannel::Blue.index()] =
                self.parameters.curve_nodes[RgbCurveChannel::Red.index()];
            self.parameters.curve_num_nodes[RgbCurveChannel::Green.index()] =
                self.parameters.curve_num_nodes[RgbCurveChannel::Red.index()];
            self.parameters.curve_num_nodes[RgbCurveChannel::Blue.index()] =
                self.parameters.curve_num_nodes[RgbCurveChannel::Red.index()];
            self.parameters.curve_type[RgbCurveChannel::Green.index()] =
                self.parameters.curve_type[RgbCurveChannel::Red.index()];
            self.parameters.curve_type[RgbCurveChannel::Blue.index()] =
                self.parameters.curve_type[RgbCurveChannel::Red.index()];
        }
        self.parameters.curve_autoscale = autoscale;
    }

    /// Native interpolator callback applies one type to all three channels.
    pub fn set_interpolator(&mut self, curve_type: RgbCurveType) {
        self.parameters.curve_type = [curve_type; CHANNELS];
    }

    pub fn select_channel(&mut self, channel: RgbCurveChannel) -> Result<(), EditorError> {
        if !self.channel_tabs_visible() && channel != RgbCurveChannel::Red {
            return Err(EditorError::ChannelUnavailable);
        }
        self.channel = channel;
        Ok(())
    }

    pub fn select_nearest(&mut self, mouse_x: f32, mouse_y: f32) -> Result<i32, EditorError> {
        if !mouse_x.is_finite() || !mouse_y.is_finite() {
            return Err(EditorError::NonFiniteCoordinate);
        }
        let channel = self.channel.index();
        let count = self.parameters.curve_num_nodes[channel] as usize;
        let mut minimum = NODE_HIT_RADIUS_SQUARED;
        let mut nearest = -1;
        for index in 0..count {
            let node = self.parameters.curve_nodes[channel][index];
            let x = curve_to_mouse(node.x, self.zoom_factor, self.offset_x);
            let y = curve_to_mouse(node.y, self.zoom_factor, self.offset_y);
            let distance = (mouse_x - x) * (mouse_x - x) + (mouse_y - y) * (mouse_y - y);
            if distance < minimum {
                minimum = distance;
                nearest = index as i32;
            }
        }
        self.selected = nearest;
        Ok(nearest)
    }

    /// Adds a sorted node, preserving the native minimum x separation.
    pub fn add_node(&mut self, x: f32, y: f32) -> Result<usize, EditorError> {
        self.ensure_editable()?;
        if self.selected == -2 {
            return Err(EditorError::SentinelActive);
        }
        validate_coordinate(x)?;
        validate_coordinate(y)?;
        let channel = self.channel.index();
        let count = self.parameters.curve_num_nodes[channel] as usize;
        if count >= MAX_NODES {
            return Err(EditorError::MaximumNodes);
        }
        let selected = self.insertion_index(channel, x);
        if (selected > 0
            && x - self.parameters.curve_nodes[channel][selected - 1].x <= MIN_X_DISTANCE)
            || (selected < count
                && self.parameters.curve_nodes[channel][selected].x - x <= MIN_X_DISTANCE)
        {
            return Err(EditorError::MinimumXDistance);
        }
        for index in (selected..count).rev() {
            self.parameters.curve_nodes[channel][index + 1] =
                self.parameters.curve_nodes[channel][index];
        }
        self.parameters.curve_nodes[channel][selected] = RgbCurveNode::new(x, y);
        self.parameters.curve_num_nodes[channel] += 1;
        self.selected = selected as i32;
        Ok(selected)
    }

    /// Moves the selected node by native step coordinates and acceleration
    /// already applied by the caller.
    pub fn move_selected(&mut self, dx: f32, dy: f32) -> Result<bool, EditorError> {
        self.ensure_editable()?;
        if !dx.is_finite() || !dy.is_finite() {
            return Err(EditorError::NonFiniteCoordinate);
        }
        let selected = usize::try_from(self.selected).map_err(|_| EditorError::NoSelection)?;
        let channel = self.channel.index();
        let count = self.parameters.curve_num_nodes[channel] as usize;
        if selected >= count {
            return Err(EditorError::NoSelection);
        }
        let node = self.parameters.curve_nodes[channel][selected];
        let new_x = (node.x + dx).clamp(0.0, 1.0);
        let new_y = (node.y + dy).clamp(0.0, 1.0);
        if !sanity_check(
            new_x,
            selected,
            count,
            &self.parameters.curve_nodes[channel],
        ) {
            return Ok(false);
        }
        self.parameters.curve_nodes[channel][selected] = RgbCurveNode::new(new_x, new_y);
        Ok(true)
    }

    /// Secondary-click behavior: endpoint reset or internal-node deletion.
    pub fn secondary_reset_or_delete(&mut self) -> Result<(), EditorError> {
        self.ensure_editable()?;
        let selected = usize::try_from(self.selected).map_err(|_| EditorError::NoSelection)?;
        let channel = self.channel.index();
        let count = self.parameters.curve_num_nodes[channel] as usize;
        if selected >= count {
            return Err(EditorError::NoSelection);
        }
        if selected == 0 || selected + 1 == count {
            let value = if selected == 0 { 0.0 } else { 1.0 };
            self.parameters.curve_nodes[channel][selected] = RgbCurveNode::new(value, value);
        } else {
            for index in selected..count - 1 {
                self.parameters.curve_nodes[channel][index] =
                    self.parameters.curve_nodes[channel][index + 1];
            }
            self.parameters.curve_nodes[channel][count - 1] = RgbCurveNode::ZERO;
            self.parameters.curve_num_nodes[channel] -= 1;
        }
        self.selected = -2;
        Ok(())
    }

    /// Native double-click behavior, including automatic-mode G/B switch.
    pub fn double_click(&mut self, channel: RgbCurveChannel) -> Result<(), EditorError> {
        self.channel = channel;
        if !self.channel_tabs_visible() && channel != RgbCurveChannel::Red {
            self.set_autoscale(RgbCurveAutoscale::ManualRgb);
            self.selected = -2;
            return Ok(());
        }
        self.reset_curve()?;
        self.selected = -2;
        Ok(())
    }

    pub fn reset_curve(&mut self) -> Result<(), EditorError> {
        self.ensure_editable()?;
        let channel = self.channel.index();
        let default_count = self.defaults.curve_num_nodes[channel] as usize;
        self.parameters.curve_num_nodes[channel] = self.defaults.curve_num_nodes[channel];
        self.parameters.curve_type[channel] = self.defaults.curve_type[channel];
        self.parameters.curve_nodes[channel][..default_count]
            .copy_from_slice(&self.defaults.curve_nodes[channel][..default_count]);
        Ok(())
    }

    /// Native change-image reset: preserve a selected green or blue channel,
    /// while clearing transient selection and zoom/pan state.
    pub fn change_image(&mut self) {
        self.selected = -1;
        self.offset_x = 0.0;
        self.offset_y = 0.0;
        self.zoom_factor = 1.0;
    }

    /// Native double-click/reset view behavior always returns to red.
    pub fn reset_view(&mut self) {
        self.channel = RgbCurveChannel::Red;
        self.change_image();
    }

    /// Native middle-grey checkbox state transition is represented as a pure
    /// profile-backed node transform. The caller supplies the active profile;
    /// no widget or histogram state is fabricated in this leaf.
    pub fn set_compensate_middle_grey(
        &mut self,
        enabled: bool,
        profile: &super::curve::RgbCurveProfileEvidence,
    ) {
        for channel in 0..CHANNELS {
            let count = self.parameters.curve_num_nodes[channel] as usize;
            for node in &mut self.parameters.curve_nodes[channel][..count] {
                if enabled {
                    node.x = profile.compensate_middle_grey(node.x);
                    node.y = profile.compensate_middle_grey(node.y);
                } else {
                    node.x = profile.uncompensate_middle_grey(node.x);
                    node.y = profile.uncompensate_middle_grey(node.y);
                }
            }
        }
        self.parameters.compensate_middle_grey = enabled;
    }

    /// Builds the native min/mean/max picker curve and optional ±0.05 center
    /// adjustment. Picker values are already normalized by the caller.
    pub fn apply_picker_curve(
        &mut self,
        minimum: f32,
        mean: f32,
        maximum: f32,
        center_adjustment: i8,
    ) -> Result<(), EditorError> {
        self.ensure_editable()?;
        for value in [minimum, mean, maximum] {
            validate_coordinate(value)?;
        }
        if !(-1..=1).contains(&center_adjustment) {
            return Err(EditorError::InvalidPickerAdjustment);
        }
        let channel = self.channel.index();
        self.parameters.curve_nodes[channel] = self.defaults.curve_nodes[channel];
        self.parameters.curve_num_nodes[channel] = self.defaults.curve_num_nodes[channel];
        self.parameters.curve_type[channel] = self.defaults.curve_type[channel];
        let increment = 0.05 * f32::from(center_adjustment);
        self.add_picker_node(minimum, 0.0)?;
        self.add_picker_node(
            (mean - increment).clamp(0.0, 1.0),
            (mean + increment).clamp(0.0, 1.0),
        )?;
        self.add_picker_node(maximum, 0.0)?;
        if self.parameters.curve_num_nodes[channel] == 5 {
            let first = self.parameters.curve_nodes[channel][1];
            let third = self.parameters.curve_nodes[channel][3];
            self.insert_unchecked(
                channel,
                first.x - increment + (third.x - first.x) / 2.0,
                first.y + increment + (third.y - first.y) / 2.0,
            );
        }
        Ok(())
    }

    /// Recomputes the GUI curve rather than reading the quantized runtime LUT.
    pub fn evaluate_gui(&self, input: f32) -> Result<f32, EditorError> {
        let channel = self.channel.index();
        let count = self.parameters.curve_num_nodes[channel] as usize;
        let anchors: Vec<_> = self.parameters.curve_nodes[channel][..count]
            .iter()
            .map(|node| CurveAnchor::new(node.x, node.y))
            .collect();
        interpolate_value_v1(&anchors, input, self.parameters.curve_type[channel].into())
            .map(|value| value.clamp(0.0, 1.0))
            .map_err(|error| EditorError::Curve(CurveCompileError::from(error)))
    }

    /// Native zoom/pan state update from the drawing-area scroll callback.
    pub fn zoom_at(&mut self, mouse_x: f32, mouse_y: f32, delta_y: f32) -> Result<(), EditorError> {
        if !mouse_x.is_finite() || !mouse_y.is_finite() || !delta_y.is_finite() {
            return Err(EditorError::NonFiniteCoordinate);
        }
        let linear_x = mouse_x / self.zoom_factor + self.offset_x;
        let linear_y = mouse_y / self.zoom_factor + self.offset_y;
        self.zoom_factor *= 1.0 - 0.1 * delta_y;
        if self.zoom_factor < 1.0 {
            self.zoom_factor = 1.0;
        }
        self.offset_x = linear_x - mouse_x / self.zoom_factor;
        self.offset_y = linear_y - mouse_y / self.zoom_factor;
        let limit = (self.zoom_factor - 1.0) / self.zoom_factor;
        self.offset_x = self.offset_x.clamp(0.0, limit);
        self.offset_y = self.offset_y.clamp(0.0, limit);
        Ok(())
    }

    fn ensure_editable(&self) -> Result<(), EditorError> {
        if self.channel_editable() {
            Ok(())
        } else {
            Err(EditorError::ChannelUnavailable)
        }
    }

    fn insertion_index(&self, channel: usize, x: f32) -> usize {
        self.parameters.curve_nodes[channel][..self.parameters.curve_num_nodes[channel] as usize]
            .iter()
            .position(|node| node.x > x)
            .unwrap_or(self.parameters.curve_num_nodes[channel] as usize)
    }

    fn add_picker_node(&mut self, x: f32, y: f32) -> Result<(), EditorError> {
        let channel = self.channel.index();
        let count = self.parameters.curve_num_nodes[channel] as usize;
        if count >= MAX_NODES {
            return Err(EditorError::MaximumNodes);
        }
        self.insert_unchecked(channel, x, y);
        Ok(())
    }

    fn insert_unchecked(&mut self, channel: usize, x: f32, y: f32) {
        let count = self.parameters.curve_num_nodes[channel] as usize;
        let selected = self.insertion_index(channel, x);
        for index in (selected..count).rev() {
            self.parameters.curve_nodes[channel][index + 1] =
                self.parameters.curve_nodes[channel][index];
        }
        self.parameters.curve_nodes[channel][selected] = RgbCurveNode::new(x, y);
        self.parameters.curve_num_nodes[channel] += 1;
        self.selected = selected as i32;
    }
}

fn validate_coordinate(value: f32) -> Result<(), EditorError> {
    if !value.is_finite() {
        return Err(EditorError::NonFiniteCoordinate);
    }
    if !(0.0..=1.0).contains(&value) {
        return Err(EditorError::CoordinateOutOfRange);
    }
    Ok(())
}

fn is_identity(parameters: &RgbCurveParametersV1, channel: RgbCurveChannel) -> bool {
    let channel = channel.index();
    let count = parameters.curve_num_nodes[channel] as usize;
    parameters.curve_nodes[channel][..count]
        .iter()
        .all(|node| node.x == node.y)
}

fn sanity_check(x: f32, selected: usize, nodes: usize, curve: &[RgbCurveNode; MAX_NODES]) -> bool {
    if (selected > 0 && x - curve[selected - 1].x <= MIN_X_DISTANCE)
        || (selected + 1 < nodes && curve[selected + 1].x - x <= MIN_X_DISTANCE)
    {
        return false;
    }
    if (selected > 0 && curve[selected - 1].x >= x)
        || (selected + 1 < nodes && curve[selected + 1].x <= x)
    {
        return false;
    }
    true
}

fn curve_to_mouse(value: f32, zoom_factor: f32, offset: f32) -> f32 {
    (value - offset) * zoom_factor
}

#[derive(Debug, PartialEq, Eq)]
pub enum EditorError {
    ChannelUnavailable,
    MaximumNodes,
    MinimumXDistance,
    NoSelection,
    SentinelActive,
    NonFiniteCoordinate,
    CoordinateOutOfRange,
    InvalidPickerAdjustment,
    Curve(CurveCompileError),
}

impl fmt::Display for EditorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ChannelUnavailable => {
                formatter.write_str("RGB Curve channel is unavailable in automatic mode")
            }
            Self::MaximumNodes => formatter.write_str("RGB Curve has reached its 20-node maximum"),
            Self::MinimumXDistance => formatter.write_str("RGB Curve nodes are too close in x"),
            Self::NoSelection => formatter.write_str("RGB Curve has no selected node"),
            Self::SentinelActive => {
                formatter.write_str("RGB Curve editor deletion/reset sentinel is active")
            }
            Self::NonFiniteCoordinate => formatter.write_str("RGB Curve coordinate is non-finite"),
            Self::CoordinateOutOfRange => {
                formatter.write_str("RGB Curve coordinate is outside [0, 1]")
            }
            Self::InvalidPickerAdjustment => {
                formatter.write_str("RGB Curve picker adjustment must be -1, 0, or 1")
            }
            Self::Curve(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for EditorError {}
