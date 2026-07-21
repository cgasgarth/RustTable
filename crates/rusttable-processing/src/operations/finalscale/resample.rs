use super::{FinalScaleExecutionError, FinalScaleKernel, FinalScalePlan};
use rusttable_image::Roi;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ResampleTap {
    index: u32,
    weight: f32,
}

impl ResampleTap {
    #[must_use]
    pub const fn index(self) -> u32 {
        self.index
    }

    #[must_use]
    pub const fn weight(self) -> f32 {
        self.weight
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AxisCoefficients {
    source_size: u32,
    destination_size: u32,
    entries: Vec<Vec<ResampleTap>>,
}

impl AxisCoefficients {
    pub(crate) fn new(source_size: u32, destination_size: u32, kernel: FinalScaleKernel) -> Self {
        let entries = (0..destination_size)
            .map(|destination| coefficients(source_size, destination_size, destination, kernel))
            .collect();
        Self {
            source_size,
            destination_size,
            entries,
        }
    }

    #[must_use]
    pub const fn source_size(&self) -> u32 {
        self.source_size
    }

    #[must_use]
    pub const fn destination_size(&self) -> u32 {
        self.destination_size
    }

    #[must_use]
    pub fn at(&self, destination: u32) -> Option<&[ResampleTap]> {
        self.entries.get(destination as usize).map(Vec::as_slice)
    }

    pub fn iter(&self) -> impl Iterator<Item = &[ResampleTap]> {
        self.entries.iter().map(Vec::as_slice)
    }
}

pub(crate) fn execute<F: Fn() -> bool>(
    plan: &FinalScalePlan,
    input: &[f32],
    input_roi: Roi,
    output_roi: Roi,
    channels: usize,
    stride: usize,
    cancelled: F,
) -> Result<Vec<f32>, FinalScaleExecutionError> {
    validate(plan, input, input_roi, output_roi, channels, stride)?;
    let output_len = usize::try_from(output_roi.width())
        .ok()
        .and_then(|width| {
            usize::try_from(output_roi.height())
                .ok()
                .and_then(|height| width.checked_mul(height))
        })
        .and_then(|pixels| pixels.checked_mul(channels))
        .ok_or(FinalScaleExecutionError::ArithmeticOverflow)?;
    let mut output = vec![0.0; output_len];
    for local_y in 0..output_roi.height() {
        if cancelled() {
            return Err(FinalScaleExecutionError::Cancelled);
        }
        let global_y = output_roi.y() + local_y;
        let y_coefficients = plan
            .coefficients_y()
            .at(global_y)
            .ok_or(FinalScaleExecutionError::ArithmeticOverflow)?;
        for local_x in 0..output_roi.width() {
            let global_x = output_roi.x() + local_x;
            let x_coefficients = plan
                .coefficients_x()
                .at(global_x)
                .ok_or(FinalScaleExecutionError::ArithmeticOverflow)?;
            let destination = (usize::try_from(local_y).unwrap_or(usize::MAX)
                * usize::try_from(output_roi.width()).unwrap_or(usize::MAX)
                + usize::try_from(local_x).unwrap_or(usize::MAX))
            .checked_mul(channels)
            .ok_or(FinalScaleExecutionError::ArithmeticOverflow)?;
            for channel in 0..channels {
                let mut value = 0.0;
                for y_tap in y_coefficients {
                    for x_tap in x_coefficients {
                        let source_x = x_tap.index().saturating_sub(input_roi.x());
                        let source_y = y_tap.index().saturating_sub(input_roi.y());
                        let source = usize::try_from(source_y)
                            .ok()
                            .and_then(|row| row.checked_mul(stride))
                            .and_then(|offset| {
                                usize::try_from(source_x)
                                    .ok()
                                    .and_then(|column| column.checked_mul(channels))
                                    .and_then(|column| offset.checked_add(column))
                            })
                            .and_then(|offset| offset.checked_add(channel))
                            .ok_or(FinalScaleExecutionError::ArithmeticOverflow)?;
                        let sample =
                            *input
                                .get(source)
                                .ok_or(FinalScaleExecutionError::InvalidShape {
                                    expected: stride
                                        .checked_mul(input_roi.height() as usize)
                                        .ok_or(FinalScaleExecutionError::ArithmeticOverflow)?,
                                    actual: input.len(),
                                })?;
                        if !sample.is_finite() {
                            return Err(FinalScaleExecutionError::NonFiniteInput);
                        }
                        value += sample * x_tap.weight() * y_tap.weight();
                    }
                }
                output[destination + channel] = value;
            }
        }
    }
    Ok(output)
}

fn validate(
    plan: &FinalScalePlan,
    input: &[f32],
    input_roi: Roi,
    output_roi: Roi,
    channels: usize,
    stride: usize,
) -> Result<(), FinalScaleExecutionError> {
    if !(1..=4).contains(&channels) {
        return Err(FinalScaleExecutionError::UnsupportedChannels(channels));
    }
    if !within(input_roi, plan.source_roi()) || !within(output_roi, plan.output_roi()) {
        return Err(FinalScaleExecutionError::InvalidRoi);
    }
    let minimum_stride = usize::try_from(input_roi.width())
        .ok()
        .and_then(|width| width.checked_mul(channels))
        .ok_or(FinalScaleExecutionError::ArithmeticOverflow)?;
    if stride < minimum_stride {
        return Err(FinalScaleExecutionError::InvalidStride {
            minimum: minimum_stride,
            actual: stride,
        });
    }
    let expected = stride
        .checked_mul(input_roi.height() as usize)
        .ok_or(FinalScaleExecutionError::ArithmeticOverflow)?;
    if input.len() < expected {
        return Err(FinalScaleExecutionError::InvalidShape {
            expected,
            actual: input.len(),
        });
    }
    Ok(())
}

fn within(roi: Roi, bounds: Roi) -> bool {
    roi.x() >= bounds.x()
        && roi.y() >= bounds.y()
        && roi.right() <= bounds.right()
        && roi.bottom() <= bounds.bottom()
}

fn coefficients(
    source_size: u32,
    destination_size: u32,
    destination: u32,
    kernel: FinalScaleKernel,
) -> Vec<ResampleTap> {
    let coordinate = f64::from(destination) * f64::from(source_size) / f64::from(destination_size);
    let base = coordinate.floor() as i64;
    let offsets: Vec<i64> = match kernel {
        FinalScaleKernel::Nearest => vec![i64::from(coordinate.fract() >= 0.5)],
        FinalScaleKernel::Bilinear => vec![0, 1],
        FinalScaleKernel::Bicubic => vec![-1, 0, 1, 2],
        FinalScaleKernel::Lanczos => vec![-2, -1, 0, 1, 2, 3],
    };
    let mut taps = offsets
        .into_iter()
        .map(|offset| {
            let index = reflect(base + offset, source_size);
            let distance = coordinate - (base + offset) as f64;
            let weight = match kernel {
                FinalScaleKernel::Nearest => 1.0_f32,
                FinalScaleKernel::Bilinear => (1.0 - distance.abs()) as f32,
                FinalScaleKernel::Bicubic => cubic_weight(distance),
                FinalScaleKernel::Lanczos => lanczos_weight(distance, 3.0),
            };
            ResampleTap { index, weight }
        })
        .collect::<Vec<_>>();
    let sum: f32 = taps.iter().map(|tap| tap.weight).sum();
    if sum != 0.0 && sum.is_finite() {
        for tap in &mut taps {
            tap.weight /= sum;
        }
    }
    taps
}

fn reflect(value: i64, extent: u32) -> u32 {
    if extent <= 1 {
        return 0;
    }
    let period = i64::from(extent) * 2 - 2;
    let wrapped = ((value % period) + period) % period;
    let reflected = if wrapped >= i64::from(extent) {
        period - wrapped
    } else {
        wrapped
    };
    reflected as u32
}

fn cubic_weight(value: f64) -> f32 {
    let value = value.abs();
    if value <= 1.0 {
        (1.5 * value * value * value - 2.5 * value * value + 1.0) as f32
    } else if value < 2.0 {
        (-0.5 * value * value * value + 2.5 * value * value - 4.0 * value + 2.0) as f32
    } else {
        0.0
    }
}

fn lanczos_weight(value: f64, support: f64) -> f32 {
    let value = value.abs();
    if value == 0.0 {
        return 1.0;
    }
    if value >= support {
        return 0.0;
    }
    let pi_value = std::f64::consts::PI * value;
    ((pi_value.sin() / pi_value) * (pi_value / support).sin() / (pi_value / support)) as f32
}
