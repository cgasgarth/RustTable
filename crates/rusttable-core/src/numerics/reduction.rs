use std::collections::BTreeMap;
use std::ops::Range;

use super::NumericalError;

/// Fixed logical partitioning used independently of worker count or completion order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReductionPlan {
    input_len: usize,
    leaf_size: usize,
    ranges: Vec<Range<usize>>,
}

impl ReductionPlan {
    /// Creates fixed contiguous leaves for one logical input.
    ///
    /// # Errors
    ///
    /// Rejects a zero leaf size.
    pub fn new(input_len: usize, leaf_size: usize) -> Result<Self, NumericalError> {
        if leaf_size == 0 {
            return Err(NumericalError::InvalidReductionPlan);
        }
        let ranges = (0..input_len)
            .step_by(leaf_size)
            .map(|start| start..start.saturating_add(leaf_size).min(input_len))
            .collect();
        Ok(Self {
            input_len,
            leaf_size,
            ranges,
        })
    }

    #[must_use]
    pub fn ranges(&self) -> &[Range<usize>] {
        &self.ranges
    }

    #[must_use]
    pub const fn input_len(&self) -> usize {
        self.input_len
    }

    #[must_use]
    pub const fn leaf_size(&self) -> usize {
        self.leaf_size
    }

    #[must_use]
    pub const fn leaf_count(&self) -> usize {
        self.ranges.len()
    }
}

/// Merges indexed leaves with a fixed left-to-right binary tree.
/// Completion order therefore cannot alter floating-point expression order.
///
/// # Errors
///
/// Rejects duplicate, missing, or out-of-range leaf indexes.
pub fn merge_indexed<T, I, F>(
    expected_leaves: usize,
    partials: I,
    identity: T,
    mut merge: F,
) -> Result<T, NumericalError>
where
    I: IntoIterator<Item = (usize, T)>,
    F: FnMut(T, T) -> T,
{
    let mut ordered = BTreeMap::new();
    for (index, partial) in partials {
        if index >= expected_leaves {
            return Err(NumericalError::InvalidReductionPlan);
        }
        if ordered.insert(index, partial).is_some() {
            return Err(NumericalError::DuplicateReductionLeaf);
        }
    }
    if ordered.len() != expected_leaves {
        return Err(NumericalError::MissingReductionLeaf);
    }
    if expected_leaves == 0 {
        return Ok(identity);
    }
    let mut level = ordered.into_values().collect::<Vec<_>>();
    while level.len() > 1 {
        let mut next = Vec::with_capacity(level.len().div_ceil(2));
        let mut values = level.into_iter();
        while let Some(left) = values.next() {
            next.push(match values.next() {
                Some(right) => merge(left, right),
                None => left,
            });
        }
        level = next;
    }
    level.pop().ok_or(NumericalError::MissingReductionLeaf)
}

/// Sums `f32` values using the supplied fixed partition and merge tree.
///
/// # Errors
///
/// Rejects plan mismatch, non-finite input, or non-finite partial/output.
pub fn deterministic_sum_f32(values: &[f32], plan: &ReductionPlan) -> Result<f32, NumericalError> {
    if plan.input_len() != values.len() {
        return Err(NumericalError::InvalidReductionPlan);
    }
    let mut partials = Vec::with_capacity(plan.leaf_count());
    for (index, range) in plan.ranges().iter().enumerate() {
        let mut sum = 0.0_f32;
        for value in &values[range.clone()] {
            if !value.is_finite() {
                return Err(NumericalError::NonFinite);
            }
            sum += value;
        }
        if !sum.is_finite() {
            return Err(NumericalError::ReductionOverflow);
        }
        partials.push((index, sum));
    }
    let result = merge_indexed(plan.leaf_count(), partials, 0.0, |left, right| left + right)?;
    if result.is_finite() {
        Ok(result)
    } else {
        Err(NumericalError::ReductionOverflow)
    }
}

/// Sums `f64` planning/accumulation values with a fixed partition and merge tree.
///
/// # Errors
///
/// Rejects plan mismatch, non-finite input, or non-finite partial/output.
pub fn deterministic_sum_f64(values: &[f64], plan: &ReductionPlan) -> Result<f64, NumericalError> {
    if plan.input_len() != values.len() {
        return Err(NumericalError::InvalidReductionPlan);
    }
    let mut partials = Vec::with_capacity(plan.leaf_count());
    for (index, range) in plan.ranges().iter().enumerate() {
        let mut sum = 0.0_f64;
        for value in &values[range.clone()] {
            if !value.is_finite() {
                return Err(NumericalError::NonFinite);
            }
            sum += value;
        }
        if !sum.is_finite() {
            return Err(NumericalError::ReductionOverflow);
        }
        partials.push((index, sum));
    }
    let result = merge_indexed(plan.leaf_count(), partials, 0.0, |left, right| left + right)?;
    if result.is_finite() {
        Ok(result)
    } else {
        Err(NumericalError::ReductionOverflow)
    }
}

/// Merges indexed integer histograms with the fixed binary tree.
///
/// # Errors
///
/// Rejects leaf/index mismatch, inconsistent bin counts, and counter overflow.
pub fn merge_histograms<I>(
    expected_leaves: usize,
    bin_count: usize,
    partials: I,
) -> Result<Vec<u64>, NumericalError>
where
    I: IntoIterator<Item = (usize, Vec<u64>)>,
{
    let mut checked = Vec::new();
    for (index, bins) in partials {
        if bins.len() != bin_count {
            return Err(NumericalError::InvalidReductionPlan);
        }
        checked.push((index, Ok(bins)));
    }
    merge_indexed(
        expected_leaves,
        checked,
        Ok(vec![0; bin_count]),
        |left: Result<Vec<u64>, NumericalError>, right| {
            let mut left = left?;
            for (target, amount) in left.iter_mut().zip(right?) {
                *target = target
                    .checked_add(amount)
                    .ok_or(NumericalError::ReductionOverflow)?;
            }
            Ok(left)
        },
    )?
}

/// Deterministic raw bivariate moments suitable for covariance inputs.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct BivariateMoments {
    count: u32,
    sum_x: f64,
    sum_y: f64,
    sum_xy: f64,
}

impl BivariateMoments {
    /// Reduces finite sample pairs with one fixed tree.
    ///
    /// # Errors
    ///
    /// Rejects plan mismatch, non-finite input, or overflow.
    pub fn reduce(samples: &[(f64, f64)], plan: &ReductionPlan) -> Result<Self, NumericalError> {
        if plan.input_len() != samples.len() {
            return Err(NumericalError::InvalidReductionPlan);
        }
        if u32::try_from(samples.len()).is_err() {
            return Err(NumericalError::InvalidReductionPlan);
        }
        let mut partials = Vec::with_capacity(plan.leaf_count());
        for (index, range) in plan.ranges().iter().enumerate() {
            let mut moments = Self::default();
            for &(x, y) in &samples[range.clone()] {
                if !x.is_finite() || !y.is_finite() {
                    return Err(NumericalError::NonFinite);
                }
                moments.count += 1;
                moments.sum_x += x;
                moments.sum_y += y;
                moments.sum_xy += x * y;
            }
            if !moments.is_finite() {
                return Err(NumericalError::ReductionOverflow);
            }
            partials.push((index, moments));
        }
        let result = merge_indexed(plan.leaf_count(), partials, Self::default(), Self::merged)?;
        if result.is_finite() {
            Ok(result)
        } else {
            Err(NumericalError::ReductionOverflow)
        }
    }

    #[must_use]
    pub const fn count(self) -> u32 {
        self.count
    }

    /// Returns population covariance for a non-empty sample set.
    #[must_use]
    pub fn covariance_population(self) -> Option<f64> {
        if self.count == 0 {
            return None;
        }
        let count = f64::from(self.count);
        Some((self.sum_xy / count) - (self.sum_x / count) * (self.sum_y / count))
    }

    fn merged(self, right: Self) -> Self {
        Self {
            count: self.count + right.count,
            sum_x: self.sum_x + right.sum_x,
            sum_y: self.sum_y + right.sum_y,
            sum_xy: self.sum_xy + right.sum_xy,
        }
    }

    fn is_finite(self) -> bool {
        self.sum_x.is_finite() && self.sum_y.is_finite() && self.sum_xy.is_finite()
    }
}
