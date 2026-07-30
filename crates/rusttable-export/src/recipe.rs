#![allow(clippy::missing_errors_doc, clippy::too_many_arguments)]

use std::collections::BTreeMap;
use std::fmt;
use std::fmt::Write as _;

use rusttable_color::{BlackPointCompensation, ColorEncoding, ProfileId, RenderingIntent};
use rusttable_core::template::{EncoderDescriptor, Template, TemplateContext};
use rusttable_core::{ContentHash, EditId, PhotoId, RenderSizeRequest, Revision};
use rusttable_image::OutputFormat;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::capabilities::format_id;
use crate::{
    AlphaPolicy, CapabilityReport, DependencySnapshot, DitherPolicy, EncoderSettings,
    ExportPriority, ExportRequest, Interpolation, MetadataAction, MetadataPolicy, OutputProfile,
    PipelineQuality, PixelEncoding,
};

pub const EXPORT_RECIPE_SCHEMA: &str = "rusttable.export-recipe.v1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecipeId(String);

impl RecipeId {
    /// Creates an opaque, stable recipe identifier.
    pub fn new(value: impl Into<String>) -> Result<Self, RecipeError> {
        let value = value.into();
        if !crate::contract::opaque_identifier(&value) {
            return Err(RecipeError::InvalidIdentifier { field: "recipe ID" });
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RecipeRevision(u64);

impl RecipeRevision {
    pub const FIRST: Self = Self(1);

    #[must_use]
    pub const fn new(value: u64) -> Option<Self> {
        if value > 0 { Some(Self(value)) } else { None }
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    pub const fn next(self) -> Result<Self, RecipeError> {
        match self.0.checked_add(1) {
            Some(value) => Ok(Self(value)),
            None => Err(RecipeError::RevisionOverflow),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecipeTemplate {
    id: String,
    version: u32,
}

impl RecipeTemplate {
    pub fn new(id: impl Into<String>, version: u32) -> Result<Self, RecipeError> {
        let id = id.into();
        if !crate::contract::opaque_identifier(&id) || version == 0 {
            return Err(RecipeError::InvalidIdentifier {
                field: "filename template",
            });
        }
        Ok(Self { id, version })
    }

    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    #[must_use]
    pub const fn version(&self) -> u32 {
        self.version
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecipeDestination {
    id: String,
    collision: crate::CollisionPolicy,
    credential_ref: Option<String>,
    parameters: BTreeMap<String, String>,
}

impl RecipeDestination {
    pub fn new(
        id: impl Into<String>,
        collision: crate::CollisionPolicy,
    ) -> Result<Self, RecipeError> {
        let id = id.into();
        if !crate::contract::opaque_identifier(&id) {
            return Err(RecipeError::InvalidIdentifier {
                field: "destination ID",
            });
        }
        Ok(Self {
            id,
            collision,
            credential_ref: None,
            parameters: BTreeMap::new(),
        })
    }

    #[must_use]
    pub fn with_credential_ref(mut self, reference: impl Into<String>) -> Self {
        self.credential_ref = Some(reference.into());
        self
    }

    #[must_use]
    pub fn with_parameter(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.parameters.insert(name.into(), value.into());
        self
    }

    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    #[must_use]
    pub const fn collision(&self) -> crate::CollisionPolicy {
        self.collision
    }

    #[must_use]
    pub fn credential_ref(&self) -> Option<&str> {
        self.credential_ref.as_deref()
    }

    #[must_use]
    pub fn parameters(&self) -> &BTreeMap<String, String> {
        &self.parameters
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostSuccessAction {
    id: String,
    parameters: BTreeMap<String, String>,
}

impl PostSuccessAction {
    pub fn new(id: impl Into<String>) -> Result<Self, RecipeError> {
        let id = id.into();
        if !crate::contract::opaque_identifier(&id) {
            return Err(RecipeError::InvalidIdentifier {
                field: "post-success action",
            });
        }
        Ok(Self {
            id,
            parameters: BTreeMap::new(),
        })
    }

    #[must_use]
    pub fn with_parameter(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.parameters.insert(name.into(), value.into());
        self
    }

    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    #[must_use]
    pub fn parameters(&self) -> &BTreeMap<String, String> {
        &self.parameters
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportRecipeDraft {
    pub id: RecipeId,
    pub name: String,
    pub description: String,
    pub encoder_id: String,
    pub encoder_settings: EncoderSettings,
    pub destination: RecipeDestination,
    pub size: RenderSizeRequest,
    pub quality: PipelineQuality,
    pub interpolation: Interpolation,
    pub output_profile: OutputProfileSpec,
    pub intent: RenderingIntent,
    pub black_point_compensation: BlackPointCompensation,
    pub pixel_encoding: PixelEncoding,
    pub alpha: AlphaPolicy,
    pub dither: DitherPolicy,
    pub metadata: MetadataPolicy,
    pub filename_template: RecipeTemplate,
    pub post_success: Vec<PostSuccessAction>,
    pub enabled: bool,
    pub built_in: bool,
}

impl ExportRecipeDraft {
    pub fn new(
        id: RecipeId,
        name: impl Into<String>,
        encoder_id: impl Into<String>,
        destination: RecipeDestination,
        filename_template: RecipeTemplate,
    ) -> Self {
        let encoder_id = encoder_id.into();
        let format = format_from_encoder(&encoder_id).unwrap_or(OutputFormat::Png);
        Self {
            id,
            name: name.into(),
            description: String::new(),
            encoder_id,
            encoder_settings: EncoderSettings::new(format),
            destination,
            size: RenderSizeRequest::Source,
            quality: PipelineQuality::Standard,
            interpolation: Interpolation::Bilinear,
            output_profile: OutputProfileSpec::builtin(ColorEncoding::SrgbD65),
            intent: RenderingIntent::Relative,
            black_point_compensation: BlackPointCompensation::Disabled,
            pixel_encoding: PixelEncoding::Integer {
                channels: crate::ChannelLayout::Rgba,
                depth: crate::BitDepth::Eight,
                color: ColorEncoding::SrgbD65,
            },
            alpha: AlphaPolicy::Preserve,
            dither: DitherPolicy::None,
            metadata: MetadataPolicy::default(),
            filename_template,
            post_success: Vec::new(),
            enabled: true,
            built_in: false,
        }
    }

    #[must_use]
    pub fn description(mut self, value: impl Into<String>) -> Self {
        self.description = value.into();
        self
    }

    #[must_use]
    pub fn encoder_settings(mut self, value: EncoderSettings) -> Self {
        self.encoder_settings = value;
        self
    }

    #[must_use]
    pub const fn size(mut self, value: RenderSizeRequest) -> Self {
        self.size = value;
        self
    }

    #[must_use]
    pub const fn pixel_encoding(mut self, value: PixelEncoding) -> Self {
        self.pixel_encoding = value;
        self
    }

    #[must_use]
    pub const fn output_profile(mut self, value: OutputProfileSpec) -> Self {
        self.output_profile = value;
        self
    }

    #[must_use]
    pub fn post_success(mut self, value: Vec<PostSuccessAction>) -> Self {
        self.post_success = value;
        self
    }

    #[must_use]
    pub const fn built_in(mut self, value: bool) -> Self {
        self.built_in = value;
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutputProfileSpec {
    encoding: ColorEncoding,
    reference: Option<ProfileReference>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProfileReference {
    hash: [u8; 32],
    size: u64,
}

impl OutputProfileSpec {
    #[must_use]
    pub const fn builtin(encoding: ColorEncoding) -> Self {
        Self {
            encoding,
            reference: None,
        }
    }

    #[must_use]
    pub const fn external(encoding: ColorEncoding, hash: [u8; 32], size: u64) -> Self {
        Self {
            encoding,
            reference: Some(ProfileReference { hash, size }),
        }
    }

    #[must_use]
    pub const fn encoding(self) -> ColorEncoding {
        self.encoding
    }

    #[must_use]
    pub const fn reference(self) -> Option<ProfileReference> {
        self.reference
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportRecipe {
    pub(crate) id: RecipeId,
    pub(crate) revision: RecipeRevision,
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) encoder_id: String,
    pub(crate) encoder_settings: EncoderSettings,
    pub(crate) destination: RecipeDestination,
    pub(crate) size: RenderSizeRequest,
    pub(crate) quality: PipelineQuality,
    pub(crate) interpolation: Interpolation,
    pub(crate) output_profile: OutputProfileSpec,
    pub(crate) intent: RenderingIntent,
    pub(crate) black_point_compensation: BlackPointCompensation,
    pub(crate) pixel_encoding: PixelEncoding,
    pub(crate) alpha: AlphaPolicy,
    pub(crate) dither: DitherPolicy,
    pub(crate) metadata: MetadataPolicy,
    pub(crate) filename_template: RecipeTemplate,
    pub(crate) post_success: Vec<PostSuccessAction>,
    pub(crate) enabled: bool,
    pub(crate) built_in: bool,
    pub(crate) content_hash: [u8; 32],
}

impl ExportRecipe {
    pub fn from_draft(draft: ExportRecipeDraft) -> Result<Self, RecipeError> {
        let recipe = Self {
            id: draft.id,
            revision: RecipeRevision::FIRST,
            name: draft.name,
            description: draft.description,
            encoder_id: draft.encoder_id,
            encoder_settings: draft.encoder_settings,
            destination: draft.destination,
            size: draft.size,
            quality: draft.quality,
            interpolation: draft.interpolation,
            output_profile: draft.output_profile,
            intent: draft.intent,
            black_point_compensation: draft.black_point_compensation,
            pixel_encoding: draft.pixel_encoding,
            alpha: draft.alpha,
            dither: draft.dither,
            metadata: draft.metadata,
            filename_template: draft.filename_template,
            post_success: draft.post_success,
            enabled: draft.enabled,
            built_in: draft.built_in,
            content_hash: [0; 32],
        };
        recipe.with_hash()
    }

    #[must_use]
    pub fn id(&self) -> &RecipeId {
        &self.id
    }

    #[must_use]
    pub const fn revision(&self) -> RecipeRevision {
        self.revision
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub fn description(&self) -> &str {
        &self.description
    }

    #[must_use]
    pub fn encoder_id(&self) -> &str {
        &self.encoder_id
    }

    #[must_use]
    pub fn encoder_settings(&self) -> &EncoderSettings {
        &self.encoder_settings
    }

    #[must_use]
    pub fn destination(&self) -> &RecipeDestination {
        &self.destination
    }

    #[must_use]
    pub const fn size(&self) -> RenderSizeRequest {
        self.size
    }

    #[must_use]
    pub const fn quality(&self) -> PipelineQuality {
        self.quality
    }

    #[must_use]
    pub const fn interpolation(&self) -> Interpolation {
        self.interpolation
    }

    #[must_use]
    pub const fn output_profile(&self) -> OutputProfileSpec {
        self.output_profile
    }

    #[must_use]
    pub const fn pixel_encoding(&self) -> PixelEncoding {
        self.pixel_encoding
    }

    #[must_use]
    pub const fn alpha(&self) -> AlphaPolicy {
        self.alpha
    }

    #[must_use]
    pub const fn dither(&self) -> DitherPolicy {
        self.dither
    }

    #[must_use]
    pub const fn metadata(&self) -> MetadataPolicy {
        self.metadata
    }

    #[must_use]
    pub fn filename_template(&self) -> &RecipeTemplate {
        &self.filename_template
    }

    #[must_use]
    pub fn post_success(&self) -> &[PostSuccessAction] {
        &self.post_success
    }

    #[must_use]
    pub const fn enabled(&self) -> bool {
        self.enabled
    }

    #[must_use]
    pub const fn built_in(&self) -> bool {
        self.built_in
    }

    #[must_use]
    pub const fn content_hash(&self) -> [u8; 32] {
        self.content_hash
    }

    /// Returns a new immutable revision; the previous recipe remains unchanged.
    pub fn revised(&self, draft: ExportRecipeDraft) -> Result<Self, RecipeError> {
        if draft.id != self.id {
            return Err(RecipeError::IdentityChanged);
        }
        let mut next = Self::from_draft(draft)?;
        next.revision = self.revision.next()?;
        next.with_hash()
    }

    pub fn disabled(&self) -> Result<Self, RecipeError> {
        let mut next = self.clone();
        next.revision = self.revision.next()?;
        next.enabled = false;
        next.with_hash()
    }

    /// Resolves the recipe into a fully explicit request after a caller supplies the photo/edit
    /// snapshot and the registered filename template. No recipe field is silently downgraded.
    pub fn resolve_request(
        &self,
        photo_id: PhotoId,
        edit_id: EditId,
        edit_revision: Revision,
        template: Template,
        context: TemplateContext,
        dependencies: DependencySnapshot,
        profile: Option<ProfileId>,
    ) -> Result<ExportRequest, RecipeError> {
        self.validate()?;
        let output_profile = match (self.output_profile.reference(), profile) {
            (Some(_), Some(profile)) => {
                OutputProfile::external(self.output_profile.encoding(), profile)
            }
            (Some(_), None) => return Err(RecipeError::MissingProfileReference),
            (None, _) => OutputProfile::builtin(self.output_profile.encoding()),
        };
        let mut destination = crate::DestinationSettings::new(
            self.destination.id.clone(),
            self.destination.collision,
        );
        for (name, value) in &self.destination.parameters {
            destination = destination.with_parameter(name.clone(), value.clone());
        }
        let request = ExportRequest::for_edit(
            photo_id,
            edit_id,
            edit_revision,
            crate::ArtifactKind::Image,
            template,
            context,
            dependencies,
        )
        .with_size(self.size)
        .with_quality(self.quality)
        .with_interpolation(self.interpolation)
        .with_rendering_intent(self.intent)
        .with_black_point_compensation(self.black_point_compensation)
        .with_output_profile(output_profile)
        .with_pixel_encoding(self.pixel_encoding)
        .with_alpha_policy(self.alpha)
        .with_dither_policy(self.dither)
        .with_metadata_policy(self.metadata)
        .with_encoder_settings(self.encoder_settings.clone())
        .with_destination(destination)
        .with_priority(ExportPriority::Normal)
        .with_encoder(EncoderDescriptor::new(
            self.encoder_id.clone(),
            format_id(self.encoder_settings.format()),
            &[format_id(self.encoder_settings.format())],
        ));
        if !self.enabled {
            return Err(RecipeError::Disabled);
        }
        request.validate().map_err(RecipeError::InvalidRequest)?;
        Ok(request)
    }

    pub fn validate(&self) -> Result<(), RecipeError> {
        if self.name.trim().is_empty() || self.name.len() > 256 {
            return Err(RecipeError::InvalidField { field: "name" });
        }
        if self.description.len() > 4096 || !crate::contract::opaque_identifier(&self.encoder_id) {
            return Err(RecipeError::InvalidField { field: "encoder" });
        }
        if self.size.resolve(1, 1).is_err() {
            return Err(RecipeError::InvalidSize);
        }
        if self.pixel_encoding.color() != self.output_profile.encoding() {
            return Err(RecipeError::InvalidField {
                field: "pixel encoding/profile",
            });
        }
        if self.alpha == AlphaPolicy::Require && !self.pixel_encoding.channels().has_alpha() {
            return Err(RecipeError::InvalidField { field: "alpha" });
        }
        if let Some(reference) = self.destination.credential_ref.as_deref()
            && !crate::contract::opaque_identifier(reference)
        {
            return Err(RecipeError::SecretMaterial {
                field: "credential_ref".to_owned(),
            });
        }
        for (name, value) in self
            .encoder_settings
            .parameters()
            .iter()
            .chain(self.destination.parameters.iter())
        {
            if name.trim().is_empty() {
                return Err(RecipeError::InvalidField {
                    field: "parameter name",
                });
            }
            if is_secret_key(name) || is_secret_value(value) {
                return Err(RecipeError::SecretMaterial {
                    field: name.to_owned(),
                });
            }
        }
        if self.built_in && !self.enabled {
            return Err(RecipeError::InvalidField {
                field: "built-in enabled",
            });
        }
        Ok(())
    }

    pub fn capability_report(
        &self,
        capabilities: &crate::CapabilitySet,
    ) -> Result<CapabilityReport, RecipeError> {
        let template = Template::parse("${source_stem}")
            .map_err(|_| RecipeError::InvalidField { field: "template" })?;
        let context = TemplateContext::new();
        let dependencies = DependencySnapshot::new(Revision::ZERO, Revision::ZERO).with_asset(
            crate::Dependency::new("recipe-validation", ContentHash::Sha256([1; 32])),
        );
        let request = self.resolve_request(
            PhotoId::new(1).ok_or(RecipeError::InvalidRequest(
                crate::ExportValidationError::MissingPhotoId,
            ))?,
            EditId::new(1).ok_or(RecipeError::InvalidRequest(
                crate::ExportValidationError::MissingEditRevision,
            ))?,
            Revision::ZERO,
            template,
            context,
            dependencies,
            None,
        )?;
        Ok(capabilities.negotiate(&request))
    }

    /// Canonical persistence encoding. It includes only opaque credential references, never
    /// credential material; `export_json` redacts even those references for sharing.
    pub fn canonical_json(&self) -> Result<String, RecipeError> {
        self.validate()?;
        canonical_string(&self.document(true))
    }

    #[must_use]
    pub fn export_json(&self) -> String {
        let mut document = self.document(true);
        redact_credentials(&mut document);
        canonical_string(&document).unwrap_or_else(|_| "{}".to_owned())
    }

    #[must_use]
    pub fn display_summary(&self) -> String {
        format!(
            "{}@{} ({})",
            self.name,
            self.revision.get(),
            self.encoder_id
        )
    }

    pub fn from_canonical_json(input: &str) -> Result<Self, RecipeError> {
        let value: Value = serde_json::from_str(input).map_err(|_| RecipeError::MalformedJson)?;
        let schema = crate::recipe_parse::string(&value, "schema")?;
        if schema != EXPORT_RECIPE_SCHEMA {
            return Err(RecipeError::UnknownSchema(schema));
        }
        let recipe = crate::recipe_parse::parse_recipe(&value)?;
        recipe.validate()?;
        let expected = hex_digest(&recipe.content_hash_without_self());
        if crate::recipe_parse::string(&value, "content_hash")? != expected {
            return Err(RecipeError::HashMismatch);
        }
        Ok(recipe)
    }

    pub fn with_id(&self, id: RecipeId) -> Result<Self, RecipeError> {
        let mut clone = self.clone();
        clone.id = id;
        clone.revision = RecipeRevision::FIRST;
        clone.with_hash()
    }

    fn with_hash(mut self) -> Result<Self, RecipeError> {
        self.validate()?;
        self.content_hash = self.content_hash_without_self();
        Ok(self)
    }

    fn content_hash_without_self(&self) -> [u8; 32] {
        Sha256::digest(
            canonical_string(&self.document(false))
                .unwrap_or_default()
                .as_bytes(),
        )
        .into()
    }

    fn document(&self, include_hash: bool) -> Value {
        let mut root = Map::new();
        root.insert("schema".into(), Value::String(EXPORT_RECIPE_SCHEMA.into()));
        root.insert("id".into(), Value::String(self.id.0.clone()));
        root.insert("revision".into(), Value::from(self.revision.get()));
        root.insert("name".into(), Value::String(self.name.clone()));
        root.insert(
            "description".into(),
            Value::String(self.description.clone()),
        );
        root.insert("encoder_id".into(), Value::String(self.encoder_id.clone()));
        root.insert(
            "encoder_settings".into(),
            encoder_settings_value(&self.encoder_settings),
        );
        root.insert("destination".into(), destination_value(&self.destination));
        root.insert("size".into(), size_value(self.size));
        root.insert(
            "quality".into(),
            Value::String(quality_id(self.quality).into()),
        );
        root.insert(
            "interpolation".into(),
            Value::String(interpolation_id(self.interpolation).into()),
        );
        root.insert("output_profile".into(), profile_value(self.output_profile));
        root.insert(
            "intent".into(),
            Value::String(intent_id(self.intent).into()),
        );
        root.insert(
            "black_point_compensation".into(),
            Value::String(bpc_id(self.black_point_compensation).into()),
        );
        root.insert(
            "pixel_encoding".into(),
            pixel_encoding_value(self.pixel_encoding),
        );
        root.insert("alpha".into(), Value::String(alpha_id(self.alpha).into()));
        root.insert(
            "dither".into(),
            Value::String(dither_id(self.dither).into()),
        );
        root.insert("metadata".into(), metadata_value(self.metadata));
        root.insert("filename_template".into(), serde_json::json!({"id": self.filename_template.id, "version": self.filename_template.version}));
        root.insert(
            "post_success".into(),
            Value::Array(self.post_success.iter().map(action_value).collect()),
        );
        root.insert("enabled".into(), Value::Bool(self.enabled));
        root.insert("built_in".into(), Value::Bool(self.built_in));
        if include_hash {
            root.insert(
                "content_hash".into(),
                Value::String(hex_digest(&self.content_hash)),
            );
        }
        Value::Object(root)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportConflictPolicy {
    CreateNewId,
    ReplaceMatchingRevision,
    Reject,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecipeError {
    InvalidIdentifier { field: &'static str },
    InvalidField { field: &'static str },
    InvalidSize,
    SecretMaterial { field: String },
    RevisionOverflow,
    IdentityChanged,
    Disabled,
    MissingProfileReference,
    InvalidRequest(crate::ExportValidationError),
    MalformedJson,
    UnknownSchema(String),
    HashMismatch,
    UnsupportedValue { field: &'static str },
}

impl fmt::Display for RecipeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "export recipe error: {self:?}")
    }
}

impl std::error::Error for RecipeError {}

fn canonical_string(value: &Value) -> Result<String, RecipeError> {
    serde_json::to_string(value).map_err(|_| RecipeError::MalformedJson)
}

fn redact_credentials(value: &mut Value) {
    if let Some(destination) = value.get_mut("destination").and_then(Value::as_object_mut)
        && destination.contains_key("credential_ref")
    {
        destination.insert("credential_ref".into(), Value::String("[redacted]".into()));
    }
}

fn is_secret_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    ["secret", "password", "token", "private_key", "access_key"]
        .iter()
        .any(|word| key.contains(word))
}

fn is_secret_value(value: &str) -> bool {
    value.starts_with("-----BEGIN") || value.contains("://") && value.contains('@')
}

fn encoder_settings_value(settings: &EncoderSettings) -> Value {
    serde_json::json!({
        "format": format_id(settings.format()),
        "parameters": settings.parameters(),
    })
}

fn destination_value(destination: &RecipeDestination) -> Value {
    let mut value = serde_json::json!({
        "id": destination.id,
        "collision": collision_id(destination.collision),
        "parameters": destination.parameters,
    });
    if let Some(reference) = &destination.credential_ref {
        value["credential_ref"] = Value::String(reference.clone());
    }
    value
}

fn size_value(size: RenderSizeRequest) -> Value {
    match size {
        RenderSizeRequest::Source => serde_json::json!({"mode": "source"}),
        RenderSizeRequest::Exact { width, height } => {
            serde_json::json!({"mode": "exact", "width": width, "height": height})
        }
        RenderSizeRequest::Fit {
            max_width,
            max_height,
        } => serde_json::json!({"mode": "fit", "max_width": max_width, "max_height": max_height}),
        RenderSizeRequest::LongEdge(edge) => serde_json::json!({"mode": "long_edge", "edge": edge}),
    }
}

fn profile_value(profile: OutputProfileSpec) -> Value {
    let mut value = serde_json::json!({"encoding": format!("{:?}", profile.encoding)});
    if let Some(reference) = profile.reference {
        value["reference"] = serde_json::json!({
            "sha256": hex_digest(&reference.hash),
            "size": reference.size,
        });
    }
    value
}

fn pixel_encoding_value(encoding: PixelEncoding) -> Value {
    match encoding {
        PixelEncoding::Float16 { channels, color } => {
            serde_json::json!({"kind": "f16", "channels": format!("{:?}", channels), "color": format!("{:?}", color)})
        }
        PixelEncoding::Float32 { channels, color } => {
            serde_json::json!({"kind": "f32", "channels": format!("{:?}", channels), "color": format!("{:?}", color)})
        }
        PixelEncoding::Integer {
            channels,
            depth,
            color,
        } => {
            serde_json::json!({"kind": "integer", "channels": format!("{:?}", channels), "depth": format!("{:?}", depth), "color": format!("{:?}", color)})
        }
    }
}

fn metadata_value(metadata: MetadataPolicy) -> Value {
    serde_json::json!({
        "exif": action_id(metadata.exif), "iptc": action_id(metadata.iptc), "xmp": action_id(metadata.xmp),
        "gps": action_id(metadata.gps), "faces_and_regions": action_id(metadata.faces_and_regions),
        "ratings_labels_tags": action_id(metadata.ratings_labels_tags), "history": action_id(metadata.history),
        "thumbnail": action_id(metadata.thumbnail), "icc_and_cicp": action_id(metadata.icc_and_cicp),
        "software_and_version": action_id(metadata.software_and_version), "user_fields": action_id(metadata.user_fields),
    })
}

fn action_value(action: &PostSuccessAction) -> Value {
    serde_json::json!({"id": action.id, "parameters": action.parameters})
}

fn collision_id(value: crate::CollisionPolicy) -> &'static str {
    match value {
        crate::CollisionPolicy::CreateNew => "create_new",
        crate::CollisionPolicy::ReplaceExisting => "replace_existing",
        crate::CollisionPolicy::Fail => "fail",
        crate::CollisionPolicy::SkipIfSame => "skip_if_same",
        crate::CollisionPolicy::UniqueSuffix => "unique_suffix",
        crate::CollisionPolicy::VersionRevision => "version_revision",
    }
}
fn quality_id(value: PipelineQuality) -> &'static str {
    match value {
        PipelineQuality::Draft => "draft",
        PipelineQuality::Standard => "standard",
        PipelineQuality::High => "high",
        PipelineQuality::Reference => "reference",
    }
}
fn interpolation_id(value: Interpolation) -> &'static str {
    match value {
        Interpolation::Nearest => "nearest",
        Interpolation::Bilinear => "bilinear",
        Interpolation::Bicubic => "bicubic",
        Interpolation::Lanczos => "lanczos",
    }
}
fn intent_id(value: RenderingIntent) -> &'static str {
    match value {
        RenderingIntent::Perceptual => "perceptual",
        RenderingIntent::Relative => "relative",
        RenderingIntent::Saturation => "saturation",
        RenderingIntent::Absolute => "absolute",
    }
}
fn bpc_id(value: BlackPointCompensation) -> &'static str {
    match value {
        BlackPointCompensation::Disabled => "disabled",
        BlackPointCompensation::Enabled => "enabled",
    }
}
fn alpha_id(value: AlphaPolicy) -> &'static str {
    match value {
        AlphaPolicy::Preserve => "preserve",
        AlphaPolicy::ReplaceOpaque => "replace_opaque",
        AlphaPolicy::Require => "require",
        AlphaPolicy::Ignore => "ignore",
    }
}
fn dither_id(value: DitherPolicy) -> &'static str {
    match value {
        DitherPolicy::None => "none",
        DitherPolicy::Ordered8x8 => "ordered_8x8",
        DitherPolicy::ErrorDiffusion => "error_diffusion",
    }
}
fn action_id(value: MetadataAction) -> &'static str {
    match value {
        MetadataAction::Include => "include",
        MetadataAction::Exclude => "exclude",
        MetadataAction::Redact => "redact",
    }
}
pub(crate) fn format_from_encoder(value: &str) -> Option<OutputFormat> {
    match value.to_ascii_lowercase().as_str() {
        "png" => Some(OutputFormat::Png),
        "jpeg" | "jpg" => Some(OutputFormat::Jpeg),
        "jpeg-xl" | "jxl" => Some(OutputFormat::JpegXl),
        "tiff" | "tif" => Some(OutputFormat::Tiff),
        "webp" => Some(OutputFormat::Webp),
        "pdf" => Some(OutputFormat::Pdf),
        "xcf" => Some(OutputFormat::Xcf),
        "avif" => Some(OutputFormat::Avif),
        "heif" => Some(OutputFormat::Heif),
        "heic" => Some(OutputFormat::Heic),
        "j2k" | "j2c" => Some(OutputFormat::Jpeg2000),
        "jp2" => Some(OutputFormat::Jp2),
        "openexr" | "exr" => Some(OutputFormat::OpenExr),
        _ => None,
    }
}

fn hex_digest(bytes: &[u8; 32]) -> String {
    let mut output = String::with_capacity(64);
    for byte in bytes {
        write!(output, "{byte:02x}").expect("writing to String cannot fail");
    }
    output
}
