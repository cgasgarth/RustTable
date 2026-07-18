use std::time::Duration;

use rusttable_testkit::bench::{
    BenchmarkRunner, CancellationToken, HostFingerprint, MetricSample, ReceiptStatus,
    compare_baseline, initial_scenarios,
};

fn sample(wall_time_ns: u64) -> MetricSample {
    MetricSample {
        wall_time_ns,
        cpu_time_ns: wall_time_ns / 2,
        peak_resident_bytes: 100,
        allocated_bytes: 50,
        allocation_count: 2,
        cache_hits: 3,
        cache_misses: 1,
        decoded_megapixels: 1,
        processed_megapixels: 1,
        preview_latency_ns: Some(wall_time_ns),
        gpu_upload_bytes: 0,
        gpu_download_bytes: 0,
        gpu_dispatch_ns: None,
    }
}

#[test]
fn scenarios_have_stable_identity_and_receipts_compare() {
    let scenario = initial_scenarios().into_iter().next().expect("scenario");
    let host = HostFingerprint::local("source", "debug");
    let runner = BenchmarkRunner::default();
    let baseline = runner
        .receipt(
            &scenario,
            host.clone(),
            "a".repeat(64),
            vec![sample(100), sample(120), sample(110)],
            ReceiptStatus::Completed,
        )
        .expect("baseline");
    let current = runner
        .receipt(
            &scenario,
            host,
            "a".repeat(64),
            vec![sample(110), sample(130), sample(120)],
            ReceiptStatus::Completed,
        )
        .expect("current");
    let comparison = compare_baseline(&current, &baseline).expect("same host comparison");
    assert!(comparison.compared);
    assert_eq!(scenario.workload_identity(), scenario.workload_identity());
    assert!(
        current
            .stable_json()
            .expect("receipt JSON")
            .contains("scenario_id")
    );
}

#[test]
fn host_mismatch_and_incomplete_runs_fail_closed() {
    let scenario = initial_scenarios().into_iter().next().expect("scenario");
    let runner = BenchmarkRunner::default();
    let host = HostFingerprint::local("source", "debug");
    assert!(
        runner
            .receipt(
                &scenario,
                host.clone(),
                "a".repeat(64),
                vec![sample(100)],
                ReceiptStatus::TimedOut
            )
            .is_err()
    );
    let baseline = runner
        .receipt(
            &scenario,
            host,
            "a".repeat(64),
            vec![sample(100), sample(100), sample(100)],
            ReceiptStatus::Completed,
        )
        .expect("baseline");
    let mut other_host = HostFingerprint::local("source", "debug");
    other_host.cpu_model = "different".to_owned();
    let current = runner
        .receipt(
            &scenario,
            other_host,
            "a".repeat(64),
            vec![sample(100), sample(100), sample(100)],
            ReceiptStatus::Completed,
        )
        .expect("current");
    assert!(compare_baseline(&current, &baseline).is_err());
}

#[test]
fn isolated_process_is_cancelled_or_completes_within_bound() {
    let token = CancellationToken::new();
    let receipt = BenchmarkRunner::default().run_isolated_command(
        "true",
        &[],
        Duration::from_secs(1),
        &token,
    );
    assert!(receipt.is_ok());
}
