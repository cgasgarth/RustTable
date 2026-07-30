use std::time::Duration;

use rusttable_testkit::bench::{
    BenchmarkRunner, BenchmarkScenario, BuildIdentity, CancellationToken, ComparisonStatus,
    EnvironmentQualification, HostIdentity, MetricSample, MetricValue, ReceiptStatus,
    RunEnvironment, ScenarioState, compare_baseline, initial_scenarios,
};

fn active_scenario() -> BenchmarkScenario {
    BenchmarkScenario {
        id: "test-active-raster-decode".to_owned(),
        state: ScenarioState::Active,
        fixture_ids: vec!["test-fixture".to_owned()],
        fixture_content_sha256: vec![
            "f16d05ec6b29248d2c61adb1e9263f78e4f7bace1b955014a2d17872cfe4064d".to_owned(),
        ],
        width: 4,
        height: 3,
        operations: vec!["decode.raster".to_owned()],
        thread_count: 1,
        memory_cap_bytes: 1024,
        timeout_ms: 1_000,
        expected_output_sha256: Some(
            "8b194f24b630211a098816e7c9ad938858c159ce3376008d568989c68abc42e7".to_owned(),
        ),
        warmup_iterations: 2,
        repetitions: 5,
        requires_gpu: false,
        blocking_issue: None,
    }
}

fn expected_output(scenario: &BenchmarkScenario) -> String {
    scenario
        .expected_output_sha256
        .clone()
        .expect("active scenario expected output")
}

fn sample(sequence: u32, wall_time_ns: u64) -> MetricSample {
    MetricSample {
        sequence,
        wall_time_ns,
        cpu_time_ns: wall_time_ns / 2,
        peak_resident_bytes: 100,
        allocated_bytes: 50,
        allocation_count: 2,
        cache_hits: 3,
        cache_misses: 1,
        decoded_megapixels: 2,
        processed_megapixels: 2,
        preview_latency_ns: Some(wall_time_ns),
        gpu_upload_bytes: MetricValue::Unavailable,
        gpu_download_bytes: MetricValue::Unavailable,
        gpu_dispatch_ns: MetricValue::Unavailable,
    }
}

fn environment(source: &str) -> RunEnvironment {
    RunEnvironment {
        host: HostIdentity::local(),
        build: BuildIdentity::local(source, "release"),
        qualification: EnvironmentQualification::qualified(),
    }
}

fn receipt(source: &str, values: &[u64]) -> rusttable_testkit::bench::BenchmarkReceipt {
    let scenario = active_scenario();
    let output = expected_output(&scenario);
    BenchmarkRunner::default()
        .receipt(
            &scenario,
            environment(source),
            output,
            values
                .iter()
                .enumerate()
                .map(|(index, value)| sample(u32::try_from(index).expect("sequence"), *value))
                .collect(),
            ReceiptStatus::Completed,
        )
        .expect("valid receipt")
}

#[test]
fn registered_scenarios_remain_blocked_until_real_workloads_exist() {
    let scenarios = initial_scenarios();
    assert!(
        !scenarios
            .iter()
            .any(|scenario| scenario.state == ScenarioState::Active)
    );
    assert!(
        scenarios
            .iter()
            .any(|scenario| scenario.state == ScenarioState::Qualification)
    );
    assert!(
        scenarios
            .iter()
            .any(|scenario| scenario.state == ScenarioState::InactivePlaceholder)
    );
    for scenario in scenarios {
        scenario.validate().expect("scenario is governed");
        if scenario.state != ScenarioState::Active {
            assert!(scenario.blocking_issue.is_some());
            assert!(scenario.expected_output_sha256.is_none());
        }
    }
}

#[test]
fn host_and_build_identity_are_independent_and_builds_are_receipted() {
    let baseline = receipt("commit-a", &[100, 110, 120, 130, 140]);
    let current = receipt("commit-b", &[110, 120, 130, 140, 150]);
    assert_eq!(baseline.environment.host, current.environment.host);
    assert_ne!(
        baseline.environment.build.source_commit,
        current.environment.build.source_commit
    );
    let comparison = compare_baseline(&current, &baseline).expect("same host compares");
    assert_eq!(comparison.status, ComparisonStatus::Comparable);
    assert_eq!(comparison.current_build.source_commit, "commit-b");
    assert_eq!(comparison.baseline_build.source_commit, "commit-a");
    assert!(comparison.compared);
    assert!(comparison.latency.expect("latency delta").p95_percent_milli > 0);
}

#[test]
fn output_is_validated_before_timing_and_placeholders_cannot_receipt() {
    let scenario = active_scenario();
    let mut wrong = expected_output(&scenario).into_bytes();
    wrong[0] = if wrong[0] == b'0' { b'1' } else { b'0' };
    assert!(
        BenchmarkRunner::default()
            .receipt(
                &scenario,
                environment("commit"),
                String::from_utf8(wrong).expect("hash utf8"),
                vec![
                    sample(0, 100),
                    sample(1, 100),
                    sample(2, 100),
                    sample(3, 100),
                    sample(4, 100)
                ],
                ReceiptStatus::Completed,
            )
            .is_err()
    );
    let placeholder = initial_scenarios()
        .into_iter()
        .find(|scenario| scenario.state == ScenarioState::InactivePlaceholder)
        .expect("placeholder");
    assert!(matches!(
        BenchmarkRunner::default().receipt(
            &placeholder,
            environment("commit"),
            String::new(),
            Vec::new(),
            ReceiptStatus::Completed,
        ),
        Err(rusttable_testkit::bench::BenchmarkError::ScenarioNotActive(
            _
        ))
    ));

    let mut missing_oracle = active_scenario();
    missing_oracle.expected_output_sha256 = None;
    assert!(missing_oracle.validate().is_err());
}

#[test]
fn receipts_have_deterministic_serialization() {
    let receipt = receipt("commit", &[100, 110, 120, 130, 140]);
    let first = receipt.stable_json().expect("stable receipt");
    let second = receipt.stable_json().expect("stable receipt");
    assert_eq!(first, second);
}

#[test]
fn percentiles_uncertainty_and_overflow_are_deterministic() {
    let receipt = receipt("commit", &[100, 110, 120, 130, 140]);
    assert_eq!(receipt.summary.latency.p50_ns, 120);
    assert_eq!(receipt.summary.latency.p95_ns, 140);
    assert_eq!(receipt.summary.latency.p99_ns, 140);
    assert_eq!(receipt.summary.uncertainty.median_absolute_deviation_ns, 10);
    assert!(receipt.summary.uncertainty.stable);
    assert_eq!(receipt.summary.processed_megapixels, 10);
    assert!(receipt.summary.throughput_pixels_per_second > 0);

    let scenario = active_scenario();
    let overflowing = (0..5).map(|sequence| MetricSample {
        allocated_bytes: u64::MAX,
        ..sample(sequence, 100)
    });
    assert!(matches!(
        BenchmarkRunner::default().receipt(
            &scenario,
            environment("commit"),
            expected_output(&scenario),
            overflowing.collect(),
            ReceiptStatus::Completed,
        ),
        Err(rusttable_testkit::bench::BenchmarkError::Overflow)
    ));
}

#[test]
fn host_mismatch_is_not_comparable_and_duplicate_samples_fail() {
    let baseline = receipt("commit-a", &[100, 100, 100, 100, 100]);
    let mut current_environment = environment("commit-b");
    current_environment.host.cpu_model = "different".to_owned();
    let scenario = active_scenario();
    let current = BenchmarkRunner::default()
        .receipt(
            &scenario,
            current_environment,
            expected_output(&scenario),
            (0..5).map(|index| sample(index, 100)).collect(),
            ReceiptStatus::Completed,
        )
        .expect("current receipt");
    let comparison = compare_baseline(&current, &baseline).expect("typed mismatch");
    assert!(matches!(
        comparison.status,
        ComparisonStatus::NotComparable { .. }
    ));
    assert!(!comparison.compared);

    let duplicate = (0..5).map(|index| sample(index.min(1), 100)).collect();
    assert!(
        BenchmarkRunner::default()
            .receipt(
                &scenario,
                environment("commit"),
                expected_output(&scenario),
                duplicate,
                ReceiptStatus::Completed,
            )
            .is_err()
    );
}

#[test]
fn isolated_process_receipt_reports_owned_cleanup() {
    let token = CancellationToken::new();
    let receipt = BenchmarkRunner::default().run_isolated_command(
        "true",
        &[],
        Duration::from_secs(1),
        &token,
    );
    assert_eq!(
        receipt.expect("isolated command").cleanup,
        "process-tree-owned"
    );
}
