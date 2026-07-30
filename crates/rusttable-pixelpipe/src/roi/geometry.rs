use super::{RationalScale, RoiError, RoiRect, RoiSupport};
use rusttable_image::{ImageDimensions, Orientation};

pub(super) fn translate(roi: RoiRect, dx: i64, dy: i64) -> Result<RoiRect, String> {
    let x = i64::from(roi.x) + dx;
    let y = i64::from(roi.y) + dy;
    if x < 0 || y < 0 || x > i64::from(u32::MAX) || y > i64::from(u32::MAX) {
        return Err("translated ROI is outside nonnegative logical space".to_owned());
    }
    RoiRect::new(
        u32::try_from(x).map_err(|error| error.to_string())?,
        u32::try_from(y).map_err(|error| error.to_string())?,
        roi.width,
        roi.height,
    )
    .map_err(|error| error.to_string())
}

pub(super) fn translate_clipped(roi: RoiRect, dx: i64, dy: i64, bounds: RoiRect) -> RoiRect {
    let x = i64::from(roi.x) + dx;
    let y = i64::from(roi.y) + dy;
    let right = x + i64::from(roi.width);
    let bottom = y + i64::from(roi.height);
    let left = x.max(i64::from(bounds.x));
    let top = y.max(i64::from(bounds.y));
    let clipped_right = right.min(i64::from(bounds.right()));
    let clipped_bottom = bottom.min(i64::from(bounds.bottom()));
    let left = u32::try_from(left.clamp(0, i64::from(u32::MAX))).expect("clamped coordinate");
    let top = u32::try_from(top.clamp(0, i64::from(u32::MAX))).expect("clamped coordinate");
    let clipped_right = u32::try_from(clipped_right.clamp(i64::from(left), i64::from(u32::MAX)))
        .expect("clamped coordinate");
    let clipped_bottom = u32::try_from(clipped_bottom.clamp(i64::from(top), i64::from(u32::MAX)))
        .expect("clamped coordinate");
    RoiRect {
        x: left,
        y: top,
        width: clipped_right.saturating_sub(left),
        height: clipped_bottom.saturating_sub(top),
    }
}

pub(super) fn expand(roi: RoiRect, support: RoiSupport) -> Result<RoiRect, RoiError> {
    let x = roi.x.saturating_sub(support.left);
    let y = roi.y.saturating_sub(support.top);
    let right = roi
        .right()
        .checked_add(support.right)
        .ok_or(RoiError::Overflow)?;
    let bottom = roi
        .bottom()
        .checked_add(support.bottom)
        .ok_or(RoiError::Overflow)?;
    RoiRect::new(x, y, right.saturating_sub(x), bottom.saturating_sub(y))
}

pub(super) fn dimensions_for_bounds(bounds: RoiRect) -> Result<ImageDimensions, String> {
    ImageDimensions::new(bounds.right(), bounds.bottom()).map_err(|error| error.to_string())
}

pub(super) fn scaled_dimensions(
    dimensions: ImageDimensions,
    x: RationalScale,
    y: RationalScale,
) -> Result<ImageDimensions, String> {
    let width =
        ceil_ratio(dimensions.width(), x).ok_or_else(|| "scaled width overflowed".to_owned())?;
    let height =
        ceil_ratio(dimensions.height(), y).ok_or_else(|| "scaled height overflowed".to_owned())?;
    ImageDimensions::new(width, height).map_err(|error| error.to_string())
}

fn ceil_ratio(value: u32, scale: RationalScale) -> Option<u32> {
    u64::from(value)
        .checked_mul(u64::from(scale.numerator()))?
        .checked_add(u64::from(scale.denominator()) - 1)?
        .checked_div(u64::from(scale.denominator()))
        .and_then(|v| u32::try_from(v).ok())
}

pub(super) fn scale_out(
    roi: RoiRect,
    x: RationalScale,
    y: RationalScale,
) -> Result<RoiRect, String> {
    let left = floor_ratio(roi.x, x).ok_or_else(|| "scaled x overflowed".to_owned())?;
    let top = floor_ratio(roi.y, y).ok_or_else(|| "scaled y overflowed".to_owned())?;
    let right = ceil_ratio(roi.right(), x).ok_or_else(|| "scaled right overflowed".to_owned())?;
    let bottom =
        ceil_ratio(roi.bottom(), y).ok_or_else(|| "scaled bottom overflowed".to_owned())?;
    RoiRect::new(
        left,
        top,
        right.saturating_sub(left),
        bottom.saturating_sub(top),
    )
    .map_err(|error| error.to_string())
}

pub(super) fn scale_in(
    roi: RoiRect,
    x: RationalScale,
    y: RationalScale,
) -> Result<RoiRect, String> {
    let x_scale =
        RationalScale::new(x.denominator(), x.numerator()).map_err(|error| error.to_string())?;
    let y_scale =
        RationalScale::new(y.denominator(), y.numerator()).map_err(|error| error.to_string())?;
    scale_out(roi, x_scale, y_scale)
}

fn floor_ratio(value: u32, scale: RationalScale) -> Option<u32> {
    u64::from(value)
        .checked_mul(u64::from(scale.numerator()))
        .and_then(|v| v.checked_div(u64::from(scale.denominator())))
        .and_then(|v| u32::try_from(v).ok())
}

pub(super) fn map_orientation(
    roi: RoiRect,
    source: ImageDimensions,
    orientation: Orientation,
) -> Result<RoiRect, String> {
    let w = i64::from(source.width());
    let h = i64::from(source.height());
    let (x0, y0, x1, y1) = (
        i64::from(roi.x),
        i64::from(roi.y),
        i64::from(roi.right()),
        i64::from(roi.bottom()),
    );
    let corners = [(x0, y0), (x1, y0), (x0, y1), (x1, y1)];
    let mapped = corners.map(|(x, y)| match orientation {
        Orientation::Normal => (x, y),
        Orientation::FlipHorizontal => (w - x, y),
        Orientation::Rotate180 => (w - x, h - y),
        Orientation::FlipVertical => (x, h - y),
        Orientation::Transpose => (y, x),
        Orientation::Rotate90 => (h - y, x),
        Orientation::Transverse => (h - y, w - x),
        Orientation::Rotate270 => (y, w - x),
    });
    let min_x = mapped.iter().map(|point| point.0).min().unwrap_or(0);
    let min_y = mapped.iter().map(|point| point.1).min().unwrap_or(0);
    let max_x = mapped.iter().map(|point| point.0).max().unwrap_or(0);
    let max_y = mapped.iter().map(|point| point.1).max().unwrap_or(0);
    if min_x < 0 || min_y < 0 || max_x < min_x || max_y < min_y {
        return Err("orientation produced invalid ROI".to_owned());
    }
    RoiRect::new(
        u32::try_from(min_x).map_err(|error| error.to_string())?,
        u32::try_from(min_y).map_err(|error| error.to_string())?,
        u32::try_from(max_x - min_x).map_err(|error| error.to_string())?,
        u32::try_from(max_y - min_y).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())
}
