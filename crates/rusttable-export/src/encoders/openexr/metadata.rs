use std::collections::HashMap;

use exr::meta::attribute::Chromaticities;
use exr::prelude::{AttributeValue, ImageAttributes, LayerAttributes, SmallVec, Text, Vec2};
use rusttable_color::ColorEncoding;

use crate::{CanonicalArtifact, ExportMetadata};

const ICC_ATTRIBUTE: &str = "rusttable.icc_profile";
const XMP_ATTRIBUTE: &str = "rusttable.xmp";
const EXIF_ATTRIBUTE: &str = "rusttable.exif";
const IPTC_ATTRIBUTE: &str = "rusttable.iptc";
const PACKET_ATTRIBUTE: &str = "rusttable.metadata_packet";
const ALPHA_ATTRIBUTE: &str = "rusttable.alpha_association";
const COLOR_ATTRIBUTE: &str = "rusttable.color_encoding";

pub(crate) struct Attributes {
    pub image: ImageAttributes,
    pub layer: LayerAttributes,
}

pub(crate) fn build(artifact: &CanonicalArtifact<'_>) -> Result<Attributes, &'static str> {
    let descriptor = artifact.image().descriptor();
    let size = descriptor.dimensions();
    let mut image = ImageAttributes::with_size((
        usize::try_from(size.width()).map_err(|_| "width")?,
        usize::try_from(size.height()).map_err(|_| "height")?,
    ));
    let mut layer = LayerAttributes {
        white_luminance: Some(100.0),
        software_name: Some(Text::new_or_panic("RustTable")),
        ..LayerAttributes::default()
    };

    if !descriptor.format().channels().is_mosaic()
        && descriptor.format().channels().channels() >= 3
        && let Some(chromaticities) = chromaticities(descriptor.color_encoding())
    {
        image.chromaticities = Some(chromaticities);
    }
    insert_color_attribute(&mut image.other, descriptor.color_encoding())?;
    let alpha = match descriptor.format().alpha() {
        rusttable_image::AlphaMode::None => "none",
        rusttable_image::AlphaMode::Straight => "straight",
        rusttable_image::AlphaMode::Premultiplied => "premultiplied",
    };
    insert_text(&mut layer.other, ALPHA_ATTRIBUTE, alpha)?;
    insert_metadata(&mut layer.other, artifact.metadata())?;
    Ok(Attributes { image, layer })
}

fn chromaticities(encoding: ColorEncoding) -> Option<Chromaticities> {
    let space = encoding.builtin()?;
    let primaries = space.primaries()?;
    let (red_x, red_y) = primaries.red();
    let (green_x, green_y) = primaries.green();
    let (blue_x, blue_y) = primaries.blue();
    let (white_x, white_y) = primaries.white().xy();
    Some(Chromaticities {
        red: Vec2(red_x.get(), red_y.get()),
        green: Vec2(green_x.get(), green_y.get()),
        blue: Vec2(blue_x.get(), blue_y.get()),
        white: Vec2(white_x, white_y),
    })
}

fn insert_metadata(
    other: &mut HashMap<Text, AttributeValue>,
    metadata: &ExportMetadata,
) -> Result<(), &'static str> {
    insert_bytes(other, ICC_ATTRIBUTE, metadata.icc_profile(), "icc")?;
    insert_bytes(other, XMP_ATTRIBUTE, metadata.xmp(), "xmp")?;
    insert_bytes(other, EXIF_ATTRIBUTE, metadata.exif(), "exif")?;
    insert_bytes(other, IPTC_ATTRIBUTE, metadata.iptc(), "iptc")?;
    if let Some(packet) = metadata.packet() {
        let bytes = packet.canonical_bytes();
        insert_bytes(
            other,
            PACKET_ATTRIBUTE,
            Some(bytes.as_slice()),
            "metadata-packet",
        )?;
    }
    for text in metadata.text() {
        let name = format!("rusttable.text.{}", text.keyword());
        insert_text(other, &name, text.value())?;
    }
    Ok(())
}

fn insert_bytes(
    other: &mut HashMap<Text, AttributeValue>,
    name: &str,
    value: Option<&[u8]>,
    type_hint: &str,
) -> Result<(), &'static str> {
    if let Some(value) = value {
        if value.is_empty() || value.len() > 16 * 1024 * 1024 {
            return Err("metadata attribute exceeds the safe bound");
        }
        let key = Text::new_or_none(name).ok_or("metadata attribute name is not ASCII")?;
        let type_hint = Text::new_or_none(type_hint).ok_or("metadata type hint is not ASCII")?;
        other.insert(
            key,
            AttributeValue::Bytes {
                type_hint,
                bytes: SmallVec::from_slice(value),
            },
        );
    }
    Ok(())
}

fn insert_text(
    other: &mut HashMap<Text, AttributeValue>,
    name: &str,
    value: &str,
) -> Result<(), &'static str> {
    if value.is_empty() || value.len() > 1024 || value.contains('\0') {
        return Err("metadata text exceeds the safe bound");
    }
    let key = Text::new_or_none(name).ok_or("metadata attribute name is not ASCII")?;
    let value = Text::new_or_none(value).ok_or("metadata text is not ASCII")?;
    other.insert(key, AttributeValue::Text(value));
    Ok(())
}

fn insert_color_attribute(
    other: &mut HashMap<Text, AttributeValue>,
    encoding: ColorEncoding,
) -> Result<(), &'static str> {
    let value = format!("{encoding:?}");
    insert_text(other, COLOR_ATTRIBUTE, &value)
}
