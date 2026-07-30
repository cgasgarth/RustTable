use rusttable_image::{ImageView, OwnedImage};
use rusttable_metadata::{
    CanonicalExifOutput, ImageMetadata, MetadataEntry, MetadataOutput, MetadataOutputLimits,
    MetadataPacket, MetadataText as CoreMetadataText, MetadataValue, Orientation, PositiveRational,
};
use std::num::NonZeroU32;

/// The canonical image and metadata pair handed to export encoders.
#[derive(Debug)]
pub struct CanonicalArtifact<'a> {
    image: &'a OwnedImage,
    metadata: ExportMetadata,
}

impl<'a> CanonicalArtifact<'a> {
    /// Creates an artifact from an already validated image allocation.
    #[must_use]
    pub const fn new(image: &'a OwnedImage, metadata: ExportMetadata) -> Self {
        Self { image, metadata }
    }

    #[must_use]
    pub const fn image(&self) -> &'a OwnedImage {
        self.image
    }

    /// Borrows the image through its checked descriptor.
    ///
    /// # Errors
    ///
    /// Returns the image view error if the checked image allocation is invalid.
    pub fn view(&self) -> Result<ImageView<'a>, rusttable_image::ImageViewError> {
        self.image.view()
    }

    #[must_use]
    pub const fn metadata(&self) -> &ExportMetadata {
        &self.metadata
    }
}

/// Metadata payloads that the canonical artifact boundary can currently carry.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ExportMetadata {
    icc_profile: Option<Vec<u8>>,
    exif: Option<Vec<u8>>,
    xmp: Option<Vec<u8>>,
    iptc: Option<Vec<u8>>,
    density: Option<Density>,
    text: Vec<MetadataText>,
    packet: Option<MetadataPacket>,
}

impl ExportMetadata {
    #[must_use]
    pub fn with_icc_profile(mut self, profile: impl Into<Vec<u8>>) -> Self {
        self.icc_profile = Some(profile.into());
        self
    }

    #[must_use]
    pub fn with_exif(mut self, exif: impl Into<Vec<u8>>) -> Self {
        self.exif = Some(exif.into());
        self
    }

    #[must_use]
    pub fn with_xmp(mut self, xmp: impl Into<Vec<u8>>) -> Self {
        self.xmp = Some(xmp.into());
        self
    }

    #[must_use]
    pub fn with_iptc(mut self, iptc: impl Into<Vec<u8>>) -> Self {
        self.iptc = Some(iptc.into());
        self
    }

    #[must_use]
    pub const fn with_density(mut self, density: Density) -> Self {
        self.density = Some(density);
        self
    }

    #[must_use]
    pub fn with_text(mut self, keyword: impl Into<String>, value: impl Into<String>) -> Self {
        self.text.push(MetadataText {
            keyword: keyword.into(),
            value: value.into(),
        });
        self
    }

    /// # Panics
    ///
    /// Panics only if the compile-time metadata output limits are invalid.
    #[must_use]
    pub fn from_packet(packet: MetadataPacket) -> Self {
        // Format views are canonical evidence, not necessarily format-native
        // payloads. Build the supported EXIF subset through the existing
        // bounded metadata output abstraction and keep the complete packet as
        // deterministic UTF-8 XMP evidence for PNG/iTXt round trips.
        let exif = image_metadata_from_packet(&packet).and_then(|metadata| {
            CanonicalExifOutput::new(
                MetadataOutputLimits::new(16 * 1024 * 1024, 32, 1 << 20, 16 * 1024 * 1024)
                    .expect("constant metadata output limits are valid"),
            )
            .encode_exif(&metadata)
            .ok()
            .flatten()
            .map(|encoded| encoded.as_bytes().to_vec())
        });
        let xmp = Some(packet_xmp(&packet).into_bytes());
        Self {
            icc_profile: None,
            exif,
            xmp,
            // PNG has no standard IPTC chunk. IPTC properties remain in the
            // packet/XMP evidence only when the immutable policy includes them.
            iptc: None,
            density: None,
            text: Vec::new(),
            packet: Some(packet),
        }
    }

    #[must_use]
    pub fn with_packet(mut self, packet: MetadataPacket) -> Self {
        self = Self::from_packet(packet);
        self
    }

    #[must_use]
    pub fn packet(&self) -> Option<&MetadataPacket> {
        self.packet.as_ref()
    }

    #[must_use]
    pub fn icc_profile(&self) -> Option<&[u8]> {
        self.icc_profile.as_deref()
    }

    #[must_use]
    pub fn exif(&self) -> Option<&[u8]> {
        self.exif.as_deref()
    }

    #[must_use]
    pub fn xmp(&self) -> Option<&[u8]> {
        self.xmp.as_deref()
    }

    #[must_use]
    pub fn iptc(&self) -> Option<&[u8]> {
        self.iptc.as_deref()
    }

    #[must_use]
    pub const fn density(&self) -> Option<Density> {
        self.density
    }

    #[must_use]
    pub fn text(&self) -> &[MetadataText] {
        &self.text
    }
}

fn image_metadata_from_packet(packet: &MetadataPacket) -> Option<ImageMetadata> {
    let mut entries = Vec::new();
    for property in packet.properties() {
        if property.namespace != "exif" {
            continue;
        }
        let entry = match (property.name.as_str(), &property.value) {
            ("CameraMake", MetadataValue::Text(value)) => CoreMetadataText::new(value)
                .ok()
                .map(MetadataEntry::CameraMake),
            ("CameraModel", MetadataValue::Text(value)) => CoreMetadataText::new(value)
                .ok()
                .map(MetadataEntry::CameraModel),
            ("LensModel", MetadataValue::Text(value)) => CoreMetadataText::new(value)
                .ok()
                .map(MetadataEntry::LensModel),
            ("CaptureDateTimeOriginal", MetadataValue::DateTime(value)) => {
                CoreMetadataText::new(value)
                    .ok()
                    .map(MetadataEntry::CaptureDateTimeOriginal)
            }
            ("Orientation", MetadataValue::Integer(value)) => u8::try_from(*value)
                .ok()
                .and_then(|value| Orientation::from_u8(value).ok())
                .map(MetadataEntry::Orientation),
            (
                "ExposureTime",
                MetadataValue::Rational {
                    numerator,
                    denominator,
                },
            ) => positive_rational(*numerator, *denominator).map(MetadataEntry::ExposureTime),
            (
                "FNumber",
                MetadataValue::Rational {
                    numerator,
                    denominator,
                },
            ) => positive_rational(*numerator, *denominator).map(MetadataEntry::FNumber),
            ("IsoSpeed", MetadataValue::Integer(value)) => u32::try_from(*value)
                .ok()
                .and_then(NonZeroU32::new)
                .map(MetadataEntry::IsoSpeed),
            (
                "FocalLength",
                MetadataValue::Rational {
                    numerator,
                    denominator,
                },
            ) => positive_rational(*numerator, *denominator).map(MetadataEntry::FocalLength),
            _ => None,
        };
        if let Some(entry) = entry {
            entries.push(entry);
        }
    }
    ImageMetadata::from_entries(entries).ok()
}

fn positive_rational(numerator: i64, denominator: u64) -> Option<PositiveRational> {
    u64::try_from(numerator)
        .ok()
        .and_then(|numerator| PositiveRational::new(numerator, denominator).ok())
}

fn packet_xmp(packet: &MetadataPacket) -> String {
    let mut xmp = String::from(
        "<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"><rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\"><rdf:Description xmlns:rusttable=\"https://rusttable.app/ns/1.0/\">",
    );
    for property in packet.properties() {
        xmp.push_str("<rusttable:Property rusttable:name=\"");
        xmp.push_str(&escape_xml(&format!(
            "{}:{}",
            property.namespace, property.name
        )));
        xmp.push_str("\">");
        xmp.push_str(&escape_xml(&format!("{:?}", property.value)));
        xmp.push_str("</rusttable:Property>");
    }
    xmp.push_str("</rdf:Description></rdf:RDF></x:xmpmeta>");
    xmp
}

fn escape_xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// A bounded text metadata field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetadataText {
    keyword: String,
    value: String,
}

impl MetadataText {
    #[must_use]
    pub fn keyword(&self) -> &str {
        &self.keyword
    }

    #[must_use]
    pub fn value(&self) -> &str {
        &self.value
    }
}

/// Physical pixel density.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Density {
    x: u32,
    y: u32,
    unit: DensityUnit,
}

impl Density {
    /// Creates a nonzero density.
    ///
    /// # Errors
    ///
    /// Returns `DensityError::Zero` when either axis is zero.
    pub const fn new(x: u32, y: u32, unit: DensityUnit) -> Result<Self, DensityError> {
        if x == 0 || y == 0 {
            return Err(DensityError::Zero);
        }
        Ok(Self { x, y, unit })
    }

    #[must_use]
    pub const fn x(self) -> u32 {
        self.x
    }
    #[must_use]
    pub const fn y(self) -> u32 {
        self.y
    }
    #[must_use]
    pub const fn unit(self) -> DensityUnit {
        self.unit
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DensityUnit {
    Inch,
    Centimeter,
    Meter,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DensityError {
    Zero,
}
