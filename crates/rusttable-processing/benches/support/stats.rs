use std::fmt;

#[derive(Debug, PartialEq, Eq)]
pub enum StatsError {
    Empty,
    Overflow,
}

impl fmt::Display for StatsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Empty => "no performance samples",
            Self::Overflow => "performance statistic overflow",
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

pub fn p95_within_budget(samples: &[u128], limit_ns: u64) -> Result<bool, StatsError> {
    Ok(nearest_rank(samples)? <= u128::from(limit_ns))
}
