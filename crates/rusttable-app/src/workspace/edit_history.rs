use std::collections::VecDeque;
use std::fmt;

use super::{BasicEditDraft, BasicEditValueError};

/// A validated-at-application-time mutation of a basic edit draft.
///
/// The variants keep the four editable values distinct so a reducer cannot accidentally apply
/// an RGB value to exposure (or vice versa). Values are validated by [`BasicEditSession::apply`]
/// through the draft's existing range and finiteness checks.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BasicEditMutation {
    SetExposureStops(f64),
    SetRgbRed(f64),
    SetRgbGreen(f64),
    SetRgbBlue(f64),
    Reset,
}

impl BasicEditMutation {
    fn apply_to(self, mut draft: BasicEditDraft) -> Result<BasicEditDraft, BasicEditValueError> {
        match self {
            Self::SetExposureStops(value) => draft.set_exposure_stops(value)?,
            Self::SetRgbRed(value) => draft.set_rgb_red(value)?,
            Self::SetRgbGreen(value) => draft.set_rgb_green(value)?,
            Self::SetRgbBlue(value) => draft.set_rgb_blue(value)?,
            Self::Reset => {
                draft.set_exposure_stops(0.0)?;
                draft.set_rgb_red(1.0)?;
                draft.set_rgb_green(1.0)?;
                draft.set_rgb_blue(1.0)?;
            }
        }
        Ok(draft)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct EditTransition {
    before: BasicEditDraft,
    after: BasicEditDraft,
}

/// In-memory undo/redo state for one basic-edit editing session.
///
/// A session owns only draft snapshots. The source [`rusttable_core::Edit`] remains encapsulated
/// by [`BasicEditDraft`], and no history is persisted. `capacity` bounds the undo stack; redo
/// entries are bounded by the same stack because a new mutation clears them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BasicEditSession {
    current: BasicEditDraft,
    undo: VecDeque<EditTransition>,
    redo: Vec<EditTransition>,
    capacity: usize,
}

impl BasicEditSession {
    /// Starts a session at `draft` with at most `capacity` undoable changes.
    ///
    /// A capacity of zero keeps the current draft live but disables undo and redo.
    #[must_use]
    pub fn new(draft: BasicEditDraft, capacity: usize) -> Self {
        Self {
            current: draft,
            undo: VecDeque::with_capacity(capacity),
            redo: Vec::new(),
            capacity,
        }
    }

    /// Returns the current draft without exposing its internal edit representation.
    #[must_use]
    pub const fn draft(&self) -> &BasicEditDraft {
        &self.current
    }

    /// Returns the number of currently available undo operations.
    #[cfg(test)]
    #[must_use]
    pub fn undo_len(&self) -> usize {
        self.undo.len()
    }

    /// Returns the number of currently available redo operations.
    #[cfg(test)]
    #[must_use]
    pub fn redo_len(&self) -> usize {
        self.redo.len()
    }

    /// Applies a mutation and returns whether it changed the draft.
    ///
    /// Invalid values leave the session, including its redo stack, unchanged. An exact typed
    /// draft match is treated as a no-op; no approximate floating-point comparison is used.
    ///
    /// # Errors
    ///
    /// Returns [`BasicEditValueError`] when the mutation has an invalid typed value.
    pub fn apply(&mut self, mutation: BasicEditMutation) -> Result<bool, BasicEditValueError> {
        let next = mutation.apply_to(self.current.clone())?;
        if next == self.current {
            return Ok(false);
        }

        let transition = EditTransition {
            before: self.current.clone(),
            after: next.clone(),
        };
        self.current = next;
        self.redo.clear();

        if self.capacity == 0 {
            self.undo.clear();
        } else {
            if self.undo.len() == self.capacity {
                let _ = self.undo.pop_front();
            }
            self.undo.push_back(transition);
        }

        Ok(true)
    }

    /// Reverts the newest retained mutation and returns whether anything was undone.
    pub fn undo(&mut self) -> bool {
        let Some(transition) = self.undo.pop_back() else {
            return false;
        };
        self.current = transition.before.clone();
        self.redo.push(transition);
        true
    }

    /// Reapplies the newest undone mutation and returns whether anything was redone.
    pub fn redo(&mut self) -> bool {
        let Some(transition) = self.redo.pop() else {
            return false;
        };
        self.current = transition.after.clone();
        self.undo.push_back(transition);
        if self.undo.len() > self.capacity {
            let _ = self.undo.pop_front();
        }
        true
    }
}

impl fmt::Display for BasicEditMutation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::SetExposureStops(_) => "exposure stops",
            Self::SetRgbRed(_) => "RGB red gain",
            Self::SetRgbGreen(_) => "RGB green gain",
            Self::SetRgbBlue(_) => "RGB blue gain",
            Self::Reset => "basic edit defaults",
        };
        formatter.write_str(name)
    }
}

#[cfg(test)]
mod tests {
    use rusttable_core::{
        Edit, EditId, FiniteF64, Operation, OperationId, OperationKey, OperationOpacity,
        ParameterName, ParameterValue, PhotoId, Revision,
    };

    use super::{BasicEditDraft, BasicEditMutation, BasicEditSession};

    fn session(capacity: usize) -> BasicEditSession {
        BasicEditSession::new(draft(), capacity)
    }

    fn draft() -> BasicEditDraft {
        let edit = Edit::from_parts(
            EditId::new(1).expect("test edit ID is nonzero"),
            PhotoId::new(2).expect("test photo ID is nonzero"),
            Revision::ZERO,
            Revision::ZERO,
            [
                operation(10, "rusttable.exposure", [("stops", scalar(0.0))]),
                operation(
                    20,
                    "rusttable.rgb_gain",
                    [
                        ("red", scalar(1.0)),
                        ("green", scalar(1.0)),
                        ("blue", scalar(1.0)),
                    ],
                ),
            ],
        )
        .expect("test edit is valid");
        BasicEditDraft::from_edit(&edit).expect("test edit has a basic draft")
    }

    fn scalar(value: f64) -> ParameterValue {
        ParameterValue::Scalar(FiniteF64::new(value).expect("test scalar is finite"))
    }

    fn operation<const N: usize>(
        id: u128,
        key: &'static str,
        parameters: [(&'static str, ParameterValue); N],
    ) -> Operation {
        Operation::new_with_opacity(
            OperationId::new(id).expect("test operation ID is nonzero"),
            OperationKey::new(key).expect("test operation key is valid"),
            true,
            OperationOpacity::ONE,
            parameters.into_iter().map(|(name, value)| {
                (
                    ParameterName::new(name).expect("test parameter name is valid"),
                    value,
                )
            }),
        )
        .expect("test operation is valid")
    }

    fn assert_values(session: &BasicEditSession, exposure: f64, red: f64) {
        assert_scalar(session.draft().exposure_stops(), exposure);
        assert_scalar(session.draft().rgb_red(), red);
    }

    fn assert_scalar(actual: f64, expected: f64) {
        assert_eq!(
            FiniteF64::new(actual).expect("draft value is finite"),
            FiniteF64::new(expected).expect("test value is finite")
        );
    }

    #[test]
    fn applies_typed_mutations_and_undoes_and_redoes_them() {
        let mut session = session(4);
        assert_eq!(session.undo_len(), 0);
        assert_eq!(session.redo_len(), 0);

        assert!(
            session
                .apply(BasicEditMutation::SetExposureStops(2.5))
                .expect("valid exposure")
        );
        assert!(
            session
                .apply(BasicEditMutation::SetRgbRed(0.25))
                .expect("valid red gain")
        );
        assert_values(&session, 2.5, 0.25);
        assert_eq!(session.undo_len(), 2);

        assert!(session.undo());
        assert_values(&session, 2.5, 1.0);
        assert!(session.redo());
        assert_values(&session, 2.5, 0.25);
        assert!(!session.redo());
    }

    #[test]
    fn new_effective_mutation_invalidates_redo() {
        let mut session = session(4);
        session
            .apply(BasicEditMutation::SetExposureStops(1.0))
            .expect("valid exposure");
        session
            .apply(BasicEditMutation::SetRgbRed(0.5))
            .expect("valid red gain");
        assert!(session.undo());

        session
            .apply(BasicEditMutation::SetRgbGreen(1.5))
            .expect("valid green gain");
        assert_eq!(session.redo_len(), 0);
        assert!(!session.redo());
        assert_values(&session, 1.0, 1.0);
        assert_scalar(session.draft().rgb_green(), 1.5);
    }

    #[test]
    fn retains_only_the_newest_bounded_changes() {
        let mut session = session(2);
        for value in [1.0, 2.0, 3.0] {
            session
                .apply(BasicEditMutation::SetExposureStops(value))
                .expect("valid exposure");
        }

        assert_eq!(session.undo_len(), 2);
        assert!(session.undo());
        assert!(session.undo());
        assert!(!session.undo());
        assert_scalar(session.draft().exposure_stops(), 1.0);
    }

    #[test]
    fn invalid_mutation_does_not_change_or_clear_history() {
        let mut session = session(2);
        session
            .apply(BasicEditMutation::SetExposureStops(1.0))
            .expect("valid exposure");
        assert!(session.undo());
        assert_eq!(session.redo_len(), 1);

        let result = session.apply(BasicEditMutation::SetExposureStops(f64::NAN));
        assert!(result.is_err());
        assert_values(&session, 0.0, 1.0);
        assert_eq!(session.redo_len(), 1);
    }

    #[test]
    fn no_op_does_not_create_history_or_invalidate_redo() {
        let mut session = session(2);
        session
            .apply(BasicEditMutation::SetExposureStops(1.0))
            .expect("valid exposure");
        assert!(session.undo());

        assert!(
            !session
                .apply(BasicEditMutation::SetExposureStops(0.0))
                .expect("valid no-op")
        );
        assert_eq!(session.undo_len(), 0);
        assert_eq!(session.redo_len(), 1);
    }

    #[test]
    fn zero_capacity_keeps_changes_but_has_no_history() {
        let mut session = session(0);
        assert!(
            session
                .apply(BasicEditMutation::SetRgbBlue(0.25))
                .expect("valid blue gain")
        );
        assert_scalar(session.draft().rgb_blue(), 0.25);
        assert_eq!(session.undo_len(), 0);
        assert!(!session.undo());
        assert!(!session.redo());
    }

    #[test]
    fn reset_is_one_undoable_typed_mutation() {
        let mut session = session(2);
        session
            .apply(BasicEditMutation::SetExposureStops(-2.0))
            .expect("valid exposure");
        session
            .apply(BasicEditMutation::SetRgbRed(0.25))
            .expect("valid red gain");
        session
            .apply(BasicEditMutation::SetRgbGreen(1.5))
            .expect("valid green gain");
        session
            .apply(BasicEditMutation::SetRgbBlue(1.75))
            .expect("valid blue gain");

        assert!(session.apply(BasicEditMutation::Reset).expect("reset"));
        assert_values(&session, 0.0, 1.0);
        assert_scalar(session.draft().rgb_green(), 1.0);
        assert_scalar(session.draft().rgb_blue(), 1.0);
        assert!(session.undo());
        assert_values(&session, -2.0, 0.25);
        assert_scalar(session.draft().rgb_green(), 1.5);
        assert_scalar(session.draft().rgb_blue(), 1.75);
    }
}
