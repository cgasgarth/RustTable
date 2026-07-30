#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MetadataLimits {
    pub(crate) source_bytes: u64,
    pub(crate) exif_bytes: u64,
    pub(crate) jpeg_segments: u32,
    pub(crate) png_chunks: u32,
    pub(crate) ifd_nesting: u32,
    pub(crate) ifd_entries: u32,
    pub(crate) value_bytes: u64,
}

impl MetadataLimits {
    /// Creates nonzero source, payload, container, IFD, and value caps.
    ///
    /// # Errors
    ///
    /// Returns [`crate::MetadataLimitsError::ZeroLimit`] when any cap is zero.
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
            source_bytes: max_source_bytes,
            exif_bytes: max_exif_bytes,
            jpeg_segments: max_jpeg_segments,
            png_chunks: max_png_chunks,
            ifd_nesting: max_ifd_nesting,
            ifd_entries: max_ifd_entries,
            value_bytes: max_value_bytes,
        })
    }

    #[must_use]
    pub const fn max_source_bytes(self) -> u64 {
        self.source_bytes
    }

    #[must_use]
    pub const fn max_exif_bytes(self) -> u64 {
        self.exif_bytes
    }

    #[must_use]
    pub const fn max_jpeg_segments(self) -> u32 {
        self.jpeg_segments
    }

    #[must_use]
    pub const fn max_png_chunks(self) -> u32 {
        self.png_chunks
    }

    #[must_use]
    pub const fn max_ifd_nesting(self) -> u32 {
        self.ifd_nesting
    }

    #[must_use]
    pub const fn max_ifd_entries(self) -> u32 {
        self.ifd_entries
    }

    #[must_use]
    pub const fn max_value_bytes(self) -> u64 {
        self.value_bytes
    }
}

/// Explicit bounds for standalone IPTC-IIM and RDF/XMP packets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MetadataPacketLimits {
    pub(crate) source_bytes: u64,
    pub(crate) packet_bytes: u64,
    pub(crate) xml_nodes: u32,
    pub(crate) xml_depth: u32,
    pub(crate) properties: u32,
    pub(crate) collection_items: u32,
    pub(crate) text_bytes: u64,
}

impl MetadataPacketLimits {
    /// Creates nonzero source, packet, XML, collection, and text caps.
    ///
    /// # Errors
    ///
    /// Returns [`crate::MetadataLimitsError::ZeroLimit`] when any cap is zero.
    #[allow(clippy::too_many_arguments)]
    pub const fn new(
        max_source_bytes: u64,
        max_packet_bytes: u64,
        max_xml_nodes: u32,
        max_xml_depth: u32,
        max_properties: u32,
        max_collection_items: u32,
        max_text_bytes: u64,
    ) -> Result<Self, crate::MetadataLimitsError> {
        if max_source_bytes == 0
            || max_packet_bytes == 0
            || max_xml_nodes == 0
            || max_xml_depth == 0
            || max_properties == 0
            || max_collection_items == 0
            || max_text_bytes == 0
        {
            return Err(crate::MetadataLimitsError::ZeroLimit);
        }
        Ok(Self {
            source_bytes: max_source_bytes,
            packet_bytes: max_packet_bytes,
            xml_nodes: max_xml_nodes,
            xml_depth: max_xml_depth,
            properties: max_properties,
            collection_items: max_collection_items,
            text_bytes: max_text_bytes,
        })
    }

    #[must_use]
    pub const fn max_source_bytes(self) -> u64 {
        self.source_bytes
    }

    #[must_use]
    pub const fn max_packet_bytes(self) -> u64 {
        self.packet_bytes
    }

    #[must_use]
    pub const fn max_xml_nodes(self) -> u32 {
        self.xml_nodes
    }

    #[must_use]
    pub const fn max_xml_depth(self) -> u32 {
        self.xml_depth
    }

    #[must_use]
    pub const fn max_properties(self) -> u32 {
        self.properties
    }

    #[must_use]
    pub const fn max_collection_items(self) -> u32 {
        self.collection_items
    }

    #[must_use]
    pub const fn max_text_bytes(self) -> u64 {
        self.text_bytes
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MetadataOutputLimits {
    pub(crate) payload_bytes: u64,
    pub(crate) ifd_entries: u32,
    pub(crate) value_bytes: u64,
    pub(crate) allocation_bytes: u64,
}

impl MetadataOutputLimits {
    /// Creates explicit, bounded limits for canonical EXIF output.
    ///
    /// # Errors
    ///
    /// Returns [`crate::MetadataOutputLimitsError`] when a limit is zero, inconsistent,
    /// or cannot be represented by the output format.
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
            payload_bytes: max_payload_bytes,
            ifd_entries: max_ifd_entries,
            value_bytes: max_value_bytes,
            allocation_bytes: max_allocation_bytes,
        })
    }

    #[must_use]
    pub const fn max_payload_bytes(self) -> u64 {
        self.payload_bytes
    }

    #[must_use]
    pub const fn max_ifd_entries(self) -> u32 {
        self.ifd_entries
    }

    #[must_use]
    pub const fn max_value_bytes(self) -> u64 {
        self.value_bytes
    }

    #[must_use]
    pub const fn max_allocation_bytes(self) -> u64 {
        self.allocation_bytes
    }
}
