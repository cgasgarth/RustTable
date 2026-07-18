use std::fmt;

#[derive(Debug, PartialEq, Eq)]
pub enum StatsError {
    Empty,
    Overflow,
    ZeroCalibration,
}

impl fmt::Display for StatsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Empty => "no performance samples",
            Self::Overflow => "performance statistic overflow",
            Self::ZeroCalibration => "calibration sample was zero",
        })
    }
}
impl std::error::Error for StatsError {}

pub fn nearest_rank(values: &[u128]) -> Result<u128, StatsError> {
    if values.is_empty() {
        return Err(StatsError::Empty);
    }
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let rank = ((95_u128 * sorted.len() as u128)
        .checked_add(99)
        .ok_or(StatsError::Overflow)?
        / 100) as usize;
    Ok(sorted[rank.checked_sub(1).ok_or(StatsError::Empty)?])
}

pub fn median(values: &[u128]) -> Result<u128, StatsError> {
    if values.is_empty() {
        return Err(StatsError::Empty);
    }
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    Ok(sorted[(sorted.len() - 1) / 2])
}

pub fn normalized_p95(
    workload_ns: &[u128],
    calibration_ns: &[u128],
    work_units: u64,
    calibration_iterations: u64,
) -> Result<u128, StatsError> {
    if workload_ns.is_empty() || workload_ns.len() != calibration_ns.len() {
        return Err(StatsError::Empty);
    }
    if calibration_ns.contains(&0) {
        return Err(StatsError::ZeroCalibration);
    }
    let mut ratios = Vec::with_capacity(workload_ns.len());
    for (&workload, &calibration) in workload_ns.iter().zip(calibration_ns) {
        let numerator = workload
            .checked_mul(u128::from(calibration_iterations))
            .and_then(|value| value.checked_mul(1_000_000))
            .ok_or(StatsError::Overflow)?;
        let denominator = u128::from(work_units)
            .checked_mul(calibration)
            .ok_or(StatsError::Overflow)?;
        ratios.push(numerator / denominator);
    }
    nearest_rank(&ratios)
}
