use rusttable_core::{FiniteF64, PhotoId};

const EXPOSURE_MINIMUM: f64 = -5.0;
const EXPOSURE_MAXIMUM: f64 = 5.0;
const EXPOSURE_STEP: f64 = 0.01;
const RGB_GAIN_MINIMUM: f64 = 0.0;
const RGB_GAIN_MAXIMUM: f64 = 2.0;
const RGB_GAIN_STEP: f64 = 0.001;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BasicEditField {
    Exposure,
    RedGain,
    GreenGain,
    BlueGain,
}

impl BasicEditField {
    pub const ALL: [Self; 4] = [
        Self::Exposure,
        Self::RedGain,
        Self::GreenGain,
        Self::BlueGain,
    ];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Exposure => "Exposure",
            Self::RedGain => "Red gain",
            Self::GreenGain => "Green gain",
            Self::BlueGain => "Blue gain",
        }
    }

    #[must_use]
    pub const fn unit(self) -> &'static str {
        match self {
            Self::Exposure => "stops",
            Self::RedGain | Self::GreenGain | Self::BlueGain => "gain",
        }
    }

    #[must_use]
    const fn minimum(self) -> f64 {
        match self {
            Self::Exposure => EXPOSURE_MINIMUM,
            Self::RedGain | Self::GreenGain | Self::BlueGain => RGB_GAIN_MINIMUM,
        }
    }

    #[must_use]
    const fn maximum(self) -> f64 {
        match self {
            Self::Exposure => EXPOSURE_MAXIMUM,
            Self::RedGain | Self::GreenGain | Self::BlueGain => RGB_GAIN_MAXIMUM,
        }
    }

    #[must_use]
    const fn step(self) -> f64 {
        match self {
            Self::Exposure => EXPOSURE_STEP,
            Self::RedGain | Self::GreenGain | Self::BlueGain => RGB_GAIN_STEP,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BasicEditValues {
    exposure: FiniteF64,
    red_gain: FiniteF64,
    green_gain: FiniteF64,
    blue_gain: FiniteF64,
}

impl BasicEditValues {
    #[must_use]
    pub const fn from_finite(
        exposure: FiniteF64,
        red_gain: FiniteF64,
        green_gain: FiniteF64,
        blue_gain: FiniteF64,
    ) -> Self {
        Self {
            exposure,
            red_gain,
            green_gain,
            blue_gain,
        }
    }

    #[must_use]
    pub fn defaults() -> Self {
        Self::from_defaults()
    }

    #[must_use]
    pub fn value(self, field: BasicEditField) -> FiniteF64 {
        match field {
            BasicEditField::Exposure => self.exposure,
            BasicEditField::RedGain => self.red_gain,
            BasicEditField::GreenGain => self.green_gain,
            BasicEditField::BlueGain => self.blue_gain,
        }
    }

    #[must_use]
    pub fn display_value(self, field: BasicEditField) -> String {
        let precision = match field {
            BasicEditField::Exposure => 2,
            BasicEditField::RedGain | BasicEditField::GreenGain | BasicEditField::BlueGain => 3,
        };
        format!("{:.*}", precision, self.value(field).get())
    }

    fn from_defaults() -> Self {
        Self {
            exposure: finite(EXPOSURE_MINIMUM, 0.0, EXPOSURE_MAXIMUM),
            red_gain: finite(RGB_GAIN_MINIMUM, 1.0, RGB_GAIN_MAXIMUM),
            green_gain: finite(RGB_GAIN_MINIMUM, 1.0, RGB_GAIN_MAXIMUM),
            blue_gain: finite(RGB_GAIN_MINIMUM, 1.0, RGB_GAIN_MAXIMUM),
        }
    }

    fn adjust(&mut self, field: BasicEditField, increase: bool) {
        let current = self.value(field).get();
        let delta = if increase {
            field.step()
        } else {
            -field.step()
        };
        let next = (current + delta).clamp(field.minimum(), field.maximum());
        let value = finite(field.minimum(), next, field.maximum());
        match field {
            BasicEditField::Exposure => self.exposure = value,
            BasicEditField::RedGain => self.red_gain = value,
            BasicEditField::GreenGain => self.green_gain = value,
            BasicEditField::BlueGain => self.blue_gain = value,
        }
    }

    fn reset(&mut self) {
        *self = Self::from_defaults();
    }
}

#[must_use]
fn finite(minimum: f64, value: f64, maximum: f64) -> FiniteF64 {
    debug_assert!((minimum..=maximum).contains(&value));
    FiniteF64::new(value).expect("basic edit bounds are finite")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BasicEditSaveState {
    Clean,
    Unsaved,
    Saving,
    Failed,
    Conflict,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BasicEditInspectorViewModel {
    photo_id: PhotoId,
    saved: BasicEditValues,
    draft: BasicEditValues,
    save_state: BasicEditSaveState,
}

impl BasicEditInspectorViewModel {
    #[must_use]
    pub fn new(photo_id: PhotoId) -> Self {
        let values = BasicEditValues::defaults();
        Self::with_values(photo_id, values)
    }

    #[must_use]
    pub const fn with_values(photo_id: PhotoId, values: BasicEditValues) -> Self {
        Self {
            photo_id,
            saved: values,
            draft: values,
            save_state: BasicEditSaveState::Clean,
        }
    }

    #[must_use]
    pub const fn photo_id(self) -> PhotoId {
        self.photo_id
    }

    #[must_use]
    pub const fn values(self) -> BasicEditValues {
        self.draft
    }

    #[must_use]
    pub const fn save_state(self) -> BasicEditSaveState {
        self.save_state
    }

    #[must_use]
    pub const fn is_dirty(self) -> bool {
        !matches!(self.save_state, BasicEditSaveState::Clean)
    }

    pub fn increment(&mut self, field: BasicEditField) {
        self.draft.adjust(field, true);
        self.mark_unsaved();
    }

    pub fn decrement(&mut self, field: BasicEditField) {
        self.draft.adjust(field, false);
        self.mark_unsaved();
    }

    pub fn reset(&mut self) {
        self.draft.reset();
        self.mark_unsaved();
    }

    pub fn set_draft_values(&mut self, values: BasicEditValues) {
        self.draft = values;
        self.mark_unsaved();
    }

    pub fn apply_saved_values(&mut self, values: BasicEditValues) {
        self.saved = values;
        self.draft = values;
        self.save_state = BasicEditSaveState::Clean;
    }

    pub fn begin_save(&mut self) {
        self.save_state = BasicEditSaveState::Saving;
    }

    pub fn mark_save_failed(&mut self) {
        self.save_state = BasicEditSaveState::Failed;
    }

    pub fn mark_save_conflicted(&mut self) {
        self.save_state = BasicEditSaveState::Conflict;
    }

    pub fn request_save(&mut self) {
        self.begin_save();
    }

    fn mark_unsaved(&mut self) {
        self.save_state = if self.draft == self.saved {
            BasicEditSaveState::Clean
        } else {
            BasicEditSaveState::Unsaved
        };
    }
}

#[cfg(test)]
mod tests {
    use rusttable_core::{FiniteF64, PhotoId};

    use super::{BasicEditField, BasicEditInspectorViewModel, BasicEditSaveState, BasicEditValues};

    fn photo_id() -> PhotoId {
        PhotoId::new(7).expect("test photo ID is non-zero")
    }

    #[test]
    fn defaults_are_finite_typed_and_displayable() {
        let values = BasicEditValues::defaults();

        assert_eq!(
            values.value(BasicEditField::Exposure),
            FiniteF64::new(0.0).expect("zero is finite")
        );
        assert_eq!(
            values.value(BasicEditField::RedGain),
            FiniteF64::new(1.0).expect("one is finite")
        );
        assert_eq!(values.display_value(BasicEditField::Exposure), "0.00");
        assert_eq!(values.display_value(BasicEditField::BlueGain), "1.000");
        assert!(FiniteF64::new(values.value(BasicEditField::GreenGain).get()).is_ok());
    }

    #[test]
    fn increments_and_decrements_clamp_at_operation_bounds() {
        let mut inspector = BasicEditInspectorViewModel::new(photo_id());

        for _ in 0..600 {
            inspector.increment(BasicEditField::Exposure);
        }
        assert_eq!(
            inspector.values().value(BasicEditField::Exposure),
            FiniteF64::new(5.0).expect("maximum exposure is finite")
        );
        for _ in 0..3_000 {
            inspector.decrement(BasicEditField::RedGain);
        }
        assert_eq!(
            inspector.values().value(BasicEditField::RedGain),
            FiniteF64::new(0.0).expect("minimum gain is finite")
        );
    }

    #[test]
    fn save_lifecycle_preserves_draft_until_successful_projection() {
        let mut inspector = BasicEditInspectorViewModel::new(photo_id());
        inspector.increment(BasicEditField::GreenGain);
        assert_eq!(inspector.save_state(), BasicEditSaveState::Unsaved);

        let unsaved_values = inspector.values();
        inspector.begin_save();
        assert_eq!(inspector.save_state(), BasicEditSaveState::Saving);
        assert_eq!(inspector.values(), unsaved_values);

        inspector.mark_save_failed();
        assert_eq!(inspector.save_state(), BasicEditSaveState::Failed);
        assert_eq!(inspector.values(), unsaved_values);

        inspector.apply_saved_values(unsaved_values);
        assert_eq!(inspector.save_state(), BasicEditSaveState::Clean);
        assert_eq!(inspector.values(), unsaved_values);
    }

    #[test]
    fn editing_after_a_failed_save_returns_to_unsaved() {
        let mut inspector = BasicEditInspectorViewModel::new(photo_id());
        inspector.increment(BasicEditField::GreenGain);
        inspector.begin_save();
        inspector.mark_save_failed();

        inspector.increment(BasicEditField::GreenGain);

        assert_eq!(inspector.save_state(), BasicEditSaveState::Unsaved);
    }
}
