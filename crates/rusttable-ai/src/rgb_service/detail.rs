use sha2::{Digest, Sha256};

pub const DETAIL_RECOVERY_VERSION: u32 = 1;
pub const DETAIL_BAND_MULTIPLIERS: [f32; 5] = [0.25, 0.15, 0.05, 0.02, 0.01];

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DetailRecoverySettings {
    alpha: f32,
}

impl DetailRecoverySettings {
    pub fn new(alpha: f32) -> Result<Self, DetailError> {
        if !alpha.is_finite() || !(0.0..=1.0).contains(&alpha) {
            return Err(DetailError::InvalidAlpha);
        }
        Ok(Self { alpha })
    }
    pub fn from_model_strength(strength: u8) -> Result<Self, DetailError> {
        if strength > 100 {
            return Err(DetailError::InvalidAlpha);
        }
        Self::new(f32::from(100 - strength) / 100.0)
    }
    #[must_use]
    pub const fn alpha(self) -> f32 {
        self.alpha
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DetailRecoveryPlan {
    version: u32,
    alpha: f32,
    multipliers: [f32; 5],
}

impl DetailRecoveryPlan {
    pub fn new(alpha: f32) -> Result<Self, DetailError> {
        DetailRecoverySettings::new(alpha).map(Self::from_settings)
    }

    #[must_use]
    pub const fn from_settings(settings: DetailRecoverySettings) -> Self {
        Self {
            version: DETAIL_RECOVERY_VERSION,
            alpha: settings.alpha,
            multipliers: DETAIL_BAND_MULTIPLIERS,
        }
    }
    #[must_use]
    pub const fn version(self) -> u32 {
        self.version
    }
    #[must_use]
    pub const fn alpha(self) -> f32 {
        self.alpha
    }
    #[must_use]
    pub const fn multipliers(self) -> [f32; 5] {
        self.multipliers
    }
    #[must_use]
    pub const fn enabled(self) -> bool {
        self.alpha > 0.0
    }

    #[must_use]
    pub fn identity(self) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(b"rusttable-ai-detail-plan-v1");
        hasher.update(self.version.to_le_bytes());
        hasher.update(self.alpha.to_bits().to_le_bytes());
        for multiplier in self.multipliers {
            hasher.update(multiplier.to_bits().to_le_bytes());
        }
        hasher.finalize().into()
    }

    pub fn recover(
        self,
        original: &[[f32; 4]],
        output: &mut [[f32; 4]],
        width: u32,
        height: u32,
        cancellation: &crate::CancellationToken,
    ) -> Result<(Vec<f32>, DetailReceipt), DetailError> {
        recover_detail(original, output, width, height, self, || {
            cancellation.is_cancelled()
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DetailReceipt {
    pub version: u32,
    pub alpha: f32,
    pub mean: f64,
    pub variance: f64,
    pub sigma: f64,
    pub thresholds: [f64; 5],
    pub residual_hash: [u8; 32],
}

pub(crate) fn recover_detail(
    original: &[[f32; 4]],
    output: &mut [[f32; 4]],
    width: u32,
    height: u32,
    plan: DetailRecoveryPlan,
    cancelled: impl Fn() -> bool,
) -> Result<(Vec<f32>, DetailReceipt), DetailError> {
    if original.len() != output.len() {
        return Err(DetailError::PixelCountMismatch);
    }
    let width = usize::try_from(width).map_err(|_| DetailError::ArithmeticOverflow)?;
    let height = usize::try_from(height).map_err(|_| DetailError::ArithmeticOverflow)?;
    if width.checked_mul(height) != Some(original.len()) {
        return Err(DetailError::PixelCountMismatch);
    }
    let mut residual = Vec::with_capacity(original.len());
    for (source, denoised) in original.iter().zip(output.iter()) {
        if cancelled() {
            return Err(DetailError::Cancelled);
        }
        let value = luma(source) - luma(denoised);
        if !value.is_finite() {
            return Err(DetailError::NonFinite);
        }
        residual.push(value);
    }
    let (mean, variance) = mean_variance(&residual);
    let sigma = variance.sqrt();
    let thresholds = plan.multipliers.map(|value| sigma * f64::from(value));
    let filtered = five_band(&residual, width, height, thresholds, &cancelled)?;
    for (pixel, detail) in output.iter_mut().zip(filtered.iter().copied()) {
        let delta = plan.alpha * detail;
        for channel in &mut pixel[..3] {
            *channel += delta;
        }
        if pixel.iter().any(|value| !value.is_finite()) {
            return Err(DetailError::NonFinite);
        }
    }
    let mut hasher = Sha256::new();
    for value in &filtered {
        hasher.update(value.to_bits().to_le_bytes());
    }
    Ok((
        filtered,
        DetailReceipt {
            version: plan.version,
            alpha: plan.alpha,
            mean,
            variance,
            sigma,
            thresholds,
            residual_hash: hasher.finalize().into(),
        },
    ))
}

fn luma(pixel: &[f32; 4]) -> f32 {
    0.2126 * pixel[0] + 0.7152 * pixel[1] + 0.0722 * pixel[2]
}

fn mean_variance(values: &[f32]) -> (f64, f64) {
    if values.is_empty() {
        return (0.0, 0.0);
    }
    let mut sum = 0.0;
    let mut correction = 0.0;
    for value in values {
        let adjusted = f64::from(*value) - correction;
        let next = sum + adjusted;
        correction = (next - sum) - adjusted;
        sum = next;
    }
    let mean = sum / values.len() as f64;
    let mut variance_sum = 0.0;
    correction = 0.0;
    for value in values {
        let adjusted = (f64::from(*value) - mean).powi(2) - correction;
        let next = variance_sum + adjusted;
        correction = (next - variance_sum) - adjusted;
        variance_sum = next;
    }
    (mean, (variance_sum / values.len() as f64).max(0.0))
}

fn five_band(
    input: &[f32],
    width: usize,
    height: usize,
    thresholds: [f64; 5],
    cancelled: &impl Fn() -> bool,
) -> Result<Vec<f32>, DetailError> {
    let mut current = input.to_vec();
    for (band, threshold) in thresholds.into_iter().enumerate() {
        if cancelled() {
            return Err(DetailError::Cancelled);
        }
        let low = blur(&current, width, height, 1_usize << band);
        for index in 0..current.len() {
            let detail = f64::from(current[index]) - f64::from(low[index]);
            current[index] = (f64::from(low[index]) + soft_threshold(detail, threshold)) as f32;
        }
    }
    Ok(current)
}

fn blur(input: &[f32], width: usize, height: usize, step: usize) -> Vec<f32> {
    const KERNEL: [f64; 5] = [1.0 / 16.0, 4.0 / 16.0, 6.0 / 16.0, 4.0 / 16.0, 1.0 / 16.0];
    let mut horizontal = vec![0.0; input.len()];
    let mut output = vec![0.0; input.len()];
    for y in 0..height {
        for x in 0..width {
            horizontal[y * width + x] = KERNEL
                .into_iter()
                .enumerate()
                .map(|(tap, coefficient)| {
                    let offset = i64::try_from(tap).unwrap_or(i64::MAX) - 2;
                    coefficient
                        * f64::from(
                            input[y * width
                                + reflect(
                                    i64::try_from(x).unwrap_or(i64::MAX)
                                        + offset * i64::try_from(step).unwrap_or(i64::MAX),
                                    width,
                                )],
                        )
                })
                .sum::<f64>() as f32;
        }
    }
    for y in 0..height {
        for x in 0..width {
            output[y * width + x] = KERNEL
                .into_iter()
                .enumerate()
                .map(|(tap, coefficient)| {
                    let offset = i64::try_from(tap).unwrap_or(i64::MAX) - 2;
                    coefficient
                        * f64::from(
                            horizontal[reflect(
                                i64::try_from(y).unwrap_or(i64::MAX)
                                    + offset * i64::try_from(step).unwrap_or(i64::MAX),
                                height,
                            ) * width
                                + x],
                        )
                })
                .sum::<f64>() as f32;
        }
    }
    output
}

pub(crate) fn reflect(index: i64, length: usize) -> usize {
    if length <= 1 {
        return 0;
    }
    let length = i64::try_from(length).unwrap_or(i64::MAX);
    let period = 2 * (length - 1);
    let normalized = index.rem_euclid(period);
    if normalized < length {
        normalized as usize
    } else {
        (period - normalized) as usize
    }
}

fn soft_threshold(value: f64, threshold: f64) -> f64 {
    if value > threshold {
        value - threshold
    } else if value < -threshold {
        value + threshold
    } else {
        0.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetailError {
    InvalidAlpha,
    PixelCountMismatch,
    ArithmeticOverflow,
    NonFinite,
    Cancelled,
}

impl std::fmt::Display for DetailError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "RGB AI detail recovery error: {self:?}")
    }
}
impl std::error::Error for DetailError {}
