#![allow(
    clippy::manual_inspect,
    clippy::manual_let_else,
    clippy::missing_errors_doc,
    clippy::redundant_closure_for_method_calls,
    clippy::semicolon_if_nothing_returned,
    clippy::wildcard_imports
)]

use super::model::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AiBatchControllerError {
    Service(AiBatchServiceError),
    InvalidState,
}
impl From<AiBatchServiceError> for AiBatchControllerError {
    fn from(value: AiBatchServiceError) -> Self {
        Self::Service(value)
    }
}

pub struct AiBatchController<S> {
    service: S,
    selection: Vec<AiBatchSelection>,
    recipe: AiBatchRecipe,
    state: AiBatchState,
}
impl<S: AiBatchServicePort> AiBatchController<S> {
    #[must_use]
    pub fn new(service: S) -> Self {
        Self {
            service,
            selection: Vec::new(),
            recipe: AiBatchRecipe::default(),
            state: AiBatchState::default(),
        }
    }
    #[must_use]
    pub const fn state(&self) -> &AiBatchState {
        &self.state
    }
    #[must_use]
    pub const fn recipe(&self) -> &AiBatchRecipe {
        &self.recipe
    }
    pub fn set_selection(&mut self, selection: Vec<AiBatchSelection>) {
        self.selection = selection;
        self.state = if self.selection.is_empty() {
            AiBatchState::Empty
        } else {
            AiBatchState::Reviewing(AiBatchReview::new(self.recipe.clone(), Vec::new()))
        };
    }
    pub fn dispatch(&mut self, action: AiBatchAction) -> Result<(), AiBatchControllerError> {
        match action {
            AiBatchAction::SelectTask(value) => self.recipe.set_task(value),
            AiBatchAction::SelectModel(value) => self.recipe.set_model(value),
            AiBatchAction::SelectProvider(value) => self.recipe.set_provider(value),
            AiBatchAction::SetStrength(value) => self.recipe.set_strength(value),
            AiBatchAction::SetPolicy(policy) => {
                if let AiBatchState::Reviewing(review) = &mut self.state {
                    review.set_policy(policy);
                }
            }
            AiBatchAction::Review => self.review()?,
            AiBatchAction::Confirm => self.confirm()?,
            AiBatchAction::Pause => self.control(|service, id| service.pause(id))?,
            AiBatchAction::Resume => self.control(|service, id| service.resume(id))?,
            AiBatchAction::Cancel => self.control(|service, id| service.cancel(id))?,
            AiBatchAction::RetryFailed => self.control(|service, id| service.retry_failed(id))?,
            AiBatchAction::Reconcile => self.control(|service, id| service.reconcile(id))?,
            AiBatchAction::RemoveHistory => {
                self.control(|service, id| service.remove_history(id))?
            }
        }
        Ok(())
    }
    fn review(&mut self) -> Result<(), AiBatchControllerError> {
        let review = self
            .service
            .review(&self.selection, &self.recipe)
            .map_err(|error| {
                self.state = AiBatchState::Unavailable {
                    detail: error.to_string(),
                };
                error
            })?;
        self.state = AiBatchState::Reviewing(review);
        Ok(())
    }
    fn confirm(&mut self) -> Result<(), AiBatchControllerError> {
        let AiBatchState::Reviewing(review) = &self.state else {
            return Err(AiBatchControllerError::InvalidState);
        };
        let summary = self.service.preflight(review)?;
        if !review.can_enqueue() {
            self.state = AiBatchState::Failed {
                detail: "No eligible rows satisfy the enqueue policy.".to_owned(),
            };
            return Ok(());
        }
        self.state = AiBatchState::Preflight {
            review: review.clone(),
            summary,
        };
        Ok(())
    }
    fn control(
        &mut self,
        command: impl FnOnce(&mut S, u64) -> Result<(), AiBatchServiceError>,
    ) -> Result<(), AiBatchControllerError> {
        let id = match self.state {
            AiBatchState::Queued { batch_id, .. }
            | AiBatchState::Running { batch_id, .. }
            | AiBatchState::Paused { batch_id }
            | AiBatchState::Recovering { batch_id, .. }
            | AiBatchState::Complete { batch_id } => batch_id,
            _ => return Err(AiBatchControllerError::InvalidState),
        };
        command(&mut self.service, id)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Unavailable;
    impl AiBatchServicePort for Unavailable {
        fn review(
            &mut self,
            _: &[AiBatchSelection],
            _: &AiBatchRecipe,
        ) -> Result<AiBatchReview, AiBatchServiceError> {
            Err(AiBatchServiceError::Unavailable)
        }
        fn preflight(
            &mut self,
            _: &AiBatchReview,
        ) -> Result<AiBatchPreflight, AiBatchServiceError> {
            Err(AiBatchServiceError::Unavailable)
        }
        fn enqueue(
            &mut self,
            _: &AiBatchReview,
            _: &AiBatchPreflight,
        ) -> Result<u64, AiBatchServiceError> {
            Err(AiBatchServiceError::Unavailable)
        }
        fn pause(&mut self, _: u64) -> Result<(), AiBatchServiceError> {
            Err(AiBatchServiceError::Unavailable)
        }
        fn resume(&mut self, _: u64) -> Result<(), AiBatchServiceError> {
            Err(AiBatchServiceError::Unavailable)
        }
        fn cancel(&mut self, _: u64) -> Result<(), AiBatchServiceError> {
            Err(AiBatchServiceError::Unavailable)
        }
        fn retry_failed(&mut self, _: u64) -> Result<(), AiBatchServiceError> {
            Err(AiBatchServiceError::Unavailable)
        }
        fn reconcile(&mut self, _: u64) -> Result<(), AiBatchServiceError> {
            Err(AiBatchServiceError::Unavailable)
        }
        fn remove_history(&mut self, _: u64) -> Result<(), AiBatchServiceError> {
            Err(AiBatchServiceError::Unavailable)
        }
    }
    #[test]
    fn unavailable_backend_is_not_presented_as_progress() {
        let mut controller = AiBatchController::new(Unavailable);
        controller.set_selection(Vec::new());
        assert!(matches!(controller.state(), AiBatchState::Empty));
        assert!(controller.dispatch(AiBatchAction::Review).is_err());
    }
}
