use rusttable_color::ColorEncoding;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

#[path = "descriptor_color.rs"]
mod descriptor_color;
pub use descriptor_color::{colorin_descriptor, primaries_descriptor};

const MAX_ID: usize = 96;
const MAX_PARAMETERS: usize = 256;
const MAX_ENUM_TAGS: usize = 128;
const MAX_TEXT: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct DescriptorId {
    pub compatibility_name: String,
    pub rust_id: String,
    pub schema_version: u16,
    pub parameter_version: u16,
    pub implementation_version: u16,
}

impl DescriptorId {
    /// Creates a validated, version-separated operation identity.
    ///
    /// # Errors
    ///
    /// Returns an error when either identifier is invalid or a version is zero.
    pub fn new(
        compatibility_name: impl Into<String>,
        rust_id: impl Into<String>,
        schema_version: u16,
        parameter_version: u16,
        implementation_version: u16,
    ) -> Result<Self, DescriptorError> {
        let value = Self {
            compatibility_name: compatibility_name.into(),
            rust_id: rust_id.into(),
            schema_version,
            parameter_version,
            implementation_version,
        };
        validate_key(&value.compatibility_name)?;
        validate_key(&value.rust_id)?;
        if schema_version == 0 || parameter_version == 0 || implementation_version == 0 {
            return Err(DescriptorError::InvalidVersion);
        }
        Ok(value)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ParameterKind {
    Bool,
    Integer {
        minimum: i64,
        maximum: i64,
    },
    Scalar {
        minimum: f64,
        maximum: f64,
    },
    Vector {
        dimensions: u8,
        minimum: f64,
        maximum: f64,
    },
    Matrix {
        rows: u8,
        columns: u8,
        minimum: f64,
        maximum: f64,
    },
    Curve {
        maximum_points: u16,
    },
    Enum {
        tags: Vec<String>,
    },
    Color {
        allow_external_profile: bool,
    },
    ProfileRef,
    FileRef,
    ContentRef,
    Text {
        maximum_bytes: u16,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ParameterDefault {
    Bool(bool),
    Integer(i64),
    Scalar(f64),
    Vector(Vec<f64>),
    Matrix(Vec<f64>),
    Curve(Vec<(f64, f64)>),
    Enum(String),
    Color(ColorEncoding),
    ProfileRef(String),
    FileRef(String),
    ContentRef(String),
    Text(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Condition {
    Always,
    IsSet { parameter: String },
    All(Vec<Self>),
    Any(Vec<Self>),
    Not(Box<Self>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ParameterRole {
    Processing,
    Geometry,
    Color,
    Mask,
    Presentation,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParameterDescriptor {
    pub id: String,
    pub kind: ParameterKind,
    pub default: ParameterDefault,
    pub required: bool,
    pub introduced_version: u16,
    pub removed_version: Option<u16>,
    pub unit: Option<String>,
    pub step: Option<f64>,
    pub precision: u8,
    pub role: ParameterRole,
    pub cache_affecting: bool,
    pub animatable: bool,
    pub ui_hint: Option<String>,
    pub condition: Option<Condition>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationFlags(u32);

impl OperationFlags {
    pub const MANDATORY: Self = Self(1 << 0);
    pub const HIDDEN: Self = Self(1 << 1);
    pub const DEPRECATED: Self = Self(1 << 2);
    pub const MULTI_INSTANCE: Self = Self(1 << 3);
    pub const STYLE_ELIGIBLE: Self = Self(1 << 4);
    pub const HISTORY_VISIBLE: Self = Self(1 << 5);
    pub const FULL_IMAGE: Self = Self(1 << 6);
    pub const TILEABLE: Self = Self(1 << 7);
    pub const DETERMINISTIC_CPU: Self = Self(1 << 8);
    pub const DETERMINISTIC_GPU: Self = Self(1 << 9);
    pub const GEOMETRY: Self = Self(1 << 10);
    pub const SCALE: Self = Self(1 << 11);
    pub const FORMAT: Self = Self(1 << 12);
    pub const COLOR: Self = Self(1 << 13);
    pub const MASKS: Self = Self(1 << 14);
    pub const BLENDING: Self = Self(1 << 15);
    pub const ANALYSIS: Self = Self(1 << 16);

    #[must_use]
    pub const fn empty() -> Self {
        Self(0)
    }

    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    #[must_use]
    pub const fn insert(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RoiKind {
    Identity,
    Neighborhood,
    Crop,
    Scale,
    Distortion,
    FullImage,
    PreparedBinding,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TilingContract {
    pub overlap_pixels: u32,
    pub alignment_pixels: u32,
    pub minimum_tile_edge: u32,
    pub preferred_tile_edge: u32,
    pub temporary_multiplier_milli: u32,
    pub input_multiplier_milli: u32,
    pub output_multiplier_milli: u32,
}

#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityContract {
    pub cpu_supported: bool,
    pub gpu_tier: Option<u8>,
    pub required_features: Vec<String>,
    pub required_formats: Vec<String>,
    pub deterministic_cpu: bool,
    pub deterministic_gpu: bool,
    pub fallback_to_cpu: bool,
    pub precision: String,
    pub modes: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AlphaPolicy {
    Preserve,
    Replace,
    Require,
    Ignore,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NonFinitePolicy {
    Reject,
    Preserve,
    Clamp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImagePredicate {
    pub channels: u8,
    pub alpha: AlphaPolicy,
    pub encodings: Vec<ColorEncoding>,
    pub nonfinite: NonFinitePolicy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputOutputContract {
    pub input: ImagePredicate,
    pub output: ImagePredicate,
    pub derives_output_encoding: bool,
}

#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MaskBlendContract {
    pub consumes_mask: bool,
    pub publishes_mask: bool,
    pub blend_if: bool,
    pub geometry: bool,
    pub analysis: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MigrationContract {
    pub source_versions: Vec<u16>,
    pub target_version: u16,
    pub opaque_unknown_allowed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UiHint {
    pub label_key: String,
    pub group_key: String,
    pub control: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OperationDescriptor {
    pub id: DescriptorId,
    pub parameters: Vec<ParameterDescriptor>,
    pub flags: OperationFlags,
    pub stage: String,
    pub roi: RoiKind,
    pub tiling: TilingContract,
    pub capability: CapabilityContract,
    pub io: InputOutputContract,
    pub mask_blend: MaskBlendContract,
    pub migration: MigrationContract,
    pub ui: Option<UiHint>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DescriptorError {
    InvalidKey,
    InvalidVersion,
    DuplicateParameter(String),
    ParameterLimit,
    InvalidDefault(String),
    InvalidRange(String),
    InvalidCondition,
    InvalidTiling,
    InvalidCapability,
    InvalidUiKey,
    CanonicalEncoding(String),
}

impl fmt::Display for DescriptorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidKey => formatter.write_str("descriptor key is invalid"),
            Self::InvalidVersion => formatter.write_str("descriptor versions must be nonzero"),
            Self::DuplicateParameter(name) => write!(formatter, "duplicate parameter {name}"),
            Self::ParameterLimit => formatter.write_str("descriptor has too many parameters"),
            Self::InvalidDefault(name) => write!(formatter, "invalid default for {name}"),
            Self::InvalidRange(name) => write!(formatter, "invalid range for {name}"),
            Self::InvalidCondition => formatter.write_str("conditional metadata is invalid"),
            Self::InvalidTiling => formatter.write_str("tiling contract is invalid"),
            Self::InvalidCapability => formatter.write_str("capability contract is invalid"),
            Self::InvalidUiKey => formatter.write_str("UI metadata key is invalid"),
            Self::CanonicalEncoding(error) => {
                write!(formatter, "canonical descriptor encoding failed: {error}")
            }
        }
    }
}

impl std::error::Error for DescriptorError {}

impl OperationDescriptor {
    /// Validates all bounded descriptor fields and cross-field invariants.
    ///
    /// # Errors
    ///
    /// Returns the first deterministic validation failure.
    pub fn validate(&self) -> Result<(), DescriptorError> {
        if self.parameters.len() > MAX_PARAMETERS {
            return Err(DescriptorError::ParameterLimit);
        }
        if self.stage.is_empty() || self.stage.len() > MAX_ID || !self.stage.is_ascii() {
            return Err(DescriptorError::InvalidKey);
        }
        if self.tiling.alignment_pixels == 0
            || self.tiling.minimum_tile_edge == 0
            || self.tiling.preferred_tile_edge < self.tiling.minimum_tile_edge
            || self.tiling.temporary_multiplier_milli == 0
            || self.tiling.input_multiplier_milli == 0
            || self.tiling.output_multiplier_milli == 0
        {
            return Err(DescriptorError::InvalidTiling);
        }
        if !self.capability.cpu_supported && self.capability.gpu_tier.is_none()
            || self.capability.deterministic_gpu && self.capability.gpu_tier.is_none()
            || self.capability.precision.is_empty()
            || self.capability.required_features.len() > 64
            || self.capability.required_formats.len() > 64
        {
            return Err(DescriptorError::InvalidCapability);
        }
        for predicate in [&self.io.input, &self.io.output] {
            if !(1..=4).contains(&predicate.channels) || predicate.encodings.is_empty() {
                return Err(DescriptorError::InvalidCondition);
            }
        }
        if self.flags.contains(OperationFlags::FULL_IMAGE)
            && self.flags.contains(OperationFlags::TILEABLE)
        {
            return Err(DescriptorError::InvalidTiling);
        }
        let mut names = BTreeSet::new();
        for parameter in &self.parameters {
            validate_key(&parameter.id)?;
            if !names.insert(&parameter.id) {
                return Err(DescriptorError::DuplicateParameter(parameter.id.clone()));
            }
            validate_parameter(parameter)?;
        }
        for parameter in &self.parameters {
            if let Some(condition) = &parameter.condition {
                validate_condition(condition, &names)?;
            }
        }
        if let Some(ui) = &self.ui {
            for key in [&ui.label_key, &ui.group_key, &ui.control] {
                if key.is_empty() || key.len() > MAX_ID || !key.is_ascii() {
                    return Err(DescriptorError::InvalidUiKey);
                }
            }
        }
        Ok(())
    }

    /// Encodes the validated descriptor in its canonical postcard form.
    ///
    /// # Errors
    ///
    /// Returns a validation or encoding error.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, DescriptorError> {
        self.validate()?;
        let mut canonical = self.clone();
        canonical
            .parameters
            .sort_by(|left, right| left.id.cmp(&right.id));
        postcard::to_allocvec(&canonical)
            .map_err(|error| DescriptorError::CanonicalEncoding(error.to_string()))
    }

    /// Hashes the canonical descriptor bytes.
    ///
    /// # Errors
    ///
    /// Returns a validation or encoding error.
    pub fn canonical_hash(&self) -> Result<[u8; 32], DescriptorError> {
        Ok(Sha256::digest(self.canonical_bytes()?).into())
    }
}

fn validate_key(value: &str) -> Result<(), DescriptorError> {
    if value.is_empty()
        || value.len() > MAX_ID
        || !value.is_ascii()
        || value.bytes().any(|byte| {
            !(byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || byte == b'.'
                || byte == b'_'
                || byte == b'-')
        })
    {
        return Err(DescriptorError::InvalidKey);
    }
    Ok(())
}

#[allow(clippy::too_many_lines)]
fn validate_parameter(parameter: &ParameterDescriptor) -> Result<(), DescriptorError> {
    if parameter.introduced_version == 0
        || parameter
            .removed_version
            .is_some_and(|version| version <= parameter.introduced_version)
    {
        return Err(DescriptorError::InvalidVersion);
    }
    if parameter
        .step
        .is_some_and(|step| !step.is_finite() || step <= 0.0)
        || parameter.precision > 15
    {
        return Err(DescriptorError::InvalidRange(parameter.id.clone()));
    }
    match (&parameter.kind, &parameter.default) {
        (ParameterKind::Bool, ParameterDefault::Bool(_))
        | (ParameterKind::Integer { .. }, ParameterDefault::Integer(_))
        | (ParameterKind::Scalar { .. }, ParameterDefault::Scalar(_))
        | (ParameterKind::Vector { .. }, ParameterDefault::Vector(_))
        | (ParameterKind::Matrix { .. }, ParameterDefault::Matrix(_))
        | (ParameterKind::Curve { .. }, ParameterDefault::Curve(_))
        | (ParameterKind::Enum { .. }, ParameterDefault::Enum(_))
        | (ParameterKind::Color { .. }, ParameterDefault::Color(_))
        | (ParameterKind::ProfileRef, ParameterDefault::ProfileRef(_))
        | (ParameterKind::FileRef, ParameterDefault::FileRef(_))
        | (ParameterKind::ContentRef, ParameterDefault::ContentRef(_))
        | (ParameterKind::Text { .. }, ParameterDefault::Text(_)) => {}
        _ => return Err(DescriptorError::InvalidDefault(parameter.id.clone())),
    }
    match &parameter.kind {
        ParameterKind::Integer { minimum, maximum } if minimum > maximum => {
            Err(DescriptorError::InvalidRange(parameter.id.clone()))
        }
        ParameterKind::Scalar { minimum, maximum }
            if !minimum.is_finite() || !maximum.is_finite() || minimum > maximum =>
        {
            Err(DescriptorError::InvalidRange(parameter.id.clone()))
        }
        ParameterKind::Vector {
            dimensions,
            minimum,
            maximum,
        } if *dimensions == 0
            || *dimensions > 16
            || !minimum.is_finite()
            || !maximum.is_finite()
            || minimum > maximum =>
        {
            Err(DescriptorError::InvalidRange(parameter.id.clone()))
        }
        ParameterKind::Matrix {
            rows,
            columns,
            minimum,
            maximum,
        } if *rows == 0
            || *columns == 0
            || *rows > 16
            || *columns > 16
            || !minimum.is_finite()
            || !maximum.is_finite()
            || minimum > maximum =>
        {
            Err(DescriptorError::InvalidRange(parameter.id.clone()))
        }
        ParameterKind::Curve { maximum_points }
            if *maximum_points == 0 || *maximum_points > 1024 =>
        {
            Err(DescriptorError::InvalidRange(parameter.id.clone()))
        }
        ParameterKind::Enum { tags } if tags.is_empty() || tags.len() > MAX_ENUM_TAGS => {
            Err(DescriptorError::InvalidRange(parameter.id.clone()))
        }
        ParameterKind::Text { maximum_bytes }
            if *maximum_bytes == 0 || *maximum_bytes as usize > MAX_TEXT =>
        {
            Err(DescriptorError::InvalidRange(parameter.id.clone()))
        }
        _ => Ok(()),
    }?;
    match (&parameter.kind, &parameter.default) {
        (ParameterKind::Integer { minimum, maximum }, ParameterDefault::Integer(value))
            if value < minimum || value > maximum =>
        {
            invalid_default(&parameter.id)
        }
        (ParameterKind::Scalar { minimum, maximum }, ParameterDefault::Scalar(value))
            if !value.is_finite() || value < minimum || value > maximum =>
        {
            invalid_default(&parameter.id)
        }
        (
            ParameterKind::Vector {
                dimensions,
                minimum,
                maximum,
            },
            ParameterDefault::Vector(values),
        ) if values.len() != usize::from(*dimensions)
            || values
                .iter()
                .any(|value| !value.is_finite() || value < minimum || value > maximum) =>
        {
            invalid_default(&parameter.id)
        }
        (
            ParameterKind::Matrix {
                rows,
                columns,
                minimum,
                maximum,
            },
            ParameterDefault::Matrix(values),
        ) if values.len() != usize::from(*rows) * usize::from(*columns)
            || values
                .iter()
                .any(|value| !value.is_finite() || value < minimum || value > maximum) =>
        {
            invalid_default(&parameter.id)
        }
        (ParameterKind::Curve { maximum_points }, ParameterDefault::Curve(points))
            if points.is_empty()
                || points.len() > usize::from(*maximum_points)
                || points.iter().any(|(x, y)| {
                    !x.is_finite() || !y.is_finite() || *x < 0.0 || *x > 1.0 || *y < 0.0 || *y > 1.0
                }) =>
        {
            invalid_default(&parameter.id)
        }
        (ParameterKind::Enum { tags }, ParameterDefault::Enum(value))
            if !tags.iter().any(|tag| tag == value) =>
        {
            invalid_default(&parameter.id)
        }
        (ParameterKind::Text { maximum_bytes }, ParameterDefault::Text(value))
            if value.len() > usize::from(*maximum_bytes) =>
        {
            invalid_default(&parameter.id)
        }
        _ => Ok(()),
    }
}

fn invalid_default(name: &str) -> Result<(), DescriptorError> {
    Err(DescriptorError::InvalidDefault(name.to_owned()))
}

fn validate_condition(
    condition: &Condition,
    names: &BTreeSet<&String>,
) -> Result<(), DescriptorError> {
    fn visit(
        condition: &Condition,
        names: &BTreeSet<&String>,
        depth: usize,
    ) -> Result<(), DescriptorError> {
        if depth > 16 {
            return Err(DescriptorError::InvalidCondition);
        }
        match condition {
            Condition::Always => Ok(()),
            Condition::IsSet { parameter } if names.iter().any(|name| *name == parameter) => Ok(()),
            Condition::IsSet { .. } => Err(DescriptorError::InvalidCondition),
            Condition::All(conditions) | Condition::Any(conditions) => {
                if conditions.len() > 64 {
                    return Err(DescriptorError::InvalidCondition);
                }
                for child in conditions {
                    visit(child, names, depth + 1)?;
                }
                Ok(())
            }
            Condition::Not(child) => visit(child, names, depth + 1),
        }
    }
    visit(condition, names, 0)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchemaDiff {
    Identical,
    CompatibleAddition,
    RequiredVersionBump,
    ParameterRemoved,
    TypeChanged,
    DefaultChanged,
    CapabilityChanged,
}

#[must_use]
pub fn diff_schema(old: &OperationDescriptor, new: &OperationDescriptor) -> SchemaDiff {
    if old == new {
        return SchemaDiff::Identical;
    }
    if old.capability != new.capability {
        return SchemaDiff::CapabilityChanged;
    }
    let old_parameters: BTreeMap<_, _> = old.parameters.iter().map(|p| (&p.id, p)).collect();
    let new_parameters: BTreeMap<_, _> = new.parameters.iter().map(|p| (&p.id, p)).collect();
    if old_parameters
        .keys()
        .any(|key| !new_parameters.contains_key(key))
    {
        return SchemaDiff::ParameterRemoved;
    }
    if old_parameters.iter().any(|(key, old_parameter)| {
        new_parameters
            .get(key)
            .is_some_and(|new_parameter| old_parameter.kind != new_parameter.kind)
    }) {
        return SchemaDiff::TypeChanged;
    }
    if old_parameters.iter().any(|(key, old_parameter)| {
        new_parameters
            .get(key)
            .is_some_and(|new_parameter| old_parameter.default != new_parameter.default)
    }) {
        return SchemaDiff::DefaultChanged;
    }
    if old.id.parameter_version == new.id.parameter_version {
        SchemaDiff::CompatibleAddition
    } else {
        SchemaDiff::RequiredVersionBump
    }
}

#[must_use]
///
/// # Panics
///
/// This function cannot panic because its fixed descriptor identity is valid.
pub fn exposure_descriptor() -> OperationDescriptor {
    OperationDescriptor {
        id: DescriptorId::new("exposure", "rusttable.exposure", 1, 1, 1).expect("static ID"),
        parameters: vec![ParameterDescriptor {
            id: "stops".to_owned(),
            kind: ParameterKind::Scalar {
                minimum: -18.0,
                maximum: 18.0,
            },
            default: ParameterDefault::Scalar(0.0),
            required: true,
            introduced_version: 1,
            removed_version: None,
            unit: Some("ev".to_owned()),
            step: Some(0.01),
            precision: 2,
            role: ParameterRole::Processing,
            cache_affecting: true,
            animatable: true,
            ui_hint: Some("slider".to_owned()),
            condition: None,
        }],
        flags: OperationFlags::DETERMINISTIC_CPU.insert(OperationFlags::TILEABLE),
        stage: "scene-linear".to_owned(),
        roi: RoiKind::Identity,
        tiling: TilingContract {
            overlap_pixels: 0,
            alignment_pixels: 1,
            minimum_tile_edge: 1,
            preferred_tile_edge: 256,
            temporary_multiplier_milli: 1000,
            input_multiplier_milli: 1000,
            output_multiplier_milli: 1000,
        },
        capability: CapabilityContract {
            cpu_supported: true,
            gpu_tier: None,
            required_features: Vec::new(),
            required_formats: Vec::new(),
            deterministic_cpu: true,
            deterministic_gpu: false,
            fallback_to_cpu: true,
            precision: "f32".to_owned(),
            modes: Vec::new(),
        },
        io: default_io_contract(),
        mask_blend: default_mask_blend(),
        migration: MigrationContract {
            source_versions: vec![1],
            target_version: 1,
            opaque_unknown_allowed: true,
        },
        ui: Some(UiHint {
            label_key: "operation.exposure".to_owned(),
            group_key: "group.basic".to_owned(),
            control: "slider".to_owned(),
        }),
    }
}

#[must_use]
///
/// # Panics
///
/// This function cannot panic because its fixed descriptor identity is valid.
pub fn rgb_gain_descriptor() -> OperationDescriptor {
    let mut descriptor = exposure_descriptor();
    descriptor.id = DescriptorId::new("rgbgain", "rusttable.rgb_gain", 1, 1, 1).expect("static ID");
    descriptor.parameters = ["red", "green", "blue"]
        .into_iter()
        .map(|id| ParameterDescriptor {
            id: id.to_owned(),
            kind: ParameterKind::Scalar {
                minimum: 0.0,
                maximum: 64.0,
            },
            default: ParameterDefault::Scalar(1.0),
            required: true,
            introduced_version: 1,
            removed_version: None,
            unit: None,
            step: Some(0.001),
            precision: 3,
            role: ParameterRole::Color,
            cache_affecting: true,
            animatable: true,
            ui_hint: Some("slider".to_owned()),
            condition: None,
        })
        .collect();
    descriptor.ui = Some(UiHint {
        label_key: "operation.rgb_gain".to_owned(),
        group_key: "group.color".to_owned(),
        control: "triplet".to_owned(),
    });
    descriptor.flags = descriptor.flags.insert(OperationFlags::COLOR);
    descriptor
}

#[must_use]
///
/// # Panics
///
/// This function cannot panic because its fixed descriptor identity is valid.
pub fn linear_offset_descriptor() -> OperationDescriptor {
    let mut descriptor = exposure_descriptor();
    descriptor.id =
        DescriptorId::new("linear-offset", "rusttable.linear_offset", 1, 1, 1).expect("static ID");
    descriptor.parameters = vec![ParameterDescriptor {
        id: "value".to_owned(),
        kind: ParameterKind::Scalar {
            minimum: -64.0,
            maximum: 64.0,
        },
        default: ParameterDefault::Scalar(0.0),
        required: true,
        introduced_version: 1,
        removed_version: None,
        unit: None,
        step: Some(0.001),
        precision: 3,
        role: ParameterRole::Processing,
        cache_affecting: true,
        animatable: true,
        ui_hint: Some("slider".to_owned()),
        condition: None,
    }];
    descriptor.ui = Some(UiHint {
        label_key: "operation.linear_offset".to_owned(),
        group_key: "group.basic".to_owned(),
        control: "slider".to_owned(),
    });
    descriptor
}

#[must_use]
#[allow(clippy::assigning_clones, clippy::missing_panics_doc)]
pub fn highlights_descriptor() -> OperationDescriptor {
    let mut descriptor = exposure_descriptor();
    descriptor.id =
        DescriptorId::new("highlights", "rusttable.highlights", 4, 4, 1).expect("static ID");
    descriptor.parameters = vec![
        scalar_parameter("method", 0.0, 5.0, 5.0, ParameterRole::Processing),
        scalar_parameter("blend_l", 0.0, 2.0, 1.0, ParameterRole::Color),
        scalar_parameter("blend_c", 0.0, 2.0, 0.0, ParameterRole::Color),
        scalar_parameter("strength", 0.0, 1.0, 0.0, ParameterRole::Processing),
        scalar_parameter("clip", 0.0, 2.0, 1.0, ParameterRole::Processing),
        scalar_parameter("noise_level", 0.0, 0.5, 0.0, ParameterRole::Processing),
        scalar_parameter("iterations", 1.0, 256.0, 30.0, ParameterRole::Processing),
        scalar_parameter("scales", 0.0, 11.0, 6.0, ParameterRole::Geometry),
        scalar_parameter("candidating", 0.0, 1.0, 0.4, ParameterRole::Mask),
        scalar_parameter("combine", 0.0, 8.0, 2.0, ParameterRole::Mask),
        scalar_parameter("recovery", 0.0, 6.0, 0.0, ParameterRole::Processing),
        scalar_parameter("solid_color", 0.0, 1.0, 0.0, ParameterRole::Color),
    ];
    descriptor.id.schema_version = 4;
    descriptor.id.parameter_version = 4;
    descriptor.flags = OperationFlags::DETERMINISTIC_CPU
        .insert(OperationFlags::DETERMINISTIC_GPU)
        .insert(OperationFlags::FULL_IMAGE)
        .insert(OperationFlags::COLOR)
        .insert(OperationFlags::MASKS)
        .insert(OperationFlags::BLENDING)
        .insert(OperationFlags::ANALYSIS);
    descriptor.stage = "raw-highlight-reconstruction".to_owned();
    descriptor.roi = RoiKind::FullImage;
    descriptor.tiling.overlap_pixels = 2048;
    descriptor.tiling.preferred_tile_edge = 1024;
    descriptor.capability = reconstruction_capability();
    descriptor.io = reconstruction_io();
    descriptor.mask_blend = MaskBlendContract {
        consumes_mask: false,
        publishes_mask: true,
        blend_if: true,
        geometry: false,
        analysis: true,
    };
    descriptor.migration = MigrationContract {
        source_versions: vec![1, 2, 3, 4],
        target_version: 4,
        opaque_unknown_allowed: true,
    };
    descriptor.ui = Some(UiHint {
        label_key: "operation.highlights".to_owned(),
        group_key: "group.basic".to_owned(),
        control: "highlights-reconstruction".to_owned(),
    });
    descriptor
}

#[must_use]
#[allow(clippy::assigning_clones, clippy::missing_panics_doc)]
pub fn color_reconstruction_descriptor() -> OperationDescriptor {
    let mut descriptor = exposure_descriptor();
    descriptor.id = DescriptorId::new(
        "colorreconstruction",
        "rusttable.colorreconstruction",
        3,
        3,
        1,
    )
    .expect("static ID");
    descriptor.parameters = vec![
        scalar_parameter("threshold", 50.0, 150.0, 100.0, ParameterRole::Mask),
        scalar_parameter("spatial", 0.0, 1000.0, 400.0, ParameterRole::Geometry),
        scalar_parameter("range", 0.0, 50.0, 10.0, ParameterRole::Color),
        scalar_parameter("hue", 0.0, 1.0, 0.66, ParameterRole::Color),
        scalar_parameter("precedence", 0.0, 2.0, 0.0, ParameterRole::Color),
    ];
    descriptor.flags = OperationFlags::DETERMINISTIC_CPU
        .insert(OperationFlags::DETERMINISTIC_GPU)
        .insert(OperationFlags::FULL_IMAGE)
        .insert(OperationFlags::COLOR)
        .insert(OperationFlags::MASKS)
        .insert(OperationFlags::BLENDING)
        .insert(OperationFlags::ANALYSIS);
    descriptor.stage = "post-demosaic-color-reconstruction".to_owned();
    descriptor.roi = RoiKind::FullImage;
    descriptor.tiling.overlap_pixels = 1000;
    descriptor.tiling.preferred_tile_edge = 1024;
    descriptor.capability = reconstruction_capability();
    descriptor.io = reconstruction_io();
    descriptor.mask_blend = MaskBlendContract {
        consumes_mask: false,
        publishes_mask: true,
        blend_if: true,
        geometry: false,
        analysis: true,
    };
    descriptor.migration = MigrationContract {
        source_versions: vec![1, 2, 3],
        target_version: 3,
        opaque_unknown_allowed: true,
    };
    descriptor.ui = Some(UiHint {
        label_key: "operation.colorreconstruction".to_owned(),
        group_key: "group.basic".to_owned(),
        control: "color-reconstruction".to_owned(),
    });
    descriptor
}

fn scalar_parameter(
    id: &str,
    minimum: f64,
    maximum: f64,
    default: f64,
    role: ParameterRole,
) -> ParameterDescriptor {
    ParameterDescriptor {
        id: id.to_owned(),
        kind: ParameterKind::Scalar { minimum, maximum },
        default: ParameterDefault::Scalar(default),
        required: false,
        introduced_version: 1,
        removed_version: None,
        unit: None,
        step: Some(0.001),
        precision: 3,
        role,
        cache_affecting: true,
        animatable: true,
        ui_hint: Some("slider".to_owned()),
        condition: None,
    }
}

fn reconstruction_capability() -> CapabilityContract {
    CapabilityContract {
        cpu_supported: true,
        gpu_tier: Some(1),
        required_features: vec![
            "f32-storage".to_owned(),
            "deterministic-row-major".to_owned(),
        ],
        required_formats: vec!["rgba32float".to_owned()],
        deterministic_cpu: true,
        deterministic_gpu: true,
        fallback_to_cpu: true,
        precision: "f32".to_owned(),
        modes: vec!["preview".to_owned(), "full".to_owned(), "export".to_owned()],
    }
}

fn reconstruction_io() -> InputOutputContract {
    let image = ImagePredicate {
        channels: 3,
        alpha: AlphaPolicy::Preserve,
        encodings: vec![ColorEncoding::LinearSrgbD65],
        nonfinite: NonFinitePolicy::Reject,
    };
    InputOutputContract {
        input: image.clone(),
        output: image,
        derives_output_encoding: false,
    }
}

fn default_io_contract() -> InputOutputContract {
    let image = ImagePredicate {
        channels: 3,
        alpha: AlphaPolicy::Preserve,
        encodings: vec![ColorEncoding::LinearSrgbD65],
        nonfinite: NonFinitePolicy::Reject,
    };
    InputOutputContract {
        input: image.clone(),
        output: image,
        derives_output_encoding: false,
    }
}

fn default_mask_blend() -> MaskBlendContract {
    MaskBlendContract {
        consumes_mask: false,
        publishes_mask: false,
        blend_if: false,
        geometry: false,
        analysis: false,
    }
}

#[cfg(test)]
#[path = "descriptor_tests.rs"]
mod descriptor_tests;
