#![allow(clippy::struct_excessive_bools)]

use std::collections::BTreeSet;

use rusttable_image::OutputFormat;

use crate::{AlphaPolicy, BitDepth, ChannelLayout, ExportRequest, MetadataAction, PixelEncoding};

/// A stable metadata field identifier used by capability negotiation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MetadataField {
    Exif,
    Iptc,
    Xmp,
    Gps,
    FacesAndRegions,
    RatingsLabelsTags,
    History,
    Thumbnail,
    IccAndCicp,
    SoftwareAndVersion,
    UserFields,
}

/// The encoder-side part of the canonical export capability contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncoderCapabilityDescriptor {
    id: String,
    formats: BTreeSet<OutputFormat>,
    channel_layouts: BTreeSet<ChannelLayout>,
    bit_depths: BTreeSet<BitDepth>,
    float16: bool,
    float32: bool,
    profiles: bool,
    alpha: bool,
    metadata: BTreeSet<MetadataField>,
}

impl EncoderCapabilityDescriptor {
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            formats: BTreeSet::new(),
            channel_layouts: BTreeSet::new(),
            bit_depths: BTreeSet::new(),
            float16: false,
            float32: false,
            profiles: false,
            alpha: false,
            metadata: BTreeSet::new(),
        }
    }

    #[must_use]
    pub fn all_formats(mut self) -> Self {
        self.formats.extend([
            OutputFormat::Png,
            OutputFormat::Jpeg,
            OutputFormat::JpegXl,
            OutputFormat::Tiff,
            OutputFormat::Webp,
            OutputFormat::Pdf,
            OutputFormat::Xcf,
            OutputFormat::Avif,
            OutputFormat::Heif,
            OutputFormat::Heic,
            OutputFormat::Jpeg2000,
            OutputFormat::Jp2,
            OutputFormat::OpenExr,
        ]);
        self
    }

    #[must_use]
    pub fn with_format(mut self, format: OutputFormat) -> Self {
        self.formats.insert(format);
        self
    }

    #[must_use]
    pub fn with_channel_layout(mut self, layout: ChannelLayout) -> Self {
        self.channel_layouts.insert(layout);
        self
    }

    #[must_use]
    pub fn with_bit_depth(mut self, depth: BitDepth) -> Self {
        self.bit_depths.insert(depth);
        self
    }

    #[must_use]
    pub const fn supports_float16(mut self) -> Self {
        self.float16 = true;
        self
    }

    #[must_use]
    pub const fn supports_float32(mut self) -> Self {
        self.float32 = true;
        self
    }

    #[must_use]
    pub const fn supports_profiles(mut self) -> Self {
        self.profiles = true;
        self
    }

    #[must_use]
    pub const fn supports_alpha(mut self) -> Self {
        self.alpha = true;
        self
    }

    #[must_use]
    pub fn with_metadata(mut self, field: MetadataField) -> Self {
        self.metadata.insert(field);
        self
    }

    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    #[must_use]
    pub fn formats(&self) -> &BTreeSet<OutputFormat> {
        &self.formats
    }

    #[must_use]
    pub fn channel_layouts(&self) -> &BTreeSet<ChannelLayout> {
        &self.channel_layouts
    }

    #[must_use]
    pub fn bit_depths(&self) -> &BTreeSet<BitDepth> {
        &self.bit_depths
    }

    #[must_use]
    pub const fn supports_float16_value(&self) -> bool {
        self.float16
    }

    #[must_use]
    pub const fn supports_float32_value(&self) -> bool {
        self.float32
    }

    #[must_use]
    pub const fn supports_profiles_value(&self) -> bool {
        self.profiles
    }

    #[must_use]
    pub const fn supports_alpha_value(&self) -> bool {
        self.alpha
    }

    #[must_use]
    pub fn supports_metadata(&self, field: MetadataField) -> bool {
        self.metadata.contains(&field)
    }
}

/// Destination behavior that is part of recipe/request validation without exposing a path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DestinationCapabilityDescriptor {
    id: String,
    replace_existing: bool,
}

impl DestinationCapabilityDescriptor {
    #[must_use]
    pub fn new(id: impl Into<String>, replace_existing: bool) -> Self {
        Self {
            id: id.into(),
            replace_existing,
        }
    }

    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    #[must_use]
    pub const fn supports_replace_existing(&self) -> bool {
        self.replace_existing
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CapabilityFinding {
    EncoderIdMismatch {
        requested: String,
        available: String,
    },
    DestinationIdMismatch {
        requested: String,
        available: String,
    },
    UnsupportedFormat {
        format: OutputFormat,
    },
    UnsupportedChannels {
        layout: ChannelLayout,
    },
    UnsupportedBitDepth {
        depth: BitDepth,
    },
    UnsupportedFloat16,
    UnsupportedFloat32,
    UnsupportedProfile,
    UnsupportedAlpha,
    UnsupportedMetadata {
        field: MetadataField,
        action: MetadataAction,
    },
    UnsupportedCollisionPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityAlternative {
    pub field: &'static str,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CapabilityReport {
    findings: Vec<CapabilityFinding>,
    alternatives: Vec<CapabilityAlternative>,
}

impl CapabilityReport {
    #[must_use]
    pub const fn is_supported(&self) -> bool {
        self.findings.is_empty()
    }

    #[must_use]
    pub fn findings(&self) -> &[CapabilityFinding] {
        &self.findings
    }

    #[must_use]
    pub fn alternatives(&self) -> &[CapabilityAlternative] {
        &self.alternatives
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilitySet {
    pub encoder: EncoderCapabilityDescriptor,
    pub destination: DestinationCapabilityDescriptor,
}

impl CapabilitySet {
    #[must_use]
    pub const fn new(
        encoder: EncoderCapabilityDescriptor,
        destination: DestinationCapabilityDescriptor,
    ) -> Self {
        Self {
            encoder,
            destination,
        }
    }

    #[must_use]
    pub fn negotiate(&self, request: &ExportRequest) -> CapabilityReport {
        let mut report = CapabilityReport::default();
        let requested_encoder = request
            .encoder_settings()
            .parameters()
            .get("encoder_id")
            .map_or("", String::as_str);
        if !requested_encoder.is_empty() && requested_encoder != self.encoder.id() {
            report.findings.push(CapabilityFinding::EncoderIdMismatch {
                requested: requested_encoder.to_owned(),
                available: self.encoder.id().to_owned(),
            });
        }
        if !self
            .destination
            .id()
            .eq(request.destination().destination_id())
        {
            report
                .findings
                .push(CapabilityFinding::DestinationIdMismatch {
                    requested: request.destination().destination_id().to_owned(),
                    available: self.destination.id().to_owned(),
                });
        }
        let settings = request.encoder_settings();
        if !self.encoder.formats().contains(&settings.format()) {
            report.findings.push(CapabilityFinding::UnsupportedFormat {
                format: settings.format(),
            });
            for format in self.encoder.formats() {
                report.alternatives.push(CapabilityAlternative {
                    field: "format",
                    value: format_id(*format).to_owned(),
                });
            }
        }
        let encoding = request.pixel_encoding();
        if !self
            .encoder
            .channel_layouts()
            .contains(&encoding.channels())
        {
            report
                .findings
                .push(CapabilityFinding::UnsupportedChannels {
                    layout: encoding.channels(),
                });
        }
        match encoding {
            PixelEncoding::Float16 { .. } if !self.encoder.supports_float16_value() => {
                report.findings.push(CapabilityFinding::UnsupportedFloat16);
            }
            PixelEncoding::Float32 { .. } if !self.encoder.supports_float32_value() => {
                report.findings.push(CapabilityFinding::UnsupportedFloat32);
            }
            PixelEncoding::Integer { depth, .. } if !self.encoder.bit_depths().contains(&depth) => {
                report
                    .findings
                    .push(CapabilityFinding::UnsupportedBitDepth { depth });
            }
            _ => {}
        }
        if request.output_profile().profile_id().is_some()
            && !self.encoder.supports_profiles_value()
        {
            report.findings.push(CapabilityFinding::UnsupportedProfile);
        }
        if request.alpha_policy() == AlphaPolicy::Require && !self.encoder.supports_alpha_value() {
            report.findings.push(CapabilityFinding::UnsupportedAlpha);
        }
        for (field, action) in metadata_actions(request.metadata_policy()) {
            if action != MetadataAction::Exclude && !self.encoder.supports_metadata(field) {
                report
                    .findings
                    .push(CapabilityFinding::UnsupportedMetadata { field, action });
            }
        }
        if request.destination().collision() == crate::CollisionPolicy::ReplaceExisting
            && !self.destination.supports_replace_existing()
        {
            report
                .findings
                .push(CapabilityFinding::UnsupportedCollisionPolicy);
        }
        report
    }
}

fn metadata_actions(policy: crate::MetadataPolicy) -> [(MetadataField, MetadataAction); 11] {
    [
        (MetadataField::Exif, policy.exif),
        (MetadataField::Iptc, policy.iptc),
        (MetadataField::Xmp, policy.xmp),
        (MetadataField::Gps, policy.gps),
        (MetadataField::FacesAndRegions, policy.faces_and_regions),
        (MetadataField::RatingsLabelsTags, policy.ratings_labels_tags),
        (MetadataField::History, policy.history),
        (MetadataField::Thumbnail, policy.thumbnail),
        (MetadataField::IccAndCicp, policy.icc_and_cicp),
        (
            MetadataField::SoftwareAndVersion,
            policy.software_and_version,
        ),
        (MetadataField::UserFields, policy.user_fields),
    ]
}

pub(crate) fn format_id(format: OutputFormat) -> &'static str {
    match format {
        OutputFormat::Png => "png",
        OutputFormat::Jpeg => "jpeg",
        OutputFormat::JpegXl => "jpeg-xl",
        OutputFormat::Tiff => "tiff",
        OutputFormat::Webp => "webp",
        OutputFormat::Pdf => "pdf",
        OutputFormat::Xcf => "xcf",
        OutputFormat::Avif => "avif",
        OutputFormat::Heif => "heif",
        OutputFormat::Heic => "heic",
        OutputFormat::Jpeg2000 => "j2k",
        OutputFormat::Jp2 => "jp2",
        OutputFormat::OpenExr => "openexr",
    }
}
