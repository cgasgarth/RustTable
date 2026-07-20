use std::collections::BTreeMap;

use crate::{MetadataPacket, MetadataProperty};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FormatViewKind {
    Exif,
    IptcIim,
    Xmp,
    JpegMarkers,
    PngChunks,
    TiffTags,
    AvifHeifItems,
    JxlBoxes,
    PdfXmp,
    XcfParasites,
    SidecarXmp,
    Jp2Boxes,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatView {
    pub kind: FormatViewKind,
    pub bytes: Vec<u8>,
    pub properties: Vec<String>,
}

impl MetadataPacket {
    #[must_use]
    pub fn view(&self, kind: FormatViewKind) -> Option<&FormatView> {
        self.views.get(&kind)
    }

    pub fn views(&self) -> impl Iterator<Item = (&FormatViewKind, &FormatView)> {
        self.views.iter()
    }
}

pub(crate) fn build_views(properties: &[MetadataProperty]) -> BTreeMap<FormatViewKind, FormatView> {
    let kinds = [
        FormatViewKind::Exif,
        FormatViewKind::IptcIim,
        FormatViewKind::Xmp,
        FormatViewKind::JpegMarkers,
        FormatViewKind::PngChunks,
        FormatViewKind::TiffTags,
        FormatViewKind::AvifHeifItems,
        FormatViewKind::JxlBoxes,
        FormatViewKind::PdfXmp,
        FormatViewKind::XcfParasites,
        FormatViewKind::SidecarXmp,
        FormatViewKind::Jp2Boxes,
    ];
    kinds
        .into_iter()
        .map(|kind| {
            let mut bytes = Vec::new();
            bytes.extend_from_slice(b"RustTable-Metadata-View/1\0");
            bytes.extend_from_slice(format!("{kind:?}\0").as_bytes());
            let mut names = Vec::with_capacity(properties.len());
            for property in properties {
                names.push(format!("{}:{}", property.namespace, property.name));
                bytes.extend_from_slice(property.namespace.as_bytes());
                bytes.push(b'\0');
                bytes.extend_from_slice(property.name.as_bytes());
                bytes.push(b'\0');
                encode_value(&mut bytes, &property.value);
            }
            (
                kind,
                FormatView {
                    kind,
                    bytes,
                    properties: names,
                },
            )
        })
        .collect()
}

fn encode_value(bytes: &mut Vec<u8>, value: &crate::MetadataValue) {
    match value {
        crate::MetadataValue::Text(value) | crate::MetadataValue::DateTime(value) => {
            bytes.push(b't');
            bytes.extend_from_slice(value.as_bytes());
        }
        crate::MetadataValue::Integer(value) => {
            bytes.push(b'i');
            bytes.extend_from_slice(&value.to_be_bytes());
        }
        crate::MetadataValue::Rational {
            numerator,
            denominator,
        } => {
            bytes.push(b'r');
            bytes.extend_from_slice(&numerator.to_be_bytes());
            bytes.extend_from_slice(&denominator.to_be_bytes());
        }
        crate::MetadataValue::SignedRational {
            numerator,
            denominator,
        } => {
            bytes.push(b's');
            bytes.extend_from_slice(&numerator.to_be_bytes());
            bytes.extend_from_slice(&denominator.to_be_bytes());
        }
        crate::MetadataValue::Float(value) => {
            bytes.push(b'f');
            bytes.extend_from_slice(&value.to_be_bytes());
        }
        crate::MetadataValue::Binary(value) => {
            bytes.push(b'b');
            bytes.extend_from_slice(value);
        }
        crate::MetadataValue::Array(values) => {
            bytes.push(b'a');
            for value in values {
                encode_value(bytes, value);
            }
        }
        crate::MetadataValue::Region {
            x,
            y,
            width,
            height,
        } => {
            bytes.push(b'g');
            bytes.extend_from_slice(&x.to_be_bytes());
            bytes.extend_from_slice(&y.to_be_bytes());
            bytes.extend_from_slice(&width.to_be_bytes());
            bytes.extend_from_slice(&height.to_be_bytes());
        }
    }
    bytes.push(0);
}
