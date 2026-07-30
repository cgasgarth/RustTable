use rusttable_gpu::{
    DeviceGeneration, GpuResourcePool, InitializationPolicy, PoolError, ResourceClass,
    ResourcePoolConfig, ResourceRequest,
};

fn pool() -> GpuResourcePool {
    GpuResourcePool::new(
        DeviceGeneration::new(1),
        ResourcePoolConfig {
            configured_cap: 1024,
            memory_hint: None,
            addressable_limit: 1024,
            backend_overhead: 0,
            soft_percent: 80,
        },
    )
}

fn request(size: u64) -> ResourceRequest {
    ResourceRequest::new(
        ResourceClass::buffer(DeviceGeneration::new(1), size, 1),
        InitializationPolicy::Uploaded,
    )
}

#[test]
fn pool_reuses_exact_classes_and_accounts_idle_bytes() {
    let pool = pool();
    let first = pool.try_acquire(request(64)).expect("acquire");
    let id = first.id();
    first.release().expect("return");
    assert_eq!(pool.metrics().accounting.idle_bytes, 64);
    let second = pool.try_acquire(request(64)).expect("reuse");
    assert_eq!(second.id(), id);
    assert_eq!(pool.metrics().accounting.reuse_count, 1);
}

#[test]
fn in_flight_resources_cannot_be_released_or_reused_before_retirement() {
    let pool = pool();
    let lease = pool.try_acquire(request(128)).expect("acquire");
    let token = lease.submit().expect("submit");
    assert_eq!(pool.metrics().accounting.in_flight_bytes, 128);
    assert!(pool.try_acquire(request(1024)).is_err());
    token.retire().expect("retire");
    assert_eq!(pool.metrics().accounting.in_flight_bytes, 0);
}

#[test]
fn loss_advances_generation_and_rejects_old_requests() {
    let pool = pool();
    pool.lose_device(DeviceGeneration::new(1)).expect("loss");
    assert!(matches!(
        pool.try_acquire(request(1)),
        Err(PoolError::UnsupportedGeneration { .. })
    ));
    assert_eq!(pool.generation().value(), 2);
}

#[test]
fn pinned_idle_resources_survive_pressure() {
    let pool = pool();
    let pinned_request = request(500).pinned(true);
    pool.try_acquire(pinned_request)
        .expect("acquire")
        .release()
        .expect("return");
    assert!(pool.try_acquire(request(600)).is_err());
    assert_eq!(pool.metrics().accounting.evictions, 0);
}
