use std::collections::BTreeMap;

use super::assembly::Assembly;
use super::{
    AssemblyPlan, AssemblyReceipt, AttemptFailure, AttemptFailureKind, AttemptId, AttemptOutcome,
    AttemptReceipt, AttemptResources, CleanupStatus, OutputFragment, PublicationBackend,
    PublicationReceipt, RecoveryContext, RecoveryDecision, RecoveryError, RecoveryRequest,
    TileCandidate,
};
use crate::{CompletionOutcome, CompletionReceipt, EncodingReceipt, ReceiptStatus, SubmissionId};

#[derive(Debug)]
struct ActiveAttempt {
    id: AttemptId,
    number: u8,
    candidate: TileCandidate,
    assembly: Assembly,
    resources: Option<AttemptResources>,
    dispatches: Vec<EncodingReceipt>,
    submissions: BTreeMap<SubmissionId, CompletionReceipt>,
}

impl ActiveAttempt {
    fn receipt(
        &self,
        outcome: AttemptOutcome,
        failure: Option<AttemptFailureKind>,
        retired_resources: usize,
    ) -> AttemptReceipt {
        AttemptReceipt {
            id: self.id,
            number: self.number,
            candidate: self.candidate,
            outcome,
            dispatches: self.dispatches.len(),
            submissions: self.submissions.len(),
            retired_resources,
            discarded: outcome != AttemptOutcome::Succeeded,
            failure,
        }
    }

    fn release_resources(&mut self) -> Result<usize, RecoveryError> {
        self.resources
            .take()
            .map_or(Ok(0), AttemptResources::release)
            .map_err(Into::into)
    }

    fn discard_resources(&mut self) -> Result<usize, RecoveryError> {
        self.resources
            .take()
            .map_or(Ok(0), AttemptResources::discard)
            .map_err(Into::into)
    }

    fn completed_resources(&self) -> Result<usize, RecoveryError> {
        for (id, receipt) in &self.submissions {
            if !matches!(receipt.outcome, CompletionOutcome::Completed) {
                return Err(RecoveryError::SubmissionFailed(
                    *id,
                    receipt.outcome.clone(),
                ));
            }
        }
        Ok(self
            .submissions
            .values()
            .map(|receipt| receipt.retired_resources)
            .sum())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SessionState {
    Ready,
    CpuFallback,
    Succeeded,
    Cancelled,
    Obsolete,
    Rejected,
    Published,
}

/// Finite, one-way coordinator for GPU attempts and publication.
#[derive(Debug)]
pub struct RecoverySession {
    request: RecoveryRequest,
    next_candidate: usize,
    next_attempt: u32,
    active: Option<ActiveAttempt>,
    receipts: Vec<AttemptReceipt>,
    success: Option<AssemblyReceipt>,
    state: SessionState,
}

impl RecoverySession {
    #[must_use]
    pub fn new(request: RecoveryRequest) -> Self {
        Self {
            request,
            next_candidate: 0,
            next_attempt: 1,
            active: None,
            receipts: Vec::new(),
            success: None,
            state: SessionState::Ready,
        }
    }

    #[must_use]
    pub const fn context(&self) -> RecoveryContext {
        self.request.context
    }

    #[must_use]
    pub fn attempts(&self) -> &[AttemptReceipt] {
        &self.receipts
    }

    #[must_use]
    pub const fn active_attempt(&self) -> Option<AttemptId> {
        match &self.active {
            Some(active) => Some(active.id),
            None => None,
        }
    }

    /// Returns the active attempt's immutable assembly contract.
    #[must_use]
    pub fn active_assembly(&self) -> Option<&AssemblyPlan> {
        self.active.as_ref().map(|active| active.assembly.plan())
    }

    /// Starts the next planner-supplied candidate in deterministic order.
    pub fn begin_attempt(&mut self) -> Result<AttemptId, RecoveryError> {
        if self.request.cancellation.is_cancelled() {
            self.state = SessionState::Cancelled;
            return Err(RecoveryError::Cancelled);
        }
        if self.state != SessionState::Ready || self.active.is_some() {
            return Err(RecoveryError::NotReady);
        }
        let plan = self
            .request
            .attempt_plan(self.next_candidate)
            .ok_or(RecoveryError::NotReady)?;
        let id = AttemptId::new(self.next_attempt);
        self.next_attempt = self
            .next_attempt
            .checked_add(1)
            .ok_or(RecoveryError::NotReady)?;
        self.next_candidate += 1;
        self.active = Some(ActiveAttempt {
            id,
            number: u8::try_from(self.next_candidate).map_err(|_| RecoveryError::NotReady)?,
            candidate: plan.candidate,
            assembly: Assembly::new(plan.assembly.clone(), id),
            resources: None,
            dispatches: Vec::new(),
            submissions: BTreeMap::new(),
        });
        Ok(id)
    }

    pub fn attach_resources(
        &mut self,
        id: AttemptId,
        resources: AttemptResources,
    ) -> Result<(), RecoveryError> {
        let active = self.active_mut(id)?;
        if active.resources.is_some() {
            return Err(RecoveryError::AttemptAlreadySubmitted(id));
        }
        active.resources = Some(resources);
        Ok(())
    }

    pub fn take_resources(&mut self, id: AttemptId) -> Result<AttemptResources, RecoveryError> {
        self.active_mut(id)?
            .resources
            .take()
            .ok_or(RecoveryError::NotReady)
    }

    pub fn record_dispatch(
        &mut self,
        id: AttemptId,
        receipt: EncodingReceipt,
    ) -> Result<(), RecoveryError> {
        if !matches!(receipt.status, ReceiptStatus::Encoded)
            || receipt.command_count == 0
            || receipt.error.is_some()
        {
            return Err(RecoveryError::DispatchNotEncoded);
        }
        self.active_mut(id)?.dispatches.push(receipt);
        Ok(())
    }

    pub fn record_submission(
        &mut self,
        id: AttemptId,
        submission: SubmissionId,
    ) -> Result<(), RecoveryError> {
        let active = self.active_mut(id)?;
        if active.submissions.contains_key(&submission) {
            return Err(RecoveryError::DuplicateSubmission(submission));
        }
        active.submissions.insert(
            submission,
            CompletionReceipt {
                id: submission,
                outcome: CompletionOutcome::Failed("pending".to_owned()),
                retired_resources: 0,
            },
        );
        Ok(())
    }

    pub fn record_completion(
        &mut self,
        id: AttemptId,
        receipt: CompletionReceipt,
    ) -> Result<(), RecoveryError> {
        let active = self.active_mut(id)?;
        let Some(slot) = active.submissions.get_mut(&receipt.id) else {
            return Err(RecoveryError::UnknownSubmission(receipt.id));
        };
        if !matches!(slot.outcome, CompletionOutcome::Failed(ref message) if message == "pending") {
            return Err(RecoveryError::DuplicateSubmission(receipt.id));
        }
        *slot = receipt;
        Ok(())
    }

    pub fn accept_output(
        &mut self,
        id: AttemptId,
        fragment: OutputFragment,
    ) -> Result<(), RecoveryError> {
        self.active_mut(id)?.assembly.accept(fragment)
    }

    pub fn complete_attempt(&mut self, id: AttemptId) -> Result<AssemblyReceipt, RecoveryError> {
        if self.request.cancellation.is_cancelled() {
            return Err(RecoveryError::Cancelled);
        }
        let mut active = self.active.take().ok_or(RecoveryError::NoActiveAttempt)?;
        if active.id != id {
            self.active = Some(active);
            return Err(RecoveryError::WrongAttempt(id));
        }
        if active.dispatches.is_empty() {
            self.active = Some(active);
            return Err(RecoveryError::DispatchNotEncoded);
        }
        if let Some(pending) = active.submissions.iter().find_map(|(submission, receipt)| {
            matches!(&receipt.outcome, CompletionOutcome::Failed(message) if message == "pending")
                .then_some(*submission)
        }) {
            self.active = Some(active);
            return Err(RecoveryError::SubmissionNotComplete(pending));
        }
        let retired_resources = match active.completed_resources() {
            Ok(retired_resources) => retired_resources,
            Err(error) => {
                self.active = Some(active);
                return Err(error);
            }
        };
        let assembly = match active.assembly.finish() {
            Ok(receipt) => receipt,
            Err(error) => {
                self.active = Some(active);
                return Err(error);
            }
        };
        let released_resources = match active.release_resources() {
            Ok(released_resources) => released_resources,
            Err(error) => {
                self.state = SessionState::Rejected;
                return Err(error);
            }
        };
        let attempt = active.receipt(
            AttemptOutcome::Succeeded,
            None,
            retired_resources.saturating_add(released_resources),
        );
        self.receipts.push(attempt.clone());
        self.success = Some(assembly.clone());
        self.state = SessionState::Succeeded;
        Ok(assembly)
    }

    pub fn fail_attempt(
        &mut self,
        id: AttemptId,
        failure: AttemptFailure,
    ) -> Result<RecoveryDecision, RecoveryError> {
        let AttemptFailure { kind, cleanup } = failure;
        let active = self.active.take().ok_or(RecoveryError::NoActiveAttempt)?;
        if active.id != id {
            self.active = Some(active);
            return Err(RecoveryError::WrongAttempt(id));
        }
        if cleanup == CleanupStatus::Uncertain {
            let mut active = active;
            let _ = active.discard_resources();
            self.state = SessionState::Rejected;
            return Err(RecoveryError::CleanupUncertain);
        }
        let retired_resources = active.completed_resources().unwrap_or(0);
        let mut active = active;
        let released = match active.discard_resources() {
            Ok(released) => released,
            Err(error) => {
                self.state = SessionState::Rejected;
                return Err(error);
            }
        };
        let receipt = active.receipt(
            match &kind {
                AttemptFailureKind::Cancelled => AttemptOutcome::Cancelled,
                AttemptFailureKind::Obsolete => AttemptOutcome::Obsolete,
                _ => AttemptOutcome::Failed,
            },
            Some(kind.clone()),
            retired_resources.saturating_add(released),
        );
        self.receipts.push(receipt.clone());
        match &kind {
            AttemptFailureKind::Cancelled => {
                self.state = SessionState::Cancelled;
                Ok(RecoveryDecision::Cancelled { disposed: receipt })
            }
            AttemptFailureKind::Obsolete => {
                self.state = SessionState::Obsolete;
                Ok(RecoveryDecision::Obsolete { disposed: receipt })
            }
            AttemptFailureKind::OutOfMemory
                if self.next_candidate < self.request.candidates.len() =>
            {
                self.state = SessionState::Ready;
                Ok(RecoveryDecision::Retry {
                    disposed: receipt,
                    next: self.request.candidates[self.next_candidate],
                })
            }
            _ if self.request.allow_cpu_fallback => {
                self.state = SessionState::CpuFallback;
                Ok(RecoveryDecision::CpuFallback { disposed: receipt })
            }
            _ => {
                self.state = SessionState::Rejected;
                Err(RecoveryError::CpuFallbackDisabled)
            }
        }
    }

    pub fn cancel(&mut self) -> Result<RecoveryDecision, RecoveryError> {
        self.request.cancellation.cancel();
        if let Some(id) = self.active_attempt() {
            self.fail_attempt(id, AttemptFailure::cancelled())
        } else {
            self.state = SessionState::Cancelled;
            Err(RecoveryError::Cancelled)
        }
    }

    pub fn publish(
        &mut self,
        current: RecoveryContext,
    ) -> Result<PublicationReceipt, RecoveryError> {
        self.validate_publication(current)?;
        let success = self
            .success
            .clone()
            .ok_or(RecoveryError::NothingToPublish)?;
        self.state = SessionState::Published;
        Ok(PublicationReceipt {
            context: current,
            backend: PublicationBackend::Gpu,
            output_identity: success.output_identity,
            coverage: success.coverage,
            attempts: self.receipts.clone(),
        })
    }

    pub fn publish_cpu(
        &mut self,
        current: RecoveryContext,
        output_identity: [u8; 32],
    ) -> Result<PublicationReceipt, RecoveryError> {
        self.validate_publication(current)?;
        if self.state != SessionState::CpuFallback {
            return Err(RecoveryError::NotReady);
        }
        self.state = SessionState::Published;
        Ok(PublicationReceipt {
            context: current,
            backend: PublicationBackend::Cpu,
            output_identity,
            coverage: self.request.assembly.coverage().clone(),
            attempts: self.receipts.clone(),
        })
    }

    fn validate_publication(&mut self, current: RecoveryContext) -> Result<(), RecoveryError> {
        if self.state == SessionState::Published {
            return Err(RecoveryError::AlreadyPublished);
        }
        if self.request.cancellation.is_cancelled() {
            self.state = SessionState::Cancelled;
            return Err(RecoveryError::Cancelled);
        }
        if current != self.request.context {
            return Err(RecoveryError::StaleContext);
        }
        Ok(())
    }

    fn active_mut(&mut self, id: AttemptId) -> Result<&mut ActiveAttempt, RecoveryError> {
        let active = self.active.as_mut().ok_or(RecoveryError::NoActiveAttempt)?;
        if active.id != id {
            return Err(RecoveryError::WrongAttempt(id));
        }
        Ok(active)
    }
}
