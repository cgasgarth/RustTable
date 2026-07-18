#[path = "../benches/support/mod.rs"]
mod support;

use std::path::PathBuf;
use support::cli::{Command, parse};
use support::config::{CASES, ConfigError, parse as parse_config};
use support::stats::{StatsError, median, nearest_rank, normalized_p95};

fn valid() -> String {
    "schema_version\t1\ncase\twarmup_count\tsample_count\twork_units\tcalibration_iterations\tlimit_ppm\nphoto_build_and_iterate_128_assets\t1\t5\t128\t1000\t2000000\nrender_256x256_two_step_pipeline\t1\t5\t65536\t1000\t2000000\n".to_owned()
}

#[test]
fn parses_exact_case_set() {
    let configs = parse_config(&valid()).unwrap();
    assert_eq!(
        configs
            .iter()
            .map(|case| case.name.as_str())
            .collect::<Vec<_>>(),
        CASES
    );
}

#[test]
fn rejects_malformed_configuration_branches() {
    let duplicate = valid().replace(
        "render_256x256_two_step_pipeline",
        "photo_build_and_iterate_128_assets",
    );
    let zero = valid().replace("\t1\t5\t128", "\t0\t5\t128");
    let extra = valid().replace("\t2000000\nrender", "\t2000000\textra\nrender");
    for text in [
        "",
        "schema_version\t2\n",
        "schema_version\t1\nbad\n",
        &valid().replace("photo_build_and_iterate_128_assets", "unknown"),
        &duplicate,
        &zero,
        &extra,
    ] {
        assert!(parse_config(text).is_err());
    }
}

#[test]
fn computes_nearest_rank_and_normalized_statistics() {
    assert_eq!(median(&[5, 1, 3, 2, 4]).unwrap(), 3);
    assert_eq!(nearest_rank(&[5, 1, 3, 2, 4]).unwrap(), 5);
    assert_eq!(
        normalized_p95(&[10, 20, 30, 40, 50], &[10, 10, 10, 10, 10], 1, 10).unwrap(),
        50_000_000
    );
    assert_eq!(
        normalized_p95(&[1], &[0], 1, 1),
        Err(StatsError::ZeroCalibration)
    );
}

#[test]
fn rejects_empty_and_overflow_statistics() {
    assert_eq!(median(&[]), Err(StatsError::Empty));
    assert_eq!(
        normalized_p95(&[u128::MAX], &[1], 1, u64::MAX),
        Err(StatsError::Overflow)
    );
}

#[test]
fn parses_cargo_sentinel_and_user_cli() {
    assert_eq!(
        parse(&["bench".into(), "--bench".into(), "--list".into()]),
        Ok(Command::List(None))
    );
    assert_eq!(
        parse(&[
            "bench".into(),
            "--bench".into(),
            "--check".into(),
            "--config".into(),
            "budgets.tsv".into()
        ]),
        Ok(Command::Check(Some(PathBuf::from("budgets.tsv"))))
    );
    assert!(parse(&["bench".into(), "--list".into()]).is_err());
    assert!(
        parse(&[
            "bench".into(),
            "--bench".into(),
            "--check".into(),
            "--check".into()
        ])
        .is_err()
    );
    assert!(parse(&["bench".into(), "--bench".into(), "--bench".into()]).is_err());
}

#[allow(dead_code)]
fn _keep_error_type_visible(_: ConfigError) {}
