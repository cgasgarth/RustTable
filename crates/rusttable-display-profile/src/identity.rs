use sha2::{Digest, Sha256};
use std::{fmt, fmt::Write as _, num::NonZeroU32};

const MAX_IDENTITY_PART_BYTES: usize = 256;

/// Privacy-safe, stable monitor identity. Raw EDID and platform labels are never retained.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MonitorId([u8; 32]);

impl MonitorId {
    /// Hashes stable platform/display characteristics while excluding EDID serial and labels.
    #[must_use]
    pub fn from_platform_parts(
        platform: &str,
        connector: Option<&str>,
        manufacturer: Option<&str>,
        model: Option<&str>,
        edid: Option<&[u8]>,
    ) -> Self {
        let mut hasher = Sha256::new();
        hash_part(&mut hasher, platform);
        hash_part(&mut hasher, connector.unwrap_or(""));
        hash_part(&mut hasher, manufacturer.unwrap_or(""));
        hash_part(&mut hasher, model.unwrap_or(""));
        if let Some(edid) = edid {
            hasher.update(edid_characteristics(edid));
        }
        Self(hasher.finalize().into())
    }

    #[must_use]
    pub const fn bytes(self) -> [u8; 32] {
        self.0
    }

    #[must_use]
    pub fn short_hex(self) -> String {
        let mut value = String::with_capacity(16);
        for byte in &self.0[..8] {
            write!(&mut value, "{byte:02x}").expect("writing to String cannot fail");
        }
        value
    }
}

impl fmt::Debug for MonitorId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("MonitorId")
            .field(&self.short_hex())
            .finish()
    }
}

impl fmt::Display for MonitorId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MonitorGeometry {
    pub x: i32,
    pub y: i32,
    pub width: NonZeroU32,
    pub height: NonZeroU32,
    pub scale_factor: NonZeroU32,
}

impl MonitorGeometry {
    /// # Errors
    ///
    /// Returns an error for a zero-sized monitor or invalid scale factor.
    pub fn new(
        x: i32,
        y: i32,
        width: u32,
        height: u32,
        scale_factor: u32,
    ) -> Result<Self, MonitorIdError> {
        // Alias validation keeps platform labels out of the privacy-safe service state.
        Ok(Self {
            x,
            y,
            width: NonZeroU32::new(width).ok_or(MonitorIdError::InvalidGeometry)?,
            height: NonZeroU32::new(height).ok_or(MonitorIdError::InvalidGeometry)?,
            scale_factor: NonZeroU32::new(scale_factor).ok_or(MonitorIdError::InvalidGeometry)?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MonitorDescriptor {
    id: MonitorId,
    alias: String,
    geometry: MonitorGeometry,
    hdr: HdrDescriptor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HdrDescriptor {
    pub supported: bool,
    pub active: bool,
}

impl MonitorDescriptor {
    /// Constructs a descriptor from an adapter's safe, already-normalized evidence.
    ///
    /// `alias` is intentionally caller-controlled and bounded; platform-provided names should be
    /// replaced by a UI-local label before entering the service.
    ///
    /// # Errors
    ///
    /// Returns an error when the alias is empty, oversized, or contains control text.
    pub fn new(
        id: MonitorId,
        alias: impl Into<String>,
        geometry: MonitorGeometry,
        hdr: HdrDescriptor,
    ) -> Result<Self, MonitorIdError> {
        let alias = alias.into();
        if alias.is_empty()
            || alias.len() > MAX_IDENTITY_PART_BYTES
            || alias.chars().any(char::is_control)
        {
            return Err(MonitorIdError::InvalidAlias);
        }
        Ok(Self {
            id,
            alias,
            geometry,
            hdr,
        })
    }

    #[must_use]
    pub const fn id(&self) -> MonitorId {
        self.id
    }

    #[must_use]
    pub fn alias(&self) -> &str {
        &self.alias
    }

    #[must_use]
    pub const fn geometry(&self) -> MonitorGeometry {
        self.geometry
    }

    #[must_use]
    pub const fn hdr(&self) -> HdrDescriptor {
        self.hdr
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MonitorIdError {
    InvalidAlias,
    InvalidGeometry,
}

impl fmt::Display for MonitorIdError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidAlias => "monitor alias is empty, oversized, or contains control text",
            Self::InvalidGeometry => "monitor geometry has a zero dimension or scale factor",
        })
    }
}

impl std::error::Error for MonitorIdError {}

fn hash_part(hasher: &mut Sha256, value: &str) {
    let bounded = value
        .as_bytes()
        .get(..MAX_IDENTITY_PART_BYTES)
        .unwrap_or(value.as_bytes());
    hasher.update((bounded.len() as u64).to_le_bytes());
    hasher.update(bounded);
}

fn edid_characteristics(edid: &[u8]) -> Vec<u8> {
    // EDID serial/name descriptor blocks are deliberately omitted. These fields retain the
    // manufacturer/product/timing identity needed to follow a physical monitor without logging
    // a device serial number or user-facing label.
    let mut characteristics = Vec::with_capacity(32);
    characteristics.extend_from_slice(edid.get(..8).unwrap_or(edid));
    characteristics.extend_from_slice(edid.get(8..12).unwrap_or_default());
    characteristics.extend_from_slice(edid.get(18..25).unwrap_or_default());
    characteristics
}
