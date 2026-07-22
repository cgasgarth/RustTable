use rawler::rawimage::RawImage;

use super::{convert_rect, dimensions, invalid, orientation, safe_text};
use crate::raw::{RawCameraIdentity, RawContainerProbe, RawDecodeError, RawHeader, RawRect};

pub(super) fn header_from_backend(
    image: &RawImage,
    probe: &RawContainerProbe,
) -> Result<RawHeader, RawDecodeError> {
    let dimensions = dimensions(image.width, image.height)?;
    let full_area = RawRect::new(0, 0, dimensions.width, dimensions.height).map_err(invalid)?;
    let active_area = image
        .active_area
        .map(convert_rect)
        .transpose()?
        .unwrap_or(full_area);
    let crop_area = image
        .crop_area
        .map(convert_rect)
        .transpose()?
        .unwrap_or(active_area);
    let bit_depth = probe
        .evidence
        .bit_depth
        .or_else(|| image.camera.bps.and_then(|value| u8::try_from(value).ok()))
        .unwrap_or_else(|| u8::try_from(image.bps).unwrap_or_default());
    Ok(RawHeader {
        container: probe.container,
        dimensions,
        active_area,
        crop_area,
        orientation: orientation(image.orientation),
        camera: RawCameraIdentity {
            maker: safe_text(&image.make),
            model: safe_text(&image.model),
            normalized_maker: safe_text(&image.clean_make),
            normalized_model: safe_text(&image.clean_model),
            mode: safe_text(&image.camera.mode),
        },
        compression: probe.evidence.compression.clone(),
        bit_depth,
    })
}

impl Default for super::RawlerRawDecoder {
    fn default() -> Self {
        Self::new()
    }
}
