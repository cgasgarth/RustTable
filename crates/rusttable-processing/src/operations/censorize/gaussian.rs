#![allow(clippy::similar_names, clippy::too_many_lines)]

use super::execution::{CensorizeExecutionError, CensorizePixel};
use crate::RasterDimensions;

#[derive(Debug, Clone, Copy)]
pub(super) struct ReferenceGaussian {
    sigma: f32,
}

impl ReferenceGaussian {
    pub(super) fn new(sigma: f32) -> Self {
        Self { sigma }
    }

    pub(super) fn apply<F: FnMut() -> bool>(
        self,
        input: &[CensorizePixel],
        dimensions: RasterDimensions,
        mut cancelled: F,
    ) -> Result<Vec<CensorizePixel>, CensorizeExecutionError> {
        let width = usize::try_from(dimensions.width())
            .map_err(|_| CensorizeExecutionError::ArithmeticOverflow)?;
        let height = usize::try_from(dimensions.height())
            .map_err(|_| CensorizeExecutionError::ArithmeticOverflow)?;
        let expected = width
            .checked_mul(height)
            .ok_or(CensorizeExecutionError::ArithmeticOverflow)?;
        if input.len() != expected {
            return Err(CensorizeExecutionError::DimensionsMismatch {
                expected,
                actual: input.len(),
            });
        }
        let (a0, a1, a2, a3, b1, b2, coefp, coefn) = params(self.sigma);
        let mut temp = vec![CensorizePixel::new(0.0, 0.0, 0.0, 0.0); expected];
        let mut output = vec![CensorizePixel::new(0.0, 0.0, 0.0, 0.0); expected];

        for x in 0..width {
            if cancelled() {
                return Err(CensorizeExecutionError::Cancelled);
            }
            let mut xp = [0.0; 4];
            let mut yb = [0.0; 4];
            let mut yp = [0.0; 4];
            let first = input[x].channels();
            for c in 0..4 {
                xp[c] = clamp(first[c]);
                yb[c] = xp[c] * coefp;
                yp[c] = yb[c];
            }
            for y in 0..height {
                let index = y * width + x;
                let xc = input[index].channels();
                let mut yc = [0.0; 4];
                for c in 0..4 {
                    let value = clamp(xc[c]);
                    yc[c] = a0 * value + a1 * xp[c] - b1 * yp[c] - b2 * yb[c];
                    xp[c] = value;
                    yb[c] = yp[c];
                    yp[c] = yc[c];
                }
                temp[index] = CensorizePixel::from_channels(yc);
            }
            let last = input[(height - 1) * width + x].channels();
            let mut xn = [0.0; 4];
            let mut xa = [0.0; 4];
            let mut yn = [0.0; 4];
            let mut ya = [0.0; 4];
            for c in 0..4 {
                xn[c] = clamp(last[c]);
                xa[c] = xn[c];
                yn[c] = xn[c] * coefn;
                ya[c] = yn[c];
            }
            for y in (0..height).rev() {
                let index = y * width + x;
                let xc = input[index].channels();
                let mut yc = [0.0; 4];
                for c in 0..4 {
                    let value = clamp(xc[c]);
                    yc[c] = a2 * xn[c] + a3 * xa[c] - b1 * yn[c] - b2 * ya[c];
                    xa[c] = xn[c];
                    xn[c] = value;
                    ya[c] = yn[c];
                    yn[c] = yc[c];
                    let prior = temp[index].channels();
                    let mut sum = prior;
                    sum[c] += yc[c];
                    temp[index] = CensorizePixel::from_channels(sum);
                }
            }
        }
        for y in 0..height {
            if cancelled() {
                return Err(CensorizeExecutionError::Cancelled);
            }
            let row = y * width;
            let first = temp[row].channels();
            let mut xp = [0.0; 4];
            let mut yb = [0.0; 4];
            let mut yp = [0.0; 4];
            for c in 0..4 {
                xp[c] = clamp(first[c]);
                yb[c] = xp[c] * coefp;
                yp[c] = yb[c];
            }
            for x in 0..width {
                let index = row + x;
                let xc = temp[index].channels();
                let mut yc = [0.0; 4];
                for c in 0..4 {
                    let value = clamp(xc[c]);
                    yc[c] = a0 * value + a1 * xp[c] - b1 * yp[c] - b2 * yb[c];
                    xp[c] = value;
                    yb[c] = yp[c];
                    yp[c] = yc[c];
                }
                output[index] = CensorizePixel::from_channels(yc);
            }
            let last = temp[row + width - 1].channels();
            let mut xn = [0.0; 4];
            let mut xa = [0.0; 4];
            let mut yn = [0.0; 4];
            let mut ya = [0.0; 4];
            for c in 0..4 {
                xn[c] = clamp(last[c]);
                xa[c] = xn[c];
                yn[c] = xn[c] * coefn;
                ya[c] = yn[c];
            }
            for x in (0..width).rev() {
                let index = row + x;
                let xc = temp[index].channels();
                let mut yc = [0.0; 4];
                for c in 0..4 {
                    let value = clamp(xc[c]);
                    yc[c] = a2 * xn[c] + a3 * xa[c] - b1 * yn[c] - b2 * ya[c];
                    xa[c] = xn[c];
                    xn[c] = value;
                    ya[c] = yn[c];
                    yn[c] = yc[c];
                    let mut sum = output[index].channels();
                    sum[c] += yc[c];
                    output[index] = CensorizePixel::from_channels(sum);
                }
            }
        }
        Ok(output)
    }
}

fn params(sigma: f32) -> (f32, f32, f32, f32, f32, f32, f32, f32) {
    let alpha = 1.695 / sigma;
    let ema = (-alpha).exp();
    let ema2 = (-2.0 * alpha).exp();
    let b1 = -2.0 * ema;
    let b2 = ema2;
    let k = (1.0 - ema) * (1.0 - ema) / (1.0 + 2.0 * alpha * ema - ema2);
    let a0 = k;
    let a1 = k * (alpha - 1.0) * ema;
    let a2 = k * (alpha + 1.0) * ema;
    let a3 = -k * ema2;
    let denominator = 1.0 + b1 + b2;
    let coefp = (a0 + a1) / denominator;
    let coefn = (a2 + a3) / denominator;
    (a0, a1, a2, a3, b1, b2, coefp, coefn)
}

fn clamp(value: f32) -> f32 {
    value.clamp(0.0, f32::MAX)
}
