use super::{SPOTS_MAX_ENTRIES, SpotsMode, SpotsParametersV2};
use crate::{FiniteF32, LinearRgb, RasterDimensions};
use rusttable_image::{ImageDimensions, Roi};
use rusttable_masks::MaskRaster;
use sha2::{Digest, Sha256};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SpotsFormKind {
    Circle,
    Ellipse,
    Path,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SpotsForm {
    id: u32,
    kind: SpotsFormKind,
    mask: MaskRaster,
}

impl SpotsForm {
    pub fn from_mask(
        id: u32,
        mask: MaskRaster,
        kind: SpotsFormKind,
    ) -> Result<Self, SpotsExecutionError> {
        if id == 0 {
            return Err(SpotsExecutionError::InvalidFormId);
        }
        Ok(Self { id, kind, mask })
    }

    pub fn circle(
        id: u32,
        dimensions: RasterDimensions,
        center: (f32, f32),
        radius: f32,
    ) -> Result<Self, SpotsExecutionError> {
        if !center.0.is_finite() || !center.1.is_finite() || !radius.is_finite() || radius < 0.0 {
            return Err(SpotsExecutionError::InvalidGeometry);
        }
        let values = mask_values(dimensions, |x, y| {
            let distance = (x - center.0).hypot(y - center.1);
            smooth_disk(distance, radius)
        })?;
        Self::from_mask(
            id,
            MaskRaster::new(dimensions.width(), dimensions.height(), values)
                .map_err(SpotsExecutionError::Mask)?,
            SpotsFormKind::Circle,
        )
    }

    pub fn ellipse(
        id: u32,
        dimensions: RasterDimensions,
        center: (f32, f32),
        radii: (f32, f32),
    ) -> Result<Self, SpotsExecutionError> {
        if !center.0.is_finite()
            || !center.1.is_finite()
            || !radii.0.is_finite()
            || !radii.1.is_finite()
            || radii.0 <= 0.0
            || radii.1 <= 0.0
        {
            return Err(SpotsExecutionError::InvalidGeometry);
        }
        let values = mask_values(dimensions, |x, y| {
            let distance = ((x - center.0) / radii.0).hypot((y - center.1) / radii.1);
            smooth_disk(distance, 1.0)
        })?;
        Self::from_mask(
            id,
            MaskRaster::new(dimensions.width(), dimensions.height(), values)
                .map_err(SpotsExecutionError::Mask)?,
            SpotsFormKind::Ellipse,
        )
    }

    #[must_use]
    pub const fn id(&self) -> u32 {
        self.id
    }

    #[must_use]
    pub const fn kind(&self) -> SpotsFormKind {
        self.kind
    }

    #[must_use]
    pub fn mask(&self) -> &MaskRaster {
        &self.mask
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SpotsEntry {
    form_id: u32,
    mode: SpotsMode,
    source_offset: (i32, i32),
    opacity: FiniteF32,
    hardness: FiniteF32,
    feather: u32,
}

impl SpotsEntry {
    pub fn new(
        form_id: u32,
        mode: SpotsMode,
        source_offset: (i32, i32),
        opacity: f32,
    ) -> Result<Self, SpotsExecutionError> {
        validate_opacity(opacity)?;
        if form_id == 0 {
            return Err(SpotsExecutionError::InvalidFormId);
        }
        if !mode.is_supported() || mode == SpotsMode::None {
            return Err(SpotsExecutionError::UnsupportedMode(mode));
        }
        Ok(Self {
            form_id,
            mode,
            source_offset,
            opacity: FiniteF32::new(opacity).expect("validated finite opacity"),
            hardness: FiniteF32::new(1.0).expect("one is finite"),
            feather: 0,
        })
    }

    pub fn with_form_semantics(
        mut self,
        hardness: f32,
        feather: u32,
    ) -> Result<Self, SpotsExecutionError> {
        validate_opacity(hardness)?;
        self.hardness = FiniteF32::new(hardness).expect("validated finite hardness");
        self.feather = feather;
        Ok(self)
    }

    #[must_use]
    pub const fn form_id(self) -> u32 {
        self.form_id
    }

    #[must_use]
    pub const fn mode(self) -> SpotsMode {
        self.mode
    }

    #[must_use]
    pub const fn source_offset(self) -> (i32, i32) {
        self.source_offset
    }

    #[must_use]
    pub const fn opacity(self) -> f32 {
        self.opacity.get()
    }

    #[must_use]
    pub const fn hardness(self) -> f32 {
        self.hardness.get()
    }

    #[must_use]
    pub const fn feather(self) -> u32 {
        self.feather
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct SpotsConfig {
    forms: Vec<SpotsForm>,
    entries: Vec<SpotsEntry>,
}

impl SpotsConfig {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn with_form(mut self, form: SpotsForm) -> Self {
        self.forms.push(form);
        self
    }

    #[must_use]
    pub fn with_entry(mut self, entry: SpotsEntry) -> Self {
        self.entries.push(entry);
        self
    }

    #[must_use]
    pub fn forms(&self) -> &[SpotsForm] {
        &self.forms
    }

    #[must_use]
    pub fn entries(&self) -> &[SpotsEntry] {
        &self.entries
    }

    pub fn from_parameters(parameters: &SpotsParametersV2) -> Result<Self, SpotsExecutionError> {
        let mut config = Self::new();
        for (form_id, mode) in parameters.ordered_entries() {
            config = config.with_entry(SpotsEntry::new(form_id, mode, (0, 0), 1.0)?);
        }
        Ok(config)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpotsReceipt {
    identity: [u8; 32],
    spot_count: u32,
    destination_roi: Roi,
    source_roi: Roi,
}

impl SpotsReceipt {
    #[must_use]
    pub const fn identity(&self) -> [u8; 32] {
        self.identity
    }

    #[must_use]
    pub const fn spot_count(&self) -> u32 {
        self.spot_count
    }

    #[must_use]
    pub const fn destination_roi(&self) -> Roi {
        self.destination_roi
    }

    #[must_use]
    pub const fn source_roi(&self) -> Roi {
        self.source_roi
    }
}

#[derive(Debug, Clone)]
struct ResolvedEntry {
    entry: SpotsEntry,
    mask: MaskRaster,
    bounds: Bounds,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Bounds {
    left: i64,
    top: i64,
    right: i64,
    bottom: i64,
}

impl Bounds {
    const EMPTY: Self = Self {
        left: i64::MAX,
        top: i64::MAX,
        right: i64::MIN,
        bottom: i64::MIN,
    };

    fn include(self, other: Self) -> Self {
        if self.is_empty() {
            return other;
        }
        if other.is_empty() {
            return self;
        }
        Self {
            left: self.left.min(other.left),
            top: self.top.min(other.top),
            right: self.right.max(other.right),
            bottom: self.bottom.max(other.bottom),
        }
    }

    const fn is_empty(self) -> bool {
        self.left > self.right || self.top > self.bottom
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpotsExecutionError {
    ArithmeticOverflow,
    DimensionsMismatch { expected: usize, actual: usize },
    InvalidFormId,
    DuplicateFormId(u32),
    MissingForm(u32),
    InvalidGeometry,
    InvalidOpacity,
    UnsupportedMode(SpotsMode),
    SourceOutOfBounds { form_id: u32 },
    Mask(rusttable_masks::MaskExecutionError),
    Cancelled,
    NonFiniteResult { pixel: usize, channel: usize },
}

impl fmt::Display for SpotsExecutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ArithmeticOverflow => formatter.write_str("spots arithmetic overflowed"),
            Self::DimensionsMismatch { expected, actual } => {
                write!(formatter, "spots expected {expected} pixels, got {actual}")
            }
            Self::InvalidFormId => formatter.write_str("spots form IDs must be positive"),
            Self::DuplicateFormId(id) => write!(formatter, "spots form ID {id} is duplicated"),
            Self::MissingForm(id) => write!(formatter, "spots form ID {id} is unavailable"),
            Self::InvalidGeometry => formatter.write_str("spots geometry is invalid"),
            Self::InvalidOpacity => {
                formatter.write_str("spots opacity/hardness must be finite in 0..=1")
            }
            Self::UnsupportedMode(mode) => write!(formatter, "spots mode {mode:?} is unsupported"),
            Self::SourceOutOfBounds { form_id } => {
                write!(
                    formatter,
                    "spots source ROI for form {form_id} is outside the input"
                )
            }
            Self::Mask(error) => error.fmt(formatter),
            Self::Cancelled => formatter.write_str("spots execution was cancelled"),
            Self::NonFiniteResult { pixel, channel } => {
                write!(
                    formatter,
                    "spots result pixel {pixel} channel {channel} is non-finite"
                )
            }
        }
    }
}

impl std::error::Error for SpotsExecutionError {}

#[derive(Debug, Clone)]
pub struct SpotsPlan {
    dimensions: RasterDimensions,
    entries: Vec<ResolvedEntry>,
    destination_roi: Roi,
    source_roi: Roi,
    identity: [u8; 32],
}

impl SpotsPlan {
    pub fn new(
        config: SpotsConfig,
        dimensions: RasterDimensions,
    ) -> Result<Self, SpotsExecutionError> {
        let image_dimensions = ImageDimensions::new(dimensions.width(), dimensions.height())
            .map_err(|_| SpotsExecutionError::ArithmeticOverflow)?;
        if config.entries.len() > SPOTS_MAX_ENTRIES {
            return Err(SpotsExecutionError::ArithmeticOverflow);
        }
        let mut forms = std::collections::BTreeMap::new();
        for form in config.forms {
            let form_id = form.id;
            if form.mask.width() != dimensions.width() || form.mask.height() != dimensions.height()
            {
                return Err(SpotsExecutionError::DimensionsMismatch {
                    expected: usize::try_from(dimensions.pixel_count())
                        .map_err(|_| SpotsExecutionError::ArithmeticOverflow)?,
                    actual: usize::try_from(
                        u64::from(form.mask.width()) * u64::from(form.mask.height()),
                    )
                    .map_err(|_| SpotsExecutionError::ArithmeticOverflow)?,
                });
            }
            if forms.insert(form_id, form).is_some() {
                return Err(SpotsExecutionError::DuplicateFormId(form_id));
            }
        }
        let mut resolved = Vec::with_capacity(config.entries.len());
        let mut destination = Bounds::EMPTY;
        let mut source = Bounds::EMPTY;
        for entry in config.entries {
            let form = forms
                .get(&entry.form_id)
                .ok_or(SpotsExecutionError::MissingForm(entry.form_id))?;
            let mask = if entry.feather == 0 {
                form.mask.clone()
            } else {
                form.mask
                    .feather(entry.feather)
                    .map_err(SpotsExecutionError::Mask)?
            };
            let bounds = active_bounds(&mask)?;
            if bounds.is_empty() {
                resolved.push(ResolvedEntry {
                    entry,
                    mask,
                    bounds,
                });
                continue;
            }
            let source_bounds = Bounds {
                left: bounds.left + i64::from(entry.source_offset.0),
                top: bounds.top + i64::from(entry.source_offset.1),
                right: bounds.right + i64::from(entry.source_offset.0),
                bottom: bounds.bottom + i64::from(entry.source_offset.1),
            };
            if source_bounds.left < 0
                || source_bounds.top < 0
                || source_bounds.right > i64::from(dimensions.width())
                || source_bounds.bottom > i64::from(dimensions.height())
            {
                return Err(SpotsExecutionError::SourceOutOfBounds {
                    form_id: entry.form_id,
                });
            }
            destination = destination.include(bounds);
            source = source.include(source_bounds);
            resolved.push(ResolvedEntry {
                entry,
                mask,
                bounds,
            });
        }
        let destination_roi = to_roi(destination, image_dimensions)?;
        let source_roi = to_roi(source, image_dimensions)?;
        let identity = plan_identity(dimensions, &resolved);
        Ok(Self {
            dimensions,
            entries: resolved,
            destination_roi,
            source_roi,
            identity,
        })
    }

    #[must_use]
    pub const fn destination_roi(&self) -> Roi {
        self.destination_roi
    }

    #[must_use]
    pub const fn source_roi(&self) -> Roi {
        self.source_roi
    }

    #[must_use]
    pub const fn identity(&self) -> [u8; 32] {
        self.identity
    }

    pub fn entries(&self) -> impl Iterator<Item = SpotsEntry> + '_ {
        self.entries.iter().map(|entry| entry.entry)
    }

    pub fn execute(
        &self,
        input: &[LinearRgb],
        is_cancelled: impl Fn() -> bool,
    ) -> Result<(Vec<LinearRgb>, SpotsReceipt), SpotsExecutionError> {
        let expected = usize::try_from(self.dimensions.pixel_count())
            .map_err(|_| SpotsExecutionError::ArithmeticOverflow)?;
        if input.len() != expected {
            return Err(SpotsExecutionError::DimensionsMismatch {
                expected,
                actual: input.len(),
            });
        }
        let source = input.to_vec();
        let mut output = source.clone();
        for resolved in &self.entries {
            if is_cancelled() {
                return Err(SpotsExecutionError::Cancelled);
            }
            if resolved.bounds.is_empty() || resolved.entry.opacity().to_bits() == 0 {
                continue;
            }
            let correction = match resolved.entry.mode() {
                SpotsMode::Clone => [0.0; 3],
                SpotsMode::Heal => {
                    heal_correction(&source, &resolved.mask, resolved.entry, self.dimensions)?
                }
                SpotsMode::None => continue,
                SpotsMode::Unknown(mode) => {
                    return Err(SpotsExecutionError::UnsupportedMode(SpotsMode::Unknown(
                        mode,
                    )));
                }
            };
            apply_entry(
                &source,
                &mut output,
                &resolved.mask,
                resolved.entry,
                self.dimensions,
                correction,
                &is_cancelled,
            )?;
        }
        validate_output(&output)?;
        Ok((
            output,
            SpotsReceipt {
                identity: self.identity,
                spot_count: u32::try_from(self.entries.len())
                    .expect("spots entry count is bounded"),
                destination_roi: self.destination_roi,
                source_roi: self.source_roi,
            },
        ))
    }

    pub fn execute_linear_rgb(
        &self,
        pixels: &mut [LinearRgb],
        is_cancelled: impl Fn() -> bool,
    ) -> Result<SpotsReceipt, SpotsExecutionError> {
        let (output, receipt) = self.execute(pixels, is_cancelled)?;
        pixels.copy_from_slice(&output);
        Ok(receipt)
    }
}

fn validate_opacity(value: f32) -> Result<(), SpotsExecutionError> {
    if value.is_finite() && (0.0..=1.0).contains(&value) {
        Ok(())
    } else {
        Err(SpotsExecutionError::InvalidOpacity)
    }
}

fn mask_values(
    dimensions: RasterDimensions,
    mut coverage: impl FnMut(f32, f32) -> f32,
) -> Result<Vec<f32>, SpotsExecutionError> {
    let width =
        usize::try_from(dimensions.width()).map_err(|_| SpotsExecutionError::ArithmeticOverflow)?;
    let height = usize::try_from(dimensions.height())
        .map_err(|_| SpotsExecutionError::ArithmeticOverflow)?;
    let mut values = Vec::with_capacity(
        width
            .checked_mul(height)
            .ok_or(SpotsExecutionError::ArithmeticOverflow)?,
    );
    for y in 0..height {
        for x in 0..width {
            values.push(coverage(x as f32, y as f32).clamp(0.0, 1.0));
        }
    }
    Ok(values)
}

fn smooth_disk(distance: f32, radius: f32) -> f32 {
    if radius == 0.0 {
        return f32::from(u8::from(distance == 0.0));
    }
    let edge = (1.0 - distance / radius).clamp(0.0, 1.0);
    edge * edge * (3.0 - 2.0 * edge)
}

fn active_bounds(mask: &MaskRaster) -> Result<Bounds, SpotsExecutionError> {
    let width =
        usize::try_from(mask.width()).map_err(|_| SpotsExecutionError::ArithmeticOverflow)?;
    let mut bounds = Bounds::EMPTY;
    for (index, value) in mask.values().iter().copied().enumerate() {
        if value.to_bits() == 0 {
            continue;
        }
        let x =
            i64::try_from(index % width).map_err(|_| SpotsExecutionError::ArithmeticOverflow)?;
        let y =
            i64::try_from(index / width).map_err(|_| SpotsExecutionError::ArithmeticOverflow)?;
        bounds = bounds.include(Bounds {
            left: x,
            top: y,
            right: x + 1,
            bottom: y + 1,
        });
    }
    Ok(bounds)
}

fn to_roi(bounds: Bounds, dimensions: ImageDimensions) -> Result<Roi, SpotsExecutionError> {
    if bounds.is_empty() {
        return Roi::new(0, 0, 0, 0).map_err(|_| SpotsExecutionError::ArithmeticOverflow);
    }
    let left =
        u32::try_from(bounds.left.max(0)).map_err(|_| SpotsExecutionError::ArithmeticOverflow)?;
    let top =
        u32::try_from(bounds.top.max(0)).map_err(|_| SpotsExecutionError::ArithmeticOverflow)?;
    let right = u32::try_from(bounds.right.min(i64::from(dimensions.width())))
        .map_err(|_| SpotsExecutionError::ArithmeticOverflow)?;
    let bottom = u32::try_from(bounds.bottom.min(i64::from(dimensions.height())))
        .map_err(|_| SpotsExecutionError::ArithmeticOverflow)?;
    Roi::new(
        left,
        top,
        right.saturating_sub(left),
        bottom.saturating_sub(top),
    )
    .map_err(|_| SpotsExecutionError::ArithmeticOverflow)
}

fn offset_index(
    x: usize,
    y: usize,
    dimensions: RasterDimensions,
    offset: (i32, i32),
) -> Result<usize, SpotsExecutionError> {
    let source_x = i64::try_from(x)
        .map_err(|_| SpotsExecutionError::ArithmeticOverflow)?
        .checked_add(i64::from(offset.0))
        .ok_or(SpotsExecutionError::ArithmeticOverflow)?;
    let source_y = i64::try_from(y)
        .map_err(|_| SpotsExecutionError::ArithmeticOverflow)?
        .checked_add(i64::from(offset.1))
        .ok_or(SpotsExecutionError::ArithmeticOverflow)?;
    if source_x < 0
        || source_y < 0
        || source_x >= i64::from(dimensions.width())
        || source_y >= i64::from(dimensions.height())
    {
        return Err(SpotsExecutionError::ArithmeticOverflow);
    }
    let width =
        usize::try_from(dimensions.width()).map_err(|_| SpotsExecutionError::ArithmeticOverflow)?;
    usize::try_from(source_y)
        .ok()
        .and_then(|row| row.checked_mul(width))
        .and_then(|row| row.checked_add(usize::try_from(source_x).ok()?))
        .ok_or(SpotsExecutionError::ArithmeticOverflow)
}

fn heal_correction(
    source: &[LinearRgb],
    mask: &MaskRaster,
    entry: SpotsEntry,
    dimensions: RasterDimensions,
) -> Result<[f32; 3], SpotsExecutionError> {
    let mut target = [0.0; 3];
    let mut source_mean = [0.0; 3];
    let mut weight = 0.0;
    let width =
        usize::try_from(dimensions.width()).map_err(|_| SpotsExecutionError::ArithmeticOverflow)?;
    for (index, coverage) in mask.values().iter().copied().enumerate() {
        if coverage.to_bits() == 0 {
            continue;
        }
        let x = index % width;
        let y = index / width;
        let source_index = offset_index(x, y, dimensions, entry.source_offset())?;
        let destination = source[index];
        let sampled = source[source_index];
        let factor = coverage * entry.opacity();
        target[0] += destination.red().get() * factor;
        target[1] += destination.green().get() * factor;
        target[2] += destination.blue().get() * factor;
        source_mean[0] += sampled.red().get() * factor;
        source_mean[1] += sampled.green().get() * factor;
        source_mean[2] += sampled.blue().get() * factor;
        weight += factor;
    }
    if weight == 0.0 {
        return Ok([0.0; 3]);
    }
    Ok([
        target[0] / weight - source_mean[0] / weight,
        target[1] / weight - source_mean[1] / weight,
        target[2] / weight - source_mean[2] / weight,
    ])
}

fn apply_entry(
    source: &[LinearRgb],
    output: &mut [LinearRgb],
    mask: &MaskRaster,
    entry: SpotsEntry,
    dimensions: RasterDimensions,
    correction: [f32; 3],
    is_cancelled: &impl Fn() -> bool,
) -> Result<(), SpotsExecutionError> {
    let width =
        usize::try_from(dimensions.width()).map_err(|_| SpotsExecutionError::ArithmeticOverflow)?;
    for (index, coverage) in mask.values().iter().copied().enumerate() {
        if index % width == 0 && is_cancelled() {
            return Err(SpotsExecutionError::Cancelled);
        }
        if coverage.to_bits() == 0 {
            continue;
        }
        let x = index % width;
        let y = index / width;
        let sampled = source[offset_index(x, y, dimensions, entry.source_offset())?];
        let candidate = match entry.mode() {
            SpotsMode::Clone => sampled,
            SpotsMode::Heal => LinearRgb::new(
                finite(sampled.red().get() + correction[0])?,
                finite(sampled.green().get() + correction[1])?,
                finite(sampled.blue().get() + correction[2])?,
            ),
            SpotsMode::None => continue,
            SpotsMode::Unknown(mode) => {
                return Err(SpotsExecutionError::UnsupportedMode(SpotsMode::Unknown(
                    mode,
                )));
            }
        };
        let softened = coverage.powf(1.0 / entry.hardness().max(0.01));
        let factor = (softened * entry.opacity()).clamp(0.0, 1.0);
        let current = output[index];
        output[index] = LinearRgb::new(
            finite(current.red().get() * (1.0 - factor) + candidate.red().get() * factor)?,
            finite(current.green().get() * (1.0 - factor) + candidate.green().get() * factor)?,
            finite(current.blue().get() * (1.0 - factor) + candidate.blue().get() * factor)?,
        );
    }
    Ok(())
}

fn finite(value: f32) -> Result<FiniteF32, SpotsExecutionError> {
    FiniteF32::new(value).map_err(|_| SpotsExecutionError::NonFiniteResult {
        pixel: 0,
        channel: 0,
    })
}

fn validate_output(output: &[LinearRgb]) -> Result<(), SpotsExecutionError> {
    for (pixel, value) in output.iter().enumerate() {
        for (channel, channel_value) in [value.red(), value.green(), value.blue()]
            .into_iter()
            .enumerate()
        {
            if !channel_value.get().is_finite() {
                return Err(SpotsExecutionError::NonFiniteResult { pixel, channel });
            }
        }
    }
    Ok(())
}

fn plan_identity(dimensions: RasterDimensions, entries: &[ResolvedEntry]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"rusttable.spots.plan.v1");
    hasher.update(dimensions.width().to_le_bytes());
    hasher.update(dimensions.height().to_le_bytes());
    for resolved in entries {
        hasher.update(resolved.entry.form_id().to_le_bytes());
        hasher.update(resolved.entry.source_offset().0.to_le_bytes());
        hasher.update(resolved.entry.source_offset().1.to_le_bytes());
        hasher.update(resolved.entry.opacity().to_bits().to_le_bytes());
        hasher.update(resolved.entry.hardness().to_bits().to_le_bytes());
        hasher.update(resolved.entry.feather().to_le_bytes());
        hasher.update(resolved.mask.identity());
        hasher.update(format!("{:?}", resolved.entry.mode()).as_bytes());
    }
    hasher.finalize().into()
}
