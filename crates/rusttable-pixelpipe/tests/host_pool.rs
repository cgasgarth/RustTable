use rusttable_image::PixelFormat;
use rusttable_pixelpipe::{
    BufferUsage, HostBufferPool, InitializationPolicy, PoolBudgets, PriorityClass,
    ResourceMetadata, temporary_buffer_request,
};

#[test]
fn temporary_request_is_bound_to_the_resource_estimate() {
    let metadata = ResourceMetadata::new(8, 16, 96, 64).expect("metadata");
    let request = temporary_buffer_request(
        metadata,
        PixelFormat::canonical_rgba_f32(),
        InitializationPolicy::Zeroed,
        PriorityClass::Background,
    )
    .expect("temporary request");
    assert_eq!(request.bytes(), 96);
    assert_eq!(request.class().usage(), BufferUsage::Temporary);
    assert_eq!(request.class().alignment().bytes(), 64);

    let pool = HostBufferPool::new(PoolBudgets::new(256, 0, 4, 256, 128, 1).expect("budgets"));
    let lease = pool.try_acquire(request).expect("planned lease");
    assert_eq!(lease.requested_bytes(), Some(96));
}
