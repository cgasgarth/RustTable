use std::fmt;

use crate::evaluate::{
    BasicAdjPlanSet, FrameBoundaryMode, FrameBoundaryOptions,
    evaluate_graph_at_frame_boundaries_with_plans, evaluate_steps, evaluate_steps_with_frame,
    graph_has_discrete_geometry,
};
use crate::{
    CompiledOperationGraph, EvaluationError, LinearRgb, RasterDimensions, WorkingRgbImage,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RasterRowWindow {
    start_row: usize,
    row_count: usize,
}

impl RasterRowWindow {
    #[must_use]
    pub const fn new(start_row: usize, row_count: usize) -> Self {
        Self {
            start_row,
            row_count,
        }
    }

    /// Constructs and validates a row window for explicit raster dimensions.
    ///
    /// # Errors
    ///
    /// Returns the exact checked range failure for the supplied dimensions.
    pub fn checked(
        start_row: usize,
        row_count: usize,
        dimensions: RasterDimensions,
    ) -> Result<Self, RasterRowWindowError> {
        Self::new(start_row, row_count).validate(dimensions)
    }

    /// Validates this immutable row range against explicit raster dimensions.
    ///
    /// # Errors
    ///
    /// Returns a distinct error for an empty, out-of-bounds, or overflowing
    /// range.
    pub fn validate(self, dimensions: RasterDimensions) -> Result<Self, RasterRowWindowError> {
        if self.row_count == 0 {
            return Err(RasterRowWindowError::ZeroRowCount {
                start_row: self.start_row,
                dimensions,
            });
        }
        let end_row = self.start_row.checked_add(self.row_count).ok_or(
            RasterRowWindowError::ArithmeticOverflow {
                start_row: self.start_row,
                row_count: self.row_count,
                dimensions,
            },
        )?;
        let height = usize::try_from(dimensions.height()).map_err(|_| {
            RasterRowWindowError::ArithmeticOverflow {
                start_row: self.start_row,
                row_count: self.row_count,
                dimensions,
            }
        })?;
        if self.start_row >= height {
            return Err(RasterRowWindowError::StartOutOfBounds {
                start_row: self.start_row,
                row_count: self.row_count,
                dimensions,
            });
        }
        if end_row > height {
            return Err(RasterRowWindowError::EndOutOfBounds {
                start_row: self.start_row,
                row_count: self.row_count,
                end_row,
                dimensions,
            });
        }
        Ok(self)
    }

    #[must_use]
    pub const fn start_row(self) -> usize {
        self.start_row
    }

    #[must_use]
    pub const fn row_count(self) -> usize {
        self.row_count
    }

    #[must_use]
    pub const fn checked_end_row(self) -> Option<usize> {
        self.start_row.checked_add(self.row_count)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RasterRowWindowError {
    ZeroRowCount {
        start_row: usize,
        dimensions: RasterDimensions,
    },
    StartOutOfBounds {
        start_row: usize,
        row_count: usize,
        dimensions: RasterDimensions,
    },
    EndOutOfBounds {
        start_row: usize,
        row_count: usize,
        end_row: usize,
        dimensions: RasterDimensions,
    },
    ArithmeticOverflow {
        start_row: usize,
        row_count: usize,
        dimensions: RasterDimensions,
    },
}

impl fmt::Display for RasterRowWindowError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroRowCount {
                start_row,
                dimensions,
            } => write!(
                formatter,
                "row window at {start_row} has zero rows for {dimensions:?}"
            ),
            Self::StartOutOfBounds {
                start_row,
                row_count,
                dimensions,
            } => write!(
                formatter,
                "row window start {start_row} with {row_count} rows is outside {dimensions:?}"
            ),
            Self::EndOutOfBounds {
                start_row,
                row_count,
                end_row,
                dimensions,
            } => write!(
                formatter,
                "row window {start_row}..{end_row} with {row_count} rows exceeds {dimensions:?}"
            ),
            Self::ArithmeticOverflow {
                start_row,
                row_count,
                dimensions,
            } => write!(
                formatter,
                "row window start {start_row} plus {row_count} rows overflows for {dimensions:?}"
            ),
        }
    }
}

impl std::error::Error for RasterRowWindowError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvaluatedRowWindow {
    dimensions: RasterDimensions,
    window: RasterRowWindow,
    pixels: Vec<LinearRgb>,
}

impl EvaluatedRowWindow {
    #[must_use]
    pub const fn dimensions(&self) -> RasterDimensions {
        self.dimensions
    }

    #[must_use]
    pub const fn window(&self) -> RasterRowWindow {
        self.window
    }

    #[must_use]
    pub const fn start_row(&self) -> usize {
        self.window.start_row()
    }

    #[must_use]
    pub const fn row_count(&self) -> usize {
        self.window.row_count()
    }

    #[must_use]
    pub fn pixel_slice(&self) -> &[LinearRgb] {
        &self.pixels
    }

    pub fn pixels(&self) -> impl Iterator<Item = &LinearRgb> {
        self.pixels.iter()
    }

    /// Returns a pixel by zero-based row and column within this window.
    #[must_use]
    pub fn pixel(&self, row_offset: usize, column: usize) -> Option<&LinearRgb> {
        let width = usize::try_from(self.dimensions.width()).ok()?;
        if row_offset >= self.row_count() || column >= width {
            return None;
        }
        let index = row_offset.checked_mul(width)?.checked_add(column)?;
        self.pixels.get(index)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GraphWindowEvaluationError {
    Window { source: RasterRowWindowError },
    Evaluation { source: Box<EvaluationError> },
}

impl fmt::Display for GraphWindowEvaluationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Window { source } => write!(formatter, "invalid graph row window: {source}"),
            Self::Evaluation { source } => {
                write!(formatter, "graph row evaluation failed: {source}")
            }
        }
    }
}

impl std::error::Error for GraphWindowEvaluationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Window { source } => Some(source),
            Self::Evaluation { source } => Some(source.as_ref()),
        }
    }
}

/// Evaluates an immutable graph into a new full-frame working image.
///
/// # Errors
///
/// Returns the same deterministic arithmetic error as pipeline evaluation.
pub fn evaluate_graph(
    graph: &CompiledOperationGraph,
    input: &WorkingRgbImage,
) -> Result<WorkingRgbImage, EvaluationError> {
    evaluate_graph_with_basicadj_plans(graph, input, None)
}

/// Evaluates a graph using one previously resolved automatic-basicadj set.
/// Supplying this set is what makes tiled and full-frame execution share the
/// same automatic values.
///
/// # Errors
///
/// Returns the first graph-operation or pixel-evaluation failure.
pub fn evaluate_graph_with_basicadj_plans(
    graph: &CompiledOperationGraph,
    input: &WorkingRgbImage,
    plans: Option<&BasicAdjPlanSet>,
) -> Result<WorkingRgbImage, EvaluationError> {
    if graph_has_discrete_geometry(graph) {
        let alpha = vec![1.0; input.pixel_slice().len()];
        return evaluate_graph_at_frame_boundaries_with_plans(
            graph,
            input,
            &alpha,
            FrameBoundaryOptions::new(FrameBoundaryMode::Preview),
            plans,
            || false,
        )
        .map(|evaluated| evaluated.image().clone());
    }
    let (pixels, frame) = evaluate_steps_with_frame(
        graph
            .nodes()
            .map(|node| (node.pipeline_step_index(), node.prepared())),
        input.pixel_slice(),
        input.dimensions(),
        0,
        input.frame(),
        plans,
    )?;
    Ok(WorkingRgbImage::from_validated_parts_with_frame(
        input.dimensions(),
        pixels,
        frame,
    ))
}

/// Evaluates one validated contiguous row window with global diagnostics.
///
/// # Errors
///
/// Returns a window error before allocating or evaluating when the range is
/// invalid, or an evaluation error with full-frame pixel coordinates.
pub fn evaluate_graph_window(
    graph: &CompiledOperationGraph,
    input: &WorkingRgbImage,
    window: RasterRowWindow,
) -> Result<EvaluatedRowWindow, GraphWindowEvaluationError> {
    let window = window
        .validate(input.dimensions())
        .map_err(|source| GraphWindowEvaluationError::Window { source })?;
    let width = usize::try_from(input.dimensions().width()).map_err(|_| {
        GraphWindowEvaluationError::Window {
            source: RasterRowWindowError::ArithmeticOverflow {
                start_row: window.start_row(),
                row_count: window.row_count(),
                dimensions: input.dimensions(),
            },
        }
    })?;
    let start_index = window.start_row().checked_mul(width).ok_or_else(|| {
        GraphWindowEvaluationError::Window {
            source: RasterRowWindowError::ArithmeticOverflow {
                start_row: window.start_row(),
                row_count: window.row_count(),
                dimensions: input.dimensions(),
            },
        }
    })?;
    let pixel_count = width.checked_mul(window.row_count()).ok_or_else(|| {
        GraphWindowEvaluationError::Window {
            source: RasterRowWindowError::ArithmeticOverflow {
                start_row: window.start_row(),
                row_count: window.row_count(),
                dimensions: input.dimensions(),
            },
        }
    })?;
    let end_index =
        start_index
            .checked_add(pixel_count)
            .ok_or_else(|| GraphWindowEvaluationError::Window {
                source: RasterRowWindowError::ArithmeticOverflow {
                    start_row: window.start_row(),
                    row_count: window.row_count(),
                    dimensions: input.dimensions(),
                },
            })?;
    let source = input
        .pixel_slice()
        .get(start_index..end_index)
        .ok_or_else(|| GraphWindowEvaluationError::Window {
            source: RasterRowWindowError::ArithmeticOverflow {
                start_row: window.start_row(),
                row_count: window.row_count(),
                dimensions: input.dimensions(),
            },
        })?;
    let pixels = evaluate_steps(
        graph
            .nodes()
            .map(|node| (node.pipeline_step_index(), node.prepared())),
        source,
        input.dimensions(),
        start_index,
    )
    .map_err(|source| GraphWindowEvaluationError::Evaluation {
        source: Box::new(source),
    })?;
    Ok(EvaluatedRowWindow {
        dimensions: input.dimensions(),
        window,
        pixels,
    })
}
