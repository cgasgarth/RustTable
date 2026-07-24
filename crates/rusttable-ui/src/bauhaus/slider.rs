//! State and numeric behavior for Darktable's Bauhaus slider.
//!
//! This is a direct safe-Rust port of the slider data and range/value helpers
//! in `src/bauhaus/bauhaus.c` and declarations in
//! `src/bauhaus/bauhaus.h`. GTK ownership and event routing remain in the
//! sibling `slider_input` adapter.

// GTK exposes doubles while every numeric field and calculation in the pinned
// Bauhaus implementation uses C `float`; narrowing at this boundary is the
// behavior being ported.
#![allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]

use std::{error::Error, fmt};

/// Maximum number of gradient stops accepted by a Bauhaus slider.
pub const MAX_GRADIENT_STOPS: usize = 20;

/// One source-order-preserving slider gradient stop.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GradientStop {
    position: f32,
    rgb: [f32; 3],
}

impl GradientStop {
    /// Returns the normalized stop position.
    #[must_use]
    pub const fn position(self) -> f64 {
        self.position as f64
    }

    /// Returns the stop's red, green, and blue components.
    #[must_use]
    pub const fn rgb(self) -> [f64; 3] {
        [self.rgb[0] as f64, self.rgb[1] as f64, self.rgb[2] as f64]
    }
}

/// Built-in mappings supported by the retained Bauhaus slider.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SliderCurve {
    /// Direct normalized mapping.
    Linear,
    /// Direct mapping with the normalized direction reversed.
    ReverseLinear,
    /// Darktable's three-decade logarithmic mapping.
    Log10,
}

#[derive(Debug, Clone, Copy)]
enum CurveDirection {
    Set,
    Get,
}

impl SliderCurve {
    fn apply(self, value: f32, direction: CurveDirection) -> f32 {
        match self {
            Self::Linear => value,
            Self::ReverseLinear => 1.0_f32 - value,
            Self::Log10 => match direction {
                CurveDirection::Set => (value * 999.0_f32 + 1.0_f32).log10() / 3.0_f32,
                CurveDirection::Get => {
                    (f32::exp(std::f32::consts::LN_10 * value * 3.0_f32) - 1.0_f32) / 999.0_f32
                }
            },
        }
    }
}

/// Range selected for Darktable's automatic step calculation.
///
/// The original reads `bauhaus/zoom_step` each time the step is requested, so
/// this remains a call-time policy rather than per-slider state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutomaticStepPolicy {
    /// Use the temporary range currently visible in the slider.
    VisibleRange,
    /// Use the preferred soft range even when the visible range has expanded.
    SoftRange,
}

/// Invalid construction at the safe Rust boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SliderModelError {
    /// At least one numeric constructor argument is not representable as a
    /// finite C `float`.
    NonFinite,
    /// The hard minimum is greater than the hard maximum.
    InvalidRange,
}

impl fmt::Display for SliderModelError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFinite => formatter.write_str("Bauhaus slider values must be finite"),
            Self::InvalidRange => {
                formatter.write_str("Bauhaus slider minimum must not be greater than maximum")
            }
        }
    }
}

impl Error for SliderModelError {}

/// Authoritative numeric state for Darktable's Bauhaus slider responsibility.
#[derive(Debug, Clone, PartialEq)]
pub struct BauhausSliderModel {
    position: f32,
    step: f32,
    default: f32,
    minimum: f32,
    maximum: f32,
    soft_minimum: f32,
    soft_maximum: f32,
    hard_minimum: f32,
    hard_maximum: f32,
    digits: i32,
    stops: Vec<GradientStop>,
    fill_feedback: bool,
    format: String,
    factor: f32,
    offset: f32,
    curve: SliderCurve,
}

impl BauhausSliderModel {
    /// Builds the state initialized by
    /// `dt_bauhaus_slider_new_with_range_and_feedback`.
    ///
    /// A zero step selects Darktable's automatic decade-based step. Negative
    /// finite steps are retained; `effective_step` applies their magnitude
    /// with the display factor's sign, as `copysignf` does in the source.
    ///
    /// # Errors
    ///
    /// Returns an error for values that are not representable as finite C
    /// `float`s or for an inverted hard range. Equal bounds and finite
    /// defaults outside the hard range are source-valid.
    pub fn new(
        minimum: f64,
        maximum: f64,
        step: f64,
        default: f64,
        digits: i32,
        fill_feedback: bool,
    ) -> Result<Self, SliderModelError> {
        if minimum > maximum {
            return Err(SliderModelError::InvalidRange);
        }
        let minimum = finite_f32(minimum)?;
        let maximum = finite_f32(maximum)?;
        let step = finite_f32(step)?;
        let default = finite_f32(default)?;
        Ok(Self {
            position: (default - minimum) / (maximum - minimum),
            step,
            default,
            minimum,
            maximum,
            soft_minimum: minimum,
            soft_maximum: maximum,
            hard_minimum: minimum,
            hard_maximum: maximum,
            digits,
            stops: Vec::new(),
            fill_feedback,
            format: String::new(),
            factor: 1.0_f32,
            offset: 0.0_f32,
            curve: SliderCurve::Linear,
        })
    }

    /// Returns the current raw value.
    #[must_use]
    #[allow(clippy::float_cmp)] // Darktable treats an exactly collapsed range as a sentinel.
    pub fn value(&self) -> f64 {
        f64::from(self.value_f32())
    }

    #[allow(clippy::float_cmp)] // Darktable treats an exactly collapsed range as a sentinel.
    fn value_f32(&self) -> f32 {
        if self.maximum == self.minimum {
            return self.maximum;
        }
        let raw = self.curve.apply(self.position, CurveDirection::Get);
        self.minimum + raw * (self.maximum - self.minimum)
    }

    /// Returns the current value after display factor and offset.
    #[must_use]
    pub fn display_value(&self) -> f64 {
        f64::from(self.value_f32() * self.factor + self.offset)
    }

    /// Sets a raw value using Darktable's hard clamping, degree wrapping,
    /// visible-range expansion, curve mapping, and display-unit rounding.
    #[allow(clippy::float_cmp)] // Exact equality selects Darktable's wrap/range branches.
    pub fn set_value(&mut self, value: f64) {
        self.set_value_f32(value as f32);
    }

    #[allow(clippy::float_cmp)] // Exact equality selects Darktable's wrap/range branches.
    fn set_value_f32(&mut self, value: f32) {
        if value.is_nan() {
            return;
        }

        let clamped = value.max(self.hard_minimum).min(self.hard_maximum);
        let wrapped = if clamped == value || self.format != "°" {
            clamped
        } else {
            let width = self.hard_maximum - self.hard_minimum;
            self.hard_minimum + (value + self.hard_maximum - 2.0_f32 * self.hard_minimum) % width
        };

        if wrapped == clamped {
            self.minimum = self.minimum.min(clamped);
            self.maximum = self.maximum.max(clamped);
        } else {
            self.minimum = self.hard_minimum;
            self.maximum = self.hard_maximum;
        }

        if self.maximum == self.minimum {
            self.position = 0.0_f32;
            return;
        }
        let raw = (wrapped - self.minimum) / (self.maximum - self.minimum);
        self.set_normalized_position_f32(self.curve.apply(raw, CurveDirection::Set));
    }

    /// Sets a value expressed in the displayed factor/offset units.
    pub fn set_display_value(&mut self, value: f64) {
        self.set_value_f32((value as f32 - self.offset) / self.factor);
    }

    /// Restores the source default and soft visible range.
    pub fn reset(&mut self) {
        self.minimum = self.soft_minimum;
        self.maximum = self.soft_maximum;
        self.set_value_f32(self.default);
    }

    /// Returns the current normalized position after the configured curve.
    #[must_use]
    pub const fn normalized_position(&self) -> f64 {
        self.position as f64
    }

    /// Returns the currently visible range.
    #[must_use]
    pub const fn visible_range(&self) -> (f64, f64) {
        (self.minimum as f64, self.maximum as f64)
    }

    /// Returns the preferred soft range.
    #[must_use]
    pub const fn soft_range(&self) -> (f64, f64) {
        (self.soft_minimum as f64, self.soft_maximum as f64)
    }

    /// Returns the accepted hard range.
    #[must_use]
    pub const fn hard_range(&self) -> (f64, f64) {
        (self.hard_minimum as f64, self.hard_maximum as f64)
    }

    /// Replaces the visible range without changing the normalized position.
    ///
    /// This is the state transition used by an accepted nonzero
    /// `_slider_zoom_range`: the value-centered range calculation guarantees
    /// that the saved normalized position still represents the same value.
    #[must_use]
    pub fn set_visible_range_preserving_position(&mut self, minimum: f64, maximum: f64) -> bool {
        let Some((minimum, maximum)) = self.valid_visible_range(minimum, maximum) else {
            return false;
        };
        self.minimum = minimum;
        self.maximum = maximum;
        true
    }

    /// Replaces the visible range and recomputes the normalized position for
    /// the current raw value.
    ///
    /// This matches the zero-zoom hard/soft toggle followed by
    /// `dt_bauhaus_slider_set` in the source.
    #[must_use]
    pub fn set_visible_range_preserving_value(&mut self, minimum: f64, maximum: f64) -> bool {
        let Some((minimum, maximum)) = self.valid_visible_range(minimum, maximum) else {
            return false;
        };
        let value = self.value_f32();
        self.minimum = minimum;
        self.maximum = maximum;
        self.set_value_f32(value);
        true
    }

    /// Changes both soft bounds in source call order.
    pub fn set_soft_range(&mut self, minimum: f64, maximum: f64) {
        self.set_soft_minimum(minimum);
        self.set_soft_maximum(maximum);
    }

    /// Changes the preferred lower bound while preserving the current value.
    pub fn set_soft_minimum(&mut self, minimum: f64) {
        let Some(minimum) = finite_f32_option(minimum) else {
            return;
        };
        let old_value = self.value_f32();
        let minimum = minimum.max(self.hard_minimum).min(self.hard_maximum);
        self.minimum = minimum;
        self.soft_minimum = minimum;
        self.set_value_f32(old_value);
    }

    /// Changes the preferred upper bound while preserving the current value.
    pub fn set_soft_maximum(&mut self, maximum: f64) {
        let Some(maximum) = finite_f32_option(maximum) else {
            return;
        };
        let old_value = self.value_f32();
        let maximum = maximum.max(self.hard_minimum).min(self.hard_maximum);
        self.maximum = maximum;
        self.soft_maximum = maximum;
        self.set_value_f32(old_value);
    }

    /// Changes the hard lower bound using Darktable's range-collapse behavior.
    pub fn set_hard_minimum(&mut self, minimum: f64) {
        let Some(minimum) = finite_f32_option(minimum) else {
            return;
        };
        self.set_hard_minimum_f32(minimum);
    }

    fn set_hard_minimum_f32(&mut self, minimum: f32) {
        let old_value = self.value_f32();
        self.hard_minimum = minimum;
        self.minimum = self.minimum.max(minimum);
        self.soft_minimum = self.soft_minimum.max(minimum);
        if minimum > self.hard_maximum {
            self.set_hard_maximum_f32(minimum);
        }
        self.set_value_f32(old_value.max(minimum));
    }

    /// Changes the hard upper bound using Darktable's range-collapse behavior.
    pub fn set_hard_maximum(&mut self, maximum: f64) {
        let Some(maximum) = finite_f32_option(maximum) else {
            return;
        };
        self.set_hard_maximum_f32(maximum);
    }

    fn set_hard_maximum_f32(&mut self, maximum: f32) {
        let old_value = self.value_f32();
        self.hard_maximum = maximum;
        self.maximum = self.maximum.min(maximum);
        self.soft_maximum = self.soft_maximum.min(maximum);
        if maximum < self.hard_minimum {
            self.set_hard_minimum_f32(maximum);
        }
        self.set_value_f32(old_value.min(maximum));
    }

    /// Returns the source default.
    #[must_use]
    pub const fn default_value(&self) -> f64 {
        self.default as f64
    }

    /// Changes the source default without changing the current value.
    pub fn set_default_value(&mut self, default: f64) {
        if let Some(default) = finite_f32_option(default) {
            self.default = default;
        }
    }

    /// Returns the configured decimal digits.
    #[must_use]
    pub const fn digits(&self) -> i32 {
        self.digits
    }

    /// Changes the configured decimal digits.
    pub const fn set_digits(&mut self, digits: i32) {
        self.digits = digits;
    }

    /// Returns the explicit source step, where zero means automatic.
    #[must_use]
    pub const fn configured_step(&self) -> f64 {
        self.step as f64
    }

    /// Changes the explicit source step. Negative values are retained because
    /// source `copysignf` applies their magnitude with the factor's sign.
    pub fn set_step(&mut self, step: f64) {
        if let Some(step) = finite_f32_option(step) {
            self.step = step;
        }
    }

    /// Returns the explicit or automatically selected step with the factor's
    /// sign, matching `dt_bauhaus_slider_get_step`.
    #[must_use]
    pub fn effective_step(&self, policy: AutomaticStepPolicy) -> f64 {
        f64::from(self.effective_step_f32(policy))
    }

    #[allow(clippy::float_cmp)] // Zero is the source sentinel for an automatic step.
    fn effective_step_f32(&self, policy: AutomaticStepPolicy) -> f32 {
        let mut step = self.step;
        if step == 0.0_f32 {
            let (minimum, maximum) = match policy {
                AutomaticStepPolicy::VisibleRange => (self.minimum, self.maximum),
                AutomaticStepPolicy::SoftRange => (self.soft_minimum, self.soft_maximum),
            };
            let top = (maximum - minimum).min(minimum.abs().max(maximum.abs()));
            if top >= 100.0_f32 {
                step = 1.0_f32;
            } else {
                step = top * self.factor.abs() / 100.0_f32;
                let logarithm = step.log10();
                let decade = (logarithm + 0.1_f32).floor();
                step = 10.0_f32.powf(decade);
                if logarithm - decade > 0.5_f32 {
                    step *= 5.0_f32;
                }
                step /= self.factor.abs();
            }
        }
        step.copysign(self.factor)
    }

    /// Applies one source-style step with a pre-resolved accelerator speed and
    /// the live global automatic-step range policy.
    pub fn add_step(&mut self, delta: f64, speed: f64, force: bool, policy: AutomaticStepPolicy) {
        let delta = delta as f32;
        let speed = speed as f32;
        if delta == 0.0_f32 || !delta.is_finite() || !speed.is_finite() {
            return;
        }

        let value = self.value_f32();
        let mut change = if self.curve == SliderCurve::Log10 {
            value * (0.97_f32.powf(-delta * speed) - 1.0_f32)
        } else {
            delta * self.effective_step_f32(policy) * speed
        };

        let minimum_visible = 10.0_f32.powf(-(self.digits as f32)) / self.factor.abs();
        if change != 0.0_f32 && change.abs() < minimum_visible {
            change = minimum_visible.copysign(change);
        }

        if force {
            if (self.factor > 0.0_f32 && self.position < 0.0001_f32)
                || (self.factor < 0.0_f32 && self.position > 0.9999_f32)
            {
                self.minimum = if self.minimum > self.soft_minimum {
                    self.maximum
                } else {
                    self.soft_minimum
                };
            }
            if (self.factor < 0.0_f32 && self.position < 0.0001_f32)
                || (self.factor > 0.0_f32 && self.position > 0.9999_f32)
            {
                self.maximum = if self.maximum < self.soft_maximum {
                    self.minimum
                } else {
                    self.soft_maximum
                };
            }
            self.set_value_f32(value + change);
        } else if self.format == "°"
            && ((self.maximum - self.minimum) * self.factor - 360.0_f32).abs() < 1.0e-4_f32
            && (value + change).abs() / (self.maximum - self.minimum) < 2.0_f32
        {
            self.set_value_f32(value + change);
        } else {
            self.set_value_f32((value + change).max(self.minimum).min(self.maximum));
        }
    }

    /// Returns whether the marker should receive fill feedback.
    #[must_use]
    pub const fn fill_feedback(&self) -> bool {
        self.fill_feedback
    }

    /// Changes marker fill feedback.
    pub const fn set_fill_feedback(&mut self, fill_feedback: bool) {
        self.fill_feedback = fill_feedback;
    }

    /// Returns the literal suffix appended by Darktable's numeric formatter.
    #[must_use]
    pub fn format(&self) -> &str {
        &self.format
    }

    /// Changes the literal suffix and applies Darktable's percent shortcut.
    #[allow(clippy::float_cmp)] // The source shortcut applies only to the literal default factor.
    pub fn set_format(&mut self, format: impl Into<String>) {
        self.format = format.into();
        if self.format.contains('%') && self.hard_maximum.abs() <= 10.0_f32 {
            if self.factor == 1.0_f32 {
                self.factor = 100.0_f32;
            }
            self.digits -= 2;
        }
    }

    /// Returns the display multiplication factor.
    #[must_use]
    pub const fn factor(&self) -> f64 {
        self.factor as f64
    }

    /// Changes the display multiplication factor. A negative factor selects
    /// Darktable's reverse-linear curve.
    pub fn set_factor(&mut self, factor: f64) {
        let Some(factor) = finite_f32_option(factor) else {
            return;
        };
        self.factor = factor;
        if factor < 0.0_f32 {
            self.curve = SliderCurve::ReverseLinear;
        }
    }

    /// Returns the display addition offset.
    #[must_use]
    pub const fn offset(&self) -> f64 {
        self.offset as f64
    }

    /// Changes the display addition offset.
    pub fn set_offset(&mut self, offset: f64) {
        if let Some(offset) = finite_f32_option(offset) {
            self.offset = offset;
        }
    }

    /// Returns the current curve.
    #[must_use]
    pub const fn curve(&self) -> SliderCurve {
        self.curve
    }

    /// Changes the curve while preserving the current raw value.
    pub fn set_curve(&mut self, curve: SliderCurve) {
        let raw = self.curve.apply(self.position, CurveDirection::Get);
        self.position = curve.apply(raw, CurveDirection::Set);
        self.curve = curve;
    }

    /// Formats a raw value using Darktable's sign, precision, factor, offset,
    /// and literal suffix rules.
    #[must_use]
    pub fn value_text(&self, value: f64) -> String {
        let value = value as f32;
        let displayed = value * self.factor + self.offset;
        let displayed_minimum = self.hard_minimum * self.factor + self.offset;
        let displayed_maximum = self.hard_maximum * self.factor + self.offset;
        // A negative precision supplied through printf's `*` argument is
        // treated as omitted; `%f` therefore uses its default of six places.
        let precision = usize::try_from(self.digits).unwrap_or(6);
        let displayed = f64::from(displayed);
        let number = if displayed_minimum * displayed_maximum < 0.0 {
            format!("{displayed:+.precision$}")
        } else {
            format!("{displayed:.precision$}")
        };
        format!("{number}{}", self.format)
    }

    /// Adds or replaces a gradient stop. Returns `false` only when a new stop
    /// would exceed `DT_BAUHAUS_SLIDER_MAX_STOPS`.
    #[allow(clippy::float_cmp)] // Stop positions are exact identity keys in the source.
    pub fn set_stop(&mut self, position: f64, rgb: [f64; 3]) -> bool {
        let position = position as f32;
        let rgb = [rgb[0] as f32, rgb[1] as f32, rgb[2] as f32];
        if let Some(stop) = self.stops.iter_mut().find(|stop| stop.position == position) {
            stop.rgb = rgb;
            return true;
        }
        if self.stops.len() >= MAX_GRADIENT_STOPS {
            return false;
        }
        self.stops.push(GradientStop { position, rgb });
        true
    }

    /// Removes all active gradient stops.
    pub fn clear_stops(&mut self) {
        self.stops.clear();
    }

    /// Returns gradient stops in insertion order.
    #[must_use]
    pub fn stops(&self) -> &[GradientStop] {
        &self.stops
    }

    /// Commits a normalized popup position using Darktable's curve, display
    /// precision, and current visible range.
    pub fn set_normalized_position(&mut self, position: f64) {
        self.set_normalized_position_f32(position as f32);
    }

    fn set_normalized_position_f32(&mut self, position: f32) {
        let position = position.clamp(0.0_f32, 1.0_f32);
        let raw = self.curve.apply(position, CurveDirection::Get);
        let value = self.minimum + (self.maximum - self.minimum) * raw;
        let base = 10.0_f32.powf(self.digits as f32) * self.factor;
        let rounded = (base * value).round() / base;
        let raw = (rounded - self.minimum) / (self.maximum - self.minimum);
        self.position = self.curve.apply(raw, CurveDirection::Set);
    }

    fn valid_visible_range(&self, minimum: f64, maximum: f64) -> Option<(f32, f32)> {
        if minimum > maximum {
            return None;
        }
        let minimum = finite_f32_option(minimum)?;
        let maximum = finite_f32_option(maximum)?;
        (minimum >= self.hard_minimum && maximum <= self.hard_maximum).then_some((minimum, maximum))
    }
}

fn finite_f32(value: f64) -> Result<f32, SliderModelError> {
    finite_f32_option(value).ok_or(SliderModelError::NonFinite)
}

fn finite_f32_option(value: f64) -> Option<f32> {
    let value = value as f32;
    value.is_finite().then_some(value)
}
