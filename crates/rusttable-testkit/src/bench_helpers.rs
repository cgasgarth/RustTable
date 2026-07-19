use std::thread;
use std::time::{Duration, Instant};

use process_wrap::std::ChildWrapper;
use sha2::{Digest, Sha256};

use super::{
    BenchmarkError, BenchmarkSummary, MetricDelta, MetricSample, MetricValue, Percentiles,
    QualificationCheck, QualificationOutcome, Uncertainty,
};

pub(super) fn summarize(samples: &[MetricSample]) -> Result<BenchmarkSummary, BenchmarkError> {
    if samples.is_empty() {
        return Err(BenchmarkError::InvalidReceipt("no samples".to_owned()));
    }
    let wall = samples
        .iter()
        .map(|sample| sample.wall_time_ns)
        .collect::<Vec<_>>();
    let cpu = samples
        .iter()
        .map(|sample| sample.cpu_time_ns)
        .collect::<Vec<_>>();
    let preview = samples
        .iter()
        .filter_map(|sample| sample.preview_latency_ns)
        .collect::<Vec<_>>();
    let total_wall = checked_sum(samples.iter().map(|sample| sample.wall_time_ns))?;
    let processed = checked_sum(samples.iter().map(|sample| sample.processed_megapixels))?;
    let throughput = u64::try_from(
        u128::from(processed)
            .checked_mul(1_000_000)
            .and_then(|value| value.checked_mul(1_000_000_000))
            .ok_or(BenchmarkError::Overflow)?
            / u128::from(total_wall),
    )
    .map_err(|_| BenchmarkError::Overflow)?;
    Ok(BenchmarkSummary {
        samples: u32::try_from(samples.len()).map_err(|_| BenchmarkError::Overflow)?,
        latency: percentiles(&wall),
        cpu_time: percentiles(&cpu),
        preview_latency: (!preview.is_empty()).then(|| percentiles(&preview)),
        uncertainty: uncertainty(&wall)?,
        peak_resident_bytes: samples
            .iter()
            .map(|sample| sample.peak_resident_bytes)
            .max()
            .unwrap_or(0),
        allocated_bytes: checked_sum(samples.iter().map(|sample| sample.allocated_bytes))?,
        allocation_count: checked_sum(samples.iter().map(|sample| sample.allocation_count))?,
        cache_hits: checked_sum(samples.iter().map(|sample| sample.cache_hits))?,
        cache_misses: checked_sum(samples.iter().map(|sample| sample.cache_misses))?,
        decoded_megapixels: checked_sum(samples.iter().map(|sample| sample.decoded_megapixels))?,
        processed_megapixels: processed,
        throughput_pixels_per_second: throughput,
        gpu_upload_bytes: MetricValue::add_all(
            samples.iter().map(|sample| &sample.gpu_upload_bytes),
        )?,
        gpu_download_bytes: MetricValue::add_all(
            samples.iter().map(|sample| &sample.gpu_download_bytes),
        )?,
        gpu_dispatch_ns: MetricValue::add_all(
            samples.iter().map(|sample| &sample.gpu_dispatch_ns),
        )?,
    })
}

fn percentiles(values: &[u64]) -> Percentiles {
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    Percentiles {
        p50_ns: percentile(&sorted, 50),
        p95_ns: percentile(&sorted, 95),
        p99_ns: percentile(&sorted, 99),
    }
}

fn uncertainty(values: &[u64]) -> Result<Uncertainty, BenchmarkError> {
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let median = percentile(&sorted, 50);
    let mut deviations = sorted
        .iter()
        .map(|value| value.abs_diff(median))
        .collect::<Vec<_>>();
    deviations.sort_unstable();
    let mad = percentile(&deviations, 50);
    let noise = if median == 0 {
        0
    } else {
        u64::try_from(
            u128::from(mad)
                .checked_mul(100_000)
                .ok_or(BenchmarkError::Overflow)?
                / u128::from(median),
        )
        .map_err(|_| BenchmarkError::Overflow)?
    };
    Ok(Uncertainty {
        minimum_ns: *sorted.first().unwrap_or(&0),
        maximum_ns: *sorted.last().unwrap_or(&0),
        median_absolute_deviation_ns: mad,
        noise_percent_milli: noise,
        stable: noise <= 20_000,
    })
}

fn percentile(values: &[u64], percentile: usize) -> u64 {
    if values.is_empty() {
        return 0;
    }
    let index = ((values.len() - 1) * percentile).div_ceil(100);
    values[index.min(values.len() - 1)]
}

pub(super) fn metric_delta(
    current: &Percentiles,
    baseline: &Percentiles,
) -> Result<MetricDelta, BenchmarkError> {
    Ok(MetricDelta {
        p50_ns: delta(current.p50_ns, baseline.p50_ns),
        p95_ns: delta(current.p95_ns, baseline.p95_ns),
        p99_ns: delta(current.p99_ns, baseline.p99_ns),
        p50_percent_milli: percent_delta(current.p50_ns, baseline.p50_ns)?,
        p95_percent_milli: percent_delta(current.p95_ns, baseline.p95_ns)?,
        p99_percent_milli: percent_delta(current.p99_ns, baseline.p99_ns)?,
    })
}

pub(super) fn metric_value_delta(current: &MetricValue, baseline: &MetricValue) -> Option<i128> {
    match (current, baseline) {
        (MetricValue::Measured(current), MetricValue::Measured(baseline)) => {
            Some(delta(*current, *baseline))
        }
        _ => None,
    }
}

fn percent_delta(current: u64, baseline: u64) -> Result<i64, BenchmarkError> {
    if baseline == 0 {
        return Ok(0);
    }
    i64::try_from(
        delta(current, baseline)
            .checked_mul(100_000)
            .ok_or(BenchmarkError::Overflow)?
            / i128::from(baseline),
    )
    .map_err(|_| BenchmarkError::Overflow)
}

pub(super) fn delta(current: u64, baseline: u64) -> i128 {
    i128::from(current) - i128::from(baseline)
}

fn checked_sum<I>(values: I) -> Result<u64, BenchmarkError>
where
    I: IntoIterator<Item = u64>,
{
    values.into_iter().try_fold(0_u64, |total, value| {
        total.checked_add(value).ok_or(BenchmarkError::Overflow)
    })
}

pub(super) fn validate_hash(value: &str, label: &str) -> Result<(), BenchmarkError> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(BenchmarkError::InvalidReceipt(format!(
            "{label} is not a SHA-256"
        )));
    }
    Ok(())
}

pub(super) fn sha256(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

pub(super) fn fixture_hash(id: &str) -> String {
    match id {
        "corpus.raster.png.16-alpha" => {
            "0aefb09ec3e28fc3bcb60470bad9c22322e4e4ada1fd0f34d32e6894b0615557".to_owned()
        }
        "corpus.compat.library-schema" => {
            "c13273f503793f445597291745427b2b10051af9dc397b8942c9257e59b25813".to_owned()
        }
        "corpus.raw.bayer.12-2row" => {
            "a6421b2747ca2e5fba035a8e0f9a111b0d2fa1f72873432f8d17ca5d6fb85ae7".to_owned()
        }
        _ => sha256(id.as_bytes()),
    }
}

pub(super) fn env_or(name: &str, fallback: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| fallback.to_owned())
}

pub(super) fn env_optional(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
}

pub(super) fn environment_check(id: &str, variable: &str) -> QualificationCheck {
    match env_optional(variable) {
        Some(value) => QualificationCheck {
            id: id.to_owned(),
            outcome: QualificationOutcome::Passed,
            detail: format!("{variable}={value}"),
        },
        None => QualificationCheck {
            id: id.to_owned(),
            outcome: QualificationOutcome::Unavailable,
            detail: format!("{variable} is unavailable"),
        },
    }
}

pub(super) fn csv_env(name: &str) -> Vec<String> {
    env_optional(name).map_or_else(Vec::new, |value| {
        value
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .collect()
    })
}

pub(super) fn env_u32(name: &str) -> Option<u32> {
    std::env::var(name).ok()?.parse().ok()
}

pub(super) fn env_u64(name: &str) -> Option<u64> {
    std::env::var(name).ok()?.parse().ok()
}

pub(super) fn env_bool(name: &str) -> bool {
    matches!(std::env::var(name).as_deref(), Ok("1" | "true" | "yes"))
}

pub(super) fn terminate_process_tree(child: &mut dyn ChildWrapper) {
    #[cfg(unix)]
    let _ = child.signal(15);
    let deadline = Instant::now() + Duration::from_secs(1);
    while Instant::now() < deadline {
        if child.try_wait().ok().flatten().is_some() {
            return;
        }
        thread::sleep(Duration::from_millis(10));
    }
    let _ = child.start_kill();
    let _ = child.wait();
}
