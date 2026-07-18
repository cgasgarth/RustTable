#![forbid(unsafe_code)]

use std::hint::black_box;
use std::path::PathBuf;
use std::time::Instant;

mod support;
use support::cases::{consume_photo, consume_render, prepared_photo_assets, prepared_render};
use support::cli::{Command, parse};
use support::config::{CaseConfig, read};
use support::stats::{median, nearest_rank, normalized_p95};

fn main() {
    let raw_arguments = std::env::args().collect::<Vec<_>>();
    let sentinel_count = raw_arguments
        .iter()
        .skip(1)
        .filter(|argument| argument.as_str() == "--bench")
        .count();
    let mut arguments = vec![raw_arguments[0].clone(), "--bench".to_owned()];
    arguments.extend(
        raw_arguments
            .into_iter()
            .skip(1)
            .filter(|argument| argument != "--bench"),
    );
    if sentinel_count > 1 {
        arguments.insert(2, "--bench".to_owned());
    }
    let command = parse(&arguments).unwrap_or_else(|error| panic!("{error}"));
    let default_config =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../performance/budgets.tsv");
    let (list, path) = match command {
        Command::List(path) => (true, path.unwrap_or(default_config)),
        Command::Check(path) => (false, path.unwrap_or(default_config)),
    };
    let configs = read(&path).unwrap_or_else(|error| panic!("{error}"));
    if list {
        for config in configs {
            println!("{}", config.name);
        }
        return;
    }
    let mut failed = false;
    for config in configs {
        let result = run_case(&config);
        println!("{result}");
        failed |= !result.ends_with("status=pass");
    }
    println!(
        "RUSTTABLE_PERF_V1 summary status={}",
        if failed { "fail" } else { "pass" }
    );
    if failed {
        std::process::exit(1);
    }
}

fn run_case(config: &CaseConfig) -> String {
    let mut workload = Vec::with_capacity(config.sample_count as usize);
    let mut calibration = Vec::with_capacity(config.sample_count as usize);
    match config.name.as_str() {
        "photo_build_and_iterate_128_assets" => {
            let assets = prepared_photo_assets();
            for _ in 0..config.warmup_count {
                black_box(consume_photo(&assets));
            }
            for _ in 0..config.sample_count {
                workload.push(measure(|| {
                    black_box(consume_photo(&assets));
                }));
                calibration.push(measure_calibration(config.calibration_iterations));
            }
        }
        "render_256x256_two_step_pipeline" => {
            let (image, pipeline) = prepared_render();
            for _ in 0..config.warmup_count {
                black_box(consume_render(&image, &pipeline));
            }
            for _ in 0..config.sample_count {
                workload.push(measure(|| {
                    black_box(consume_render(&image, &pipeline));
                }));
                calibration.push(measure_calibration(config.calibration_iterations));
            }
        }
        _ => unreachable!("validated case set"),
    }
    let normalized = normalized_p95(
        &workload,
        &calibration,
        config.work_units,
        config.calibration_iterations,
    )
    .unwrap();
    format!(
        "RUSTTABLE_PERF_V1 case={} samples={} work_units={} raw_median_ns={} raw_p95_ns={} calibration_p95_ns={} normalized_p95_ppm={} limit_ppm={} status={}",
        config.name,
        config.sample_count,
        config.work_units,
        median(&workload).unwrap(),
        nearest_rank(&workload).unwrap(),
        nearest_rank(&calibration).unwrap(),
        normalized,
        config.limit_ppm,
        if normalized <= u128::from(config.limit_ppm) {
            "pass"
        } else {
            "fail"
        }
    )
}

fn measure(work: impl FnOnce()) -> u128 {
    let start = Instant::now();
    work();
    start.elapsed().as_nanos()
}
fn measure_calibration(iterations: u64) -> u128 {
    measure(|| {
        let mut value = 0_u64;
        for index in 0..iterations {
            value = value.wrapping_mul(31).wrapping_add(index);
        }
        black_box(value);
    })
}
