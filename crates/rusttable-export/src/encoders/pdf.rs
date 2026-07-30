//! Deterministic, single-image PDF export.
//!
//! The writer deliberately exposes only the reviewed image-PDF subset. It does not
//! claim PDF/A, PDF/X, print, destination, or multi-page support.

use std::fmt;
use std::io::{self, Write};
use std::path::Path;

use flate2::Compression as FlateCompression;
use flate2::write::ZlibEncoder;
use pdf_writer::{Content, Filter, Finish, Name, Pdf, Rect, Ref, TextStr};
use rusttable_color::ColorEncoding;
use rusttable_image::{AlphaMode, ChannelLayout, SampleType, StorageLayout};
use sha2::{Digest, Sha256};

use crate::capabilities::{EncoderCapabilityDescriptor, MetadataField};
use crate::encoders::raster::{RasterError, digest, metadata_len, row, shape, validate_metadata};
use crate::{CanonicalArtifact, EncodeBudget, EncodeCancellation, NeverCancel};

pub const SETTINGS_SCHEMA_VERSION: u16 = 1;
const MAX_OUTPUT_BYTES: usize = 512 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Unit {
    Point,
    Millimeter,
    Inch,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PageSize {
    Image,
    A4,
    Letter,
    Custom { width: f32, height: f32, unit: Unit },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Orientation {
    Portrait,
    Landscape,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    Fit,
    FillCrop,
    ActualSize,
    ExactPhysical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Alignment {
    Start,
    Center,
    End,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Overflow {
    Clip,
    Reject,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compression {
    Flate,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Settings {
    pub page: PageSize,
    pub orientation: Orientation,
    pub margin: f32,
    pub dpi: u32,
    pub placement: Placement,
    pub horizontal_alignment: Alignment,
    pub vertical_alignment: Alignment,
    pub background: [u8; 3],
    pub compression: Compression,
    pub overflow: Overflow,
    pub max_metadata_bytes: usize,
    pub max_output_bytes: usize,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            page: PageSize::Image,
            orientation: Orientation::Portrait,
            margin: 0.0,
            dpi: 300,
            placement: Placement::Fit,
            horizontal_alignment: Alignment::Center,
            vertical_alignment: Alignment::Center,
            background: [255, 255, 255],
            compression: Compression::Flate,
            overflow: Overflow::Reject,
            max_metadata_bytes: 16 * 1024 * 1024,
            max_output_bytes: MAX_OUTPUT_BYTES,
        }
    }
}

impl Settings {
    /// # Errors
    ///
    /// Returns an error when numeric settings are invalid or exceed bounded export limits.
    pub fn validate(self) -> Result<(), Error> {
        if self.dpi == 0 || self.dpi > 100_000 {
            return Err(Error::InvalidSettings("DPI"));
        }
        if !self.margin.is_finite() || self.margin < 0.0 {
            return Err(Error::InvalidSettings("margin"));
        }
        if self.max_output_bytes == 0 || self.max_output_bytes > MAX_OUTPUT_BYTES {
            return Err(Error::InvalidSettings("output limit"));
        }
        if let PageSize::Custom { width, height, .. } = self.page
            && (!width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0)
        {
            return Err(Error::InvalidSettings("custom page"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rectangle {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Receipt {
    pub page: Rectangle,
    pub image: Rectangle,
    pub raster_dimensions: rusttable_image::ImageDimensions,
    pub compression: Compression,
    pub profile_embedded: bool,
    pub metadata_embedded: bool,
    pub object_count: u32,
    pub document_id: [u8; 16],
    pub encoded_bytes: u64,
    pub artifact_sha256: [u8; 32],
    pub output_sha256: [u8; 32],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inspection {
    pub version: (u8, u8),
    pub page_count: u32,
    pub image_dimensions: rusttable_image::ImageDimensions,
    pub has_soft_mask: bool,
    pub has_icc_profile: bool,
    pub has_xmp: bool,
    pub image_sha256: [u8; 32],
}

#[derive(Debug)]
pub enum Error {
    InvalidSettings(&'static str),
    UnsupportedLayout(ChannelLayout),
    UnsupportedSample(SampleType),
    UnsupportedAlpha(AlphaMode),
    UnsupportedStorage(StorageLayout),
    UnsupportedColor(ColorEncoding),
    EmptyProfile,
    MetadataLimit { limit: usize, actual: usize },
    InvalidText,
    NonFiniteSample,
    Geometry(&'static str),
    OutputLimit { limit: usize, actual: usize },
    Cancelled,
    Io(io::Error),
    Malformed(&'static str),
    RasterMismatch,
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSettings(value) => write!(formatter, "invalid PDF settings: {value}"),
            Self::UnsupportedLayout(value) => {
                write!(formatter, "unsupported PDF layout: {value:?}")
            }
            Self::UnsupportedSample(value) => {
                write!(formatter, "unsupported PDF sample: {value:?}")
            }
            Self::UnsupportedAlpha(value) => write!(formatter, "unsupported PDF alpha: {value:?}"),
            Self::UnsupportedStorage(value) => {
                write!(formatter, "unsupported PDF storage: {value:?}")
            }
            Self::UnsupportedColor(value) => write!(formatter, "unsupported PDF color: {value:?}"),
            Self::EmptyProfile => formatter.write_str("PDF ICC profile is empty"),
            Self::MetadataLimit { limit, actual } => write!(
                formatter,
                "PDF metadata is {actual} bytes, limit is {limit}"
            ),
            Self::InvalidText => formatter.write_str("invalid PDF metadata text"),
            Self::NonFiniteSample => formatter.write_str("PDF input contains a non-finite sample"),
            Self::Geometry(value) => write!(formatter, "invalid PDF geometry: {value}"),
            Self::OutputLimit { limit, actual } => {
                write!(formatter, "PDF output is {actual} bytes, limit is {limit}")
            }
            Self::Cancelled => formatter.write_str("PDF encoding cancelled"),
            Self::Io(error) => write!(formatter, "PDF I/O failed: {error}"),
            Self::Malformed(value) => write!(formatter, "malformed PDF: {value}"),
            Self::RasterMismatch => {
                formatter.write_str("PDF image stream differs from source raster")
            }
        }
    }
}

impl std::error::Error for Error {}
impl From<io::Error> for Error {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

#[must_use]
pub fn capabilities() -> EncoderCapabilityDescriptor {
    let mut descriptor = EncoderCapabilityDescriptor::new("pdf")
        .with_format(rusttable_image::OutputFormat::Pdf)
        .with_channel_layout(crate::ChannelLayout::Gray)
        .with_channel_layout(crate::ChannelLayout::Rgb)
        .with_channel_layout(crate::ChannelLayout::Rgba)
        .with_bit_depth(crate::BitDepth::Eight)
        .supports_profiles()
        .supports_alpha();
    for field in [
        MetadataField::Exif,
        MetadataField::Iptc,
        MetadataField::Xmp,
        MetadataField::IccAndCicp,
        MetadataField::SoftwareAndVersion,
    ] {
        descriptor = descriptor.with_metadata(field);
    }
    descriptor
}

#[derive(Debug, Clone, Copy)]
pub struct Encoder {
    settings: Settings,
}

impl Encoder {
    #[must_use]
    pub const fn new(settings: Settings) -> Self {
        Self { settings }
    }

    #[must_use]
    pub const fn settings(self) -> Settings {
        self.settings
    }

    /// # Errors
    ///
    /// Returns an error when the artifact, geometry, metadata, staging budget, or cancellation
    /// contract prevents a complete PDF from being produced.
    pub fn encode_to_vec(
        &self,
        artifact: &CanonicalArtifact<'_>,
    ) -> Result<(Vec<u8>, Receipt), Error> {
        self.encode_to_vec_with_budget(artifact, EncodeBudget::default(), &NeverCancel)
    }

    /// # Errors
    ///
    /// Returns an error when the artifact, geometry, metadata, staging budget, or cancellation
    /// contract prevents a complete PDF from being produced.
    pub fn encode_to_vec_with_budget<C: EncodeCancellation>(
        &self,
        artifact: &CanonicalArtifact<'_>,
        budget: EncodeBudget,
        cancellation: &C,
    ) -> Result<(Vec<u8>, Receipt), Error> {
        self.settings.validate()?;
        if cancellation.is_cancelled() {
            return Err(Error::Cancelled);
        }
        validate(artifact, self.settings)?;
        let geometry = solve_geometry(artifact, self.settings)?;
        let rgb = compressed_channels(artifact, false, cancellation)?;
        let alpha = if artifact.image().descriptor().format().channels().channels() == 4 {
            Some(compressed_channels(artifact, true, cancellation)?)
        } else {
            None
        };
        if cancellation.is_cancelled() {
            return Err(Error::Cancelled);
        }
        let staged_bytes = rgb
            .len()
            .saturating_add(alpha.as_deref().map_or(0, <[u8]>::len));
        if u64::try_from(staged_bytes).unwrap_or(u64::MAX) > budget.memory_bytes() {
            return Err(Error::OutputLimit {
                limit: usize::try_from(budget.memory_bytes()).unwrap_or(usize::MAX),
                actual: staged_bytes,
            });
        }
        let mut pdf = Pdf::with_capacity(rgb.len().saturating_add(8192));
        pdf.set_file_id((geometry.document_id.to_vec(), geometry.document_id.to_vec()));
        write_pdf(
            &mut pdf,
            artifact,
            self.settings,
            geometry,
            &rgb,
            alpha.as_deref(),
        )?;
        let bytes = pdf.finish();
        if bytes.len() > self.settings.max_output_bytes {
            return Err(Error::OutputLimit {
                limit: self.settings.max_output_bytes,
                actual: bytes.len(),
            });
        }
        let inspection = inspect(&bytes)?;
        if inspection.image_dimensions != artifact.image().descriptor().dimensions() {
            return Err(Error::RasterMismatch);
        }
        let receipt = Receipt {
            page: geometry.page,
            image: geometry.image,
            raster_dimensions: artifact.image().descriptor().dimensions(),
            compression: self.settings.compression,
            profile_embedded: artifact.metadata().icc_profile().is_some(),
            metadata_embedded: artifact.metadata().xmp().is_some() || metadata_len(artifact) > 0,
            object_count: if alpha.is_some() { 10 } else { 9 },
            document_id: geometry.document_id,
            encoded_bytes: bytes.len() as u64,
            artifact_sha256: artifact_hash(artifact),
            output_sha256: digest(&bytes),
        };
        Ok((bytes, receipt))
    }

    /// # Errors
    ///
    /// Returns an error when encoding or atomic path staging fails.
    pub fn encode_to_path(
        &self,
        artifact: &CanonicalArtifact<'_>,
        path: &Path,
    ) -> Result<Receipt, Error> {
        let (bytes, receipt) = self.encode_to_vec(artifact)?;
        let result = (|| {
            let mut file = std::fs::File::create(path)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            Ok::<_, io::Error>(())
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(path);
        }
        result.map_err(Error::Io)?;
        Ok(receipt)
    }
}

#[derive(Debug, Clone, Copy)]
struct Geometry {
    page: Rectangle,
    image: Rectangle,
    document_id: [u8; 16],
}

fn validate(artifact: &CanonicalArtifact<'_>, settings: Settings) -> Result<(), Error> {
    let shape = shape(artifact).map_err(map_raster_error)?;
    if shape.sample_type != SampleType::U8 {
        return Err(Error::UnsupportedSample(shape.sample_type));
    }
    if matches!(
        artifact.image().descriptor().format().alpha(),
        AlphaMode::Premultiplied
    ) {
        return Err(Error::UnsupportedAlpha(AlphaMode::Premultiplied));
    }
    if matches!(
        artifact.image().descriptor().format().channels(),
        ChannelLayout::Bayer | ChannelLayout::XTrans
    ) {
        return Err(Error::UnsupportedLayout(
            artifact.image().descriptor().format().channels(),
        ));
    }
    if artifact.image().descriptor().format().storage() != StorageLayout::Interleaved {
        return Err(Error::UnsupportedStorage(
            artifact.image().descriptor().format().storage(),
        ));
    }
    let color = artifact.image().descriptor().color_encoding();
    if matches!(color, ColorEncoding::External(_)) && artifact.metadata().icc_profile().is_none() {
        return Err(Error::UnsupportedColor(color));
    }
    validate_metadata(artifact, settings.max_metadata_bytes).map_err(map_raster_error)
}

fn solve_geometry(artifact: &CanonicalArtifact<'_>, settings: Settings) -> Result<Geometry, Error> {
    let dimensions = artifact.image().descriptor().dimensions();
    let source_width = points_for_pixels(dimensions.width(), settings.dpi);
    let source_height = points_for_pixels(dimensions.height(), settings.dpi);
    let mut page = match settings.page {
        PageSize::Image => Rectangle {
            x: 0.0,
            y: 0.0,
            width: source_width,
            height: source_height,
        },
        PageSize::A4 => points(210.0, 297.0, Unit::Millimeter),
        PageSize::Letter => points(8.5, 11.0, Unit::Inch),
        PageSize::Custom {
            width,
            height,
            unit,
        } => points(width, height, unit),
    };
    if matches!(settings.orientation, Orientation::Landscape) && page.height > page.width {
        std::mem::swap(&mut page.width, &mut page.height);
    }
    let printable_width = page.width - 2.0 * settings.margin;
    let printable_height = page.height - 2.0 * settings.margin;
    if printable_width <= 0.0 || printable_height <= 0.0 {
        return Err(Error::Geometry("margins remove printable area"));
    }
    let (mut width, mut height) = match settings.placement {
        Placement::ActualSize | Placement::ExactPhysical => (source_width, source_height),
        Placement::Fit => {
            let scale = (printable_width / source_width).min(printable_height / source_height);
            (source_width * scale, source_height * scale)
        }
        Placement::FillCrop => {
            let scale = (printable_width / source_width).max(printable_height / source_height);
            (source_width * scale, source_height * scale)
        }
    };
    if matches!(
        settings.placement,
        Placement::ActualSize | Placement::ExactPhysical
    ) && (width > printable_width || height > printable_height)
        && settings.overflow == Overflow::Reject
    {
        return Err(Error::Geometry("actual-size image exceeds printable area"));
    }
    if settings.overflow == Overflow::Clip {
        width = width.min(printable_width);
        height = height.min(printable_height);
    }
    let x = settings.margin + align(printable_width - width, settings.horizontal_alignment);
    let y = settings.margin + align(printable_height - height, settings.vertical_alignment);
    let document_id = Sha256::digest(
        format!("pdf-v{SETTINGS_SCHEMA_VERSION}:{page:?}:{x:.6}:{y:.6}:{width:.6}:{height:.6}")
            .as_bytes(),
    );
    Ok(Geometry {
        page,
        image: Rectangle {
            x,
            y,
            width,
            height,
        },
        document_id: document_id[..16].try_into().expect("fixed digest length"),
    })
}

#[expect(
    clippy::cast_precision_loss,
    reason = "pdf-writer accepts f32 coordinates and checked image dimensions are finite"
)]
fn points_for_pixels(pixels: u32, dpi: u32) -> f32 {
    pixels as f32 * 72.0 / dpi as f32
}

fn points(width: f32, height: f32, unit: Unit) -> Rectangle {
    let multiplier = match unit {
        Unit::Point => 1.0,
        Unit::Millimeter => 72.0 / 25.4,
        Unit::Inch => 72.0,
    };
    Rectangle {
        x: 0.0,
        y: 0.0,
        width: width * multiplier,
        height: height * multiplier,
    }
}

const fn align(available: f32, alignment: Alignment) -> f32 {
    match alignment {
        Alignment::Start => 0.0,
        Alignment::Center => available / 2.0,
        Alignment::End => available,
    }
}

fn compressed_channels<C: EncodeCancellation>(
    artifact: &CanonicalArtifact<'_>,
    alpha: bool,
    cancellation: &C,
) -> Result<Vec<u8>, Error> {
    let descriptor = artifact.image().descriptor();
    let format = descriptor.format();
    let channel_count = format.channels().channels();
    let mut encoder = ZlibEncoder::new(Vec::new(), FlateCompression::new(6));
    let width = descriptor.dimensions().width() as usize;
    for y in 0..descriptor.dimensions().height() {
        if cancellation.is_cancelled() {
            return Err(Error::Cancelled);
        }
        let source = row(artifact, y).map_err(map_raster_error)?;
        let mut output = Vec::with_capacity(width * if alpha { 1 } else { 3 });
        for pixel in 0..width {
            let offset = pixel * channel_count;
            match (format.channels(), alpha) {
                (ChannelLayout::Gray, false) => output.push(source[offset]),
                (ChannelLayout::GrayA, false) => output.extend_from_slice(&[source[offset]; 3]),
                (ChannelLayout::Rgb | ChannelLayout::Rgba, false) => {
                    output.extend_from_slice(&source[offset..offset + 3]);
                }
                (ChannelLayout::GrayA | ChannelLayout::Rgba, true) => {
                    output.push(source[offset + channel_count - 1]);
                }
                _ => return Err(Error::UnsupportedLayout(format.channels())),
            }
        }
        encoder.write_all(&output)?;
    }
    encoder.finish().map_err(Error::Io)
}

#[expect(
    clippy::too_many_lines,
    reason = "PDF object order is the reviewed fixed subset"
)]
fn write_pdf(
    pdf: &mut Pdf,
    artifact: &CanonicalArtifact<'_>,
    settings: Settings,
    geometry: Geometry,
    rgb: &[u8],
    alpha: Option<&[u8]>,
) -> Result<(), Error> {
    let catalog_id = Ref::new(1);
    let page_tree_id = Ref::new(2);
    let page_id = Ref::new(3);
    let content_id = Ref::new(4);
    let image_id = Ref::new(5);
    let mask_id = Ref::new(6);
    let icc_id = Ref::new(7);
    let metadata_id = Ref::new(9);
    let info_id = Ref::new(10);
    let image_name = Name(b"Im1");
    let has_profile = artifact.metadata().icc_profile().is_some();
    let components: i32 =
        if artifact.image().descriptor().format().channels() == ChannelLayout::Gray {
            1
        } else {
            3
        };
    {
        let mut catalog = pdf.catalog(catalog_id);
        catalog.pages(page_tree_id).metadata(metadata_id);
        if has_profile {
            let mut intents = catalog.output_intents();
            let mut intent = intents.push();
            intent
                .subtype(pdf_writer::types::OutputIntentSubtype::Custom(Name(
                    b"RustTable",
                )))
                .output_condition(TextStr("RustTable image output"))
                .output_condition_identifier(TextStr("RustTable"))
                .dest_output_profile(icc_id)
                .finish();
        }
    }
    pdf.pages(page_tree_id).kids([page_id]).count(1);
    {
        let mut page = pdf.page(page_id);
        page.media_box(Rect::new(
            geometry.page.x,
            geometry.page.y,
            geometry.page.width,
            geometry.page.height,
        ));
        page.parent(page_tree_id).contents(content_id);
        page.resources().x_objects().pair(image_name, image_id);
        if has_profile {
            page.resources()
                .color_spaces()
                .insert(Name(b"CS"))
                .start::<pdf_writer::writers::ColorSpace>()
                .icc_based(icc_id);
        }
    }
    let mut content = Content::new();
    content.set_fill_rgb(
        f32::from(settings.background[0]) / 255.0,
        f32::from(settings.background[1]) / 255.0,
        f32::from(settings.background[2]) / 255.0,
    );
    content
        .rect(
            geometry.page.x,
            geometry.page.y,
            geometry.page.width,
            geometry.page.height,
        )
        .fill_nonzero();
    content.save_state();
    if settings.placement == Placement::FillCrop {
        content
            .rect(
                settings.margin,
                settings.margin,
                geometry.page.width - 2.0 * settings.margin,
                geometry.page.height - 2.0 * settings.margin,
            )
            .clip_nonzero();
    }
    content.transform([
        geometry.image.width,
        0.0,
        0.0,
        geometry.image.height,
        geometry.image.x,
        geometry.image.y,
    ]);
    content.x_object(image_name).restore_state();
    pdf.stream(content_id, &content.finish());
    {
        let mut image = pdf.image_xobject(image_id, rgb);
        image.filter(Filter::FlateDecode);
        image.width(
            i32::try_from(artifact.image().descriptor().dimensions().width())
                .map_err(|_| Error::Geometry("image width exceeds PDF limit"))?,
        );
        image.height(
            i32::try_from(artifact.image().descriptor().dimensions().height())
                .map_err(|_| Error::Geometry("image height exceeds PDF limit"))?,
        );
        image.bits_per_component(8);
        if has_profile {
            image.color_space_name(Name(b"CS"));
        } else if components == 1 {
            image.color_space().device_gray();
        } else {
            image.color_space().device_rgb();
        }
        if alpha.is_some() {
            image.s_mask(mask_id);
        }
        image.finish();
    }
    if let Some(alpha) = alpha {
        let mut mask = pdf.image_xobject(mask_id, alpha);
        mask.filter(Filter::FlateDecode);
        mask.width(
            i32::try_from(artifact.image().descriptor().dimensions().width())
                .map_err(|_| Error::Geometry("mask width exceeds PDF limit"))?,
        );
        mask.height(
            i32::try_from(artifact.image().descriptor().dimensions().height())
                .map_err(|_| Error::Geometry("mask height exceeds PDF limit"))?,
        );
        mask.color_space().device_gray();
        mask.bits_per_component(8);
        mask.finish();
    }
    if let Some(profile) = artifact.metadata().icc_profile() {
        pdf.icc_profile(icc_id, profile)
            .n(components)
            .alternate_name(if components == 1 {
                Name(b"DeviceGray")
            } else {
                Name(b"DeviceRGB")
            })
            .finish();
    }
    let xmp = xmp_metadata(artifact);
    pdf.metadata(metadata_id, &xmp).finish();
    pdf.document_info(info_id)
        .title(TextStr("RustTable image"))
        .author(TextStr("RustTable"))
        .subject(TextStr("Single-image export"))
        .keywords(TextStr("RustTable PDF"));
    Ok(())
}

fn xmp_metadata(artifact: &CanonicalArtifact<'_>) -> Vec<u8> {
    if let Some(xmp) = artifact.metadata().xmp() {
        return xmp.to_vec();
    }
    let mut output = String::from(
        "<?xpacket begin=\"\u{feff}\"?><x:xmpmeta xmlns:x=\"adobe:ns:meta/\"><rdf:RDF><rdf:Description xmlns:dc=\"http://purl.org/dc/elements/1.1/\">",
    );
    for field in artifact.metadata().text() {
        output.push_str("<dc:");
        output.push_str(field.keyword());
        output.push('>');
        for character in field.value().chars() {
            match character {
                '&' => output.push_str("&amp;"),
                '<' => output.push_str("&lt;"),
                '>' => output.push_str("&gt;"),
                '"' => output.push_str("&quot;"),
                _ => output.push(character),
            }
        }
        output.push_str("</dc:");
        output.push_str(field.keyword());
        output.push('>');
    }
    output.push_str("</rdf:Description></rdf:RDF></x:xmpmeta><?xpacket end=\"w\"?>");
    output.into_bytes()
}

fn artifact_hash(artifact: &CanonicalArtifact<'_>) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(artifact.image().bytes());
    hasher.update(artifact.metadata().icc_profile().unwrap_or_default());
    hasher.update(artifact.metadata().xmp().unwrap_or_default());
    hasher.finalize().into()
}

/// # Errors
///
/// Returns an error when the bytes are not a valid document in the `RustTable` image-PDF subset.
pub fn inspect(bytes: &[u8]) -> Result<Inspection, Error> {
    if !bytes.starts_with(b"%PDF-1.7")
        || !(bytes.ends_with(b"%%EOF")
            || bytes.ends_with(b"%%EOF\n")
            || bytes.ends_with(b"%%EOF\r\n"))
    {
        return Err(Error::Malformed("header or EOF"));
    }
    let page_count = count_token(bytes, b"/Type /Page") - count_token(bytes, b"/Type /Pages");
    if page_count != 1 || !bytes.windows(8).any(|window| window == b"/Count 1") {
        return Err(Error::Malformed("expected one page"));
    }
    let image = bytes
        .windows(15)
        .position(|window| window == b"/Subtype /Image")
        .ok_or(Error::Malformed("image object missing"))?;
    let width =
        parse_after(bytes, image, b"/Width ").ok_or(Error::Malformed("image width missing"))?;
    let height =
        parse_after(bytes, image, b"/Height ").ok_or(Error::Malformed("image height missing"))?;
    let dimensions = rusttable_image::ImageDimensions::new(
        u32::try_from(width).map_err(|_| Error::Malformed("image width overflow"))?,
        u32::try_from(height).map_err(|_| Error::Malformed("image height overflow"))?,
    )
    .map_err(|_| Error::Malformed("image dimensions"))?;
    let stream_start = bytes[image..]
        .windows(7)
        .position(|window| window == b"stream\n")
        .ok_or(Error::Malformed("image stream missing"))?
        + image
        + 7;
    let stream_end = bytes[stream_start..]
        .windows(9)
        .position(|window| window == b"endstream")
        .ok_or(Error::Malformed("image stream end missing"))?
        + stream_start;
    let raw = &bytes[stream_start..stream_end];
    let mut decoder = flate2::read::ZlibDecoder::new(raw);
    let mut samples = Vec::new();
    io::Read::read_to_end(&mut decoder, &mut samples).map_err(Error::Io)?;
    Ok(Inspection {
        version: (1, 7),
        page_count: 1,
        image_dimensions: dimensions,
        has_soft_mask: bytes
            .windows(b"/SMask".len())
            .any(|window| window == b"/SMask"),
        has_icc_profile: bytes
            .windows(b"/ICCBased".len())
            .any(|window| window == b"/ICCBased"),
        has_xmp: bytes
            .windows(b"/Type /Metadata".len())
            .any(|window| window == b"/Type /Metadata"),
        image_sha256: digest(&samples),
    })
}

fn count_token(bytes: &[u8], token: &[u8]) -> u32 {
    u32::try_from(
        bytes
            .windows(token.len())
            .filter(|window| *window == token)
            .count(),
    )
    .unwrap_or(u32::MAX)
}

fn parse_after(bytes: &[u8], start: usize, token: &[u8]) -> Option<u64> {
    let offset = bytes[start..]
        .windows(token.len())
        .position(|window| window == token)?
        + start
        + token.len();
    let end = bytes[offset..]
        .iter()
        .position(|byte| !byte.is_ascii_digit())
        .unwrap_or(bytes.len() - offset)
        + offset;
    std::str::from_utf8(&bytes[offset..end]).ok()?.parse().ok()
}

fn map_raster_error(error: RasterError) -> Error {
    match error {
        RasterError::Unsupported("premultiplied alpha") => {
            Error::UnsupportedAlpha(AlphaMode::Premultiplied)
        }
        RasterError::Unsupported(_) => Error::Malformed("unsupported raster contract"),
        RasterError::MetadataLimit { limit, actual } => Error::MetadataLimit { limit, actual },
        RasterError::EmptyProfile => Error::EmptyProfile,
        RasterError::InvalidText => Error::InvalidText,
        RasterError::NonFiniteSample => Error::NonFiniteSample,
        _ => Error::Malformed("invalid raster"),
    }
}
