//! Pure numeric-entry behavior retained from Darktable's Bauhaus slider.
//!
//! The character filter and capacity mirror `_popup_key_press` in
//! `src/bauhaus/bauhaus.c`; the storage size comes from
//! `DT_BAUHAUS_MAX_TEXT` in `src/bauhaus/bauhaus.h`.

use rusttable_core::common::calculator::solve;

/// Size of the source Bauhaus key buffer, including its trailing storage.
pub const MAX_TEXT_BYTES: usize = 180;

/// Maximum expression length allowed by the source's strict capacity check.
pub const MAX_EXPRESSION_BYTES: usize = MAX_TEXT_BYTES - 2;

const SLIDER_INPUT_CHARACTERS: &str = "0123456789.,%%+-*Xx/:^~ ()";

/// Source-compatible, UI-independent input buffer for a Bauhaus slider.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct NumericInputBuffer {
    expression: String,
}

impl NumericInputBuffer {
    /// Creates an empty numeric input buffer.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the expression accumulated so far.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.expression
    }

    /// Appends an accepted source slider character when capacity remains.
    pub fn push(&mut self, character: char) -> bool {
        if self.expression.len() >= MAX_EXPRESSION_BYTES
            || !SLIDER_INPUT_CHARACTERS.contains(character)
        {
            return false;
        }

        self.expression.push(character);
        true
    }

    /// Removes the last entered character, if any.
    pub fn erase_last(&mut self) -> bool {
        self.expression.pop().is_some()
    }

    /// Clears the expression when the popup opens or closes.
    pub fn clear(&mut self) {
        self.expression.clear();
    }
}

/// Evaluates a slider expression in displayed units and converts it to raw units.
///
/// This follows the source `get_val`/calculator/`set_val` path: `x` is
/// `raw_value * factor + offset`, and a finite result is mapped back with the
/// inverse transform.
#[must_use]
pub fn resolve_raw_value(
    raw_value: f64,
    factor: f64,
    offset: f64,
    expression: &str,
) -> Option<f64> {
    let displayed_value = raw_value * factor + offset;
    let resolved_displayed_value = solve(displayed_value, Some(expression));
    if !resolved_displayed_value.is_finite() {
        return None;
    }

    let resolved_raw_value = (resolved_displayed_value - offset) / factor;
    resolved_raw_value.is_finite().then_some(resolved_raw_value)
}
