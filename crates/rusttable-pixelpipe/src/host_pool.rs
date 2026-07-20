use rusttable_image::{
    AllocationClass, BufferAlignment, BufferRequest, BufferUsage, HostPoolError,
    InitializationPolicy, PixelFormat, PriorityClass,
};

use crate::ResourceMetadata;

/// Builds the one temporary-buffer request planned by a prepared operation.
/// The byte count and alignment are copied from the immutable resource
/// estimate; callers cannot acquire a differently sized temporary lease by
/// accident.
///
/// # Errors
///
/// Returns a host-pool error when the estimate cannot form a valid request.
pub fn temporary_buffer_request(
    metadata: ResourceMetadata,
    format: PixelFormat,
    initialization: InitializationPolicy,
    priority: PriorityClass,
) -> Result<BufferRequest, HostPoolError> {
    let alignment = BufferAlignment::new(
        usize::try_from(metadata.alignment()).map_err(|_| HostPoolError::ArithmeticOverflow)?,
    )?;
    let bytes = usize::try_from(metadata.temporary_bytes())
        .map_err(|_| HostPoolError::ArithmeticOverflow)?;
    BufferRequest::new(
        AllocationClass::new(format, alignment, BufferUsage::Temporary),
        bytes,
        initialization,
        priority,
    )
}
