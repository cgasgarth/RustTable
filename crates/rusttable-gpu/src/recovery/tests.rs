use super::*;
use crate::{
    CancellationToken, CompletionOutcome, CompletionReceipt, DeviceGeneration, EncodingReceipt,
    GpuResourcePool, InitializationPolicy, ReceiptStatus, ResourceClass, ResourcePoolConfig,
    ResourceRequest, SubmissionId,
};

fn context(snapshot: u64, plan: u64, generation: u64) -> RecoveryContext {
    RecoveryContext::new(
        SnapshotIdentity::from_u64(snapshot),
        PlanIdentity::from_u64(plan),
        DeviceGeneration::new(generation),
    )
}

fn assembly(context: RecoveryContext) -> AssemblyPlan {
    let tiles = [
        (0, 0, 0, 4, 2, 32),
        (1, 4, 0, 4, 2, 32),
        (2, 0, 2, 4, 2, 32),
        (3, 4, 2, 4, 2, 32),
    ]
    .into_iter()
    .map(|(id, x, y, width, height, bytes)| {
        AssemblyTile::new(
            id,
            CoverageRect::new(x, y, width, height).expect("rectangle"),
            bytes,
        )
        .expect("assembly tile")
    })
    .collect();
    AssemblyPlan::new(context, 8, 4, tiles).expect("exact assembly")
}

fn candidate(ordinal: u32, width: u32, height: u32, bytes: u64) -> TileCandidate {
    TileCandidate::new(ordinal, width, height, bytes, DeviceGeneration::new(7)).expect("candidate")
}

fn request(candidates: Vec<TileCandidate>) -> RecoveryRequest {
    let context = context(1, 2, 7);
    RecoveryRequest::new(
        context,
        candidates,
        assembly(context),
        CancellationToken::new(),
    )
    .expect("recovery request")
}

fn encoded(identity: u8) -> EncodingReceipt {
    EncodingReceipt {
        identity: [identity; 32],
        status: ReceiptStatus::Encoded,
        command_count: 1,
        submitted: true,
        error: None,
    }
}

fn complete_outputs(session: &mut RecoverySession, attempt: AttemptId) {
    let tiles = session
        .active_assembly()
        .expect("active assembly")
        .tiles()
        .to_vec();
    for tile in tiles {
        session
            .accept_output(
                attempt,
                OutputFragment::new(
                    attempt,
                    session.context(),
                    tile.id,
                    tile.coverage,
                    tile.output_bytes,
                    [u8::try_from(tile.id + 1).expect("small tile identity"); 32],
                ),
            )
            .expect("output fragment");
    }
}

fn pool() -> GpuResourcePool {
    GpuResourcePool::new(
        DeviceGeneration::new(7),
        ResourcePoolConfig {
            configured_cap: 4096,
            memory_hint: None,
            addressable_limit: u64::MAX,
            backend_overhead: 0,
            soft_percent: 100,
        },
    )
}

fn resource(pool: &GpuResourcePool) -> crate::ResourceLease {
    pool.try_acquire(ResourceRequest::new(
        ResourceClass::buffer(DeviceGeneration::new(7), 64, 0),
        InitializationPolicy::Zeroed,
    ))
    .expect("resource")
}

#[test]
fn request_rejects_more_than_two_smaller_retries() {
    let error = RecoveryRequest::new(
        context(1, 2, 7),
        vec![
            candidate(0, 8, 8, 800),
            candidate(1, 6, 6, 600),
            candidate(2, 4, 4, 400),
            candidate(3, 2, 2, 200),
        ],
        assembly(context(1, 2, 7)),
        CancellationToken::new(),
    )
    .expect_err("finite retry bound");
    assert_eq!(error, RecoveryError::TooManyCandidates(4));
}

#[test]
fn oom_retries_twice_then_requires_cpu_fallback() {
    let mut session = RecoverySession::new(request(vec![
        candidate(0, 8, 8, 800),
        candidate(1, 6, 6, 600),
        candidate(2, 4, 4, 400),
    ]));

    let first = session.begin_attempt().expect("first attempt");
    let retry = session
        .fail_attempt(first, AttemptFailure::out_of_memory())
        .expect("first retry");
    assert!(matches!(retry, RecoveryDecision::Retry { next, .. } if next.ordinal == 1));

    let second = session.begin_attempt().expect("second attempt");
    let retry = session
        .fail_attempt(second, AttemptFailure::out_of_memory())
        .expect("second retry");
    assert!(matches!(retry, RecoveryDecision::Retry { next, .. } if next.ordinal == 2));

    let third = session.begin_attempt().expect("third attempt");
    let fallback = session
        .fail_attempt(third, AttemptFailure::out_of_memory())
        .expect("CPU fallback");
    assert!(matches!(fallback, RecoveryDecision::CpuFallback { .. }));
    assert_eq!(session.attempts().len(), 3);
    assert!(session.attempts().iter().all(|receipt| receipt.discarded));
}

#[test]
fn failed_attempt_releases_unsubmitted_resources_and_partial_output() {
    let pool = pool();
    let mut session = RecoverySession::new(request(vec![
        candidate(0, 8, 8, 800),
        candidate(1, 4, 4, 400),
    ]));
    let attempt = session.begin_attempt().expect("attempt");
    session
        .attach_resources(attempt, AttemptResources::new(vec![resource(&pool)]))
        .expect("attach");
    let tile = assembly(session.context()).tiles()[0];
    session
        .accept_output(
            attempt,
            OutputFragment::new(
                attempt,
                session.context(),
                tile.id,
                tile.coverage,
                tile.output_bytes,
                [1; 32],
            ),
        )
        .expect("partial output");
    session
        .fail_attempt(attempt, AttemptFailure::out_of_memory())
        .expect("discard");
    assert_eq!(pool.metrics().accounting.leases, 0);
    assert_eq!(pool.metrics().accounting.resident_bytes, 0);
}

#[test]
fn cleanup_uncertainty_forbids_retry() {
    let pool = pool();
    let mut session = RecoverySession::new(request(vec![
        candidate(0, 8, 8, 800),
        candidate(1, 4, 4, 400),
    ]));
    let attempt = session.begin_attempt().expect("attempt");
    session
        .attach_resources(attempt, AttemptResources::new(vec![resource(&pool)]))
        .expect("attach");
    assert_eq!(
        session.fail_attempt(
            attempt,
            AttemptFailure::out_of_memory().with_cleanup(CleanupStatus::Uncertain),
        ),
        Err(RecoveryError::CleanupUncertain)
    );
    assert_eq!(session.begin_attempt(), Err(RecoveryError::NotReady));
    assert_eq!(pool.metrics().accounting.leases, 0);
    assert_eq!(pool.metrics().accounting.resident_bytes, 0);
}

#[test]
fn completed_gpu_attempt_publishes_only_after_exact_coverage() {
    let mut session = RecoverySession::new(request(vec![candidate(0, 8, 8, 800)]));
    let attempt = session.begin_attempt().expect("attempt");
    session
        .record_dispatch(attempt, encoded(4))
        .expect("dispatch");
    complete_outputs(&mut session, attempt);
    let assembly = session.complete_attempt(attempt).expect("assembly");
    assert_eq!(assembly.fragment_count, 4);
    let publication = session.publish(session.context()).expect("publish");
    assert_eq!(publication.backend, PublicationBackend::Gpu);
    assert_eq!(publication.coverage, assembly.coverage);
    assert_eq!(
        session.publish(session.context()),
        Err(RecoveryError::AlreadyPublished)
    );
}

#[test]
fn gap_and_overlap_are_rejected_before_execution() {
    let context = context(1, 2, 7);
    let gap = AssemblyPlan::new(
        context,
        8,
        2,
        vec![
            AssemblyTile::new(0, CoverageRect::new(0, 0, 3, 2).expect("rect"), 8).expect("tile"),
            AssemblyTile::new(1, CoverageRect::new(4, 0, 4, 2).expect("rect"), 8).expect("tile"),
        ],
    )
    .expect_err("gap");
    assert!(matches!(gap, CoverageError::Gap { x: 3, y: 0 }));

    let overlap = AssemblyPlan::new(
        context,
        8,
        2,
        vec![
            AssemblyTile::new(0, CoverageRect::new(0, 0, 5, 2).expect("rect"), 8).expect("tile"),
            AssemblyTile::new(1, CoverageRect::new(4, 0, 4, 2).expect("rect"), 8).expect("tile"),
        ],
    )
    .expect_err("overlap");
    assert!(matches!(overlap, CoverageError::Overlap { x: 4, y: 0 }));
}

#[test]
fn stale_and_obsolete_outputs_cannot_publish() {
    let mut session = RecoverySession::new(request(vec![candidate(0, 8, 8, 800)]));
    let attempt = session.begin_attempt().expect("attempt");
    let tile = assembly(session.context()).tiles()[0];
    let stale = RecoveryContext::new(
        SnapshotIdentity::from_u64(99),
        session.context().plan,
        session.context().generation,
    );
    assert_eq!(
        session.accept_output(
            attempt,
            OutputFragment::new(
                attempt,
                stale,
                tile.id,
                tile.coverage,
                tile.output_bytes,
                [1; 32]
            ),
        ),
        Err(RecoveryError::StaleContext)
    );
    let decision = session
        .fail_attempt(attempt, AttemptFailure::obsolete())
        .expect("obsolete");
    assert!(matches!(decision, RecoveryDecision::Obsolete { .. }));
    assert_eq!(
        session.publish(session.context()),
        Err(RecoveryError::NothingToPublish)
    );
}

#[test]
fn cancelled_gpu_work_does_not_cpu_publish() {
    let request = request(vec![candidate(0, 8, 8, 800)]);
    let token = request.cancellation.clone();
    let mut session = RecoverySession::new(request);
    let attempt = session.begin_attempt().expect("attempt");
    token.cancel();
    assert_eq!(
        session.complete_attempt(attempt),
        Err(RecoveryError::Cancelled)
    );
    let decision = session
        .fail_attempt(attempt, AttemptFailure::cancelled())
        .expect("cancel");
    assert!(matches!(decision, RecoveryDecision::Cancelled { .. }));
    assert_eq!(
        session.publish_cpu(session.context(), [3; 32]),
        Err(RecoveryError::Cancelled)
    );
}

#[test]
fn retry_uses_a_fresh_candidate_specific_assembly() {
    let context = context(1, 2, 7);
    let full = AssemblyPlan::new(
        context,
        8,
        4,
        vec![
            AssemblyTile::new(0, CoverageRect::new(0, 0, 8, 4).expect("rectangle"), 128)
                .expect("tile"),
        ],
    )
    .expect("full assembly");
    let tiled = assembly(context);
    let request = RecoveryRequest::from_attempt_plans(
        context,
        vec![
            RecoveryAttemptPlan::new(candidate(0, 8, 4, 800), full),
            RecoveryAttemptPlan::new(candidate(1, 4, 2, 400), tiled),
        ],
        CancellationToken::new(),
    )
    .expect("candidate-specific request");
    let mut session = RecoverySession::new(request);

    let first = session.begin_attempt().expect("full attempt");
    assert_eq!(
        session.active_assembly().expect("assembly").tiles().len(),
        1
    );
    session
        .fail_attempt(first, AttemptFailure::out_of_memory())
        .expect("retry");

    let second = session.begin_attempt().expect("tiled attempt");
    assert_eq!(
        session.active_assembly().expect("assembly").tiles().len(),
        4
    );
    session
        .record_dispatch(second, encoded(9))
        .expect("dispatch");
    complete_outputs(&mut session, second);
    assert_eq!(
        session
            .complete_attempt(second)
            .expect("complete")
            .fragment_count,
        4
    );
}

#[test]
fn incomplete_assembly_keeps_previous_fragments_for_completion() {
    let mut session = RecoverySession::new(request(vec![candidate(0, 8, 8, 800)]));
    let attempt = session.begin_attempt().expect("attempt");
    session
        .record_dispatch(attempt, encoded(10))
        .expect("dispatch");
    let tiles = session
        .active_assembly()
        .expect("assembly")
        .tiles()
        .to_vec();
    let first = tiles[0];
    session
        .accept_output(
            attempt,
            OutputFragment::new(
                attempt,
                session.context(),
                first.id,
                first.coverage,
                first.output_bytes,
                [1; 32],
            ),
        )
        .expect("first fragment");
    assert_eq!(
        session.complete_attempt(attempt),
        Err(RecoveryError::Coverage(CoverageError::Incomplete {
            expected: 4,
            actual: 1,
        }))
    );

    for tile in &tiles[1..] {
        session
            .accept_output(
                attempt,
                OutputFragment::new(
                    attempt,
                    session.context(),
                    tile.id,
                    tile.coverage,
                    tile.output_bytes,
                    [u8::try_from(tile.id + 1).expect("small tile identity"); 32],
                ),
            )
            .expect("remaining fragment");
    }
    assert_eq!(
        session
            .complete_attempt(attempt)
            .expect("complete")
            .fragment_count,
        4
    );
}

#[test]
fn non_oom_failure_falls_back_without_consuming_smaller_candidate() {
    let mut session = RecoverySession::new(request(vec![
        candidate(0, 8, 8, 800),
        candidate(1, 4, 4, 400),
    ]));
    let attempt = session.begin_attempt().expect("attempt");
    let decision = session
        .fail_attempt(
            attempt,
            AttemptFailure {
                kind: AttemptFailureKind::DeviceLost,
                cleanup: CleanupStatus::Complete,
            },
        )
        .expect("CPU fallback");
    assert!(matches!(decision, RecoveryDecision::CpuFallback { .. }));
    assert_eq!(session.begin_attempt(), Err(RecoveryError::NotReady));
    assert_eq!(session.attempts().len(), 1);
}

#[test]
fn disabled_cpu_fallback_is_terminal() {
    let context = context(1, 2, 7);
    let request = RecoveryRequest::with_cpu_fallback(
        context,
        vec![candidate(0, 8, 8, 800)],
        assembly(context),
        CancellationToken::new(),
        false,
    )
    .expect("request");
    let mut session = RecoverySession::new(request);
    let attempt = session.begin_attempt().expect("attempt");
    assert_eq!(
        session.fail_attempt(attempt, AttemptFailure::out_of_memory()),
        Err(RecoveryError::CpuFallbackDisabled)
    );
    assert_eq!(session.begin_attempt(), Err(RecoveryError::NotReady));
}

#[test]
fn cancellation_after_fallback_blocks_cpu_publication() {
    let request = request(vec![candidate(0, 8, 8, 800)]);
    let token = request.cancellation.clone();
    let mut session = RecoverySession::new(request);
    let attempt = session.begin_attempt().expect("attempt");
    session
        .fail_attempt(attempt, AttemptFailure::out_of_memory())
        .expect("fallback");
    token.cancel();
    assert_eq!(
        session.publish_cpu(session.context(), [7; 32]),
        Err(RecoveryError::Cancelled)
    );
}

#[test]
fn submission_receipts_gate_completion() {
    let mut session = RecoverySession::new(request(vec![candidate(0, 8, 8, 800)]));
    let attempt = session.begin_attempt().expect("attempt");
    session
        .record_dispatch(attempt, encoded(8))
        .expect("dispatch");
    let submission = SubmissionId(41);
    session
        .record_submission(attempt, submission)
        .expect("submit");
    assert_eq!(
        session.complete_attempt(attempt),
        Err(RecoveryError::SubmissionNotComplete(submission))
    );
    session
        .record_completion(
            attempt,
            CompletionReceipt {
                id: submission,
                outcome: CompletionOutcome::Obsolete,
                retired_resources: 2,
            },
        )
        .expect("obsolete completion");
    assert_eq!(
        session.complete_attempt(attempt),
        Err(RecoveryError::SubmissionFailed(
            submission,
            CompletionOutcome::Obsolete,
        ))
    );
}
