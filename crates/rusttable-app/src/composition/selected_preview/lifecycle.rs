use rusttable_core::{EditId, PhotoId, Revision};
use rusttable_pixelpipe::{CancellationReason, CancellationScope, PipelineGeneration};

/// Monotonic identity for one selected-photo preview request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PreviewSelectionToken {
    generation: u64,
    photo_id: PhotoId,
    edit_id: EditId,
    edit_revision: Revision,
}

/// Tracks which asynchronous preview result is still allowed to update the UI.
#[derive(Debug, Default)]
pub(crate) struct PreviewLifecycle {
    next_generation: u64,
    active: Option<PreviewSelectionToken>,
    active_cancellation: Option<CancellationScope>,
}

impl PreviewLifecycle {
    pub(crate) fn begin(
        &mut self,
        photo_id: PhotoId,
        edit_id: EditId,
        edit_revision: Revision,
    ) -> PreviewSelectionToken {
        self.next_generation = self
            .next_generation
            .checked_add(1)
            .expect("preview generation must not wrap");
        let generation = PipelineGeneration::new(self.next_generation)
            .expect("preview generation starts at one");
        self.invalidate(CancellationReason::SupersededGeneration(generation));
        let token = PreviewSelectionToken {
            generation: self.next_generation,
            photo_id,
            edit_id,
            edit_revision,
        };
        self.active = Some(token);
        self.active_cancellation = Some(CancellationScope::root(generation));
        token
    }

    pub(crate) fn invalidate(&mut self, reason: CancellationReason) {
        self.active = None;
        if let Some(cancellation) = self.active_cancellation.take() {
            cancellation.cancel(reason);
        }
    }

    #[must_use]
    pub(crate) fn is_current(&self, token: PreviewSelectionToken) -> bool {
        match self.active {
            Some(active) => {
                active.generation == token.generation
                    && active.photo_id == token.photo_id
                    && active.edit_id == token.edit_id
                    && active.edit_revision == token.edit_revision
            }
            None => false,
        }
    }

    #[must_use]
    pub(crate) fn cancellation_scope(
        &self,
        token: PreviewSelectionToken,
    ) -> Option<CancellationScope> {
        if self.is_current(token) {
            self.active_cancellation.clone()
        } else {
            None
        }
    }
}

impl PreviewSelectionToken {
    #[must_use]
    pub(crate) const fn generation(self) -> u64 {
        self.generation
    }

    pub(crate) const fn photo_id(self) -> PhotoId {
        self.photo_id
    }

    pub(crate) const fn edit_id(self) -> EditId {
        self.edit_id
    }

    pub(crate) const fn edit_revision(self) -> Revision {
        self.edit_revision
    }
}

#[cfg(test)]
mod tests {
    use rusttable_core::{EditId, PhotoId, Revision};
    use rusttable_pixelpipe::{CancellationReason, PipelineGeneration};

    use super::PreviewLifecycle;

    fn photo_id(value: u128) -> PhotoId {
        PhotoId::new(value).expect("non-zero test photo ID")
    }

    #[test]
    fn only_the_latest_selection_token_remains_current() {
        let mut lifecycle = PreviewLifecycle::default();
        let first = lifecycle.begin(photo_id(1), EditId::new(2).unwrap(), Revision::from_u64(1));
        let second = lifecycle.begin(photo_id(2), EditId::new(3).unwrap(), Revision::from_u64(1));

        assert!(!lifecycle.is_current(first));
        assert!(lifecycle.is_current(second));
    }

    #[test]
    fn reselection_gets_a_new_generation_for_stale_result_protection() {
        let mut lifecycle = PreviewLifecycle::default();
        let first = lifecycle.begin(photo_id(1), EditId::new(2).unwrap(), Revision::from_u64(1));
        let first_cancellation = lifecycle
            .cancellation_scope(first)
            .expect("first cancellation scope");
        let second = lifecycle.begin(photo_id(1), EditId::new(2).unwrap(), Revision::from_u64(2));

        assert_ne!(first, second);
        assert!(!lifecycle.is_current(first));
        assert!(lifecycle.is_current(second));
        let error = first_cancellation
            .check()
            .expect_err("superseded scope is cancelled");
        assert_eq!(
            error.reason(),
            CancellationReason::SupersededGeneration(
                PipelineGeneration::new(second.generation()).expect("generation")
            )
        );
        lifecycle
            .cancellation_scope(second)
            .expect("current cancellation scope")
            .check()
            .expect("current scope remains live");
    }

    #[test]
    fn invalidation_cancels_the_scope_and_revokes_the_active_token() {
        let mut lifecycle = PreviewLifecycle::default();
        let token = lifecycle.begin(photo_id(1), EditId::new(2).unwrap(), Revision::from_u64(1));
        let cancellation = lifecycle
            .cancellation_scope(token)
            .expect("active cancellation scope");

        lifecycle.invalidate(CancellationReason::SelectionChanged);

        assert!(!lifecycle.is_current(token));
        assert!(lifecycle.cancellation_scope(token).is_none());
        assert_eq!(
            cancellation
                .check()
                .expect_err("invalidated scope is cancelled")
                .reason(),
            CancellationReason::SelectionChanged
        );
    }
}
