#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MetadataLimits {
    pub(crate) max_source_bytes: u64,
    pub(crate) max_exif_bytes: u64,
    pub(crate) max_jpeg_segments: u32,
    pub(crate) max_png_chunks: u32,
    pub(crate) max_ifd_nesting: u32,
    pub(crate) max_ifd_entries: u32,
    pub(crate) max_value_bytes: u64,
}

impl MetadataLimits {
    /// Creates nonzero source, payload, container, IFD, and value caps.
    pub const fn new(
        max_source_bytes: u64,
        max_exif_bytes: u64,
        max_jpeg_segments: u32,
        max_png_chunks: u32,
        max_ifd_nesting: u32,
        max_ifd_entries: u32,
        max_value_bytes: u64,
    ) -> Result<Self, crate::MetadataLimitsError> {
        if max_source_bytes == 0
            || max_exif_bytes == 0
            || max_jpeg_segments == 0
            || max_png_chunks == 0
            || max_ifd_nesting == 0
            || max_ifd_entries == 0
            || max_value_bytes == 0
        {
            return Err(crate::MetadataLimitsError::ZeroLimit);
        }
        Ok(Self {
            max_source_bytes,
            max_exif_bytes,
            max_jpeg_segments,
            max_png_chunks,
            max_ifd_nesting,
            max_ifd_entries,
            max_value_bytes,
        })
    }

    #[must_use]
    pub const fn max_source_bytes(self) -> u64 {
        self.max_source_bytes
    }

    #[must_use]
    pub const fn max_exif_bytes(self) -> u64 {
        self.max_exif_bytes
    }

    #[must_use]
    pub const fn max_jpeg_segments(self) -> u32 {
        self.max_jpeg_segments
    }

    #[must_use]
    pub const fn max_png_chunks(self) -> u32 {
        self.max_png_chunks
    }

    #[must_use]
    pub const fn max_ifd_nesting(self) -> u32 {
        self.max_ifd_nesting
    }

    #[must_use]
    pub const fn max_ifd_entries(self) -> u32 {
        self.max_ifd_entries
    }

    #[must_use]
    pub const fn max_value_bytes(self) -> u64 {
        self.max_value_bytes
    }
}
