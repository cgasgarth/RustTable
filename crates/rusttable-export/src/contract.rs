use std::collections::BTreeMap;
use std::fmt;
use std::fmt::Write as _;

use rusttable_color::{BlackPointCompensation, ColorEncoding, ProfileId, RenderingIntent};
use rusttable_core::template::{EncoderDescriptor, EvaluationError, Template, TemplateContext};
use rusttable_core::{ContentHash, EditId, PhotoId, RenderSizeError, RenderSizeRequest, Revision};
use rusttable_image::ImageDimensions;
use rusttable_render::RenderReceipt;
use sha2::{Digest, Sha256};

use crate::{ArtifactKind, CollisionPolicy};

pub const EXPORT_CONTRACT_SCHEMA: &str = "rusttable.export-contract.v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PipelineQuality {
    Draft,
    Standard,
    High,
    Reference,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Interpolation {
    Nearest,
    Bilinear,
    Bicubic,
    Lanczos,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ChannelLayout {
    Gray,
    Rgb,
    Rgba,
}

impl ChannelLayout {
    #[must_use]
    pub const fn channels(self) -> u8 {
        match self {
            Self::Gray => 1,
            Self::Rgb => 3,
            Self::Rgba => 4,
        }
    }

    #[must_use]
    pub const fn has_alpha(self) -> bool {
        matches!(self, Self::Rgba)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BitDepth {
    Eight,
    Ten,
    Twelve,
    Sixteen,
}

impl BitDepth {
    #[must_use]
    pub const fn bits(self) -> u8 {
        match self {
            Self::Eight => 8,
            Self::Ten => 10,
            Self::Twelve => 12,
            Self::Sixteen => 16,
        }
    }

    #[must_use]
    fn bytes_per_sample(self) -> u64 {
        u64::from(self.bits().div_ceil(8))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PixelEncoding {
    Float32 {
        channels: ChannelLayout,
        color: ColorEncoding,
    },
    Float16 {
        channels: ChannelLayout,
        color: ColorEncoding,
    },
    Integer {
        channels: ChannelLayout,
        depth: BitDepth,
        color: ColorEncoding,
    },
}

impl PixelEncoding {
    #[must_use]
    pub const fn channels(self) -> ChannelLayout {
        match self {
            Self::Float32 { channels, .. }
            | Self::Float16 { channels, .. }
            | Self::Integer { channels, .. } => channels,
        }
    }

    #[must_use]
    pub const fn color(self) -> ColorEncoding {
        match self {
            Self::Float32 { color, .. }
            | Self::Float16 { color, .. }
            | Self::Integer { color, .. } => color,
        }
    }

    #[must_use]
    fn bytes_per_pixel(self) -> u64 {
        let bytes = match self {
            Self::Float32 { .. } => 4,
            Self::Float16 { .. } => 2,
            Self::Integer { depth, .. } => depth.bytes_per_sample(),
        };
        bytes * u64::from(self.channels().channels())
    }

    #[must_use]
    fn label(self) -> String {
        format!("{self:?}")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AlphaPolicy {
    Preserve,
    ReplaceOpaque,
    Require,
    Ignore,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DitherPolicy {
    None,
    Ordered8x8,
    ErrorDiffusion,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MetadataAction {
    Include,
    Exclude,
    Redact,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MetadataPolicy {
    pub exif: MetadataAction,
    pub iptc: MetadataAction,
    pub xmp: MetadataAction,
    pub gps: MetadataAction,
    pub faces_and_regions: MetadataAction,
    pub ratings_labels_tags: MetadataAction,
    pub history: MetadataAction,
    pub thumbnail: MetadataAction,
    pub icc_and_cicp: MetadataAction,
    pub software_and_version: MetadataAction,
    pub user_fields: MetadataAction,
}

impl Default for MetadataPolicy {
    fn default() -> Self {
        Self {
            exif: MetadataAction::Include,
            iptc: MetadataAction::Include,
            xmp: MetadataAction::Include,
            gps: MetadataAction::Redact,
            faces_and_regions: MetadataAction::Redact,
            ratings_labels_tags: MetadataAction::Include,
            history: MetadataAction::Exclude,
            thumbnail: MetadataAction::Include,
            icc_and_cicp: MetadataAction::Include,
            software_and_version: MetadataAction::Include,
            user_fields: MetadataAction::Redact,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncoderSettings {
    format: rusttable_image::OutputFormat,
    parameters: BTreeMap<String, String>,
}

impl EncoderSettings {
    #[must_use]
    pub fn new(format: rusttable_image::OutputFormat) -> Self {
        Self {
            format,
            parameters: BTreeMap::new(),
        }
    }

    #[must_use]
    pub fn with_parameter(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.parameters.insert(name.into(), value.into());
        self
    }

    #[must_use]
    pub const fn format(&self) -> rusttable_image::OutputFormat {
        self.format
    }

    #[must_use]
    pub fn parameters(&self) -> &BTreeMap<String, String> {
        &self.parameters
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DestinationSettings {
    destination_id: String,
    collision: CollisionPolicy,
}

impl DestinationSettings {
    /// Uses an opaque destination ID; private paths and credentials never enter the request.
    #[must_use]
    pub fn new(destination_id: impl Into<String>, collision: CollisionPolicy) -> Self {
        Self {
            destination_id: destination_id.into(),
            collision,
        }
    }

    #[must_use]
    pub fn destination_id(&self) -> &str {
        &self.destination_id
    }

    #[must_use]
    pub const fn collision(&self) -> CollisionPolicy {
        self.collision
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dependency {
    id: String,
    content_hash: ContentHash,
}

impl Dependency {
    #[must_use]
    pub fn new(id: impl Into<String>, content_hash: ContentHash) -> Self {
        Self {
            id: id.into(),
            content_hash,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DependencySnapshot {
    catalog_revision: Revision,
    edit_revision: Revision,
    style_hash: Option<ContentHash>,
    profile: Option<ProfileId>,
    assets: Vec<Dependency>,
}

impl DependencySnapshot {
    #[must_use]
    pub const fn new(catalog_revision: Revision, edit_revision: Revision) -> Self {
        Self {
            catalog_revision,
            edit_revision,
            style_hash: None,
            profile: None,
            assets: Vec::new(),
        }
    }

    #[must_use]
    pub fn with_style_hash(mut self, hash: ContentHash) -> Self {
        self.style_hash = Some(hash);
        self
    }

    #[must_use]
    pub fn with_profile(mut self, profile: ProfileId) -> Self {
        self.profile = Some(profile);
        self
    }

    #[must_use]
    pub fn with_asset(mut self, asset: Dependency) -> Self {
        self.assets.push(asset);
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ExportPriority {
    Background,
    Normal,
    Interactive,
}

#[derive(Debug, Clone)]
pub struct ExportRequest {
    pub(crate) kind: ArtifactKind,
    pub(crate) template: Template,
    pub(crate) context: TemplateContext,
    pub(crate) encoder: Option<EncoderDescriptor>,
    photo_id: Option<PhotoId>,
    edit_id: Option<EditId>,
    edit_revision: Option<Revision>,
    style_hash: Option<ContentHash>,
    quality: PipelineQuality,
    size: RenderSizeRequest,
    interpolation: Interpolation,
    output_profile: OutputProfile,
    intent: RenderingIntent,
    black_point_compensation: BlackPointCompensation,
    pixel_encoding: PixelEncoding,
    alpha: AlphaPolicy,
    dither: DitherPolicy,
    metadata: MetadataPolicy,
    encoder_settings: EncoderSettings,
    destination: DestinationSettings,
    dependencies: Option<DependencySnapshot>,
    priority: ExportPriority,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutputProfile {
    encoding: ColorEncoding,
    profile_id: Option<ProfileId>,
}

impl OutputProfile {
    #[must_use]
    pub const fn builtin(encoding: ColorEncoding) -> Self {
        Self {
            encoding,
            profile_id: None,
        }
    }

    #[must_use]
    pub const fn external(encoding: ColorEncoding, profile_id: ProfileId) -> Self {
        Self {
            encoding,
            profile_id: Some(profile_id),
        }
    }
}

impl ExportRequest {
    /// Retains the existing logical-plan constructor for callers that have not yet selected a
    /// photo revision. [`Self::validate`] rejects such a request before rendering.
    #[must_use]
    pub fn new(kind: ArtifactKind, template: Template, context: TemplateContext) -> Self {
        Self {
            kind,
            template,
            context,
            encoder: None,
            photo_id: None,
            edit_id: None,
            edit_revision: None,
            style_hash: None,
            quality: PipelineQuality::Standard,
            size: RenderSizeRequest::Source,
            interpolation: Interpolation::Bilinear,
            output_profile: OutputProfile::builtin(ColorEncoding::SrgbD65),
            intent: RenderingIntent::Relative,
            black_point_compensation: BlackPointCompensation::Disabled,
            pixel_encoding: PixelEncoding::Integer {
                channels: ChannelLayout::Rgba,
                depth: BitDepth::Eight,
                color: ColorEncoding::SrgbD65,
            },
            alpha: AlphaPolicy::Preserve,
            dither: DitherPolicy::None,
            metadata: MetadataPolicy::default(),
            encoder_settings: EncoderSettings::new(rusttable_image::OutputFormat::Png),
            destination: DestinationSettings::new("unspecified", CollisionPolicy::CreateNew),
            dependencies: None,
            priority: ExportPriority::Normal,
        }
    }

    #[must_use]
    pub fn for_edit(
        photo_id: PhotoId,
        edit_id: EditId,
        edit_revision: Revision,
        kind: ArtifactKind,
        template: Template,
        context: TemplateContext,
        dependencies: DependencySnapshot,
    ) -> Self {
        let mut request = Self::new(kind, template, context);
        request.photo_id = Some(photo_id);
        request.edit_id = Some(edit_id);
        request.edit_revision = Some(edit_revision);
        request.dependencies = Some(dependencies);
        request
    }

    #[must_use]
    pub fn with_encoder(mut self, encoder: EncoderDescriptor) -> Self {
        self.encoder = Some(encoder);
        self
    }

    #[must_use]
    pub const fn kind(&self) -> ArtifactKind {
        self.kind
    }

    #[must_use]
    pub const fn size(&self) -> RenderSizeRequest {
        self.size
    }

    #[must_use]
    pub const fn pixel_encoding(&self) -> PixelEncoding {
        self.pixel_encoding
    }

    #[must_use]
    pub const fn output_profile(&self) -> OutputProfile {
        self.output_profile
    }

    #[must_use]
    pub const fn edit_revision(&self) -> Option<Revision> {
        self.edit_revision
    }

    #[must_use]
    pub fn with_size(mut self, size: RenderSizeRequest) -> Self {
        self.size = size;
        self
    }

    #[must_use]
    pub fn with_pixel_encoding(mut self, encoding: PixelEncoding) -> Self {
        self.pixel_encoding = encoding;
        self
    }

    #[must_use]
    pub fn with_output_profile(mut self, profile: OutputProfile) -> Self {
        self.output_profile = profile;
        self
    }

    #[must_use]
    pub fn with_style_hash(mut self, hash: ContentHash) -> Self {
        self.style_hash = Some(hash);
        self
    }

    #[must_use]
    pub fn with_quality(mut self, quality: PipelineQuality) -> Self {
        self.quality = quality;
        self
    }

    #[must_use]
    pub fn with_interpolation(mut self, interpolation: Interpolation) -> Self {
        self.interpolation = interpolation;
        self
    }

    #[must_use]
    pub fn with_alpha_policy(mut self, alpha: AlphaPolicy) -> Self {
        self.alpha = alpha;
        self
    }

    #[must_use]
    pub fn with_dither_policy(mut self, dither: DitherPolicy) -> Self {
        self.dither = dither;
        self
    }

    #[must_use]
    pub fn with_encoder_settings(mut self, settings: EncoderSettings) -> Self {
        self.encoder_settings = settings;
        self
    }

    #[must_use]
    pub fn with_priority(mut self, priority: ExportPriority) -> Self {
        self.priority = priority;
        self
    }

    #[must_use]
    pub fn with_metadata_policy(mut self, metadata: MetadataPolicy) -> Self {
        self.metadata = metadata;
        self
    }

    #[must_use]
    pub fn with_destination(mut self, destination: DestinationSettings) -> Self {
        self.destination = destination;
        self
    }

    /// Validates all request-affecting fields without touching a destination.
    ///
    /// # Errors
    ///
    /// Returns an error when identity, revision, dependency, size, encoding, or opaque identifier
    /// requirements are incomplete.
    pub fn validate(&self) -> Result<(), ExportValidationError> {
        if self.photo_id.is_none() {
            return Err(ExportValidationError::MissingPhotoId);
        }
        if self.edit_id.is_none() || self.edit_revision.is_none() {
            return Err(ExportValidationError::MissingEditRevision);
        }
        let dependencies = self
            .dependencies
            .as_ref()
            .ok_or(ExportValidationError::MissingDependencySnapshot)?;
        if dependencies.assets.is_empty() {
            return Err(ExportValidationError::MissingAssetDependency);
        }
        self.size
            .resolve(1, 1)
            .map_err(ExportValidationError::InvalidSize)?;
        if !self.output_profile.encoding.is_explicit() {
            return Err(ExportValidationError::UnspecifiedOutputProfile);
        }
        if matches!(self.output_profile.encoding, ColorEncoding::External(_))
            && self.output_profile.profile_id.is_none()
        {
            return Err(ExportValidationError::MissingOutputProfileReference);
        }
        if self.pixel_encoding.color() != self.output_profile.encoding {
            return Err(ExportValidationError::EncodingProfileMismatch);
        }
        if self.alpha == AlphaPolicy::Require && !self.pixel_encoding.channels().has_alpha() {
            return Err(ExportValidationError::AlphaNotRepresentable);
        }
        if !opaque_identifier(&self.destination.destination_id) {
            return Err(ExportValidationError::InvalidOpaqueIdentifier {
                field: "destination ID",
            });
        }
        if self
            .encoder_settings
            .parameters
            .keys()
            .any(|key| key.trim().is_empty())
        {
            return Err(ExportValidationError::EmptyEncoderParameter);
        }
        if dependencies
            .assets
            .iter()
            .any(|asset| !opaque_identifier(&asset.id))
        {
            return Err(ExportValidationError::InvalidOpaqueIdentifier { field: "asset ID" });
        }
        Ok(())
    }

    /// Encodes every request-affecting field in a path- and locale-independent form.
    ///
    /// # Errors
    ///
    /// Returns an error when template evaluation fails.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, ExportContractError> {
        let (_, receipt) = self
            .template
            .evaluate(&self.context, self.encoder.as_ref())
            .map_err(ExportContractError::Evaluation)?;
        let mut output = String::new();
        writeln!(output, "schema={EXPORT_CONTRACT_SCHEMA}").expect("String cannot fail");
        writeln!(output, "kind={:?}", self.kind).expect("String cannot fail");
        writeln!(output, "template={}", self.template.ast_hash()).expect("String cannot fail");
        writeln!(output, "evaluation={}", receipt.receipt_hash()).expect("String cannot fail");
        writeln!(output, "photo={}", display_option(self.photo_id)).expect("String cannot fail");
        writeln!(output, "edit={}", display_option(self.edit_id)).expect("String cannot fail");
        writeln!(output, "revision={}", display_option(self.edit_revision))
            .expect("String cannot fail");
        writeln!(output, "style={}", hash_option(self.style_hash)).expect("String cannot fail");
        writeln!(output, "quality={:?}", self.quality).expect("String cannot fail");
        writeln!(output, "size={:?}", self.size).expect("String cannot fail");
        writeln!(output, "interpolation={:?}", self.interpolation).expect("String cannot fail");
        writeln!(output, "profile={:?}", self.output_profile).expect("String cannot fail");
        writeln!(output, "intent={:?}", self.intent).expect("String cannot fail");
        writeln!(output, "bpc={:?}", self.black_point_compensation).expect("String cannot fail");
        writeln!(output, "encoding={}", self.pixel_encoding.label()).expect("String cannot fail");
        writeln!(output, "alpha={:?}", self.alpha).expect("String cannot fail");
        writeln!(output, "dither={:?}", self.dither).expect("String cannot fail");
        writeln!(output, "metadata={:?}", self.metadata).expect("String cannot fail");
        writeln!(
            output,
            "encoder={}",
            encoder_settings_hash(&self.encoder_settings)
        )
        .expect("String cannot fail");
        writeln!(
            output,
            "destination={}|{:?}",
            opaque_string_hash(&self.destination.destination_id),
            self.destination.collision
        )
        .expect("String cannot fail");
        writeln!(
            output,
            "dependencies={}",
            hex(&dependency_hash(self.dependencies.as_ref()))
        )
        .expect("String cannot fail");
        writeln!(output, "priority={:?}", self.priority).expect("String cannot fail");
        Ok(output.into_bytes())
    }

    ///
    /// # Errors
    ///
    /// Returns an error when canonical template evaluation fails.
    pub fn request_hash(&self) -> Result<[u8; 32], ExportContractError> {
        Ok(Sha256::digest(self.canonical_bytes()?).into())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExportValidationError {
    MissingPhotoId,
    MissingEditRevision,
    MissingDependencySnapshot,
    MissingAssetDependency,
    InvalidSize(RenderSizeError),
    UnspecifiedOutputProfile,
    MissingOutputProfileReference,
    EncodingProfileMismatch,
    AlphaNotRepresentable,
    InvalidOpaqueIdentifier { field: &'static str },
    EmptyEncoderParameter,
}

impl fmt::Display for ExportValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::MissingPhotoId => "export request is missing a photo ID",
            Self::MissingEditRevision => "export request is missing an exact edit revision",
            Self::MissingDependencySnapshot => "export request is missing its dependency snapshot",
            Self::MissingAssetDependency => {
                "export request is missing its primary asset dependency"
            }
            Self::InvalidSize(error) => return error.fmt(formatter),
            Self::UnspecifiedOutputProfile => "export output profile is unspecified",
            Self::MissingOutputProfileReference => {
                "external output profile has no opaque reference"
            }
            Self::EncodingProfileMismatch => "pixel encoding and output profile disagree",
            Self::AlphaNotRepresentable => {
                "required alpha is not representable by the pixel encoding"
            }
            Self::InvalidOpaqueIdentifier { field } => {
                return write!(formatter, "{field} must be a non-empty opaque identifier");
            }
            Self::EmptyEncoderParameter => "encoder parameter names must not be empty",
        })
    }
}

impl std::error::Error for ExportValidationError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExportContractError {
    Evaluation(EvaluationError),
    Validation(ExportValidationError),
}

impl fmt::Display for ExportContractError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Evaluation(error) => write!(formatter, "export request template failed: {error}"),
            Self::Validation(error) => write!(formatter, "export request is invalid: {error}"),
        }
    }
}

impl std::error::Error for ExportContractError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactBuffer {
    dimensions: ImageDimensions,
    stride: u64,
    encoding: PixelEncoding,
    bytes: Vec<u8>,
}

impl ArtifactBuffer {
    ///
    /// # Errors
    ///
    /// Returns an error when stride, byte length, or allocation arithmetic is invalid.
    pub fn new(
        dimensions: ImageDimensions,
        stride: u64,
        encoding: PixelEncoding,
        bytes: Vec<u8>,
    ) -> Result<Self, ArtifactError> {
        let minimum = u64::from(dimensions.width())
            .checked_mul(encoding.bytes_per_pixel())
            .ok_or(ArtifactError::ArithmeticOverflow)?;
        if stride < minimum {
            return Err(ArtifactError::StrideTooSmall {
                minimum,
                actual: stride,
            });
        }
        let expected = stride
            .checked_mul(u64::from(dimensions.height()))
            .ok_or(ArtifactError::ArithmeticOverflow)?;
        if expected != u64::try_from(bytes.len()).map_err(|_| ArtifactError::ArithmeticOverflow)? {
            return Err(ArtifactError::BufferLengthMismatch {
                expected,
                actual: bytes.len(),
            });
        }
        Ok(Self {
            dimensions,
            stride,
            encoding,
            bytes,
        })
    }

    #[must_use]
    pub const fn dimensions(&self) -> ImageDimensions {
        self.dimensions
    }

    #[must_use]
    pub const fn stride(&self) -> u64 {
        self.stride
    }

    #[must_use]
    pub const fn encoding(&self) -> PixelEncoding {
        self.encoding
    }

    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArtifactError {
    ArithmeticOverflow,
    StrideTooSmall { minimum: u64, actual: u64 },
    BufferLengthMismatch { expected: u64, actual: usize },
    Request(ExportContractError),
    EncodingMismatch,
    ProfileMismatch,
    InvalidFilename,
}

impl fmt::Display for ArtifactError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "export artifact error: {self:?}")
    }
}

impl std::error::Error for ArtifactError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportArtifact {
    buffer: ArtifactBuffer,
    metadata_packet: Vec<u8>,
    filename_context: String,
    content_hash: [u8; 32],
    dependency_hash: [u8; 32],
    request_hash: [u8; 32],
    render_receipt: RenderReceipt,
}

impl ExportArtifact {
    /// Creates an artifact only after the request and complete buffer contract pass validation.
    ///
    /// # Errors
    ///
    /// Returns an error when the request, buffer encoding, profile, or filename is invalid.
    pub fn new(
        request: &ExportRequest,
        buffer: ArtifactBuffer,
        metadata_packet: Vec<u8>,
        filename: impl Into<String>,
        render_receipt: RenderReceipt,
    ) -> Result<Self, ArtifactError> {
        request
            .validate()
            .map_err(|error| ArtifactError::Request(ExportContractError::Validation(error)))?;
        if buffer.encoding != request.pixel_encoding {
            return Err(ArtifactError::EncodingMismatch);
        }
        if buffer.encoding.color() != request.output_profile.encoding {
            return Err(ArtifactError::ProfileMismatch);
        }
        let filename_context = filename.into();
        if filename_context.is_empty()
            || filename_context.starts_with('/')
            || filename_context
                .split('/')
                .any(|component| component == "..")
        {
            return Err(ArtifactError::InvalidFilename);
        }
        let dependency_hash = dependency_hash(request.dependencies.as_ref());
        let content_hash = artifact_hash(&buffer, &metadata_packet);
        let request_hash = request.request_hash().map_err(ArtifactError::Request)?;
        Ok(Self {
            buffer,
            metadata_packet,
            filename_context,
            content_hash,
            dependency_hash,
            request_hash,
            render_receipt,
        })
    }

    #[must_use]
    pub const fn buffer(&self) -> &ArtifactBuffer {
        &self.buffer
    }

    #[must_use]
    pub fn metadata_packet(&self) -> &[u8] {
        &self.metadata_packet
    }

    #[must_use]
    pub fn filename_context(&self) -> &str {
        &self.filename_context
    }

    #[must_use]
    pub const fn content_hash(&self) -> [u8; 32] {
        self.content_hash
    }

    #[must_use]
    pub const fn dependency_hash(&self) -> [u8; 32] {
        self.dependency_hash
    }

    #[must_use]
    pub const fn request_hash(&self) -> [u8; 32] {
        self.request_hash
    }

    #[must_use]
    pub const fn render_receipt(&self) -> &RenderReceipt {
        &self.render_receipt
    }
}

fn artifact_hash(buffer: &ArtifactBuffer, metadata: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(buffer.dimensions.width().to_be_bytes());
    hasher.update(buffer.dimensions.height().to_be_bytes());
    hasher.update(buffer.stride.to_be_bytes());
    hasher.update(buffer.encoding.label().as_bytes());
    hasher.update(buffer.bytes());
    hasher.update(metadata);
    hasher.finalize().into()
}

fn dependency_hash(snapshot: Option<&DependencySnapshot>) -> [u8; 32] {
    let mut hasher = Sha256::new();
    if let Some(snapshot) = snapshot {
        hasher.update(snapshot.catalog_revision.get().to_be_bytes());
        hasher.update(snapshot.edit_revision.get().to_be_bytes());
        if let Some(hash) = snapshot.style_hash {
            hasher.update(hash.bytes());
        }
        if let Some(profile) = snapshot.profile {
            hasher.update(profile.sha256());
            hasher.update(profile.size().to_be_bytes());
        }
        for asset in &snapshot.assets {
            hasher.update(asset.id.as_bytes());
            hasher.update(asset.content_hash.bytes());
        }
    }
    hasher.finalize().into()
}

fn hash_option(value: Option<ContentHash>) -> String {
    value.map_or_else(|| "none".to_owned(), |hash| hex(hash.bytes()))
}

fn opaque_identifier(value: &str) -> bool {
    !value.trim().is_empty()
        && !value.starts_with('.')
        && !value.contains('/')
        && !value.contains('\\')
        && !value.contains('\0')
}

fn opaque_string_hash(value: &str) -> String {
    hex(&Sha256::digest(value.as_bytes()).into())
}

fn encoder_settings_hash(settings: &EncoderSettings) -> String {
    let mut hasher = Sha256::new();
    hasher.update(format!("{:?}\n", settings.format).as_bytes());
    for (name, value) in &settings.parameters {
        hasher.update(name.as_bytes());
        hasher.update([0]);
        hasher.update(value.as_bytes());
        hasher.update([0]);
    }
    hex(&hasher.finalize().into())
}

fn display_option<T: fmt::Display>(value: Option<T>) -> String {
    value.map_or_else(|| "none".to_owned(), |value| value.to_string())
}

fn hex(bytes: &[u8; 32]) -> String {
    let mut output = String::with_capacity(64);
    for byte in bytes {
        write!(output, "{byte:02x}").expect("writing to a String cannot fail");
    }
    output
}
