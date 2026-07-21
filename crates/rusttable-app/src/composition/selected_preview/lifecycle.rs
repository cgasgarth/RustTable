use rusttable_core::PhotoId;

/// Monotonic identity for one selected-photo preview request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PreviewSelectionToken {
    generation: u64,
    photo_id: PhotoId,
}

/// Tracks which asynchronous preview result is still allowed to update the UI.
#[derive(Debug, Default)]
pub(crate) struct PreviewLifecycle {
    next_generation: u64,
    active: Option<PreviewSelectionToken>,
}

impl PreviewLifecycle {
    pub(crate) fn begin(&mut self, photo_id: PhotoId) -> PreviewSelectionToken {
        self.next_generation = self.next_generation.saturating_add(1);
        let token = PreviewSelectionToken {
            generation: self.next_generation,
            photo_id,
        };
        self.active = Some(token);
        token
    }

    #[must_use]
    pub(crate) fn is_current(&self, token: PreviewSelectionToken) -> bool {
        match self.active {
            Some(active) => {
                active.generation == token.generation && active.photo_id == token.photo_id
            }
            None => false,
        }
    }
}

impl PreviewSelectionToken {
    #[must_use]
    pub(crate) const fn generation(self) -> u64 {
        self.generation
    }
}

#[cfg(test)]
mod tests {
    use rusttable_core::PhotoId;

    use super::PreviewLifecycle;

    fn photo_id(value: u128) -> PhotoId {
        PhotoId::new(value).expect("non-zero test photo ID")
    }

    #[test]
    fn only_the_latest_selection_token_remains_current() {
        let mut lifecycle = PreviewLifecycle::default();
        let first = lifecycle.begin(photo_id(1));
        let second = lifecycle.begin(photo_id(2));

        assert!(!lifecycle.is_current(first));
        assert!(lifecycle.is_current(second));
    }

    #[test]
    fn reselection_gets_a_new_generation_for_stale_result_protection() {
        let mut lifecycle = PreviewLifecycle::default();
        let first = lifecycle.begin(photo_id(1));
        let second = lifecycle.begin(photo_id(1));

        assert_ne!(first, second);
        assert!(!lifecycle.is_current(first));
        assert!(lifecycle.is_current(second));
    }
}
