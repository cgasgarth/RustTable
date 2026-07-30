use super::geometry::BordersPlan;
use crate::{LinearRgb, RasterDimensions};
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BordersExecution {
    pixels: Vec<LinearRgb>,
    dimensions: RasterDimensions,
    identity: [u8; 32],
}
impl BordersExecution {
    pub fn pixels(&self) -> &[LinearRgb] {
        &self.pixels
    }
    pub const fn dimensions(&self) -> RasterDimensions {
        self.dimensions
    }
    pub const fn identity(&self) -> [u8; 32] {
        self.identity
    }
}

impl BordersPlan {
    pub fn execute(&self, input: &[LinearRgb]) -> Result<BordersExecution, BordersExecutionError> {
        self.execute_with_cancel(input, || false)
    }
    pub fn execute_with_cancel<F: Fn() -> bool>(
        &self,
        input: &[LinearRgb],
        cancelled: F,
    ) -> Result<BordersExecution, BordersExecutionError> {
        let expected = usize::try_from(self.source_dimensions().pixel_count())
            .map_err(|_| BordersExecutionError::ArithmeticOverflow)?;
        if input.len() != expected {
            return Err(BordersExecutionError::DimensionsMismatch {
                expected,
                actual: input.len(),
            });
        }
        let out = self.output_dimensions();
        let count = usize::try_from(out.pixel_count())
            .map_err(|_| BordersExecutionError::ArithmeticOverflow)?;
        let mut pixels = vec![
            self.config()
                .color
                .values()
                .map(crate::FiniteF32::get)
                .pipe_rgb();
            count
        ];
        let source = self.geometry().source_rect();
        let frame = self.geometry().frame();
        let sw = usize::try_from(self.source_dimensions().width())
            .map_err(|_| BordersExecutionError::ArithmeticOverflow)?;
        let ow =
            usize::try_from(out.width()).map_err(|_| BordersExecutionError::ArithmeticOverflow)?;
        for y in 0..out.height() {
            if cancelled() {
                return Err(BordersExecutionError::Cancelled);
            }
            for x in 0..out.width() {
                let idx = usize::try_from(y)
                    .ok()
                    .and_then(|row| row.checked_mul(ow))
                    .and_then(|row| row.checked_add(usize::try_from(x).ok()?))
                    .ok_or(BordersExecutionError::ArithmeticOverflow)?;
                if x >= source.x()
                    && x < source.x() + source.width()
                    && y >= source.y()
                    && y < source.y() + source.height()
                {
                    let si = usize::try_from(y - source.y())
                        .ok()
                        .and_then(|row| row.checked_mul(sw))
                        .and_then(|row| row.checked_add(usize::try_from(x - source.x()).ok()?))
                        .ok_or(BordersExecutionError::ArithmeticOverflow)?;
                    pixels[idx] = input[si];
                } else if frame.contains(x, y, out) {
                    pixels[idx] = self
                        .config()
                        .frame_color
                        .values()
                        .map(crate::FiniteF32::get)
                        .pipe_rgb();
                }
            }
        }
        Ok(BordersExecution {
            pixels,
            dimensions: out,
            identity: self.identity(),
        })
    }
    pub fn execute_mask(&self, input: &[f32]) -> Result<Vec<f32>, BordersExecutionError> {
        let expected = usize::try_from(self.source_dimensions().pixel_count())
            .map_err(|_| BordersExecutionError::ArithmeticOverflow)?;
        if input.len() != expected {
            return Err(BordersExecutionError::DimensionsMismatch {
                expected,
                actual: input.len(),
            });
        }
        let out = usize::try_from(self.output_dimensions().pixel_count())
            .map_err(|_| BordersExecutionError::ArithmeticOverflow)?;
        let mut result = vec![0.0; out];
        let sw = usize::try_from(self.source_dimensions().width())
            .map_err(|_| BordersExecutionError::ArithmeticOverflow)?;
        let ow = usize::try_from(self.output_dimensions().width())
            .map_err(|_| BordersExecutionError::ArithmeticOverflow)?;
        let source = self.geometry().source_rect();
        for y in 0..source.height() {
            let from = usize::try_from(y)
                .ok()
                .and_then(|v| v.checked_mul(sw))
                .ok_or(BordersExecutionError::ArithmeticOverflow)?;
            let to = usize::try_from(y + source.y())
                .ok()
                .and_then(|v| v.checked_mul(ow))
                .and_then(|v| v.checked_add(usize::try_from(source.x()).ok()?))
                .ok_or(BordersExecutionError::ArithmeticOverflow)?;
            result[to..to + sw].copy_from_slice(&input[from..from + sw]);
        }
        Ok(result)
    }
}

trait PixelPipe {
    fn pipe_rgb(self) -> LinearRgb;
}
impl PixelPipe for [f32; 3] {
    fn pipe_rgb(self) -> LinearRgb {
        LinearRgb::new(
            crate::FiniteF32::new(self[0]).expect("validated color"),
            crate::FiniteF32::new(self[1]).expect("validated color"),
            crate::FiniteF32::new(self[2]).expect("validated color"),
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BordersExecutionError {
    Cancelled,
    DimensionsMismatch { expected: usize, actual: usize },
    ArithmeticOverflow,
}
impl fmt::Display for BordersExecutionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "borders execution error: {self:?}")
    }
}
impl std::error::Error for BordersExecutionError {}

use crate::operations::borders::BordersFrame;
trait FrameContains {
    fn contains(self, x: u32, y: u32, d: RasterDimensions) -> bool;
}
impl FrameContains for BordersFrame {
    fn contains(self, x: u32, y: u32, d: RasterDimensions) -> bool {
        let inx = x >= self.frame_left() && x < self.frame_right();
        let iny = y >= self.frame_top() && y < self.frame_bottom();
        inx && iny
            && (x < self.inner_left()
                || x >= self.inner_right()
                || y < self.inner_top()
                || y >= self.inner_bottom())
            && x < d.width()
            && y < d.height()
    }
}
