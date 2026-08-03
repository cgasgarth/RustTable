//! Fine-tune popup state and pointer math for Darktable's Bauhaus slider.
//!
//! This is a direct safe-Rust port of `_slider_get_line_offset`,
//! `_slider_zoom_range`, `_popup_scroll`, and the slider branch of
//! `_window_motion_notify` in `src/bauhaus/bauhaus.c`, plus the isolated
//! shared smooth-unit accumulation from `dt_gui_get_scroll_unit_deltas` in
//! `src/gui/gtk.c`. GTK4 event/controller ownership remains in the Bauhaus
//! slider adapter.

use std::cell::RefCell;

use gtk4::gdk;

/// The normalized radius inside which the full-circle popup keeps its old
/// slider position.
const FULL_CIRCLE_DEAD_ZONE_RADIUS: f32 = 0.25;

/// One ordered raw-value range from the Bauhaus slider state.
///
/// Darktable stores every member of `dt_bauhaus_slider_data_t` as C `float`.
/// The public API remains `f64` for GTK4, but values are narrowed at entry so
/// range comparisons and arithmetic retain the source precision.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SliderRange {
    minimum: f32,
    maximum: f32,
}

impl SliderRange {
    /// Creates a range using the source's minimum/maximum ordering.
    #[must_use]
    pub const fn new(minimum: f64, maximum: f64) -> Self {
        Self::from_source(source_float(minimum), source_float(maximum))
    }

    /// Returns the lower bound.
    #[must_use]
    pub fn minimum(self) -> f64 {
        f64::from(self.minimum)
    }

    /// Returns the upper bound.
    #[must_use]
    pub fn maximum(self) -> f64 {
        f64::from(self.maximum)
    }

    /// Returns the range width.
    #[must_use]
    pub fn width(self) -> f64 {
        f64::from(self.width_source())
    }

    const fn from_source(minimum: f32, maximum: f32) -> Self {
        Self { minimum, maximum }
    }

    const fn minimum_source(self) -> f32 {
        self.minimum
    }

    const fn maximum_source(self) -> f32 {
        self.maximum
    }

    fn width_source(self) -> f32 {
        self.maximum - self.minimum
    }
}

/// The current, preferred soft, and accepted hard ranges used by popup zoom.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SliderRanges {
    visible: SliderRange,
    soft: SliderRange,
    hard: SliderRange,
}

impl SliderRanges {
    /// Creates the three source ranges.
    #[must_use]
    pub const fn new(visible: SliderRange, soft: SliderRange, hard: SliderRange) -> Self {
        Self {
            visible,
            soft,
            hard,
        }
    }

    /// Returns the currently visible range.
    #[must_use]
    pub const fn visible(self) -> SliderRange {
        self.visible
    }

    /// Returns the preferred soft range.
    #[must_use]
    pub const fn soft(self) -> SliderRange {
        self.soft
    }

    /// Returns the accepted hard range.
    #[must_use]
    pub const fn hard(self) -> SliderRange {
        self.hard
    }
}

/// Result of one `_slider_zoom_range` transition.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ZoomRangeChange {
    /// A nonzero zoom was accepted.
    Zoomed(SliderRange),
    /// A zero zoom toggled hard/soft visibility. The adapter must recapture
    /// the slider's normalized position after applying this range.
    Toggled(SliderRange),
    /// The requested zoom crossed a hard bound or became too narrow.
    Rejected,
}

/// Computes the source's pointer offset from the popup's saved normalized
/// position.
///
/// `x`, `y`, and `header_fraction` are already normalized by the future GTK4
/// adapter. Above the header boundary movement is linear; below it the popup
/// progressively reaches the quadratic fine-tuning scale.
#[must_use]
#[expect(
    clippy::suboptimal_flops,
    reason = "The source popup pointer geometry preserves its original floating-point evaluation order."
)]
pub fn line_offset(position: f64, scale: f64, x: f64, y: f64, header_fraction: f64) -> f64 {
    let position = source_float(position);
    let scale = source_float(scale);
    let x = source_float(x);
    let header_fraction = source_float(header_fraction);
    let y = source_float(y);
    let offset = if y > header_fraction {
        let y = (y - header_fraction) / (1.0_f32 - header_fraction);
        let y_squared = y * y;
        (x - y_squared * 0.5_f32 - (1.0_f32 - y_squared) * position)
            / (0.5_f32 * y_squared / scale + (1.0_f32 - y_squared))
    } else {
        x - position
    };

    f64::from((position + offset).clamp(0.0_f32, 1.0_f32) - position)
}

/// Returns the normalized loupe scale used by both pointer mapping and
/// guideline rendering.
///
/// The display factor is intentionally made positive here exactly as in the
/// source. A negative factor reverses the slider curve, not popup geometry.
#[must_use]
pub fn loupe_scale(digits: i32, visible: SliderRange, factor: f64) -> f64 {
    let factor = source_float(factor);
    f64::from(
        5.0_f32 * 10.0_f32.powf(-source_digits(digits)) / visible.width_source() / factor.abs(),
    )
}

/// Maps a linear popup pointer to the slider's normalized position.
#[must_use]
pub fn linear_pointer_position(
    old_position: f64,
    scale: f64,
    normalized_x: f64,
    normalized_y: f64,
    header_fraction: f64,
) -> f64 {
    let old_position = source_float(old_position);
    let offset = source_float(line_offset(
        f64::from(old_position),
        scale,
        normalized_x,
        normalized_y,
        header_fraction,
    ));
    f64::from(old_position + offset)
}

/// Maps pointer coordinates to a normalized position for a full-circle slider.
///
/// Both coordinates are divided by popup width before this call, matching the
/// source's circular geometry. Within the central dead zone, the position from
/// popup entry is retained.
#[must_use]
#[expect(
    clippy::suboptimal_flops,
    reason = "The source circular popup mapping preserves its original multiply/add order."
)]
pub fn full_circle_pointer_position(
    old_position: f64,
    normalized_x: f64,
    normalized_y: f64,
) -> f64 {
    let old_position = source_float(old_position);
    let center_x = 0.5_f32 - source_float(normalized_x);
    let center_y = 0.5_f32 - source_float(normalized_y);
    if center_x.hypot(center_y) < FULL_CIRCLE_DEAD_ZONE_RADIUS {
        f64::from(old_position)
    } else {
        f64::from(center_x.atan2(center_y) * -0.5_f32 * std::f32::consts::FRAC_1_PI + 0.5_f32)
    }
}

/// Returns whether source motion-state rules should begin changing the slider.
///
/// The exclusive-or distinguishes crossing a linear guideline from wrapping
/// across the discontinuity of a circular slider.
#[must_use]
pub fn should_activate_change(
    primary_button_down: bool,
    previous_offset: f64,
    current_offset: f64,
) -> bool {
    let previous_offset = source_float(previous_offset);
    let current_offset = source_float(current_offset);
    primary_button_down
        || (previous_offset != 0.0_f32
            && ((previous_offset * current_offset <= 0.0_f32)
                ^ ((previous_offset - current_offset).abs() > 0.5_f32)))
}

/// Applies Darktable's value-centered power-of-two popup zoom.
///
/// A zero delta toggles between the hard and soft ranges. Restoring the
/// current value may expand the chosen soft range, mirroring the subsequent
/// `dt_bauhaus_slider_set` call. The signed factor in the minimum-visible
/// check is deliberately retained from the pinned source.
#[must_use]
#[expect(
    clippy::suboptimal_flops,
    reason = "The source popup zoom preserves value-centered multiply/add evaluation order."
)]
pub fn zoom_range(
    ranges: SliderRanges,
    value: f64,
    digits: i32,
    factor: f64,
    zoom: f64,
) -> ZoomRangeChange {
    let value = source_float(value);
    let factor = source_float(factor);
    let zoom = source_float(zoom);

    if zoom == 0.0_f32 {
        let target = if ranges.visible == ranges.hard {
            ranges.soft
        } else {
            ranges.hard
        };
        let restored_value = value
            .max(ranges.hard.minimum_source())
            .min(ranges.hard.maximum_source());
        return ZoomRangeChange::Toggled(SliderRange::from_source(
            target.minimum_source().min(restored_value),
            target.maximum_source().max(restored_value),
        ));
    }

    let minimum_visible = 10.0_f32.powf(-source_digits(digits)) / factor;
    let multiplier = (zoom / 2.0_f32).exp2();
    let new_minimum = value - multiplier * (value - ranges.visible.minimum_source());
    let new_maximum = value + multiplier * (ranges.visible.maximum_source() - value);

    if new_minimum >= ranges.hard.minimum_source()
        && new_maximum <= ranges.hard.maximum_source()
        && new_maximum - new_minimum >= minimum_visible * 10.0_f32
    {
        ZoomRangeChange::Zoomed(SliderRange::from_source(new_minimum, new_maximum))
    } else {
        ZoomRangeChange::Rejected
    }
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "GTK numeric inputs are narrowed at the source f32 parameter boundary."
)]
const fn source_float(value: f64) -> f32 {
    value as f32
}

#[expect(
    clippy::cast_precision_loss,
    reason = "The source slider's decimal-digit count is a small f32 formatting parameter."
)]
const fn source_digits(value: i32) -> f32 {
    value as f32
}

/// State for Darktable's shared smooth-scroll unit accumulators.
///
/// The GTK adapter must first apply platform event normalization (including
/// Darktable's Quartz divisor of 50) and share one instance across every
/// controller on the GTK main thread, matching the source's process-global
/// remainders without unsafe mutable statics.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct SmoothScrollAccumulator {
    x: f64,
    y: f64,
}

impl SmoothScrollAccumulator {
    /// Adds one smooth delta and returns whole units once either axis reaches
    /// one. Fractions are retained and truncation is toward zero.
    pub fn push(&mut self, delta_x: f64, delta_y: f64) -> Option<(i32, i32)> {
        if !delta_x.is_finite() || !delta_y.is_finite() {
            return None;
        }

        let next_x = self.x + delta_x;
        let next_y = self.y + delta_y;
        if !next_x.is_finite() || !next_y.is_finite() {
            return None;
        }
        self.x = next_x;
        self.y = next_y;
        let amount_x = self.x.trunc();
        let amount_y = self.y.trunc();
        if amount_x == 0.0 && amount_y == 0.0 {
            return None;
        }

        self.x -= amount_x;
        self.y -= amount_y;
        Some((unit_as_i32(amount_x), unit_as_i32(amount_y)))
    }

    /// Adds one smooth delta and combines both axes as
    /// `dt_gui_get_scroll_unit_delta` does.
    pub fn push_sum(&mut self, delta_x: f64, delta_y: f64) -> Option<i32> {
        self.push(delta_x, delta_y)
            .map(|(x, y)| x.saturating_add(y))
    }

    /// Clears fractions for a smooth-scroll stop event.
    pub const fn stop(&mut self) {
        self.x = 0.0;
        self.y = 0.0;
    }

    /// Returns the unconsumed fractional units.
    #[must_use]
    pub const fn remainder(self) -> (f64, f64) {
        (self.x, self.y)
    }
}

thread_local! {
    // GTK controllers run on the main thread. One thread-local therefore gives
    // every production widget the process-global source remainder without an
    // unsafe mutable static.
    static SOURCE_SCROLL_UNITS: RefCell<SmoothScrollAccumulator> =
        RefCell::new(SmoothScrollAccumulator::default());
}

/// Normalizes one raw GTK scroll event through Darktable's shared unit helper.
///
/// Wheel axes become signed discrete units. Surface axes retain independent
/// fractions, including the source's Quartz divisor, and emit their summed
/// whole units after truncation toward zero.
pub(crate) fn source_scroll_unit_delta(
    unit: gdk::ScrollUnit,
    delta_x: f64,
    delta_y: f64,
) -> Option<i32> {
    if !delta_x.is_finite() || !delta_y.is_finite() {
        return None;
    }

    if unit == gdk::ScrollUnit::Wheel {
        let unit_x = wheel_unit(delta_x);
        let unit_y = wheel_unit(delta_y);
        return (unit_x != 0 || unit_y != 0).then_some(unit_x.saturating_add(unit_y));
    }
    if unit != gdk::ScrollUnit::Surface {
        return None;
    }

    #[cfg(target_os = "macos")]
    let (delta_x, delta_y) = (delta_x / 50.0, delta_y / 50.0);
    SOURCE_SCROLL_UNITS.with(|units| units.borrow_mut().push_sum(delta_x, delta_y))
}

/// Clears both source-global surface-scroll fractions at any sequence end.
pub(crate) fn reset_source_scroll_units() {
    SOURCE_SCROLL_UNITS.with(|units| units.borrow_mut().stop());
}

fn wheel_unit(delta: f64) -> i32 {
    match delta.total_cmp(&0.0) {
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => 1,
    }
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "The accumulator clamps the whole scroll amount to the i32 GTK event domain."
)]
fn unit_as_i32(amount: f64) -> i32 {
    amount.clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32
}

#[cfg(test)]
mod tests {
    use super::{reset_source_scroll_units, source_scroll_unit_delta};
    use gtk4::gdk;

    #[test]
    fn source_unit_normalization_preserves_axes_signs_scaling_and_reset() {
        reset_source_scroll_units();
        assert_eq!(
            source_scroll_unit_delta(gdk::ScrollUnit::Wheel, -8.0, 3.0),
            Some(0),
            "opposing signed wheel axes are handled and summed"
        );
        assert_eq!(
            source_scroll_unit_delta(gdk::ScrollUnit::Wheel, 0.25, 0.0),
            Some(1)
        );
        assert_eq!(
            source_scroll_unit_delta(gdk::ScrollUnit::Wheel, f64::NAN, 1.0),
            None
        );

        #[cfg(target_os = "macos")]
        let raw_unit = 50.0;
        #[cfg(not(target_os = "macos"))]
        let raw_unit = 1.0;
        assert_eq!(
            source_scroll_unit_delta(gdk::ScrollUnit::Surface, 0.6 * raw_unit, -0.6 * raw_unit,),
            None
        );
        assert_eq!(
            source_scroll_unit_delta(gdk::ScrollUnit::Surface, 0.6 * raw_unit, 0.0),
            Some(1),
            "the x fraction truncates independently toward zero"
        );
        assert_eq!(
            source_scroll_unit_delta(gdk::ScrollUnit::Surface, 0.0, -0.6 * raw_unit),
            Some(-1),
            "the y fraction truncates independently toward zero"
        );

        reset_source_scroll_units();
        assert_eq!(
            source_scroll_unit_delta(gdk::ScrollUnit::Surface, 0.0, 0.5 * raw_unit),
            None
        );
        assert_eq!(
            source_scroll_unit_delta(gdk::ScrollUnit::Surface, 0.0, f64::INFINITY),
            None,
            "non-finite input is rejected without poisoning the remainder"
        );
        assert_eq!(
            source_scroll_unit_delta(gdk::ScrollUnit::Surface, 0.0, 0.5 * raw_unit),
            Some(1)
        );

        assert_eq!(
            source_scroll_unit_delta(gdk::ScrollUnit::Surface, 0.0, 0.75 * raw_unit),
            None
        );
        reset_source_scroll_units();
        assert_eq!(
            source_scroll_unit_delta(gdk::ScrollUnit::Surface, 0.0, 0.5 * raw_unit),
            None,
            "any scroll stop discards both global fractions"
        );
        reset_source_scroll_units();
    }
}
