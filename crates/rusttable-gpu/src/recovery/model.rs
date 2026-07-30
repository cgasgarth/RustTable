use std::fmt;

use sha2::{Digest, Sha256};

use super::{AssemblyPlan, CoverageError};
use crate::{
    CancellationToken, CompletionOutcome, DeviceGeneration, PoolError, ResourceId, ResourceLease,
    SubmissionId,
};

pub const MAX_OOM_RETRIES: u8 = 2;
pub const MAX_GPU_ATTEMPTS: u8 = MAX_OOM_RETRIES + 1;

macro_rules! identity_type {
    ($name:ident) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name([u8; 32]);

        impl $name {
            #[must_use]
            pub const fn from_bytes(bytes: [u8; 32]) -> Self {
                Self(bytes)
            }

            #[must_use]
            pub fn from_u64(value: u64) -> Self {
                Self::digest(&value.to_le_bytes())
            }

            #[must_use]
            pub fn digest(bytes: &[u8]) -> Self {
                Self(Sha256::digest(bytes).into())
            }

            #[must_use]
            pub const fn bytes(self) -> [u8; 32] {
                self.0
            }
        }

        impl From<u64> for $name {
            fn from(value: u64) -> Self {
                Self::from_u64(value)
            }
        }
    };
}

identity_type!(SnapshotIdentity);
identity_type!(PlanIdentity);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TileCandidate {
    pub ordinal: u32,
    pub width: u32,
    pub height: u32,
    pub estimated_bytes: u64,
    pub generation: DeviceGeneration,
}

impl TileCandidate {
    pub const fn new(
        ordinal: u32,
        width: u32,
        height: u32,
        estimated_bytes: u64,
        generation: DeviceGeneration,
    ) -> Result<Self, RecoveryError> {
        if ordinal == u32::MAX {
            return Err(RecoveryError::InvalidCandidate("ordinal is reserved"));
        }
        if width == 0 || height == 0 || estimated_bytes == 0 {
            return Err(RecoveryError::InvalidCandidate(
                "candidate dimensions and estimate must be nonzero",
            ));
        }
        Ok(Self {
            ordinal,
            width,
            height,
            estimated_bytes,
            generation,
        })
    }

    #[must_use]
    pub const fn is_strictly_smaller_than(self, previous: Self) -> bool {
        self.width <= previous.width
            && self.height <= previous.height
            && self.estimated_bytes < previous.estimated_bytes
            && (self.width < previous.width || self.height < previous.height)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RecoveryContext {
    pub snapshot: SnapshotIdentity,
    pub plan: PlanIdentity,
    pub generation: DeviceGeneration,
}

impl RecoveryContext {
    #[must_use]
    pub const fn new(
        snapshot: SnapshotIdentity,
        plan: PlanIdentity,
        generation: DeviceGeneration,
    ) -> Self {
        Self {
            snapshot,
            plan,
            generation,
        }
    }
}

#[derive(Debug)]
pub struct AttemptResources {
    leases: Vec<ResourceLease>,
}

impl AttemptResources {
    #[must_use]
    pub fn new(leases: Vec<ResourceLease>) -> Self {
        Self { leases }
    }

    #[must_use]
    pub const fn empty() -> Self {
        Self { leases: Vec::new() }
    }

    #[must_use]
    pub const fn len(&self) -> usize {
        self.leases.len()
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.leases.is_empty()
    }

    #[must_use]
    pub fn ids(&self) -> Vec<ResourceId> {
        self.leases.iter().map(ResourceLease::id).collect()
    }

    pub fn release(self) -> Result<usize, PoolError> {
        self.dispose(ResourceLease::release)
    }

    pub fn discard(self) -> Result<usize, PoolError> {
        self.dispose(ResourceLease::discard)
    }

    fn dispose(
        self,
        dispose_lease: fn(ResourceLease) -> Result<(), PoolError>,
    ) -> Result<usize, PoolError> {
        let mut released = 0;
        let mut first_error = None;
        for lease in self.leases {
            match dispose_lease(lease) {
                Ok(()) => released += 1,
                Err(error) if first_error.is_none() => first_error = Some(error),
                Err(_) => {}
            }
        }
        first_error.map_or(Ok(released), Err)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttemptFailureKind {
    OutOfMemory,
    Cancelled,
    Obsolete,
    DeviceLost,
    Dispatch(String),
    Submission(String),
    Readback(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CleanupStatus {
    Complete,
    Uncertain,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttemptFailure {
    pub kind: AttemptFailureKind,
    pub cleanup: CleanupStatus,
}

impl AttemptFailure {
    #[must_use]
    pub const fn out_of_memory() -> Self {
        Self {
            kind: AttemptFailureKind::OutOfMemory,
            cleanup: CleanupStatus::Complete,
        }
    }

    #[must_use]
    pub const fn cancelled() -> Self {
        Self {
            kind: AttemptFailureKind::Cancelled,
            cleanup: CleanupStatus::Complete,
        }
    }

    #[must_use]
    pub const fn obsolete() -> Self {
        Self {
            kind: AttemptFailureKind::Obsolete,
            cleanup: CleanupStatus::Complete,
        }
    }

    #[must_use]
    pub const fn with_cleanup(mut self, cleanup: CleanupStatus) -> Self {
        self.cleanup = cleanup;
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AttemptId(u32);

impl AttemptId {
    #[must_use]
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn value(self) -> u32 {
        self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttemptReceipt {
    pub id: AttemptId,
    pub number: u8,
    pub candidate: TileCandidate,
    pub outcome: AttemptOutcome,
    pub dispatches: usize,
    pub submissions: usize,
    pub retired_resources: usize,
    pub discarded: bool,
    pub failure: Option<AttemptFailureKind>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttemptOutcome {
    Succeeded,
    Failed,
    Cancelled,
    Obsolete,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecoveryDecision {
    Retry {
        disposed: AttemptReceipt,
        next: TileCandidate,
    },
    CpuFallback {
        disposed: AttemptReceipt,
    },
    Cancelled {
        disposed: AttemptReceipt,
    },
    Obsolete {
        disposed: AttemptReceipt,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PublicationBackend {
    Gpu,
    Cpu,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicationReceipt {
    pub context: RecoveryContext,
    pub backend: PublicationBackend,
    pub output_identity: [u8; 32],
    pub coverage: super::CoverageReceipt,
    pub attempts: Vec<AttemptReceipt>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecoveryError {
    InvalidCandidate(&'static str),
    TooManyCandidates(usize),
    CandidateNotSmaller {
        previous: TileCandidate,
        next: TileCandidate,
    },
    CandidateGeneration {
        expected: DeviceGeneration,
        actual: DeviceGeneration,
    },
    InvalidRequest(&'static str),
    AssemblyBoundsMismatch,
    StaleContext,
    StaleAttempt {
        expected: AttemptId,
        actual: AttemptId,
    },
    UnknownTile(u32),
    DuplicateOutput(u32),
    OutputSizeMismatch {
        tile: u32,
        expected: u64,
        actual: u64,
    },
    Coverage(CoverageError),
    NoActiveAttempt,
    WrongAttempt(AttemptId),
    AttemptAlreadySubmitted(AttemptId),
    DispatchNotEncoded,
    DuplicateSubmission(SubmissionId),
    UnknownSubmission(SubmissionId),
    SubmissionNotComplete(SubmissionId),
    SubmissionFailed(SubmissionId, CompletionOutcome),
    CleanupUncertain,
    CpuFallbackDisabled,
    Cancelled,
    Obsolete,
    NotReady,
    NothingToPublish,
    AlreadyPublished,
    Resource(PoolError),
}

impl fmt::Display for RecoveryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "GPU recovery error: {self:?}")
    }
}

impl std::error::Error for RecoveryError {}

impl From<PoolError> for RecoveryError {
    fn from(error: PoolError) -> Self {
        Self::Resource(error)
    }
}

/// One validated candidate and the fresh assembly layout used by that attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryAttemptPlan {
    pub candidate: TileCandidate,
    pub assembly: AssemblyPlan,
}

impl RecoveryAttemptPlan {
    #[must_use]
    pub const fn new(candidate: TileCandidate, assembly: AssemblyPlan) -> Self {
        Self {
            candidate,
            assembly,
        }
    }
}

#[derive(Debug, Clone)]
pub struct RecoveryRequest {
    pub context: RecoveryContext,
    pub candidates: Vec<TileCandidate>,
    /// Canonical final-output coverage, retained for API compatibility and CPU fallback.
    pub assembly: AssemblyPlan,
    pub cancellation: CancellationToken,
    pub allow_cpu_fallback: bool,
    attempts: Vec<RecoveryAttemptPlan>,
}

impl RecoveryRequest {
    pub fn new(
        context: RecoveryContext,
        candidates: Vec<TileCandidate>,
        assembly: AssemblyPlan,
        cancellation: CancellationToken,
    ) -> Result<Self, RecoveryError> {
        Self::with_cpu_fallback(context, candidates, assembly, cancellation, true)
    }

    pub fn with_cpu_fallback(
        context: RecoveryContext,
        candidates: Vec<TileCandidate>,
        assembly: AssemblyPlan,
        cancellation: CancellationToken,
        allow_cpu_fallback: bool,
    ) -> Result<Self, RecoveryError> {
        let candidate_count = candidates.len();
        let attempts = candidates
            .into_iter()
            .zip(std::iter::repeat_n(assembly, candidate_count))
            .map(|(candidate, assembly)| RecoveryAttemptPlan::new(candidate, assembly))
            .collect();
        Self::from_attempt_plans_with_cpu_fallback(
            context,
            attempts,
            cancellation,
            allow_cpu_fallback,
        )
    }

    pub fn from_attempt_plans(
        context: RecoveryContext,
        attempts: Vec<RecoveryAttemptPlan>,
        cancellation: CancellationToken,
    ) -> Result<Self, RecoveryError> {
        Self::from_attempt_plans_with_cpu_fallback(context, attempts, cancellation, true)
    }

    pub fn from_attempt_plans_with_cpu_fallback(
        context: RecoveryContext,
        attempts: Vec<RecoveryAttemptPlan>,
        cancellation: CancellationToken,
        allow_cpu_fallback: bool,
    ) -> Result<Self, RecoveryError> {
        if attempts.is_empty() {
            return Err(RecoveryError::InvalidRequest(
                "at least one candidate is required",
            ));
        }
        if attempts.len() > usize::from(MAX_GPU_ATTEMPTS) {
            return Err(RecoveryError::TooManyCandidates(attempts.len()));
        }

        let canonical = &attempts[0].assembly;
        if canonical.context() != context {
            return Err(RecoveryError::StaleContext);
        }
        for (index, attempt) in attempts.iter().enumerate() {
            if attempt.assembly.context() != context {
                return Err(RecoveryError::StaleContext);
            }
            if attempt.assembly.width() != canonical.width()
                || attempt.assembly.height() != canonical.height()
            {
                return Err(RecoveryError::AssemblyBoundsMismatch);
            }
            if attempt.candidate.generation != context.generation {
                return Err(RecoveryError::CandidateGeneration {
                    expected: context.generation,
                    actual: attempt.candidate.generation,
                });
            }
            if index > 0
                && !attempt
                    .candidate
                    .is_strictly_smaller_than(attempts[index - 1].candidate)
            {
                return Err(RecoveryError::CandidateNotSmaller {
                    previous: attempts[index - 1].candidate,
                    next: attempt.candidate,
                });
            }
        }

        let candidates = attempts.iter().map(|attempt| attempt.candidate).collect();
        let assembly = canonical.clone();
        Ok(Self {
            context,
            candidates,
            assembly,
            cancellation,
            allow_cpu_fallback,
            attempts,
        })
    }

    #[must_use]
    pub fn attempt_plans(&self) -> &[RecoveryAttemptPlan] {
        &self.attempts
    }

    pub(super) fn attempt_plan(&self, index: usize) -> Option<&RecoveryAttemptPlan> {
        self.attempts.get(index)
    }
}
