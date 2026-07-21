#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::explicit_auto_deref,
    clippy::float_cmp,
    clippy::manual_midpoint,
    clippy::many_single_char_names,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::must_use_candidate,
    clippy::unnecessary_sort_by
)]

use std::fmt;

use super::geometry::{Homography, Point};
use crate::RasterDimensions;

const DEFAULT_MIN_LINE_LENGTH: f64 = 5.0;
const DEFAULT_ANGLE_TOLERANCE: f64 = 30.0;
const DEFAULT_MAX_PIXELS: u64 = 1_048_576;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LineSegment {
    start: Point,
    end: Point,
    width: f64,
    score: f64,
}

impl LineSegment {
    pub fn new(start: Point, end: Point, width: f64, score: f64) -> Result<Self, AnalysisError> {
        start
            .validate()
            .map_err(|_| AnalysisError::NonFiniteInput)?;
        end.validate().map_err(|_| AnalysisError::NonFiniteInput)?;
        if !width.is_finite() || !score.is_finite() || width <= 0.0 || score < 0.0 {
            return Err(AnalysisError::InvalidLine);
        }
        if start.x() == end.x() && start.y() == end.y() {
            return Err(AnalysisError::DegenerateLine);
        }
        Ok(Self {
            start,
            end,
            width,
            score,
        })
    }
    #[must_use]
    pub const fn start(self) -> Point {
        self.start
    }
    #[must_use]
    pub const fn end(self) -> Point {
        self.end
    }
    #[must_use]
    pub const fn width(self) -> f64 {
        self.width
    }
    #[must_use]
    pub const fn score(self) -> f64 {
        self.score
    }
    #[must_use]
    pub fn length(self) -> f64 {
        (self.end.x() - self.start.x()).hypot(self.end.y() - self.start.y())
    }
    #[must_use]
    pub fn angle_degrees(self) -> f64 {
        (self.end.y() - self.start.y())
            .atan2(self.end.x() - self.start.x())
            .to_degrees()
            .rem_euclid(180.0)
    }
    #[must_use]
    pub fn midpoint(self) -> Point {
        Point::new(
            (self.start.x() + self.end.x()) * 0.5,
            (self.start.y() + self.end.y()) * 0.5,
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LineKind {
    Irrelevant,
    Vertical,
    Horizontal,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DetectedLine {
    segment: LineSegment,
    kind: LineKind,
    weight: f64,
    selected: bool,
}

impl DetectedLine {
    #[must_use]
    pub const fn segment(&self) -> LineSegment {
        self.segment
    }
    #[must_use]
    pub const fn kind(&self) -> LineKind {
        self.kind
    }
    #[must_use]
    pub const fn weight(&self) -> f64 {
        self.weight
    }
    #[must_use]
    pub const fn selected(&self) -> bool {
        self.selected
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AnalysisConfig {
    min_line_length: f64,
    angle_tolerance: f64,
    max_pixels: u64,
    max_lines: usize,
    edge_threshold: f64,
}

impl Default for AnalysisConfig {
    fn default() -> Self {
        Self {
            min_line_length: DEFAULT_MIN_LINE_LENGTH,
            angle_tolerance: DEFAULT_ANGLE_TOLERANCE,
            max_pixels: DEFAULT_MAX_PIXELS,
            max_lines: 50,
            edge_threshold: 0.08,
        }
    }
}

impl AnalysisConfig {
    pub fn new(min_line_length: f64, angle_tolerance: f64) -> Result<Self, AnalysisError> {
        if !min_line_length.is_finite()
            || min_line_length < 2.0
            || !angle_tolerance.is_finite()
            || !(0.0..=45.0).contains(&angle_tolerance)
        {
            return Err(AnalysisError::InvalidAnalysisConfig);
        }
        Ok(Self {
            min_line_length,
            angle_tolerance,
            ..Self::default()
        })
    }
    #[must_use]
    pub const fn min_line_length(self) -> f64 {
        self.min_line_length
    }
    #[must_use]
    pub const fn angle_tolerance(self) -> f64 {
        self.angle_tolerance
    }
    #[must_use]
    pub const fn max_pixels(self) -> u64 {
        self.max_pixels
    }
    #[must_use]
    pub const fn max_lines(self) -> usize {
        self.max_lines
    }
    #[must_use]
    pub const fn with_max_pixels(mut self, max_pixels: u64) -> Self {
        self.max_pixels = max_pixels;
        self
    }
    #[must_use]
    pub const fn with_max_lines(mut self, max_lines: usize) -> Self {
        self.max_lines = max_lines;
        self
    }
    #[must_use]
    pub const fn with_edge_threshold(mut self, edge_threshold: f64) -> Self {
        self.edge_threshold = edge_threshold;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AnalysisError {
    InvalidDimensions,
    PixelCountMismatch { expected: usize, actual: usize },
    NonFiniteInput,
    InvalidLine,
    DegenerateLine,
    InvalidAnalysisConfig,
    InsufficientLines { vertical: usize, horizontal: usize },
    AmbiguousFit,
}

impl fmt::Display for AnalysisError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidDimensions => formatter.write_str("line analysis dimensions are invalid"),
            Self::PixelCountMismatch { expected, actual } => write!(
                formatter,
                "line analysis expected {expected} pixels, got {actual}"
            ),
            Self::NonFiniteInput => formatter.write_str("line analysis input is non-finite"),
            Self::InvalidLine => formatter.write_str("line segment has invalid width or score"),
            Self::DegenerateLine => formatter.write_str("line segment has zero length"),
            Self::InvalidAnalysisConfig => {
                formatter.write_str("line analysis configuration is invalid")
            }
            Self::InsufficientLines {
                vertical,
                horizontal,
            } => write!(
                formatter,
                "line analysis needs two lines per axis; got {vertical} vertical and {horizontal} horizontal"
            ),
            Self::AmbiguousFit => formatter.write_str("line analysis fit is ambiguous"),
        }
    }
}
impl std::error::Error for AnalysisError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnalysisStatus {
    Ready,
    InsufficientLines,
    Ambiguous,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AnalysisResult {
    lines: Vec<DetectedLine>,
    vertical_vanishing: Option<Point>,
    horizontal_vanishing: Option<Point>,
    correction: Option<Homography>,
    confidence: f64,
    status: AnalysisStatus,
}

pub type LineAnalysis = AnalysisResult;

impl AnalysisResult {
    #[must_use]
    pub fn lines(&self) -> &[DetectedLine] {
        &self.lines
    }
    #[must_use]
    pub const fn vertical_vanishing(&self) -> Option<Point> {
        self.vertical_vanishing
    }
    #[must_use]
    pub const fn horizontal_vanishing(&self) -> Option<Point> {
        self.horizontal_vanishing
    }
    #[must_use]
    pub const fn correction(&self) -> Option<Homography> {
        self.correction
    }
    #[must_use]
    pub const fn confidence(&self) -> f64 {
        self.confidence
    }
    #[must_use]
    pub const fn status(&self) -> AnalysisStatus {
        self.status
    }
    #[must_use]
    pub const fn is_ready(&self) -> bool {
        matches!(self.status, AnalysisStatus::Ready)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct LuminanceFrame {
    dimensions: RasterDimensions,
    pixels: Vec<f32>,
}

impl LuminanceFrame {
    pub fn new(dimensions: RasterDimensions, pixels: Vec<f32>) -> Result<Self, AnalysisError> {
        let expected = usize::try_from(dimensions.pixel_count())
            .map_err(|_| AnalysisError::InvalidDimensions)?;
        if pixels.len() != expected {
            return Err(AnalysisError::PixelCountMismatch {
                expected,
                actual: pixels.len(),
            });
        }
        if pixels.iter().any(|value| !value.is_finite()) {
            return Err(AnalysisError::NonFiniteInput);
        }
        Ok(Self { dimensions, pixels })
    }
    #[must_use]
    pub const fn dimensions(&self) -> RasterDimensions {
        self.dimensions
    }
    #[must_use]
    pub fn pixels(&self) -> &[f32] {
        &self.pixels
    }
}

pub fn detect_lines(
    frame: &LuminanceFrame,
    config: AnalysisConfig,
) -> Result<LineAnalysis, AnalysisError> {
    let dimensions = frame.dimensions();
    let pixel_count = dimensions.pixel_count();
    if pixel_count == 0 {
        return Err(AnalysisError::InvalidDimensions);
    }
    let stride = if pixel_count > config.max_pixels && config.max_pixels > 0 {
        let ratio = (pixel_count as f64 / config.max_pixels as f64)
            .sqrt()
            .ceil();
        u32::try_from(ratio as u64)
            .map_err(|_| AnalysisError::InvalidDimensions)?
            .max(1)
    } else {
        1
    };
    let width = dimensions.width() / stride;
    let height = dimensions.height() / stride;
    if width < 2 || height < 2 {
        return Err(AnalysisError::InvalidDimensions);
    }
    let mut min = f32::INFINITY;
    let mut max = f32::NEG_INFINITY;
    for value in frame.pixels() {
        min = min.min(*value);
        max = max.max(*value);
    }
    let threshold = (f64::from(max - min) * config.edge_threshold).max(1.0e-6);
    let mut segments = Vec::new();
    for y in 0..height {
        let mut run_start = None;
        for x in 1..width {
            let left = sample(frame, x * stride - stride, y * stride);
            let right = sample(frame, x * stride, y * stride);
            let edge = f64::from(right) >= f64::from(min) + threshold;
            if edge && run_start.is_none() {
                run_start = Some(x - 1);
            }
            if (!edge || x + 1 == width) && run_start.is_some() {
                let end = if edge && x + 1 == width { x } else { x - 1 };
                if end.saturating_sub(run_start.expect("set above")) >= 1 {
                    segments.push(LineSegment::new(
                        Point::new(
                            f64::from(run_start.expect("set above") * stride),
                            f64::from(y * stride),
                        ),
                        Point::new(f64::from(end * stride), f64::from(y * stride)),
                        f64::from(stride),
                        f64::from((right - left).abs()).max(threshold),
                    )?);
                }
                run_start = None;
            }
        }
    }
    for x in 0..width {
        let mut run_start = None;
        for y in 1..height {
            let top = sample(frame, x * stride, y * stride - stride);
            let bottom = sample(frame, x * stride, y * stride);
            let edge = f64::from(bottom) >= f64::from(min) + threshold;
            if edge && run_start.is_none() {
                run_start = Some(y - 1);
            }
            if (!edge || y + 1 == height) && run_start.is_some() {
                let end = if edge && y + 1 == height { y } else { y - 1 };
                if end.saturating_sub(run_start.expect("set above")) >= 1 {
                    segments.push(LineSegment::new(
                        Point::new(
                            f64::from(x * stride),
                            f64::from(run_start.expect("set above") * stride),
                        ),
                        Point::new(f64::from(x * stride), f64::from(end * stride)),
                        f64::from(stride),
                        f64::from((bottom - top).abs()).max(threshold),
                    )?);
                }
                run_start = None;
            }
        }
    }
    Ok(analyze_lines(&segments, config))
}

pub fn analyze_lines(input: &[LineSegment], config: AnalysisConfig) -> LineAnalysis {
    let mut lines: Vec<DetectedLine> = input
        .iter()
        .copied()
        .map(|segment| {
            let angle = segment.angle_degrees();
            let horizontal_distance = angle.min(180.0 - angle);
            let vertical_distance = (angle - 90.0).abs();
            let kind = if segment.length() < config.min_line_length {
                LineKind::Irrelevant
            } else if horizontal_distance <= config.angle_tolerance {
                LineKind::Horizontal
            } else if vertical_distance <= config.angle_tolerance {
                LineKind::Vertical
            } else {
                LineKind::Irrelevant
            };
            DetectedLine {
                segment,
                kind,
                weight: segment.length() * segment.width() * segment.score().max(1.0e-9),
                selected: !matches!(kind, LineKind::Irrelevant),
            }
        })
        .collect();
    lines.sort_by(|left, right| line_key(left).cmp(&line_key(right)));
    lines.truncate(config.max_lines);
    let vertical: Vec<&DetectedLine> = lines
        .iter()
        .filter(|line| line.kind == LineKind::Vertical)
        .collect();
    let horizontal: Vec<&DetectedLine> = lines
        .iter()
        .filter(|line| line.kind == LineKind::Horizontal)
        .collect();
    let vertical_vanishing = vanishing_point(&vertical);
    let horizontal_vanishing = vanishing_point(&horizontal);
    let correction = match (vertical_vanishing, horizontal_vanishing) {
        (Some(vertical), Some(horizontal)) => rectification(vertical, horizontal),
        _ => None,
    };
    let status = match (vertical.len(), horizontal.len(), correction) {
        (v, h, Some(_)) if v >= 2 && h >= 2 => AnalysisStatus::Ready,
        (v, h, _) if v < 2 || h < 2 => AnalysisStatus::InsufficientLines,
        _ => AnalysisStatus::Ambiguous,
    };
    let confidence = confidence(vertical.len(), horizontal.len(), correction.is_some());
    AnalysisResult {
        lines,
        vertical_vanishing,
        horizontal_vanishing,
        correction,
        confidence,
        status,
    }
}

fn sample(frame: &LuminanceFrame, x: u32, y: u32) -> f32 {
    let index = usize::try_from(y).expect("u32 fits usize")
        * usize::try_from(frame.dimensions.width()).expect("u32 fits usize")
        + usize::try_from(x).expect("u32 fits usize");
    frame.pixels[index]
}

fn line_key(line: &DetectedLine) -> (u8, u64, u64, u64, u64) {
    let start = line.segment.start();
    let end = line.segment.end();
    (
        match line.kind {
            LineKind::Horizontal => 0,
            LineKind::Vertical => 1,
            LineKind::Irrelevant => 2,
        },
        start.x().to_bits(),
        start.y().to_bits(),
        end.x().to_bits(),
        end.y().to_bits(),
    )
}

fn vanishing_point(lines: &[&DetectedLine]) -> Option<Point> {
    if lines.len() < 2 {
        return None;
    }
    let mut candidates = Vec::new();
    for (index, first) in lines.iter().enumerate() {
        for second in lines.iter().skip(index + 1) {
            if let Some(point) = intersect((*first).segment, (*second).segment) {
                candidates.push((point, first.weight + second.weight));
            }
        }
    }
    if candidates.is_empty() {
        return None;
    }
    candidates.sort_by(|left, right| {
        left.0
            .x()
            .total_cmp(&right.0.x())
            .then_with(|| left.0.y().total_cmp(&right.0.y()))
    });
    let total: f64 = candidates.iter().map(|candidate| candidate.1).sum();
    let mut accumulated = 0.0;
    let median = candidates
        .iter()
        .find(|candidate| {
            accumulated += candidate.1;
            accumulated >= total * 0.5
        })
        .map(|candidate| candidate.0)?;
    Some(median)
}

fn intersect(first: LineSegment, second: LineSegment) -> Option<Point> {
    let (x1, y1, x2, y2) = (
        first.start().x(),
        first.start().y(),
        first.end().x(),
        first.end().y(),
    );
    let (x3, y3, x4, y4) = (
        second.start().x(),
        second.start().y(),
        second.end().x(),
        second.end().y(),
    );
    let denominator = (x1 - x2) * (y3 - y4) - (y1 - y2) * (x3 - x4);
    if denominator.abs() <= 1.0e-9 {
        return None;
    }
    let x = ((x1 * y2 - y1 * x2) * (x3 - x4) - (x1 - x2) * (x3 * y4 - y3 * x4)) / denominator;
    let y = ((x1 * y2 - y1 * x2) * (y3 - y4) - (y1 - y2) * (x3 * y4 - y3 * x4)) / denominator;
    Point::new(x, y).validate().ok().map(|()| Point::new(x, y))
}

fn rectification(vertical: Point, horizontal: Point) -> Option<Homography> {
    let line = [
        vertical.y() - horizontal.y(),
        horizontal.x() - vertical.x(),
        vertical.x() * horizontal.y() - vertical.y() * horizontal.x(),
    ];
    if line[2].abs() <= 1.0e-9 {
        return None;
    }
    let coefficients = [
        1.0,
        0.0,
        0.0,
        0.0,
        1.0,
        0.0,
        line[0] / line[2],
        line[1] / line[2],
        1.0,
    ];
    Homography::new(coefficients).ok()
}

fn confidence(vertical: usize, horizontal: usize, has_correction: bool) -> f64 {
    if !has_correction {
        return 0.0;
    }
    let count_score = (vertical.min(8) as f64 / 8.0) * (horizontal.min(8) as f64 / 8.0);
    count_score.sqrt().min(1.0)
}
