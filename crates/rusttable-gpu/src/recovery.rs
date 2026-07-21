//! Bounded GPU-attempt recovery and deterministic output assembly.
//!
//! The model accepts planner-supplied candidates only, owns one fresh assembly
//! per attempt, permits at most two strictly smaller OOM retries, and exposes a
//! single exact CPU fallback/publication boundary.

#![allow(clippy::missing_errors_doc)]

#[path = "recovery_assembly.rs"]
mod assembly;
#[path = "recovery_model.rs"]
mod model;
#[path = "recovery_session.rs"]
mod session;

pub use assembly::{
    AssemblyPlan, AssemblyReceipt, AssemblyTile, CoverageError, CoverageReceipt, CoverageRect,
    OutputFragment,
};
pub use model::{
    AttemptFailure, AttemptFailureKind, AttemptId, AttemptOutcome, AttemptReceipt,
    AttemptResources, CleanupStatus, MAX_GPU_ATTEMPTS, MAX_OOM_RETRIES, PlanIdentity,
    PublicationBackend, PublicationReceipt, RecoveryAttemptPlan, RecoveryContext, RecoveryDecision,
    RecoveryError, RecoveryRequest, SnapshotIdentity, TileCandidate,
};
pub use session::RecoverySession;

#[cfg(test)]
#[path = "recovery_tests.rs"]
mod gpu_tiling_recovery_tests;
