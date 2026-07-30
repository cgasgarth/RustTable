use std::fmt;
use std::fs;
use std::path::{Component, Path, PathBuf};

use serde::Deserialize;

use crate::fixtures::{
    ArtifactClass, FixtureManifest, FixtureRepository, ManifestError, VerificationError, sha256_hex,
};

const HEX_LENGTH: usize = 64;

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CorpusManifest {
    pub version: u32,
    pub fixture_manifest: String,
    pub offline: bool,
    pub large_asset_policy: LargeAssetPolicy,
    #[serde(default)]
    pub required_axes: Vec<String>,
    #[serde(rename = "matrix", default)]
    pub matrix: Vec<CorpusCell>,
    #[serde(rename = "edge", default)]
    pub edges: Vec<SyntheticEdge>,
    #[serde(rename = "external", default)]
    pub external_assets: Vec<ExternalAsset>,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum LargeAssetPolicy {
    SkipLocalFailMerge,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CorpusCell {
    pub axis: String,
    pub value: String,
    pub status: SupportStatus,
    pub positive_fixture: Option<String>,
    pub malformed_fixture: String,
    #[serde(default)]
    pub expected_metadata: Vec<String>,
    pub expected_dimensions: Option<Dimensions>,
    pub expected_orientation: Option<String>,
    pub expected_output_sha256: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum SupportStatus {
    Supported,
    Unsupported,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
pub struct Dimensions {
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SyntheticEdge {
    pub id: String,
    pub kind: String,
    pub path: String,
    pub fixture: Option<String>,
    pub expected: EdgeExpectation,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum EdgeExpectation {
    Reject,
    Diagnose,
    Absent,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ExternalAsset {
    pub id: String,
    pub path: String,
    pub size: u64,
    pub sha256: String,
    pub media_type: String,
    pub required_in_merge: bool,
}

impl CorpusManifest {
    /// Parses the deterministic corpus matrix schema without touching the filesystem.
    ///
    /// # Errors
    ///
    /// Returns a typed error when TOML or the offline corpus shape is invalid.
    pub fn parse(source: &str) -> Result<Self, CorpusError> {
        let manifest: Self = toml::from_str(source).map_err(|error| CorpusError::Parse {
            message: error.to_string(),
        })?;
        manifest.validate_shape()?;
        Ok(manifest)
    }

    /// Validates matrix references against the existing rusttable-testkit fixture schema.
    ///
    /// # Errors
    ///
    /// Returns a typed error when a required cell, edge, or external asset is
    /// incomplete or references an unregistered fixture.
    pub fn validate_against(&self, fixtures: &FixtureManifest) -> Result<(), CorpusError> {
        self.validate_shape()?;
        let mut seen_cells = Vec::new();
        for cell in &self.matrix {
            let key = (cell.axis.clone(), cell.value.clone());
            if seen_cells.contains(&key) {
                return Err(CorpusError::DuplicateCell {
                    axis: cell.axis.clone(),
                    value: cell.value.clone(),
                });
            }
            seen_cells.push(key);
            validate_fixture_reference(
                fixtures,
                cell.positive_fixture.as_deref(),
                &cell.axis,
                Some(ArtifactClass::ValidBinary),
            )?;
            validate_fixture_reference(
                fixtures,
                Some(&cell.malformed_fixture),
                &cell.axis,
                Some(ArtifactClass::MalformedBinary),
            )?;
            if cell.status == SupportStatus::Supported && cell.positive_fixture.is_none() {
                return Err(CorpusError::MissingPositive {
                    axis: cell.axis.clone(),
                    value: cell.value.clone(),
                });
            }
            if cell.status == SupportStatus::Unsupported && cell.positive_fixture.is_some() {
                return Err(CorpusError::UnsupportedHasPositive {
                    axis: cell.axis.clone(),
                    value: cell.value.clone(),
                });
            }
            if let Some(dimensions) = cell.expected_dimensions
                && (dimensions.width == 0 || dimensions.height == 0)
            {
                return Err(CorpusError::InvalidDimensions {
                    axis: cell.axis.clone(),
                    value: cell.value.clone(),
                });
            }
            validate_optional_hash(
                cell.expected_output_sha256.as_deref(),
                format!("matrix {}={}", cell.axis, cell.value),
            )?;
        }
        for axis in &self.required_axes {
            if !self.matrix.iter().any(|cell| &cell.axis == axis) {
                return Err(CorpusError::MissingAxis { axis: axis.clone() });
            }
        }
        let mut edge_ids = Vec::new();
        for edge in &self.edges {
            if edge.id.is_empty() || edge_ids.contains(&edge.id) {
                return Err(CorpusError::DuplicateEdge {
                    id: edge.id.clone(),
                });
            }
            edge_ids.push(edge.id.clone());
            if edge.kind.is_empty() || edge.path.is_empty() {
                return Err(CorpusError::InvalidEdge {
                    id: edge.id.clone(),
                });
            }
            validate_fixture_reference(fixtures, edge.fixture.as_deref(), &edge.id, None)?;
            if edge.expected == EdgeExpectation::Absent && edge.fixture.is_some() {
                return Err(CorpusError::PresentAbsentEdge {
                    id: edge.id.clone(),
                });
            }
            if edge.expected != EdgeExpectation::Absent && edge.fixture.is_none() {
                return Err(CorpusError::MissingEdgeFixture {
                    id: edge.id.clone(),
                });
            }
        }
        let mut external_ids = Vec::new();
        for asset in &self.external_assets {
            if asset.id.is_empty() || external_ids.contains(&asset.id) {
                return Err(CorpusError::DuplicateExternal {
                    id: asset.id.clone(),
                });
            }
            external_ids.push(asset.id.clone());
            if asset.path.is_empty() || asset.media_type.trim().is_empty() || asset.size == 0 {
                return Err(CorpusError::InvalidExternal {
                    id: asset.id.clone(),
                });
            }
            validate_external_path(&asset.path, &asset.id)?;
            validate_optional_hash(Some(&asset.sha256), format!("external {}", asset.id))?;
        }
        Ok(())
    }

    fn validate_shape(&self) -> Result<(), CorpusError> {
        if self.version != 1 {
            return Err(CorpusError::UnsupportedVersion {
                version: self.version,
            });
        }
        if self.fixture_manifest.trim().is_empty() || !self.offline {
            return Err(CorpusError::OfflineRequired);
        }
        if self.matrix.is_empty() {
            return Err(CorpusError::EmptyMatrix);
        }
        if self.required_axes.iter().any(String::is_empty) {
            return Err(CorpusError::EmptyAxis);
        }
        Ok(())
    }
}

fn validate_external_path(path: &str, id: &str) -> Result<(), CorpusError> {
    let portable = path.replace('\\', "/");
    let candidate = Path::new(&portable);
    if portable.starts_with('/')
        || portable.contains("://")
        || candidate
            .components()
            .any(|component| matches!(component, Component::ParentDir | Component::RootDir))
    {
        return Err(CorpusError::InvalidExternal { id: id.to_owned() });
    }
    Ok(())
}

fn validate_fixture_reference(
    fixtures: &FixtureManifest,
    id: Option<&str>,
    owner: &str,
    expected_class: Option<ArtifactClass>,
) -> Result<(), CorpusError> {
    if let Some(id) = id
        && let Some(entry) = fixtures.fixture(id)
    {
        if let Some(expected_class) = expected_class
            && entry.artifact_class != expected_class
        {
            return Err(CorpusError::WrongFixtureClass {
                owner: owner.to_owned(),
                id: id.to_owned(),
                expected: expected_class,
                actual: entry.artifact_class,
            });
        }
    } else if id.is_some() {
        return Err(CorpusError::UnknownFixture {
            owner: owner.to_owned(),
            id: id.unwrap_or_default().to_owned(),
        });
    }
    Ok(())
}

fn validate_optional_hash(value: Option<&str>, owner: String) -> Result<(), CorpusError> {
    if let Some(value) = value
        && (value.len() != HEX_LENGTH || !value.bytes().all(|byte| byte.is_ascii_hexdigit()))
    {
        return Err(CorpusError::InvalidHash { owner });
    }
    Ok(())
}

/// Validation mode controls only the treatment of declared, locally absent large assets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationMode {
    Local,
    Merge,
}

pub struct CorpusRepository {
    root: PathBuf,
    corpus: CorpusManifest,
    fixtures: FixtureManifest,
}

impl CorpusRepository {
    /// Opens a corpus using the same manifest and privacy checks as all other fixtures.
    ///
    /// # Errors
    ///
    /// Returns a typed error when the corpus shape or fixture references are invalid.
    pub fn new(
        root: impl Into<PathBuf>,
        corpus: CorpusManifest,
        fixtures: FixtureManifest,
    ) -> Result<Self, CorpusError> {
        corpus.validate_against(&fixtures)?;
        Ok(Self {
            root: root.into(),
            corpus,
            fixtures,
        })
    }

    /// Verifies registered bytes offline and reports absent large assets structurally.
    ///
    /// # Errors
    ///
    /// Returns a typed error when registered fixtures drift, privacy checks fail,
    /// an external asset is unreadable, or merge mode lacks a required asset.
    pub fn verify(&self, mode: ValidationMode) -> Result<CorpusReport, CorpusError> {
        let fixture_report = FixtureRepository::new(&self.root, self.fixtures.clone())?.verify()?;
        let mut external = Vec::new();
        for asset in &self.corpus.external_assets {
            let path = self.root.join(&asset.path);
            if !path.exists() {
                if mode == ValidationMode::Merge && asset.required_in_merge {
                    return Err(CorpusError::MissingExternal {
                        id: asset.id.clone(),
                    });
                }
                external.push(ExternalStatus::Skipped {
                    id: asset.id.clone(),
                    reason: "large asset is not present in the local cache".to_owned(),
                });
                continue;
            }
            let bytes = fs::read(&path).map_err(|error| CorpusError::ExternalIo {
                id: asset.id.clone(),
                message: error.to_string(),
            })?;
            let actual_size = u64::try_from(bytes.len()).map_err(|_| CorpusError::Overflow)?;
            let actual_hash = sha256_hex(&bytes);
            if actual_size != asset.size || actual_hash != asset.sha256 {
                return Err(CorpusError::ExternalDrift {
                    id: asset.id.clone(),
                });
            }
            external.push(ExternalStatus::Present {
                id: asset.id.clone(),
            });
        }
        Ok(CorpusReport {
            fixture_report,
            external,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExternalStatus {
    Present { id: String },
    Skipped { id: String, reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CorpusReport {
    fixture_report: crate::fixtures::VerificationReport,
    external: Vec<ExternalStatus>,
}

impl CorpusReport {
    #[must_use]
    pub fn fixture_report(&self) -> &crate::fixtures::VerificationReport {
        &self.fixture_report
    }

    #[must_use]
    pub fn external(&self) -> &[ExternalStatus] {
        &self.external
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CorpusError {
    Parse {
        message: String,
    },
    UnsupportedVersion {
        version: u32,
    },
    OfflineRequired,
    EmptyMatrix,
    EmptyAxis,
    MissingAxis {
        axis: String,
    },
    DuplicateCell {
        axis: String,
        value: String,
    },
    MissingPositive {
        axis: String,
        value: String,
    },
    UnsupportedHasPositive {
        axis: String,
        value: String,
    },
    UnknownFixture {
        owner: String,
        id: String,
    },
    WrongFixtureClass {
        owner: String,
        id: String,
        expected: ArtifactClass,
        actual: ArtifactClass,
    },
    InvalidDimensions {
        axis: String,
        value: String,
    },
    InvalidHash {
        owner: String,
    },
    DuplicateEdge {
        id: String,
    },
    InvalidEdge {
        id: String,
    },
    PresentAbsentEdge {
        id: String,
    },
    MissingEdgeFixture {
        id: String,
    },
    DuplicateExternal {
        id: String,
    },
    InvalidExternal {
        id: String,
    },
    MissingExternal {
        id: String,
    },
    ExternalDrift {
        id: String,
    },
    ExternalIo {
        id: String,
        message: String,
    },
    Manifest(ManifestError),
    Verification(VerificationError),
    Overflow,
}

impl fmt::Display for CorpusError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse { message } => write!(formatter, "corpus manifest parse failed: {message}"),
            Self::UnsupportedVersion { version } => {
                write!(formatter, "unsupported corpus manifest version {version}")
            }
            Self::OfflineRequired => formatter.write_str("corpus validation must be offline"),
            Self::EmptyMatrix => formatter.write_str("corpus matrix is empty"),
            Self::EmptyAxis => formatter.write_str("corpus matrix has an empty required axis"),
            Self::MissingAxis { axis } => write!(formatter, "corpus axis {axis} has no cells"),
            Self::DuplicateCell { axis, value } => {
                write!(formatter, "duplicate corpus cell {axis}={value}")
            }
            Self::MissingPositive { axis, value } => {
                write!(
                    formatter,
                    "supported corpus cell {axis}={value} has no positive fixture"
                )
            }
            Self::UnsupportedHasPositive { axis, value } => write!(
                formatter,
                "unsupported corpus cell {axis}={value} must not claim a positive fixture"
            ),
            Self::UnknownFixture { owner, id } => {
                write!(formatter, "{owner} references unknown fixture {id}")
            }
            Self::WrongFixtureClass {
                owner,
                id,
                expected,
                actual,
            } => write!(
                formatter,
                "{owner} references {id} as {expected:?}, but it is {actual:?}"
            ),
            Self::InvalidDimensions { axis, value } => {
                write!(formatter, "corpus cell {axis}={value} has zero dimensions")
            }
            Self::InvalidHash { owner } => write!(formatter, "{owner} has an invalid SHA-256"),
            Self::DuplicateEdge { id } => write!(formatter, "duplicate corpus edge {id}"),
            Self::InvalidEdge { id } => write!(formatter, "invalid corpus edge {id}"),
            Self::PresentAbsentEdge { id } => {
                write!(formatter, "absent corpus edge {id} has a fixture")
            }
            Self::MissingEdgeFixture { id } => {
                write!(formatter, "corpus edge {id} has no fixture")
            }
            Self::DuplicateExternal { id } => write!(formatter, "duplicate external asset {id}"),
            Self::InvalidExternal { id } => write!(formatter, "invalid external asset {id}"),
            Self::MissingExternal { id } => {
                write!(formatter, "required merge corpus asset {id} is absent")
            }
            Self::ExternalDrift { id } => write!(formatter, "external asset {id} drifted"),
            Self::ExternalIo { id, message } => {
                write!(
                    formatter,
                    "external asset {id} could not be read: {message}"
                )
            }
            Self::Manifest(error) => error.fmt(formatter),
            Self::Verification(error) => error.fmt(formatter),
            Self::Overflow => formatter.write_str("corpus size arithmetic overflowed"),
        }
    }
}

impl std::error::Error for CorpusError {}

impl From<ManifestError> for CorpusError {
    fn from(error: ManifestError) -> Self {
        Self::Manifest(error)
    }
}

impl From<VerificationError> for CorpusError {
    fn from(error: VerificationError) -> Self {
        Self::Verification(error)
    }
}
