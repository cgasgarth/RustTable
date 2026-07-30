#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::missing_errors_doc,
    clippy::must_use_candidate,
    clippy::return_self_not_must_use
)]
mod pixel;

use crate::{FiniteF32, LinearRgb, RasterDimensions};
pub use pixel::RetouchPixel;
use rusttable_masks::MaskRaster;
use sha2::{Digest, Sha256};
use std::fmt;
pub const RETOUCH_SCHEMA_VERSION: u16 = 1;
pub const RETOUCH_MAX_SCALES: u8 = 8;
const WAVELET_KERNEL: [f32; 5] = [1.0 / 16.0, 4.0 / 16.0, 6.0 / 16.0, 4.0 / 16.0, 1.0 / 16.0];
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RetouchParameters {
    num_scales: u8,
    tile_edge: u32,
    memory_budget: u64,
}
impl RetouchParameters {
    #[must_use]
    pub const fn new(num_scales: u8) -> Self {
        Self {
            num_scales,
            tile_edge: 256,
            memory_budget: 512 * 1024 * 1024,
        }
    }
    #[must_use]
    pub const fn with_tile_edge(mut self, value: u32) -> Self {
        self.tile_edge = value;
        self
    }
    #[must_use]
    pub const fn with_memory_budget(mut self, value: u64) -> Self {
        self.memory_budget = value;
        self
    }
    #[must_use]
    pub const fn num_scales(self) -> u8 {
        self.num_scales
    }
    #[must_use]
    pub const fn tile_edge(self) -> u32 {
        self.tile_edge
    }
    #[must_use]
    pub const fn memory_budget(self) -> u64 {
        self.memory_budget
    }
    #[must_use]
    pub fn config(self) -> RetouchConfig {
        RetouchConfig::new()
            .with_num_scales(self.num_scales)
            .with_tile_edge(self.tile_edge)
            .with_memory_budget(usize::try_from(self.memory_budget).unwrap_or(usize::MAX))
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RetouchAlgorithm {
    Clone,
    Heal,
    Blur,
    Fill,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RetouchBlurType {
    Gaussian,
    Bilateral,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RetouchFillMode {
    Erase,
    Color,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RetouchScale {
    Original,
    Wavelet(u8),
    Residual,
}
impl RetouchScale {
    pub const fn applies_to(self, index: usize, scales: usize) -> bool {
        match self {
            Self::Original => false,
            Self::Wavelet(scale) => index == scale.saturating_sub(1) as usize,
            Self::Residual => index == scales,
        }
    }
}
#[derive(Debug, Clone, PartialEq)]
pub struct RetouchForm {
    id: u128,
    scale: RetouchScale,
    algorithm: RetouchAlgorithm,
    mask: MaskRaster,
    source_offset: (i32, i32),
    strength: f32,
    blur_type: RetouchBlurType,
    blur_radius: f32,
    fill_mode: RetouchFillMode,
    fill_color: RetouchPixel,
    fill_brightness: f32,
    heal_iterations: u16,
}
impl RetouchForm {
    pub fn new(
        id: u128,
        scale: RetouchScale,
        algorithm: RetouchAlgorithm,
        mask: MaskRaster,
    ) -> Result<Self, RetouchConfigError> {
        if id == 0 {
            return Err(RetouchConfigError::InvalidFormId);
        }
        Ok(Self {
            id,
            scale,
            algorithm,
            mask,
            source_offset: (0, 0),
            strength: 1.0,
            blur_type: RetouchBlurType::Gaussian,
            blur_radius: 10.0,
            fill_mode: RetouchFillMode::Erase,
            fill_color: RetouchPixel::new(0.0, 0.0, 0.0),
            fill_brightness: 0.0,
            heal_iterations: 200,
        })
    }
    pub const fn id(&self) -> u128 {
        self.id
    }
    pub const fn scale(&self) -> RetouchScale {
        self.scale
    }
    pub const fn algorithm(&self) -> RetouchAlgorithm {
        self.algorithm
    }
    #[must_use]
    pub fn mask(&self) -> &MaskRaster {
        &self.mask
    }
    #[must_use]
    pub const fn source_offset(&self) -> (i32, i32) {
        self.source_offset
    }
    #[must_use]
    pub const fn strength(&self) -> f32 {
        self.strength
    }
    #[must_use]
    pub const fn blur_type(&self) -> RetouchBlurType {
        self.blur_type
    }
    #[must_use]
    pub const fn blur_radius(&self) -> f32 {
        self.blur_radius
    }
    #[must_use]
    pub const fn fill_mode(&self) -> RetouchFillMode {
        self.fill_mode
    }
    #[must_use]
    pub const fn fill_color(&self) -> RetouchPixel {
        self.fill_color
    }
    #[must_use]
    pub const fn fill_brightness(&self) -> f32 {
        self.fill_brightness
    }
    #[must_use]
    pub const fn heal_iterations(&self) -> u16 {
        self.heal_iterations
    }
    #[must_use]
    pub const fn with_source_offset(mut self, x: i32, y: i32) -> Self {
        self.source_offset = (x, y);
        self
    }
    pub fn with_strength(mut self, value: f32) -> Result<Self, RetouchConfigError> {
        validate_unit(value)?;
        self.strength = value;
        Ok(self)
    }
    pub fn with_blur(
        mut self,
        blur_type: RetouchBlurType,
        radius: f32,
    ) -> Result<Self, RetouchConfigError> {
        if !radius.is_finite() || radius <= 0.0 || radius > 200.0 {
            return Err(RetouchConfigError::InvalidRadius);
        }
        self.blur_type = blur_type;
        self.blur_radius = radius;
        Ok(self)
    }
    pub fn with_fill(
        mut self,
        mode: RetouchFillMode,
        color: RetouchPixel,
        brightness: f32,
    ) -> Result<Self, RetouchConfigError> {
        if !brightness.is_finite() || !color.channels.iter().all(|value| value.is_finite()) {
            return Err(RetouchConfigError::NonFiniteParameter);
        }
        self.fill_mode = mode;
        self.fill_color = color;
        self.fill_brightness = brightness;
        Ok(self)
    }
    #[must_use]
    pub const fn with_heal_iterations(mut self, iterations: u16) -> Self {
        self.heal_iterations = iterations;
        self
    }
}
#[derive(Debug, Clone, PartialEq)]
pub struct RetouchConfig {
    num_scales: u8,
    merge_from_scale: u8,
    merge_to_scale: u8,
    display_wavelet_scale: Option<u8>,
    forms: Vec<RetouchForm>,
    tile_edge: u32,
    memory_budget: usize,
}
impl Default for RetouchConfig {
    fn default() -> Self {
        Self {
            num_scales: 0,
            merge_from_scale: 0,
            merge_to_scale: 0,
            display_wavelet_scale: None,
            forms: Vec::new(),
            tile_edge: 256,
            memory_budget: 512 * 1024 * 1024,
        }
    }
}
impl RetouchConfig {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
    #[must_use]
    pub const fn with_num_scales(mut self, value: u8) -> Self {
        self.num_scales = value;
        self
    }
    #[must_use]
    pub const fn with_merge(mut self, from: u8, to: u8) -> Self {
        self.merge_from_scale = from;
        self.merge_to_scale = to;
        self
    }
    #[must_use]
    pub const fn with_display_wavelet_scale(mut self, value: Option<u8>) -> Self {
        self.display_wavelet_scale = value;
        self
    }
    #[must_use]
    pub fn with_form(mut self, form: RetouchForm) -> Self {
        self.forms.push(form);
        self
    }
    #[must_use]
    pub const fn with_tile_edge(mut self, value: u32) -> Self {
        self.tile_edge = value;
        self
    }
    #[must_use]
    pub const fn with_memory_budget(mut self, value: usize) -> Self {
        self.memory_budget = value;
        self
    }
    #[must_use]
    pub const fn num_scales(&self) -> u8 {
        self.num_scales
    }
    #[must_use]
    pub fn forms(&self) -> &[RetouchForm] {
        &self.forms
    }
    #[must_use]
    pub const fn tile_edge(&self) -> u32 {
        self.tile_edge
    }
    #[must_use]
    pub const fn memory_budget(&self) -> usize {
        self.memory_budget
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetouchConfigError {
    InvalidFormId,
    InvalidScale,
    InvalidMerge,
    InvalidTileEdge,
    InvalidRadius,
    InvalidStrength,
    NonFiniteParameter,
    MaskDimensionsMismatch,
    DuplicateFormId,
}
impl fmt::Display for RetouchConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidFormId => "retouch form IDs must be nonzero",
            Self::InvalidScale => "retouch wavelet scale is unsupported for these dimensions",
            Self::InvalidMerge => "retouch merge range is invalid",
            Self::InvalidTileEdge => "retouch tile edge must be nonzero",
            Self::InvalidRadius => "retouch radius must be finite and in 0..=200",
            Self::InvalidStrength => "retouch strength must be finite in 0..=1",
            Self::NonFiniteParameter => "retouch parameter is non-finite",
            Self::MaskDimensionsMismatch => "retouch form mask dimensions do not match the image",
            Self::DuplicateFormId => "retouch form IDs must be unique",
        })
    }
}
impl std::error::Error for RetouchConfigError {}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetouchExecutionError {
    Config(RetouchConfigError),
    ArithmeticOverflow,
    MemoryBudgetExceeded { required: usize, budget: usize },
    DimensionsMismatch { expected: usize, actual: usize },
    Cancelled,
    NonFiniteInput { pixel: usize, channel: usize },
    NonFiniteResult { pixel: usize, channel: usize },
}
impl fmt::Display for RetouchExecutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Config(error) => error.fmt(formatter),
            Self::ArithmeticOverflow => formatter.write_str("retouch arithmetic overflowed"),
            Self::MemoryBudgetExceeded { required, budget } => write!(
                formatter,
                "retouch requires {required} bytes, budget is {budget}"
            ),
            Self::DimensionsMismatch { expected, actual } => write!(
                formatter,
                "retouch expected {expected} pixels, got {actual}"
            ),
            Self::Cancelled => formatter.write_str("retouch execution was cancelled"),
            Self::NonFiniteInput { pixel, channel } => write!(
                formatter,
                "retouch input pixel {pixel} channel {channel} is non-finite"
            ),
            Self::NonFiniteResult { pixel, channel } => write!(
                formatter,
                "retouch result pixel {pixel} channel {channel} is non-finite"
            ),
        }
    }
}
impl std::error::Error for RetouchExecutionError {}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RetouchReceipt {
    scales: u8,
    tile_count: u32,
    memory_bytes: usize,
    forms: u32,
    identity: [u8; 32],
}
impl RetouchReceipt {
    #[must_use]
    pub const fn scales(self) -> u8 {
        self.scales
    }
    #[must_use]
    pub const fn tile_count(self) -> u32 {
        self.tile_count
    }
    #[must_use]
    pub const fn memory_bytes(self) -> usize {
        self.memory_bytes
    }
    #[must_use]
    pub const fn form_count(self) -> u32 {
        self.forms
    }
    #[must_use]
    pub const fn identity(self) -> [u8; 32] {
        self.identity
    }
}
#[derive(Debug, Clone)]
pub struct RetouchPlan {
    config: RetouchConfig,
    dimensions: RasterDimensions,
    scales: u8,
    memory_bytes: usize,
    identity: [u8; 32],
}
impl RetouchPlan {
    pub fn new(
        config: RetouchConfig,
        dimensions: RasterDimensions,
    ) -> Result<Self, RetouchExecutionError> {
        if config.tile_edge == 0 {
            return Err(RetouchExecutionError::Config(
                RetouchConfigError::InvalidTileEdge,
            ));
        }
        let maximum = compatible_scale_count(dimensions);
        let scales = if config.num_scales == 0 {
            maximum
        } else {
            config.num_scales
        };
        if scales > RETOUCH_MAX_SCALES || scales > maximum {
            return Err(RetouchExecutionError::Config(
                RetouchConfigError::InvalidScale,
            ));
        }
        if config.merge_from_scale > config.merge_to_scale
            || config.merge_to_scale > scales.saturating_add(1)
        {
            return Err(RetouchExecutionError::Config(
                RetouchConfigError::InvalidMerge,
            ));
        }
        if config
            .display_wavelet_scale
            .is_some_and(|scale| scale > scales.saturating_add(1))
        {
            return Err(RetouchExecutionError::Config(
                RetouchConfigError::InvalidScale,
            ));
        }
        let count = usize::try_from(dimensions.pixel_count())
            .map_err(|_| RetouchExecutionError::ArithmeticOverflow)?;
        let levels = usize::from(scales)
            .checked_add(2)
            .ok_or(RetouchExecutionError::ArithmeticOverflow)?;
        let memory_bytes = count
            .checked_mul(levels)
            .and_then(|value| value.checked_mul(3))
            .and_then(|value| value.checked_mul(std::mem::size_of::<f32>()))
            .ok_or(RetouchExecutionError::ArithmeticOverflow)?;
        if memory_bytes > config.memory_budget {
            return Err(RetouchExecutionError::MemoryBudgetExceeded {
                required: memory_bytes,
                budget: config.memory_budget,
            });
        }
        let mut form_ids = std::collections::BTreeSet::new();
        for form in &config.forms {
            if form.mask.width() != dimensions.width() || form.mask.height() != dimensions.height()
            {
                return Err(RetouchExecutionError::Config(
                    RetouchConfigError::MaskDimensionsMismatch,
                ));
            }
            if !form_ids.insert(form.id) {
                return Err(RetouchExecutionError::Config(
                    RetouchConfigError::DuplicateFormId,
                ));
            }
            if let RetouchScale::Wavelet(scale) = form.scale
                && (scale == 0 || scale > scales)
            {
                return Err(RetouchExecutionError::Config(
                    RetouchConfigError::InvalidScale,
                ));
            }
        }
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"rusttable.retouch.plan.v1");
        bytes.push(scales);
        bytes.extend_from_slice(&dimensions.width().to_le_bytes());
        bytes.extend_from_slice(&dimensions.height().to_le_bytes());
        bytes.push(config.merge_from_scale);
        bytes.push(config.merge_to_scale);
        bytes.push(config.display_wavelet_scale.unwrap_or(u8::MAX));
        bytes.extend_from_slice(&config.tile_edge.to_le_bytes());
        bytes.extend_from_slice(&config.memory_budget.to_le_bytes());
        for form in &config.forms {
            bytes.extend_from_slice(&form.id.to_le_bytes());
            bytes.extend_from_slice(&form.mask.identity());
        }
        Ok(Self {
            config,
            dimensions,
            scales,
            memory_bytes,
            identity: Sha256::digest(bytes).into(),
        })
    }
    #[must_use]
    pub const fn scales(&self) -> u8 {
        self.scales
    }
    #[must_use]
    pub const fn memory_bytes(&self) -> usize {
        self.memory_bytes
    }
    #[must_use]
    pub const fn identity(&self) -> [u8; 32] {
        self.identity
    }
    pub fn execute(
        &self,
        input: &[RetouchPixel],
        is_cancelled: impl Fn() -> bool,
        mut progress: impl FnMut(f32),
    ) -> Result<(Vec<RetouchPixel>, RetouchReceipt), RetouchExecutionError> {
        let expected = usize::try_from(self.dimensions.pixel_count())
            .map_err(|_| RetouchExecutionError::ArithmeticOverflow)?;
        if input.len() != expected {
            return Err(RetouchExecutionError::DimensionsMismatch {
                expected,
                actual: input.len(),
            });
        }
        validate_pixels(input)?;
        if is_cancelled() {
            return Err(RetouchExecutionError::Cancelled);
        }
        let mut original = input.to_vec();
        for form in &self.config.forms {
            if matches!(form.scale, RetouchScale::Original) {
                apply_form(
                    &mut original,
                    form,
                    self.dimensions,
                    self.config.tile_edge,
                    &is_cancelled,
                )?;
            }
        }
        let mut levels = decompose(
            &original,
            self.dimensions,
            self.scales,
            &is_cancelled,
            &mut progress,
        )?;
        merge_levels(
            &mut levels,
            self.config.merge_from_scale,
            self.config.merge_to_scale,
            &is_cancelled,
        )?;
        let steps = self
            .config
            .forms
            .len()
            .saturating_add(usize::from(self.scales))
            .max(1);
        let mut completed = 0usize;
        for form in &self.config.forms {
            if is_cancelled() {
                return Err(RetouchExecutionError::Cancelled);
            }
            if matches!(form.scale, RetouchScale::Original) {
                continue;
            }
            for (index, level) in levels.iter_mut().enumerate() {
                if form.scale.applies_to(index, usize::from(self.scales)) {
                    apply_form(
                        level,
                        form,
                        self.dimensions,
                        self.config.tile_edge,
                        &is_cancelled,
                    )?;
                }
            }
            completed = completed.saturating_add(1);
            progress(completed as f32 / steps as f32);
        }
        let output = reconstruct(&levels, self.dimensions, &is_cancelled)?;
        validate_output(&output)?;
        progress(1.0);
        let tile_count = tile_count(self.dimensions, self.config.tile_edge)?;
        Ok((
            output,
            RetouchReceipt {
                scales: self.scales,
                tile_count,
                memory_bytes: self.memory_bytes,
                forms: self.config.forms.len() as u32,
                identity: self.identity,
            },
        ))
    }

    pub fn execute_linear_rgb(
        &self,
        pixels: &mut [LinearRgb],
        is_cancelled: impl Fn() -> bool,
        progress: impl FnMut(f32),
    ) -> Result<RetouchReceipt, RetouchExecutionError> {
        let input = pixels
            .iter()
            .copied()
            .map(|pixel| {
                RetouchPixel::new(pixel.red().get(), pixel.green().get(), pixel.blue().get())
            })
            .collect::<Vec<_>>();
        let (output, receipt) = self.execute(&input, is_cancelled, progress)?;
        for (destination, source) in pixels.iter_mut().zip(output) {
            let channels = source.channels();
            *destination = LinearRgb::new(
                FiniteF32::new(channels[0]).map_err(|_| {
                    RetouchExecutionError::NonFiniteResult {
                        pixel: 0,
                        channel: 0,
                    }
                })?,
                FiniteF32::new(channels[1]).map_err(|_| {
                    RetouchExecutionError::NonFiniteResult {
                        pixel: 0,
                        channel: 1,
                    }
                })?,
                FiniteF32::new(channels[2]).map_err(|_| {
                    RetouchExecutionError::NonFiniteResult {
                        pixel: 0,
                        channel: 2,
                    }
                })?,
            );
        }
        Ok(receipt)
    }
}
fn validate_unit(value: f32) -> Result<(), RetouchConfigError> {
    if value.is_finite() && (0.0..=1.0).contains(&value) {
        Ok(())
    } else {
        Err(RetouchConfigError::InvalidStrength)
    }
}
fn compatible_scale_count(dimensions: RasterDimensions) -> u8 {
    let mut edge = dimensions.width().min(dimensions.height());
    let mut scales = 0u8;
    while edge >= 4 && scales < RETOUCH_MAX_SCALES {
        edge /= 2;
        scales = scales.saturating_add(1);
    }
    scales
}
fn tile_count(dimensions: RasterDimensions, tile_edge: u32) -> Result<u32, RetouchExecutionError> {
    let x = dimensions
        .width()
        .checked_add(tile_edge - 1)
        .ok_or(RetouchExecutionError::ArithmeticOverflow)?
        / tile_edge;
    let y = dimensions
        .height()
        .checked_add(tile_edge - 1)
        .ok_or(RetouchExecutionError::ArithmeticOverflow)?
        / tile_edge;
    x.checked_mul(y)
        .ok_or(RetouchExecutionError::ArithmeticOverflow)
}
fn validate_pixels(pixels: &[RetouchPixel]) -> Result<(), RetouchExecutionError> {
    for (pixel, value) in pixels.iter().enumerate() {
        for (channel, component) in value.channels.iter().enumerate() {
            if !component.is_finite() {
                return Err(RetouchExecutionError::NonFiniteInput { pixel, channel });
            }
        }
    }
    Ok(())
}
fn validate_output(pixels: &[RetouchPixel]) -> Result<(), RetouchExecutionError> {
    for (pixel, value) in pixels.iter().enumerate() {
        for (channel, component) in value.channels.iter().enumerate() {
            if !component.is_finite() {
                return Err(RetouchExecutionError::NonFiniteResult { pixel, channel });
            }
        }
    }
    Ok(())
}
fn merge_levels(
    levels: &mut [Vec<RetouchPixel>],
    from: u8,
    to: u8,
    is_cancelled: &impl Fn() -> bool,
) -> Result<(), RetouchExecutionError> {
    if from == 0 {
        return Ok(());
    }
    let target = usize::from(from - 1);
    for source_index in (target + 1)..=usize::from(to - 1) {
        if is_cancelled() {
            return Err(RetouchExecutionError::Cancelled);
        }
        let source = std::mem::take(&mut levels[source_index]);
        for (destination, value) in levels[target].iter_mut().zip(source) {
            *destination = destination.add(value);
        }
    }
    Ok(())
}
fn decompose(
    input: &[RetouchPixel],
    dimensions: RasterDimensions,
    scales: u8,
    is_cancelled: &impl Fn() -> bool,
    progress: &mut impl FnMut(f32),
) -> Result<Vec<Vec<RetouchPixel>>, RetouchExecutionError> {
    let mut current = input.to_vec();
    let mut levels = Vec::with_capacity(usize::from(scales).saturating_add(2));
    for scale in 1..=scales {
        if is_cancelled() {
            return Err(RetouchExecutionError::Cancelled);
        }
        let low = atrous_blur(&current, dimensions, scale)?;
        levels.push(
            current
                .iter()
                .zip(&low)
                .map(|(high, low)| high.sub(*low))
                .collect(),
        );
        current = low;
        progress(f32::from(scale) / (f32::from(scales) + 1.0));
    }
    levels.push(current);
    Ok(levels)
}
fn reconstruct(
    levels: &[Vec<RetouchPixel>],
    dimensions: RasterDimensions,
    is_cancelled: &impl Fn() -> bool,
) -> Result<Vec<RetouchPixel>, RetouchExecutionError> {
    let mut output = vec![
        RetouchPixel::new(0.0, 0.0, 0.0);
        usize::try_from(dimensions.pixel_count())
            .map_err(|_| RetouchExecutionError::ArithmeticOverflow)?
    ];
    for (index, level) in levels.iter().enumerate() {
        if is_cancelled() {
            return Err(RetouchExecutionError::Cancelled);
        }
        for (destination, value) in output.iter_mut().zip(level) {
            *destination = destination.add(*value);
        }
        if index == levels.len().saturating_sub(1) {
            break;
        }
    }
    Ok(output)
}
fn atrous_blur(
    input: &[RetouchPixel],
    dimensions: RasterDimensions,
    scale: u8,
) -> Result<Vec<RetouchPixel>, RetouchExecutionError> {
    let width = usize::try_from(dimensions.width())
        .map_err(|_| RetouchExecutionError::ArithmeticOverflow)?;
    let height = usize::try_from(dimensions.height())
        .map_err(|_| RetouchExecutionError::ArithmeticOverflow)?;
    let step = 1usize
        .checked_shl(u32::from(scale.saturating_sub(1)))
        .ok_or(RetouchExecutionError::ArithmeticOverflow)?;
    let mut horizontal = vec![RetouchPixel::new(0.0, 0.0, 0.0); input.len()];
    for y in 0..height {
        for x in 0..width {
            let mut value = RetouchPixel::new(0.0, 0.0, 0.0);
            for (kernel, coefficient) in WAVELET_KERNEL.iter().enumerate() {
                let offset = kernel as isize - 2;
                let sample_x = (x as isize + offset * step as isize)
                    .clamp(0, width.saturating_sub(1) as isize)
                    as usize;
                value = value.add(input[y * width + sample_x].scale(*coefficient));
            }
            horizontal[y * width + x] = value;
        }
    }
    let mut output = vec![RetouchPixel::new(0.0, 0.0, 0.0); input.len()];
    for y in 0..height {
        for x in 0..width {
            let mut value = RetouchPixel::new(0.0, 0.0, 0.0);
            for (kernel, coefficient) in WAVELET_KERNEL.iter().enumerate() {
                let offset = kernel as isize - 2;
                let sample_y = (y as isize + offset * step as isize)
                    .clamp(0, height.saturating_sub(1) as isize)
                    as usize;
                value = value.add(horizontal[sample_y * width + x].scale(*coefficient));
            }
            output[y * width + x] = value;
        }
    }
    Ok(output)
}
fn apply_form(
    level: &mut [RetouchPixel],
    form: &RetouchForm,
    dimensions: RasterDimensions,
    tile_edge: u32,
    is_cancelled: &impl Fn() -> bool,
) -> Result<(), RetouchExecutionError> {
    let source = level.to_vec();
    let candidate = match form.algorithm {
        RetouchAlgorithm::Clone => source.clone(),
        RetouchAlgorithm::Heal => heal(&source, form, dimensions, is_cancelled)?,
        RetouchAlgorithm::Blur => blur(&source, form, dimensions)?,
        RetouchAlgorithm::Fill => fill(&source, form),
    };
    let width = usize::try_from(dimensions.width())
        .map_err(|_| RetouchExecutionError::ArithmeticOverflow)?;
    let tile_edge =
        usize::try_from(tile_edge).map_err(|_| RetouchExecutionError::ArithmeticOverflow)?;
    let height = usize::try_from(dimensions.height())
        .map_err(|_| RetouchExecutionError::ArithmeticOverflow)?;
    for tile_y in (0..height).step_by(tile_edge) {
        for tile_x in (0..width).step_by(tile_edge) {
            let end_y = tile_y.saturating_add(tile_edge).min(height);
            let end_x = tile_x.saturating_add(tile_edge).min(width);
            for y in tile_y..end_y {
                for x in tile_x..end_x {
                    if is_cancelled() {
                        return Err(RetouchExecutionError::Cancelled);
                    }
                    let index = y * width + x;
                    let mask = form.mask.values()[index] * form.strength;
                    if mask.to_bits() == 0 {
                        continue;
                    }
                    let source_index =
                        offset_index(index, width, dimensions.height(), form.source_offset)?;
                    let replacement = match form.algorithm {
                        RetouchAlgorithm::Clone | RetouchAlgorithm::Heal => candidate[source_index],
                        RetouchAlgorithm::Blur | RetouchAlgorithm::Fill => candidate[index],
                    };
                    level[index] = level[index].mix(replacement, mask);
                }
            }
        }
    }
    Ok(())
}
fn offset_index(
    index: usize,
    width: usize,
    height: u32,
    offset: (i32, i32),
) -> Result<usize, RetouchExecutionError> {
    let x = index % width;
    let y = index / width;
    let target_x =
        (x as i64 + i64::from(offset.0)).clamp(0, width.saturating_sub(1) as i64) as usize;
    let target_y = (y as i64 + i64::from(offset.1))
        .clamp(0, u64::from(height).saturating_sub(1) as i64) as usize;
    target_y
        .checked_mul(width)
        .and_then(|value| value.checked_add(target_x))
        .ok_or(RetouchExecutionError::ArithmeticOverflow)
}
fn blur(
    input: &[RetouchPixel],
    form: &RetouchForm,
    dimensions: RasterDimensions,
) -> Result<Vec<RetouchPixel>, RetouchExecutionError> {
    let radius = form.blur_radius.ceil() as usize;
    if radius == 0 {
        return Ok(input.to_vec());
    }
    let width = usize::try_from(dimensions.width())
        .map_err(|_| RetouchExecutionError::ArithmeticOverflow)?;
    let height = usize::try_from(dimensions.height())
        .map_err(|_| RetouchExecutionError::ArithmeticOverflow)?;
    let mut output = vec![RetouchPixel::new(0.0, 0.0, 0.0); input.len()];
    let sigma = (form.blur_radius / 3.0).max(0.1);
    for y in 0..height {
        for x in 0..width {
            let mut sum = RetouchPixel::new(0.0, 0.0, 0.0);
            let mut weights = 0.0;
            for dy in -(radius as isize)..=(radius as isize) {
                for dx in -(radius as isize)..=(radius as isize) {
                    let sx = (x as isize + dx).clamp(0, width.saturating_sub(1) as isize) as usize;
                    let sy = (y as isize + dy).clamp(0, height.saturating_sub(1) as isize) as usize;
                    let distance = (dx * dx + dy * dy) as f32;
                    let mut weight = (-distance / (2.0 * sigma * sigma)).exp();
                    if form.blur_type == RetouchBlurType::Bilateral {
                        let delta = input[sy * width + sx].sub(input[y * width + x]);
                        let colour = delta.channels[0] * delta.channels[0]
                            + delta.channels[1] * delta.channels[1]
                            + delta.channels[2] * delta.channels[2];
                        weight *= (-colour / 0.08).exp();
                    }
                    sum = sum.add(input[sy * width + sx].scale(weight));
                    weights += weight;
                }
            }
            output[y * width + x] = sum.scale(1.0 / weights.max(f32::MIN_POSITIVE));
        }
    }
    Ok(output)
}
fn heal(
    input: &[RetouchPixel],
    form: &RetouchForm,
    dimensions: RasterDimensions,
    is_cancelled: &impl Fn() -> bool,
) -> Result<Vec<RetouchPixel>, RetouchExecutionError> {
    let width = usize::try_from(dimensions.width())
        .map_err(|_| RetouchExecutionError::ArithmeticOverflow)?;
    let height = usize::try_from(dimensions.height())
        .map_err(|_| RetouchExecutionError::ArithmeticOverflow)?;
    let mut output = input.to_vec();
    let iterations = usize::from(form.heal_iterations.max(1));
    for _ in 0..iterations.min(2_000) {
        if is_cancelled() {
            return Err(RetouchExecutionError::Cancelled);
        }
        let previous = output.clone();
        for y in 0..height {
            for x in 0..width {
                let index = y * width + x;
                let source_index =
                    offset_index(index, width, dimensions.height(), form.source_offset)?;
                let neighbours = [
                    previous[y * width + x.saturating_sub(1)],
                    previous[y * width + x.saturating_add(1).min(width.saturating_sub(1))],
                    previous[y.saturating_sub(1) * width + x],
                    previous[y.saturating_add(1).min(height.saturating_sub(1)) * width + x],
                ];
                let average = neighbours
                    .iter()
                    .copied()
                    .fold(RetouchPixel::new(0.0, 0.0, 0.0), RetouchPixel::add)
                    .scale(0.25);
                let source_average = input[source_index];
                output[index] = input[source_index].add(average.sub(source_average));
            }
        }
    }
    Ok(output)
}
fn fill(input: &[RetouchPixel], form: &RetouchForm) -> Vec<RetouchPixel> {
    match form.fill_mode {
        RetouchFillMode::Erase => vec![RetouchPixel::new(0.0, 0.0, 0.0); input.len()],
        RetouchFillMode::Color => {
            let [red, green, blue] = form.fill_color.channels;
            vec![
                RetouchPixel::new(
                    red + form.fill_brightness,
                    green + form.fill_brightness,
                    blue + form.fill_brightness
                );
                input.len()
            ]
        }
    }
}
impl From<RetouchConfigError> for RetouchExecutionError {
    fn from(error: RetouchConfigError) -> Self {
        Self::Config(error)
    }
}
