use super::HostPoolError;
use super::storage::AlignedStorage;
use crate::ImageDescriptor;

pub struct BufferRead<'a> {
    pub(super) storage: &'a AlignedStorage,
    pub(super) length: usize,
}

impl BufferRead<'_> {
    /// Returns the requested byte length, excluding bucket slack.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.length
    }
    /// Reports whether the requested range is empty.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.length == 0
    }
    /// Reads one initialized byte.
    #[must_use]
    pub fn get(&self, offset: usize) -> Option<u8> {
        (offset < self.length)
            .then(|| self.storage.read(offset))
            .flatten()
    }
    /// Copies the requested bytes into an equally sized destination.
    ///
    /// # Errors
    ///
    /// Returns a mismatch when the destination length differs.
    pub fn copy_to(&self, destination: &mut [u8]) -> Result<(), HostPoolError> {
        if destination.len() != self.length {
            return Err(HostPoolError::DescriptorMismatch);
        }
        for (offset, byte) in destination.iter_mut().enumerate() {
            *byte = self
                .get(offset)
                .ok_or(HostPoolError::AccountingCorruption)?;
        }
        Ok(())
    }
    /// Copies the requested bytes into a new vector.
    #[must_use]
    pub fn to_vec(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(self.length);
        for offset in 0..self.length {
            let Some(byte) = self.get(offset) else {
                break;
            };
            bytes.push(byte);
        }
        bytes
    }
}

pub struct BufferWrite<'a> {
    pub(super) storage: &'a mut AlignedStorage,
    pub(super) length: usize,
}

impl BufferWrite<'_> {
    /// Returns the writable request length.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.length
    }
    /// Reports whether the writable range is empty.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.length == 0
    }
    /// Writes one byte in the checked request range.
    ///
    /// # Errors
    ///
    /// Returns a mismatch for an out-of-range offset.
    pub fn write_at(&mut self, offset: usize, value: u8) -> Result<(), HostPoolError> {
        if offset >= self.length || !self.storage.write(offset, value) {
            return Err(HostPoolError::DescriptorMismatch);
        }
        Ok(())
    }
    /// Fills the writable request range.
    pub fn fill(&mut self, value: u8) {
        for offset in 0..self.length {
            let _ = self.storage.write(offset, value);
        }
    }
    /// Copies an equally sized source into the writable request range.
    ///
    /// # Errors
    ///
    /// Returns a mismatch when the source length differs.
    pub fn copy_from(&mut self, source: &[u8]) -> Result<(), HostPoolError> {
        if source.len() != self.length {
            return Err(HostPoolError::DescriptorMismatch);
        }
        for (offset, value) in source.iter().copied().enumerate() {
            self.write_at(offset, value)?;
        }
        Ok(())
    }
}

pub struct HostImageView<'descriptor, 'bytes> {
    pub(super) descriptor: &'descriptor ImageDescriptor,
    pub(super) bytes: BufferRead<'bytes>,
}

impl HostImageView<'_, '_> {
    /// Returns the validated descriptor.
    #[must_use]
    pub const fn descriptor(&self) -> &ImageDescriptor {
        self.descriptor
    }

    /// Copies one complete plane including padding.
    ///
    /// # Errors
    ///
    /// Returns a mismatch for an invalid plane or range.
    pub fn plane(&self, index: usize) -> Result<Vec<u8>, HostPoolError> {
        let plane = self
            .descriptor
            .planes()
            .get(index)
            .ok_or(HostPoolError::DescriptorMismatch)?;
        self.range(plane.byte_offset(), plane.byte_length())
    }

    /// Copies one checked row including padding.
    ///
    /// # Errors
    ///
    /// Returns a mismatch for an invalid plane or row.
    pub fn row(&self, plane_index: usize, row: u32) -> Result<Vec<u8>, HostPoolError> {
        let plane = self
            .descriptor
            .planes()
            .get(plane_index)
            .ok_or(HostPoolError::DescriptorMismatch)?;
        if row >= plane.height() {
            return Err(HostPoolError::DescriptorMismatch);
        }
        let offset = plane
            .byte_offset()
            .checked_add(
                usize::try_from(row)
                    .map_err(|_| HostPoolError::ArithmeticOverflow)?
                    .checked_mul(plane.row_stride())
                    .ok_or(HostPoolError::ArithmeticOverflow)?,
            )
            .ok_or(HostPoolError::ArithmeticOverflow)?;
        self.range(offset, plane.row_stride())
    }

    /// Reports whether a row starts on the lease alignment boundary.
    ///
    /// # Errors
    ///
    /// Returns a mismatch for an invalid plane or row.
    pub fn row_start_is_aligned(
        &self,
        plane_index: usize,
        row: u32,
    ) -> Result<bool, HostPoolError> {
        let plane = self
            .descriptor
            .planes()
            .get(plane_index)
            .ok_or(HostPoolError::DescriptorMismatch)?;
        if row >= plane.height() {
            return Err(HostPoolError::DescriptorMismatch);
        }
        let offset = plane
            .byte_offset()
            .checked_add(
                usize::try_from(row)
                    .map_err(|_| HostPoolError::ArithmeticOverflow)?
                    .checked_mul(plane.row_stride())
                    .ok_or(HostPoolError::ArithmeticOverflow)?,
            )
            .ok_or(HostPoolError::ArithmeticOverflow)?;
        Ok(offset.is_multiple_of(self.bytes_alignment()))
    }

    fn bytes_alignment(&self) -> usize {
        self.bytes.storage_alignment()
    }

    fn range(&self, offset: usize, length: usize) -> Result<Vec<u8>, HostPoolError> {
        let end = offset
            .checked_add(length)
            .ok_or(HostPoolError::ArithmeticOverflow)?;
        if end > self.bytes.len() {
            return Err(HostPoolError::DescriptorMismatch);
        }
        let mut result = vec![0; length];
        for (index, byte) in result.iter_mut().enumerate() {
            *byte = self
                .bytes
                .get(offset + index)
                .ok_or(HostPoolError::AccountingCorruption)?;
        }
        Ok(result)
    }
}

impl BufferRead<'_> {
    fn storage_alignment(&self) -> usize {
        match self.storage {
            AlignedStorage::A64(_) => 64,
            AlignedStorage::A128(_) => 128,
            AlignedStorage::A256(_) => 256,
            AlignedStorage::A512(_) => 512,
        }
    }
}
