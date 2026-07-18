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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MetadataOutputLimits {
    pub(crate) max_payload_bytes: u64,
    pub(crate) max_ifd_entries: u32,
    pub(crate) max_value_bytes: u64,
    pub(crate) max_allocation_bytes: u64,
}

impl MetadataOutputLimits {
    /// Creates explicit, bounded limits for canonical EXIF output.
    pub const fn new(
        max_payload_bytes: u64,
        max_ifd_entries: u32,
        max_value_bytes: u64,
        max_allocation_bytes: u64,
    ) -> Result<Self, crate::MetadataOutputLimitsError> {
        use crate::{MetadataOutputLimit, MetadataOutputLimitsError};

        if max_payload_bytes == 0 {
            return Err(MetadataOutputLimitsError::ZeroLimit {
                limit: MetadataOutputLimit::PayloadBytes,
            });
        }
        if max_ifd_entries == 0 {
            return Err(MetadataOutputLimitsError::ZeroLimit {
                limit: MetadataOutputLimit::IfdEntries,
            });
        }
        if max_value_bytes == 0 {
            return Err(MetadataOutputLimitsError::ZeroLimit {
                limit: MetadataOutputLimit::ValueBytes,
            });
        }
        if max_allocation_bytes == 0 {
            return Err(MetadataOutputLimitsError::ZeroLimit {
                limit: MetadataOutputLimit::AllocationBytes,
            });
        }
        if max_value_bytes > max_payload_bytes {
            return Err(MetadataOutputLimitsError::Inconsistent {
                smaller: MetadataOutputLimit::PayloadBytes,
                larger: MetadataOutputLimit::ValueBytes,
            });
        }
        if max_payload_bytes > u32::MAX as u64 {
            return Err(MetadataOutputLimitsError::NotRepresentable {
                limit: MetadataOutputLimit::PayloadBytes,
            });
        }
        if max_ifd_entries > u16::MAX as u32 {
            return Err(MetadataOutputLimitsError::NotRepresentable {
                limit: MetadataOutputLimit::IfdEntries,
            });
        }
        if max_allocation_bytes > usize::MAX as u64 {
            return Err(MetadataOutputLimitsError::NotRepresentable {
                limit: MetadataOutputLimit::AllocationBytes,
            });
        }
        Ok(Self {
            max_payload_bytes,
            max_ifd_entries,
            max_value_bytes,
            max_allocation_bytes,
        })
    }

    #[must_use]
    pub const fn max_payload_bytes(self) -> u64 {
        self.max_payload_bytes
    }

    #[must_use]
    pub const fn max_ifd_entries(self) -> u32 {
        self.max_ifd_entries
    }

    #[must_use]
    pub const fn max_value_bytes(self) -> u64 {
        self.max_value_bytes
    }

    #[must_use]
    pub const fn max_allocation_bytes(self) -> u64 {
        self.max_allocation_bytes
    }
}
