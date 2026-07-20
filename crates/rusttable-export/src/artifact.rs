use rusttable_image::{ImageView, OwnedImage};

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
    /// Returns [`DensityError::Zero`] when either axis is zero.
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
