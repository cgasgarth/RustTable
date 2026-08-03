//! GTK-independent Crop editor state derived from Darktable `src/iop/crop.c`.
//!
//! This leaf keeps native endpoint parameters, aspect orientation, slider order,
//! and deferred pointer drags independent of widget geometry. A GTK integrator
//! supplies normalized image-space pointer coordinates and the already-resolved
//! native grab region.

use std::fmt;

/// Native minimum crop width and height as a fraction of the image bounds.
pub const MIN_CROP_SIZE: f32 = 0.01;

/// Stable Darktable operation name used by history, styles, and module order.
pub const CROP_MODULE_ID: &str = "crop";
/// Native module title.
pub const CROP_TITLE: &str = "crop";
/// Native module description.
pub const CROP_DESCRIPTION: &str = "change the framing";
/// Native module groups in declaration order.
pub const CROP_GROUP_KEYS: [&str; 2] = ["group.basic", "group.technical"];
/// Native search aliases, split from Darktable's `reframe|distortion` string.
pub const CROP_ALIASES: [&str; 2] = ["reframe", "distortion"];

/// Native source order of the four margin sliders.
pub const CROP_SLIDER_ORDER: [CropSlider; 4] = [
    CropSlider::Cx,
    CropSlider::Cw,
    CropSlider::Cy,
    CropSlider::Ch,
];

/// Source aspect selection and orientation.
///
/// For fixed ratios, `numerator` is the native absolute `ratio_d` and
/// `denominator` is `ratio_n`. Darktable records a flipped orientation by
/// negating `ratio_d`; [`Self::ratio_d`] preserves that representation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CropAspect {
    Freehand,
    OriginalImage {
        flipped: bool,
    },
    Fixed {
        numerator: i32,
        denominator: i32,
        flipped: bool,
    },
}

/// Invalid native crop ratio parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CropAspectDecodeError {
    ratio_n: i32,
    ratio_d: i32,
}

impl CropAspectDecodeError {
    #[must_use]
    pub const fn ratio_n(self) -> i32 {
        self.ratio_n
    }

    #[must_use]
    pub const fn ratio_d(self) -> i32 {
        self.ratio_d
    }
}

impl fmt::Display for CropAspectDecodeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "invalid native crop ratio ratio_n={} ratio_d={}",
            self.ratio_n, self.ratio_d
        )
    }
}

impl std::error::Error for CropAspectDecodeError {}

impl CropAspect {
    /// Decodes Darktable's native ratio pair.
    ///
    /// `Ok(None)` is the unresolved `(-1, -1)` parameter sentinel used before
    /// native GUI preferences have been loaded. It is distinct from
    /// `Ok(Some(Self::Freehand))`, which is the persisted `(0, 0)` freehand
    /// selection. Fixed ratios preserve the native `ratio_d` sign as the
    /// orientation toggle; `(0, 1)` and `(0, -1)` preserve original-image
    /// orientation.
    ///
    /// # Errors
    ///
    /// Returns an error for ratio pairs that are not valid Darktable fixed,
    /// original-image, freehand, or unresolved values.
    pub const fn decode_native_ratio(
        ratio_n: i32,
        ratio_d: i32,
    ) -> Result<Option<Self>, CropAspectDecodeError> {
        if ratio_n == -1 && ratio_d == -1 {
            return Ok(None);
        }
        if ratio_n == 0 && ratio_d == 0 {
            return Ok(Some(Self::Freehand));
        }
        if ratio_n == 0 && (ratio_d == 1 || ratio_d == -1) {
            return Ok(Some(Self::OriginalImage {
                flipped: ratio_d < 0,
            }));
        }
        if ratio_n > 0 && ratio_d != 0 && ratio_d != i32::MIN {
            return Ok(Some(Self::Fixed {
                numerator: ratio_d.abs(),
                denominator: ratio_n,
                flipped: ratio_d < 0,
            }));
        }
        Err(CropAspectDecodeError { ratio_n, ratio_d })
    }

    /// Alias for [`Self::decode_native_ratio`] at the native boundary.
    ///
    /// # Errors
    ///
    /// Returns the same checked native-ratio error as
    /// [`Self::decode_native_ratio`].
    pub const fn from_native_ratio(
        ratio_n: i32,
        ratio_d: i32,
    ) -> Result<Option<Self>, CropAspectDecodeError> {
        Self::decode_native_ratio(ratio_n, ratio_d)
    }

    /// Builds and reduces a positive fixed ratio, placing its longer term first.
    ///
    /// # Errors
    ///
    /// Returns the native fraction-format error when either term is not positive.
    pub fn fixed(numerator: i32, denominator: i32) -> Result<Self, CropAspectParseError> {
        if numerator <= 0 || denominator <= 0 {
            return Err(CropAspectParseError::InvalidFraction);
        }
        let longer = numerator.max(denominator);
        let shorter = numerator.min(denominator);
        let mut numerator = longer;
        let mut denominator = shorter;
        let divisor = gcd(numerator, denominator);
        numerator /= divisor;
        denominator /= divisor;
        Ok(Self::Fixed {
            numerator,
            denominator,
            flipped: false,
        })
    }

    /// Parses the editable native aspect field.
    ///
    /// `:` and `/` select integer-fraction parsing. Otherwise a positive decimal
    /// using `.` or `,` is converted to a fraction exactly as `_float_to_fract`.
    /// The returned error distinguishes the two source log messages. The editor
    /// action applies Darktable's invalid-input fallback to freehand.
    ///
    /// # Errors
    ///
    /// Returns [`CropAspectParseError::InvalidFraction`] for a zero or malformed
    /// fraction and [`CropAspectParseError::InvalidPositiveNumber`] for invalid
    /// decimal input.
    pub fn parse(text: &str) -> Result<Self, CropAspectParseError> {
        if let Some(separator) = text.find([':', '/'])
            && separator + 1 < text.len()
        {
            let first = parse_c_int_prefix(text).ok_or(CropAspectParseError::InvalidFraction)?;
            let second = parse_c_int_prefix(&text[separator + 1..])
                .ok_or(CropAspectParseError::InvalidFraction)?;
            return Self::fixed(first, second);
        }

        let (first, second) = decimal_fraction(text)?;
        Self::fixed(first, second).map_err(|_| CropAspectParseError::InvalidPositiveNumber)
    }

    /// Returns native `ratio_n`.
    #[must_use]
    pub const fn ratio_n(self) -> i32 {
        match self {
            Self::Freehand | Self::OriginalImage { .. } => 0,
            Self::Fixed { denominator, .. } => denominator,
        }
    }

    /// Returns native `ratio_d`, including the orientation sign.
    #[must_use]
    pub const fn ratio_d(self) -> i32 {
        match self {
            Self::Freehand => 0,
            Self::OriginalImage { flipped: false } => 1,
            Self::OriginalImage { flipped: true } => -1,
            Self::Fixed {
                numerator,
                flipped: false,
                ..
            } => numerator,
            Self::Fixed {
                numerator,
                flipped: true,
                ..
            } => -numerator,
        }
    }

    /// Reports whether native `ratio_d` is negative.
    #[must_use]
    pub const fn is_flipped(self) -> bool {
        self.ratio_d() < 0
    }

    /// Negates native `ratio_d`, leaving freehand unchanged.
    #[must_use]
    pub const fn flip(self) -> Self {
        match self {
            Self::Freehand => Self::Freehand,
            Self::OriginalImage { flipped } => Self::OriginalImage { flipped: !flipped },
            Self::Fixed {
                numerator,
                denominator,
                flipped,
            } => Self::Fixed {
                numerator,
                denominator,
                flipped: !flipped,
            },
        }
    }

    #[expect(
        clippy::cast_precision_loss,
        reason = "The fixed crop ratio is a small source integer represented in native f32 geometry."
    )]
    const fn native_ratio_component(value: i32) -> f32 {
        value as f32
    }

    fn value(self, image_width: f32, image_height: f32) -> Option<f32> {
        let mut aspect = match self {
            Self::Freehand => return None,
            Self::OriginalImage { flipped } => {
                let regular = (!flipped && image_width >= image_height)
                    || (flipped && image_width < image_height);
                if regular {
                    image_width / image_height
                } else {
                    image_height / image_width
                }
            }
            Self::Fixed {
                numerator,
                denominator,
                flipped,
            } => {
                if flipped {
                    Self::native_ratio_component(denominator)
                        / Self::native_ratio_component(numerator)
                } else {
                    Self::native_ratio_component(numerator)
                        / Self::native_ratio_component(denominator)
                }
            }
        };
        if image_width < image_height {
            aspect = aspect.recip();
        }
        Some(aspect)
    }
}

/// Decodes a Darktable native crop ratio pair at a persistence/UI boundary.
///
/// See [`CropAspect::decode_native_ratio`] for the sentinel and validation
/// semantics.
///
/// # Errors
///
/// Returns the checked native-ratio error for malformed parameter pairs.
pub const fn decode_native_ratio(
    ratio_n: i32,
    ratio_d: i32,
) -> Result<Option<CropAspect>, CropAspectDecodeError> {
    CropAspect::decode_native_ratio(ratio_n, ratio_d)
}

/// Native invalid-input categories and user-visible messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CropAspectParseError {
    InvalidFraction,
    InvalidPositiveNumber,
}

impl CropAspectParseError {
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::InvalidFraction => "invalid ratio format. it should be \"number:number\"",
            Self::InvalidPositiveNumber => "invalid ratio format. it should be a positive number",
        }
    }
}

impl fmt::Display for CropAspectParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.message())
    }
}

impl std::error::Error for CropAspectParseError {}

/// Normalized native crop endpoints.
///
/// `cx`/`cy` are left/top and `cw`/`ch` are right/bottom. The latter two are
/// deliberately not width and height.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CropBox {
    pub cx: f32,
    pub cy: f32,
    pub cw: f32,
    pub ch: f32,
}

impl CropBox {
    pub const FULL: Self = Self::new(0.0, 0.0, 1.0, 1.0);

    #[must_use]
    pub const fn new(cx: f32, cy: f32, cw: f32, ch: f32) -> Self {
        Self { cx, cy, cw, ch }
    }

    #[must_use]
    pub fn width(self) -> f32 {
        self.cw - self.cx
    }

    #[must_use]
    pub fn height(self) -> f32 {
        self.ch - self.cy
    }

    const fn is_finite(self) -> bool {
        self.cx.is_finite() && self.cy.is_finite() && self.cw.is_finite() && self.ch.is_finite()
    }
}

/// One margin slider, named after its native parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CropSlider {
    Cx,
    Cw,
    Cy,
    Ch,
}

/// A source crop-box region already resolved by the GTK hit test.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CropGrab {
    Center,
    Left,
    Top,
    Right,
    Bottom,
    TopLeft,
    TopRight,
    BottomRight,
    BottomLeft,
}

impl CropGrab {
    const fn left(self) -> bool {
        matches!(self, Self::Left | Self::TopLeft | Self::BottomLeft)
    }

    const fn top(self) -> bool {
        matches!(self, Self::Top | Self::TopLeft | Self::TopRight)
    }

    const fn right(self) -> bool {
        matches!(self, Self::Right | Self::TopRight | Self::BottomRight)
    }

    const fn bottom(self) -> bool {
        matches!(self, Self::Bottom | Self::BottomRight | Self::BottomLeft)
    }

    const fn horizontal(self) -> bool {
        self.left() || self.right()
    }

    const fn vertical(self) -> bool {
        self.top() || self.bottom()
    }
}

/// Modifiers captured when a native primary-button drag starts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct CropModifiers {
    pub shift: bool,
    pub control: bool,
}

/// Complete non-widget state supplied by native `gui_update`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CropEditorUpdate {
    pub crop: CropBox,
    pub default_crop: CropBox,
    pub aspect: CropAspect,
    pub image_width: f32,
    pub image_height: f32,
    pub bounds: CropBox,
}

/// Persistable portion of editor state.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CropEditorSnapshot {
    pub crop: CropBox,
    pub aspect: CropAspect,
}

/// Deterministic input events for the pure editor model.
#[derive(Debug, Clone, PartialEq)]
pub enum CropEditorAction {
    SetAspect(CropAspect),
    SetAspectInput(String),
    FlipAspect,
    SetSlider {
        slider: CropSlider,
        value: f32,
    },
    BeginDrag {
        grab: CropGrab,
        x: f32,
        y: f32,
        modifiers: CropModifiers,
    },
    DragTo {
        x: f32,
        y: f32,
    },
    CommitDrag,
    CancelDrag,
    /// Resets only the crop area while preserving the selected aspect.
    ResetArea,
    /// Resets the module area and aspect to native defaults.
    Reset,
    Update(CropEditorUpdate),
}

/// Deterministic result of one editor action.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CropEditorResult {
    Unchanged,
    DragStarted {
        crop: CropBox,
    },
    Deferred {
        crop: CropBox,
    },
    Committed(CropEditorSnapshot),
    DragCancelled {
        crop: CropBox,
    },
    ResetArea(CropEditorSnapshot),
    Reset(CropEditorSnapshot),
    Updated(CropEditorSnapshot),
    InvalidAspect {
        error: CropAspectParseError,
        fallback: CropEditorSnapshot,
    },
}

/// Invalid geometry supplied to the pure editor boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CropEditorError {
    NonFiniteInput,
    InvalidImageDimensions,
    InvalidBounds,
}

impl fmt::Display for CropEditorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NonFiniteInput => "crop editor input must be finite",
            Self::InvalidImageDimensions => "crop image dimensions must be positive",
            Self::InvalidBounds => {
                "crop bounds must be normalized and at least MIN_CROP_SIZE wide and high"
            }
        })
    }
}

impl std::error::Error for CropEditorError {}

#[derive(Debug, Clone, Copy, PartialEq)]
struct DragState {
    grab: CropGrab,
    start_x: f32,
    start_y: f32,
    handle_x: f32,
    handle_y: f32,
    origin: CropBox,
    modifiers: CropModifiers,
}

/// Pure source-shaped Crop editor state.
#[derive(Debug, Clone, PartialEq)]
pub struct CropEditorState {
    committed: CropBox,
    deferred: CropBox,
    default_crop: CropBox,
    aspect: CropAspect,
    image_width: f32,
    image_height: f32,
    bounds: CropBox,
    drag: Option<DragState>,
}

impl CropEditorState {
    /// Creates the editor through the same transition used for later updates.
    ///
    /// # Errors
    ///
    /// Rejects non-finite values, non-positive image dimensions, or normalized
    /// bounds smaller than [`MIN_CROP_SIZE`]. Crop endpoints themselves are
    /// clamped into valid bounds, matching native focus/update behavior.
    pub fn new(update: CropEditorUpdate) -> Result<Self, CropEditorError> {
        validate_update(update)?;
        let committed = clamp_box(update.crop, update.bounds)?;
        let default_crop = clamp_box(update.default_crop, update.bounds)?;
        Ok(Self {
            committed,
            deferred: committed,
            default_crop,
            aspect: update.aspect,
            image_width: update.image_width,
            image_height: update.image_height,
            bounds: update.bounds,
            drag: None,
        })
    }

    #[must_use]
    pub const fn committed_crop(&self) -> CropBox {
        self.committed
    }

    #[must_use]
    pub const fn deferred_crop(&self) -> CropBox {
        self.deferred
    }

    #[must_use]
    pub const fn aspect(&self) -> CropAspect {
        self.aspect
    }

    #[must_use]
    pub const fn bounds(&self) -> CropBox {
        self.bounds
    }

    #[must_use]
    pub const fn is_dragging(&self) -> bool {
        self.drag.is_some()
    }

    #[must_use]
    pub const fn snapshot(&self) -> CropEditorSnapshot {
        CropEditorSnapshot {
            crop: self.committed,
            aspect: self.aspect,
        }
    }

    /// Applies one editor event without GTK or persistence dependencies.
    ///
    /// # Errors
    ///
    /// Rejects non-finite pointer/slider input and invalid update geometry,
    /// leaving state unchanged.
    pub fn apply(&mut self, action: CropEditorAction) -> Result<CropEditorResult, CropEditorError> {
        match action {
            CropEditorAction::SetAspect(aspect) => self.set_aspect(aspect),
            CropEditorAction::SetAspectInput(text) => self.set_aspect_input(&text),
            CropEditorAction::FlipAspect => self.flip_aspect(),
            CropEditorAction::SetSlider { slider, value } => self.set_slider(slider, value),
            CropEditorAction::BeginDrag {
                grab,
                x,
                y,
                modifiers,
            } => self.begin_drag(grab, x, y, modifiers),
            CropEditorAction::DragTo { x, y } => self.drag_to(x, y),
            CropEditorAction::CommitDrag => Ok(self.commit_drag()),
            CropEditorAction::CancelDrag => Ok(self.cancel_drag()),
            CropEditorAction::ResetArea => self.reset_area(),
            CropEditorAction::Reset => self.reset(),
            CropEditorAction::Update(update) => self.update(update),
        }
    }

    fn set_aspect(&mut self, aspect: CropAspect) -> Result<CropEditorResult, CropEditorError> {
        self.drag = None;
        self.aspect = aspect;
        self.committed = apply_aspect(
            self.committed,
            self.bounds,
            self.image_width,
            self.image_height,
            self.aspect,
            Adjustment::Horizontal,
        )?;
        self.deferred = self.committed;
        Ok(CropEditorResult::Committed(self.snapshot()))
    }

    fn set_aspect_input(&mut self, text: &str) -> Result<CropEditorResult, CropEditorError> {
        match CropAspect::parse(text) {
            Ok(aspect) => self.set_aspect(aspect),
            Err(error) => {
                self.drag = None;
                self.aspect = CropAspect::Freehand;
                self.deferred = self.committed;
                Ok(CropEditorResult::InvalidAspect {
                    error,
                    fallback: self.snapshot(),
                })
            }
        }
    }

    fn flip_aspect(&mut self) -> Result<CropEditorResult, CropEditorError> {
        self.drag = None;
        self.aspect = self.aspect.flip();
        let horizontal = (self.image_width >= self.image_height) == self.aspect.is_flipped();
        self.committed = apply_aspect(
            self.committed,
            self.bounds,
            self.image_width,
            self.image_height,
            self.aspect,
            if horizontal {
                Adjustment::Horizontal
            } else {
                Adjustment::Vertical
            },
        )?;
        self.deferred = self.committed;
        Ok(CropEditorResult::Committed(self.snapshot()))
    }

    fn set_slider(
        &mut self,
        slider: CropSlider,
        value: f32,
    ) -> Result<CropEditorResult, CropEditorError> {
        if !value.is_finite() {
            return Err(CropEditorError::NonFiniteInput);
        }
        self.drag = None;
        let mut crop = self.committed;
        let adjustment = match slider {
            CropSlider::Cx => {
                crop.cx = value.clamp(self.bounds.cx, crop.cw - MIN_CROP_SIZE);
                Adjustment::Grab(CropGrab::Left)
            }
            CropSlider::Cw => {
                crop.cw = value.clamp(crop.cx + MIN_CROP_SIZE, self.bounds.cw);
                Adjustment::Grab(CropGrab::Right)
            }
            CropSlider::Cy => {
                crop.cy = value.clamp(self.bounds.cy, crop.ch - MIN_CROP_SIZE);
                Adjustment::Grab(CropGrab::Top)
            }
            CropSlider::Ch => {
                crop.ch = value.clamp(crop.cy + MIN_CROP_SIZE, self.bounds.ch);
                Adjustment::Grab(CropGrab::Bottom)
            }
        };
        self.committed = apply_aspect(
            crop,
            self.bounds,
            self.image_width,
            self.image_height,
            self.aspect,
            adjustment,
        )?;
        self.deferred = self.committed;
        Ok(CropEditorResult::Committed(self.snapshot()))
    }

    fn begin_drag(
        &mut self,
        grab: CropGrab,
        x: f32,
        y: f32,
        modifiers: CropModifiers,
    ) -> Result<CropEditorResult, CropEditorError> {
        validate_point(x, y)?;
        self.deferred = self.committed;
        let (handle_x, handle_y) = if grab == CropGrab::Center {
            (self.committed.cx, self.committed.cy)
        } else {
            let handle_x = if grab.left() {
                x - self.committed.cx
            } else if grab.right() {
                x - self.committed.cw
            } else {
                0.0
            };
            let handle_y = if grab.top() {
                y - self.committed.cy
            } else if grab.bottom() {
                y - self.committed.ch
            } else {
                0.0
            };
            (handle_x, handle_y)
        };
        self.drag = Some(DragState {
            grab,
            start_x: x,
            start_y: y,
            handle_x,
            handle_y,
            origin: self.committed,
            modifiers,
        });
        Ok(CropEditorResult::DragStarted {
            crop: self.deferred,
        })
    }

    fn drag_to(&mut self, x: f32, y: f32) -> Result<CropEditorResult, CropEditorError> {
        validate_point(x, y)?;
        let Some(drag) = self.drag else {
            return Ok(CropEditorResult::Unchanged);
        };

        let crop = if drag.grab == CropGrab::Center {
            self.move_center(drag, x, y)
        } else if drag.modifiers.shift {
            self.resize_from_center(drag, x, y)
        } else {
            self.resize_edge(drag, x, y)
        };
        self.deferred = apply_aspect(
            crop,
            self.bounds,
            self.image_width,
            self.image_height,
            self.aspect,
            Adjustment::Grab(drag.grab),
        )?;
        Ok(CropEditorResult::Deferred {
            crop: self.deferred,
        })
    }

    fn move_center(&self, drag: DragState, x: f32, y: f32) -> CropBox {
        let width = drag.origin.width();
        let height = drag.origin.height();
        let cx = if drag.modifiers.shift {
            drag.origin.cx
        } else {
            (drag.handle_x + x - drag.start_x).clamp(self.bounds.cx, self.bounds.cw - width)
        };
        let cy = if drag.modifiers.control {
            drag.origin.cy
        } else {
            (drag.handle_y + y - drag.start_y).clamp(self.bounds.cy, self.bounds.ch - height)
        };
        CropBox::new(cx, cy, cx + width, cy + height)
    }

    #[expect(
        clippy::suboptimal_flops,
        reason = "The source crop resize preserves its original center-delta floating-point order."
    )]
    fn resize_from_center(&self, drag: DragState, x: f32, y: f32) -> CropBox {
        let width = drag.origin.width();
        let height = drag.origin.height();
        let mut ratio = if drag.grab.horizontal() {
            let delta = if drag.grab.left() {
                x - drag.start_x
            } else {
                drag.start_x - x
            };
            (width - 2.0 * delta) / width
        } else {
            0.0_f32
        };
        if drag.grab.vertical() {
            let delta = if drag.grab.top() {
                y - drag.start_y
            } else {
                drag.start_y - y
            };
            ratio = ratio.max((height - 2.0 * delta) / height);
        }
        ratio = ratio.max(MIN_CROP_SIZE / width);
        ratio = ratio.max(MIN_CROP_SIZE / height);
        ratio = ratio.min(self.bounds.width() / width);
        ratio = ratio.min(self.bounds.height() / height);

        let new_width = width * ratio;
        let new_height = height * ratio;
        let cx = (drag.origin.cx - (new_width - width) * 0.5)
            .clamp(self.bounds.cx, self.bounds.cw - new_width);
        let cy = (drag.origin.cy - (new_height - height) * 0.5)
            .clamp(self.bounds.cy, self.bounds.ch - new_height);
        CropBox::new(cx, cy, cx + new_width, cy + new_height)
    }

    fn resize_edge(&self, drag: DragState, x: f32, y: f32) -> CropBox {
        let mut crop = self.deferred;
        if drag.grab.left() {
            crop.cx = (x - drag.handle_x).clamp(self.bounds.cx, crop.cw - MIN_CROP_SIZE);
        }
        if drag.grab.top() {
            crop.cy = (y - drag.handle_y).clamp(self.bounds.cy, crop.ch - MIN_CROP_SIZE);
        }
        if drag.grab.right() {
            crop.cw = (x - drag.handle_x).clamp(crop.cx + MIN_CROP_SIZE, self.bounds.cw);
        }
        if drag.grab.bottom() {
            crop.ch = (y - drag.handle_y).clamp(crop.cy + MIN_CROP_SIZE, self.bounds.ch);
        }
        crop
    }

    fn commit_drag(&mut self) -> CropEditorResult {
        if self.drag.take().is_none() {
            return CropEditorResult::Unchanged;
        }
        if self.committed == self.deferred {
            return CropEditorResult::Unchanged;
        }
        self.committed = self.deferred;
        CropEditorResult::Committed(self.snapshot())
    }

    const fn cancel_drag(&mut self) -> CropEditorResult {
        if self.drag.take().is_none() {
            return CropEditorResult::Unchanged;
        }
        self.deferred = self.committed;
        CropEditorResult::DragCancelled {
            crop: self.committed,
        }
    }

    fn reset_area(&mut self) -> Result<CropEditorResult, CropEditorError> {
        self.drag = None;
        self.committed = apply_aspect(
            CropBox::FULL,
            self.bounds,
            self.image_width,
            self.image_height,
            self.aspect,
            Adjustment::Grab(CropGrab::BottomRight),
        )?;
        self.deferred = self.committed;
        Ok(CropEditorResult::ResetArea(self.snapshot()))
    }

    fn reset(&mut self) -> Result<CropEditorResult, CropEditorError> {
        self.drag = None;
        self.aspect = CropAspect::Freehand;
        self.committed = clamp_box(self.default_crop, self.bounds)?;
        self.deferred = self.committed;
        Ok(CropEditorResult::Reset(self.snapshot()))
    }

    fn update(&mut self, update: CropEditorUpdate) -> Result<CropEditorResult, CropEditorError> {
        validate_update(update)?;
        let committed = clamp_box(update.crop, update.bounds)?;
        let default_crop = clamp_box(update.default_crop, update.bounds)?;
        self.committed = committed;
        self.deferred = committed;
        self.default_crop = default_crop;
        self.aspect = update.aspect;
        self.image_width = update.image_width;
        self.image_height = update.image_height;
        self.bounds = update.bounds;
        self.drag = None;
        Ok(CropEditorResult::Updated(self.snapshot()))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Adjustment {
    Grab(CropGrab),
    Horizontal,
    Vertical,
}

impl Adjustment {
    const fn left(self) -> bool {
        matches!(self, Self::Grab(grab) if grab.left())
    }

    const fn top(self) -> bool {
        matches!(self, Self::Grab(grab) if grab.top())
    }

    const fn horizontal(self) -> bool {
        matches!(self, Self::Horizontal) || matches!(self, Self::Grab(grab) if grab.horizontal())
    }

    const fn vertical(self) -> bool {
        matches!(self, Self::Vertical) || matches!(self, Self::Grab(grab) if grab.vertical())
    }
}

#[expect(
    clippy::suboptimal_flops,
    reason = "The source crop aspect adjustment preserves its original offset arithmetic order."
)]
fn apply_aspect(
    crop: CropBox,
    bounds: CropBox,
    image_width: f32,
    image_height: f32,
    aspect: CropAspect,
    adjustment: Adjustment,
) -> Result<CropBox, CropEditorError> {
    let Some(aspect) = aspect.value(image_width, image_height) else {
        return clamp_box(crop, bounds);
    };

    let mut x = crop.cx;
    let mut y = crop.cy;
    let mut width = crop.width();
    let mut height = crop.height();
    let target_height = image_width * width / (image_height * aspect);
    let target_width = image_height * height * aspect / image_width;

    match adjustment {
        Adjustment::Grab(CropGrab::TopLeft) => {
            x = x + width - f32::midpoint(target_width, width);
            y = y + height - f32::midpoint(target_height, height);
            width = f32::midpoint(target_width, width);
            height = f32::midpoint(target_height, height);
        }
        Adjustment::Grab(CropGrab::TopRight) => {
            y = y + height - f32::midpoint(target_height, height);
            width = f32::midpoint(target_width, width);
            height = f32::midpoint(target_height, height);
        }
        Adjustment::Grab(CropGrab::BottomRight) => {
            width = f32::midpoint(target_width, width);
            height = f32::midpoint(target_height, height);
        }
        Adjustment::Grab(CropGrab::BottomLeft) => {
            height = f32::midpoint(target_height, height);
            x = x + width - f32::midpoint(target_width, width);
            width = f32::midpoint(target_width, width);
        }
        _ if adjustment.horizontal() => {
            let offset = target_height - height;
            height += offset;
            y -= offset * 0.5;
        }
        _ if adjustment.vertical() => {
            let offset = target_width - width;
            width += offset;
            x -= offset * 0.5;
        }
        _ => {}
    }

    if x < bounds.cx {
        let previous_height = height;
        height *= (width + x - bounds.cx) / width;
        width = width + x - bounds.cx;
        x = bounds.cx;
        if adjustment.top() {
            y += previous_height - height;
        }
    }
    if y < bounds.cy {
        let previous_width = width;
        width *= (height + y - bounds.cy) / height;
        height = height + y - bounds.cy;
        y = bounds.cy;
        if adjustment.left() {
            x += previous_width - width;
        }
    }
    if x + width > bounds.cw {
        let previous_height = height;
        height *= (bounds.cw - x) / width;
        width = bounds.cw - x;
        if adjustment.top() {
            y += previous_height - height;
        }
    }
    if y + height > bounds.ch {
        let previous_width = width;
        width *= (bounds.ch - y) / height;
        height = bounds.ch - y;
        if adjustment.left() {
            x += previous_width - width;
        }
    }

    clamp_box(CropBox::new(x, y, x + width, y + height), bounds)
}

fn validate_update(update: CropEditorUpdate) -> Result<(), CropEditorError> {
    if !update.image_width.is_finite() || !update.image_height.is_finite() {
        return Err(CropEditorError::NonFiniteInput);
    }
    if update.image_width <= 0.0 || update.image_height <= 0.0 {
        return Err(CropEditorError::InvalidImageDimensions);
    }
    validate_bounds(update.bounds)?;
    if !update.crop.is_finite() || !update.default_crop.is_finite() {
        return Err(CropEditorError::NonFiniteInput);
    }
    Ok(())
}

fn validate_bounds(bounds: CropBox) -> Result<(), CropEditorError> {
    if !bounds.is_finite() {
        return Err(CropEditorError::NonFiniteInput);
    }
    if bounds.cx < 0.0
        || bounds.cy < 0.0
        || bounds.cw > 1.0
        || bounds.ch > 1.0
        || bounds.width() < MIN_CROP_SIZE
        || bounds.height() < MIN_CROP_SIZE
    {
        return Err(CropEditorError::InvalidBounds);
    }
    Ok(())
}

const fn validate_point(x: f32, y: f32) -> Result<(), CropEditorError> {
    if x.is_finite() && y.is_finite() {
        Ok(())
    } else {
        Err(CropEditorError::NonFiniteInput)
    }
}

fn clamp_box(crop: CropBox, bounds: CropBox) -> Result<CropBox, CropEditorError> {
    if !crop.is_finite() {
        return Err(CropEditorError::NonFiniteInput);
    }
    let cx = crop.cx.clamp(bounds.cx, bounds.cw - MIN_CROP_SIZE);
    let cy = crop.cy.clamp(bounds.cy, bounds.ch - MIN_CROP_SIZE);
    let cw = crop.cw.clamp(cx + MIN_CROP_SIZE, bounds.cw);
    let ch = crop.ch.clamp(cy + MIN_CROP_SIZE, bounds.ch);
    Ok(CropBox::new(cx, cy, cw, ch))
}

fn decimal_fraction(text: &str) -> Result<(i32, i32), CropAspectParseError> {
    let mut numerator: i64 = 0;
    let mut denominator: i64 = 1;
    let mut separator_found = false;
    let mut digit_found = false;

    for byte in text.bytes() {
        if separator_found {
            denominator = denominator
                .checked_mul(10)
                .ok_or(CropAspectParseError::InvalidPositiveNumber)?;
        }
        if !separator_found && matches!(byte, b'.' | b',') {
            separator_found = true;
        } else if !byte.is_ascii_digit() {
            return Err(CropAspectParseError::InvalidPositiveNumber);
        } else {
            digit_found = true;
            numerator = numerator
                .checked_mul(10)
                .and_then(|value| value.checked_add(i64::from(byte - b'0')))
                .ok_or(CropAspectParseError::InvalidPositiveNumber)?;
        }
    }

    if !digit_found || numerator == 0 {
        return Err(CropAspectParseError::InvalidPositiveNumber);
    }
    let numerator =
        i32::try_from(numerator).map_err(|_| CropAspectParseError::InvalidPositiveNumber)?;
    let denominator =
        i32::try_from(denominator).map_err(|_| CropAspectParseError::InvalidPositiveNumber)?;
    Ok((numerator, denominator))
}

fn parse_c_int_prefix(text: &str) -> Option<i32> {
    let bytes = text.as_bytes();
    let mut index = 0;
    while bytes.get(index).is_some_and(u8::is_ascii_whitespace) {
        index += 1;
    }
    let mut negative = false;
    if let Some(sign) = bytes.get(index) {
        if *sign == b'-' {
            negative = true;
            index += 1;
        } else if *sign == b'+' {
            index += 1;
        }
    }
    let start = index;
    let mut value: i64 = 0;
    while let Some(byte) = bytes.get(index).copied().filter(u8::is_ascii_digit) {
        value = value.checked_mul(10)?.checked_add(i64::from(byte - b'0'))?;
        index += 1;
    }
    if index == start {
        return None;
    }
    let value = if negative { -value } else { value };
    i32::try_from(value).ok()
}

fn gcd(mut left: i32, mut right: i32) -> i32 {
    while right != 0 {
        let remainder = left % right;
        left = right;
        right = remainder;
    }
    left.abs().max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn update(crop: CropBox, aspect: CropAspect) -> CropEditorUpdate {
        CropEditorUpdate {
            crop,
            default_crop: CropBox::FULL,
            aspect,
            image_width: 100.0,
            image_height: 100.0,
            bounds: CropBox::FULL,
        }
    }

    #[test]
    fn source_metadata_and_native_ratio_sentinels_are_exact() {
        assert_eq!(CROP_MODULE_ID, "crop");
        assert_eq!(CROP_TITLE, "crop");
        assert_eq!(CROP_DESCRIPTION, "change the framing");
        assert_eq!(CROP_GROUP_KEYS, ["group.basic", "group.technical"]);
        assert_eq!(CROP_ALIASES, ["reframe", "distortion"]);

        assert_eq!(CropAspect::decode_native_ratio(-1, -1), Ok(None));
        assert_eq!(
            CropAspect::decode_native_ratio(0, 0),
            Ok(Some(CropAspect::Freehand))
        );
        assert_eq!(
            CropAspect::decode_native_ratio(0, -1),
            Ok(Some(CropAspect::OriginalImage { flipped: true }))
        );
        assert_eq!(
            CropAspect::decode_native_ratio(5, -3),
            Ok(Some(CropAspect::Fixed {
                numerator: 3,
                denominator: 5,
                flipped: true,
            }))
        );
        assert!(CropAspect::decode_native_ratio(0, 2).is_err());
    }

    fn assert_close(actual: f32, expected: f32) {
        assert!((actual - expected).abs() < 1.0e-6, "{actual} != {expected}");
    }

    fn assert_box(actual: CropBox, expected: CropBox) {
        assert_close(actual.cx, expected.cx);
        assert_close(actual.cy, expected.cy);
        assert_close(actual.cw, expected.cw);
        assert_close(actual.ch, expected.ch);
    }

    #[test]
    fn parses_fraction_separators_and_positive_decimals() {
        assert_eq!(
            CropAspect::parse("16:9"),
            Ok(CropAspect::Fixed {
                numerator: 16,
                denominator: 9,
                flipped: false
            })
        );
        assert_eq!(CropAspect::parse("4/3"), CropAspect::fixed(4, 3));
        assert_eq!(CropAspect::parse("1.5"), CropAspect::fixed(3, 2));
        assert_eq!(CropAspect::parse("1,25"), CropAspect::fixed(5, 4));
        assert_eq!(
            CropAspect::parse("3:0"),
            Err(CropAspectParseError::InvalidFraction)
        );
        assert_eq!(
            CropAspect::parse("0"),
            Err(CropAspectParseError::InvalidPositiveNumber)
        );
        assert_eq!(
            CropAspect::parse("3:"),
            Err(CropAspectParseError::InvalidPositiveNumber)
        );
    }

    #[test]
    fn custom_ratios_are_reduced_and_long_side_is_stored_first() {
        assert_eq!(
            CropAspect::parse("1080:1920"),
            Ok(CropAspect::Fixed {
                numerator: 16,
                denominator: 9,
                flipped: false
            })
        );
        assert_eq!(CropAspect::parse("2.50"), CropAspect::fixed(5, 2));
    }

    #[test]
    fn flip_is_encoded_only_by_ratio_d_sign() {
        let ratio = CropAspect::fixed(3, 2).unwrap();
        assert_eq!(ratio.ratio_n(), 2);
        assert_eq!(ratio.ratio_d(), 3);
        assert_eq!(ratio.flip().ratio_n(), 2);
        assert_eq!(ratio.flip().ratio_d(), -3);
        assert_eq!(ratio.flip().flip(), ratio);
        assert_eq!(
            CropAspect::OriginalImage { flipped: false }
                .flip()
                .ratio_d(),
            -1
        );
        assert_eq!(CropAspect::Freehand.flip(), CropAspect::Freehand);
    }

    #[test]
    fn endpoints_and_sliders_clamp_to_native_minimum_in_source_order() {
        assert_eq!(
            CROP_SLIDER_ORDER,
            [
                CropSlider::Cx,
                CropSlider::Cw,
                CropSlider::Cy,
                CropSlider::Ch
            ]
        );
        let mut editor = CropEditorState::new(update(
            CropBox::new(-0.5, 0.995, -0.1, 0.2),
            CropAspect::Freehand,
        ))
        .unwrap();
        assert_box(editor.committed_crop(), CropBox::new(0.0, 0.99, 0.01, 1.0));
        editor
            .apply(CropEditorAction::SetSlider {
                slider: CropSlider::Cw,
                value: 0.0,
            })
            .unwrap();
        assert_close(editor.committed_crop().width(), MIN_CROP_SIZE);
    }

    #[test]
    fn fixed_ratio_is_preserved_for_edges_and_corners() {
        let ratio = CropAspect::fixed(2, 1).unwrap();
        let mut edge =
            CropEditorState::new(update(CropBox::new(0.1, 0.2, 0.5, 0.4), ratio)).unwrap();
        edge.apply(CropEditorAction::BeginDrag {
            grab: CropGrab::Right,
            x: 0.5,
            y: 0.3,
            modifiers: CropModifiers::default(),
        })
        .unwrap();
        edge.apply(CropEditorAction::DragTo { x: 0.7, y: 0.3 })
            .unwrap();
        let crop = edge.deferred_crop();
        assert_close(crop.width() / crop.height(), 2.0);
        assert_box(crop, CropBox::new(0.1, 0.15, 0.7, 0.45));

        let mut corner =
            CropEditorState::new(update(CropBox::new(0.1, 0.2, 0.5, 0.4), ratio)).unwrap();
        corner
            .apply(CropEditorAction::BeginDrag {
                grab: CropGrab::BottomRight,
                x: 0.5,
                y: 0.4,
                modifiers: CropModifiers::default(),
            })
            .unwrap();
        corner
            .apply(CropEditorAction::DragTo { x: 0.7, y: 0.5 })
            .unwrap();
        let crop = corner.deferred_crop();
        assert_close(crop.width() / crop.height(), 2.0);
        assert_box(crop, CropBox::new(0.1, 0.2, 0.7, 0.5));
    }

    #[test]
    fn center_move_uses_native_shift_and_control_axis_locks() {
        let crop = CropBox::new(0.2, 0.2, 0.6, 0.6);
        let mut vertical = CropEditorState::new(update(crop, CropAspect::Freehand)).unwrap();
        vertical
            .apply(CropEditorAction::BeginDrag {
                grab: CropGrab::Center,
                x: 0.3,
                y: 0.3,
                modifiers: CropModifiers {
                    shift: true,
                    control: false,
                },
            })
            .unwrap();
        vertical
            .apply(CropEditorAction::DragTo { x: 0.5, y: 0.6 })
            .unwrap();
        assert_box(vertical.deferred_crop(), CropBox::new(0.2, 0.5, 0.6, 0.9));

        let mut horizontal = CropEditorState::new(update(crop, CropAspect::Freehand)).unwrap();
        horizontal
            .apply(CropEditorAction::BeginDrag {
                grab: CropGrab::Center,
                x: 0.3,
                y: 0.3,
                modifiers: CropModifiers {
                    shift: false,
                    control: true,
                },
            })
            .unwrap();
        horizontal
            .apply(CropEditorAction::DragTo { x: 0.5, y: 0.6 })
            .unwrap();
        assert_box(horizontal.deferred_crop(), CropBox::new(0.4, 0.2, 0.8, 0.6));
    }

    #[test]
    fn drag_is_deferred_until_release_commit() {
        let original = CropBox::new(0.1, 0.1, 0.8, 0.8);
        let mut editor = CropEditorState::new(update(original, CropAspect::Freehand)).unwrap();
        editor
            .apply(CropEditorAction::BeginDrag {
                grab: CropGrab::Right,
                x: 0.8,
                y: 0.4,
                modifiers: CropModifiers::default(),
            })
            .unwrap();
        let result = editor
            .apply(CropEditorAction::DragTo { x: 0.6, y: 0.4 })
            .unwrap();
        assert_eq!(
            result,
            CropEditorResult::Deferred {
                crop: CropBox::new(0.1, 0.1, 0.6, 0.8)
            }
        );
        assert_eq!(editor.committed_crop(), original);
        assert_close(editor.deferred_crop().cw, 0.6);
        assert_eq!(
            editor.apply(CropEditorAction::CommitDrag).unwrap(),
            CropEditorResult::Committed(editor.snapshot())
        );
        assert_close(editor.committed_crop().cw, 0.6);
        assert!(!editor.is_dragging());
    }

    #[test]
    fn invalid_input_falls_back_to_freehand() {
        let mut editor =
            CropEditorState::new(update(CropBox::FULL, CropAspect::fixed(3, 2).unwrap())).unwrap();
        let result = editor
            .apply(CropEditorAction::SetAspectInput("no ratio".to_owned()))
            .unwrap();
        assert_eq!(editor.aspect(), CropAspect::Freehand);
        assert_eq!(
            result,
            CropEditorResult::InvalidAspect {
                error: CropAspectParseError::InvalidPositiveNumber,
                fallback: editor.snapshot()
            }
        );
    }

    #[test]
    fn reset_area_preserves_aspect_but_resets_the_crop_box() {
        let aspect = CropAspect::fixed(2, 1).unwrap();
        let mut editor =
            CropEditorState::new(update(CropBox::new(0.2, 0.2, 0.8, 0.8), aspect)).unwrap();
        assert_eq!(
            editor.apply(CropEditorAction::ResetArea).unwrap(),
            CropEditorResult::ResetArea(editor.snapshot())
        );
        assert_eq!(editor.aspect(), aspect);
        assert_box(editor.committed_crop(), CropBox::new(0.0, 0.0, 1.0, 0.5));
        assert_eq!(editor.deferred_crop(), editor.committed_crop());
    }

    #[test]
    fn reset_and_update_replace_both_deferred_and_committed_state() {
        let mut initial = update(
            CropBox::new(0.2, 0.2, 0.8, 0.8),
            CropAspect::fixed(3, 2).unwrap(),
        );
        initial.default_crop = CropBox::new(0.1, 0.15, 0.9, 0.95);
        let mut editor = CropEditorState::new(initial).unwrap();
        assert_eq!(
            editor.apply(CropEditorAction::Reset).unwrap(),
            CropEditorResult::Reset(editor.snapshot())
        );
        assert_box(editor.committed_crop(), CropBox::new(0.1, 0.15, 0.9, 0.95));
        assert_eq!(editor.aspect(), CropAspect::Freehand);
        assert_eq!(editor.deferred_crop(), editor.committed_crop());

        let replacement = CropEditorUpdate {
            crop: CropBox::new(0.3, 0.25, 0.7, 0.75),
            default_crop: CropBox::FULL,
            aspect: CropAspect::OriginalImage { flipped: true },
            image_width: 300.0,
            image_height: 200.0,
            bounds: CropBox::new(0.1, 0.1, 0.9, 0.9),
        };
        assert!(matches!(
            editor.apply(CropEditorAction::Update(replacement)).unwrap(),
            CropEditorResult::Updated(_)
        ));
        assert_box(editor.committed_crop(), replacement.crop);
        assert_eq!(editor.deferred_crop(), replacement.crop);
        assert_eq!(editor.aspect(), replacement.aspect);
        assert_eq!(editor.bounds(), replacement.bounds);
    }
}
