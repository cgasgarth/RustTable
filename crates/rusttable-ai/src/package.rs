use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::io::{Cursor, Read};

use sha2::{Digest, Sha256};
use zip::ZipArchive;

use crate::contracts::{ContractError, RegistryManifest};
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ModelIdentity([u8; 32]);

impl ModelIdentity {
    pub(crate) fn from_canonical(
        manifest: &RegistryManifest,
        model: &[u8],
        assets: &BTreeMap<String, Vec<u8>>,
    ) -> Result<Self, PackageError> {
        let mut hasher = Sha256::new();
        hasher.update(b"rusttable-rtmodel-v1\0");
        hasher.update(
            manifest
                .canonical_bytes()
                .map_err(|_| PackageError::Identity)?,
        );
        hasher.update(
            u64::try_from(model.len())
                .map_err(|_| PackageError::SizeOverflow)?
                .to_be_bytes(),
        );
        hasher.update(model);
        for (name, bytes) in assets {
            hasher.update(name.as_bytes());
            hasher.update([0]);
            hasher.update(
                u64::try_from(bytes.len())
                    .map_err(|_| PackageError::SizeOverflow)?
                    .to_be_bytes(),
            );
            hasher.update(bytes);
        }
        Ok(Self(hasher.finalize().into()))
    }

    #[must_use]
    pub const fn as_bytes(self) -> [u8; 32] {
        self.0
    }

    #[must_use]
    pub fn to_hex(self) -> String {
        hex_digest(self.0)
    }
}

pub const MODEL_MANIFEST_PATH: &str = "model.toml";
pub const MODEL_GRAPH_PATH: &str = "model.onnx";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PackageLimits {
    pub max_package_bytes: u64,
    pub max_file_bytes: u64,
    pub max_uncompressed_bytes: u64,
    pub max_files: usize,
    pub max_compression_ratio: u64,
}

impl Default for PackageLimits {
    fn default() -> Self {
        Self {
            max_package_bytes: 1_073_741_824,
            max_file_bytes: 512 * 1024 * 1024,
            max_uncompressed_bytes: 768 * 1024 * 1024,
            max_files: 64,
            max_compression_ratio: 1_000,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ModelPackage {
    manifest: RegistryManifest,
    model: Vec<u8>,
    data_assets: BTreeMap<String, Vec<u8>>,
    identity: ModelIdentity,
}

impl ModelPackage {
    pub fn from_rtmodel(bytes: &[u8], limits: PackageLimits) -> Result<Self, PackageError> {
        if u64::try_from(bytes.len()).map_err(|_| PackageError::SizeOverflow)?
            > limits.max_package_bytes
        {
            return Err(PackageError::PackageTooLarge);
        }
        let mut archive =
            ZipArchive::new(Cursor::new(bytes)).map_err(|_| PackageError::InvalidZip)?;
        if !archive.comment().is_empty() {
            return Err(PackageError::NonCanonicalZip);
        }
        if archive.is_empty() || archive.len() > limits.max_files {
            return Err(PackageError::FileCount);
        }
        let mut files = BTreeMap::new();
        let mut previous_name = None;
        let mut total = 0_u64;
        for index in 0..archive.len() {
            let mut file = archive
                .by_index(index)
                .map_err(|_| PackageError::InvalidZip)?;
            let name = file.name().to_owned();
            validate_path(&name)?;
            if file.is_dir()
                || file
                    .unix_mode()
                    .is_some_and(|mode| mode & 0o170000 == 0o120000)
            {
                return Err(PackageError::DirectoryOrSymlink);
            }
            if file.encrypted() {
                return Err(PackageError::EncryptedEntry);
            }
            if previous_name
                .as_ref()
                .is_some_and(|previous: &String| previous >= &name)
            {
                return Err(PackageError::NonCanonicalZip);
            }
            previous_name = Some(name.clone());
            let uncompressed = file.size();
            if uncompressed > limits.max_file_bytes {
                return Err(PackageError::FileTooLarge);
            }
            let compressed = file.compressed_size();
            let ratio_exceeded = compressed == 0 && uncompressed != 0
                || compressed > 0
                    && uncompressed
                        > compressed
                            .checked_mul(limits.max_compression_ratio)
                            .ok_or(PackageError::SizeOverflow)?;
            if ratio_exceeded {
                return Err(PackageError::CompressionRatio);
            }
            total = total
                .checked_add(uncompressed)
                .ok_or(PackageError::SizeOverflow)?;
            if total > limits.max_uncompressed_bytes {
                return Err(PackageError::UncompressedTooLarge);
            }
            let mut content = Vec::with_capacity(
                usize::try_from(uncompressed.min(usize::MAX as u64))
                    .map_err(|_| PackageError::SizeOverflow)?,
            );
            file.read_to_end(&mut content)
                .map_err(|_| PackageError::InvalidZip)?;
            if u64::try_from(content.len()).map_err(|_| PackageError::SizeOverflow)? != uncompressed
            {
                return Err(PackageError::InvalidZip);
            }
            if files.insert(name, content).is_some() {
                return Err(PackageError::DuplicateEntry);
            }
        }
        let manifest_bytes = files
            .get(MODEL_MANIFEST_PATH)
            .ok_or(PackageError::MissingManifest)?;
        let manifest_text =
            std::str::from_utf8(manifest_bytes).map_err(|_| PackageError::ManifestEncoding)?;
        let manifest =
            RegistryManifest::from_toml(manifest_text).map_err(PackageError::Contract)?;
        let model = files
            .get(MODEL_GRAPH_PATH)
            .ok_or(PackageError::MissingModel)?
            .clone();
        let declared: BTreeSet<_> = std::iter::once(MODEL_MANIFEST_PATH.to_owned())
            .chain(std::iter::once(MODEL_GRAPH_PATH.to_owned()))
            .chain(manifest.data_assets.iter().map(|asset| asset.name.clone()))
            .collect();
        if declared.len() != files.len() || files.keys().any(|name| !declared.contains(name)) {
            return Err(PackageError::UndeclaredEntry);
        }
        let model_hash = hex_digest(Sha256::digest(&model));
        if model_hash != manifest.hashes.model_sha256 {
            return Err(PackageError::ModelHashMismatch);
        }
        let data_assets = manifest
            .data_assets
            .iter()
            .map(|asset| {
                let bytes = files
                    .get(&asset.name)
                    .ok_or_else(|| PackageError::MissingAsset(asset.name.clone()))?;
                if u64::try_from(bytes.len()).map_err(|_| PackageError::SizeOverflow)?
                    > asset.max_bytes
                {
                    return Err(PackageError::AssetTooLarge);
                }
                if hex_digest(Sha256::digest(bytes)) != asset.sha256 {
                    return Err(PackageError::AssetHashMismatch(asset.name.clone()));
                }
                Ok((asset.name.clone(), bytes.clone()))
            })
            .collect::<Result<BTreeMap<_, _>, PackageError>>()?;
        let data_hash = hash_assets(&data_assets);
        if !manifest.hashes.data_sha256.is_empty() && data_hash != manifest.hashes.data_sha256 {
            return Err(PackageError::DataHashMismatch);
        }
        let identity = ModelIdentity::from_canonical(&manifest, &model, &data_assets)?;
        Ok(Self {
            manifest,
            model,
            data_assets,
            identity,
        })
    }

    #[must_use]
    pub const fn manifest(&self) -> &RegistryManifest {
        &self.manifest
    }

    #[must_use]
    pub fn model_bytes(&self) -> &[u8] {
        &self.model
    }

    #[must_use]
    pub fn data_assets(&self) -> &BTreeMap<String, Vec<u8>> {
        &self.data_assets
    }

    #[must_use]
    pub const fn identity(&self) -> ModelIdentity {
        self.identity
    }

    pub fn validate_graph(
        &self,
        graph: &rusttable_ai_native::GraphMetadata,
    ) -> Result<(), ContractError> {
        self.manifest.validate_graph(graph)
    }
}

fn validate_path(name: &str) -> Result<(), PackageError> {
    if name.is_empty()
        || name.starts_with('/')
        || name.contains('\\')
        || name
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
        || name.bytes().any(|byte| byte < 0x20)
    {
        return Err(PackageError::UnsafePath);
    }
    Ok(())
}

pub(crate) fn hash_assets(assets: &BTreeMap<String, Vec<u8>>) -> String {
    let mut hasher = Sha256::new();
    for (name, bytes) in assets {
        hasher.update(name.as_bytes());
        hasher.update([0]);
        hasher.update(u64::try_from(bytes.len()).unwrap_or(u64::MAX).to_be_bytes());
        hasher.update(bytes);
    }
    hex_digest(hasher.finalize())
}

pub(crate) fn hex_digest(digest: impl AsRef<[u8]>) -> String {
    digest
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PackageError {
    SizeOverflow,
    PackageTooLarge,
    InvalidZip,
    NonCanonicalZip,
    FileCount,
    UnsafePath,
    DirectoryOrSymlink,
    EncryptedEntry,
    FileTooLarge,
    CompressionRatio,
    UncompressedTooLarge,
    DuplicateEntry,
    MissingManifest,
    ManifestEncoding,
    Contract(ContractError),
    MissingModel,
    UndeclaredEntry,
    ModelHashMismatch,
    MissingAsset(String),
    AssetTooLarge,
    AssetHashMismatch(String),
    DataHashMismatch,
    Identity,
}

impl fmt::Display for PackageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid .rtmodel package: {self:?}")
    }
}

impl std::error::Error for PackageError {}
