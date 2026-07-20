use std::sync::Arc;
use std::thread;
use std::time::Duration;

use rusttable_color::ColorEncoding;
use rusttable_image::{
    AcquireOptions, AllocationClass, BufferAlignment, BufferRequest, BufferUsage,
    CancellationToken, HostBufferPool, HostPoolError, ImageDescriptor, ImageDimensions,
    InitializationPolicy, LeaseState, PixelFormat, PoolBudgets, PriorityClass,
};

fn budgets(total: usize, idle: usize) -> PoolBudgets {
    PoolBudgets::new(total, idle, 8, total, total, 1).expect("test budgets")
}

fn request(bytes: usize, initialization: InitializationPolicy) -> BufferRequest {
    BufferRequest::new(
        AllocationClass::new(
            PixelFormat::rgba8(),
            BufferAlignment::CACHE_LINE,
            BufferUsage::Temporary,
        ),
        bytes,
        initialization,
        PriorityClass::Normal,
    )
    .expect("test request")
}

#[test]
fn pooled_storage_reuses_the_exact_class_without_growing_accounting() {
    let pool = HostBufferPool::new(budgets(1024, 1024));
    let mut lease = pool
        .try_acquire(request(104, InitializationPolicy::Uninitialized))
        .expect("first lease");
    assert_eq!(lease.state(), LeaseState::ExclusiveUninitialized);
    lease
        .initialize_with(|write| {
            write.fill(7);
            Ok(())
        })
        .expect("initialize");
    assert_eq!(lease.read_view().expect("read view").get(99), Some(7));
    let first = lease.return_to_pool().expect("return");
    assert!(first.pooled());
    assert_eq!(pool.accounting().allocated_bytes(), 128);
    assert_eq!(pool.accounting().idle_bytes(), 128);

    let second = pool
        .try_acquire(request(104, InitializationPolicy::Zeroed))
        .expect("reused lease");
    assert_eq!(second.capacity(), Some(128));
    assert_eq!(pool.accounting().allocated_bytes(), 128);
    assert_eq!(pool.accounting().reuse_count(), 1);
}

#[test]
fn initialization_and_freeze_make_mutability_transitions_explicit() {
    let pool = HostBufferPool::new(budgets(1024, 0));
    let mut lease = pool
        .try_acquire(request(64, InitializationPolicy::Uninitialized))
        .expect("lease");
    assert!(matches!(
        lease.read_view(),
        Err(HostPoolError::InvalidTransition { .. })
    ));
    lease
        .write_view()
        .expect("write view")
        .copy_from(&[3; 64])
        .expect("write bytes");
    lease.mark_initialized().expect("publish initialized");
    let shared = lease.freeze().expect("freeze");
    assert_eq!(shared.state(), LeaseState::SharedImmutable);
    assert_eq!(shared.read_view().expect("read view").get(0), Some(3));
    assert_eq!(pool.accounting().outstanding_entries(), 1);
}

#[test]
fn zeroed_leases_are_readable_and_poisoned_storage_is_not_reused() {
    let pool = HostBufferPool::new(budgets(1024, 1024));
    let lease = pool
        .try_acquire(request(64, InitializationPolicy::Zeroed))
        .expect("zeroed lease");
    assert_eq!(lease.read_view().expect("zeroed read").get(10), Some(0));
    drop(lease);
    let mut poisoned = pool
        .try_acquire(request(64, InitializationPolicy::Zeroed))
        .expect("second lease");
    poisoned.poison().expect("poison");
    let receipt = poisoned.return_to_pool().expect("poison return");
    assert!(!receipt.pooled());
    assert_eq!(pool.accounting().poison_count(), 1);
    assert_eq!(pool.accounting().idle_bytes(), 0);
}

#[test]
fn descriptor_stride_and_byte_length_are_checked_by_the_lease_view() {
    let dimensions = ImageDimensions::new(3, 2).expect("dimensions");
    let descriptor = ImageDescriptor::with_strides(
        dimensions,
        PixelFormat::rgba8(),
        ColorEncoding::SrgbD65,
        None,
        rusttable_image::Orientation::Normal,
        &[16],
    )
    .expect("descriptor");
    let pool = HostBufferPool::new(budgets(1024, 0));
    let request = BufferRequest::for_image(
        &descriptor,
        BufferAlignment::CACHE_LINE,
        BufferUsage::Image,
        InitializationPolicy::Zeroed,
        PriorityClass::Interactive,
    )
    .expect("image request");
    let mut lease = pool.try_acquire(request).expect("image lease");
    lease
        .write_view()
        .expect("write")
        .write_at(16, 9)
        .expect("write row");
    let view = lease.image_view(&descriptor).expect("image view");
    assert_eq!(view.row(0, 1).expect("row")[0], 9);
    assert!(!view.row_start_is_aligned(0, 1).expect("alignment"));
    let wrong = ImageDescriptor::new(
        dimensions,
        PixelFormat::rgba8(),
        ColorEncoding::SrgbD65,
        rusttable_image::Orientation::Normal,
    )
    .expect("wrong descriptor");
    assert_eq!(
        lease.validate_descriptor(&wrong),
        Err(HostPoolError::DescriptorMismatch)
    );
}

#[test]
fn cancellation_removes_a_waiter_and_shutdown_reports_outstanding_leases() {
    let pool = Arc::new(HostBufferPool::new(budgets(64, 0)));
    let held = pool
        .try_acquire(request(64, InitializationPolicy::Zeroed))
        .expect("held lease");
    let cancellation = CancellationToken::new();
    let waiting_pool = Arc::clone(&pool);
    let waiting_token = cancellation.clone();
    let handle = thread::spawn(move || {
        waiting_pool.acquire(
            request(64, InitializationPolicy::Zeroed),
            &AcquireOptions::new()
                .with_timeout(Duration::from_secs(2))
                .with_cancellation(waiting_token),
        )
    });
    thread::sleep(Duration::from_millis(20));
    cancellation.cancel();
    assert!(matches!(
        handle.join().expect("waiter thread"),
        Err(HostPoolError::Cancelled)
    ));
    let report = pool.shutdown(Duration::ZERO);
    assert_eq!(report.outstanding_entries(), 1);
    drop(held);
    let report = pool.shutdown(Duration::from_millis(20));
    assert_eq!(report.outstanding_entries(), 0);
}

#[test]
fn reducing_idle_budget_evicts_without_revoking_outstanding_leases() {
    let pool = HostBufferPool::new(budgets(256, 128));
    let lease = pool
        .try_acquire(request(64, InitializationPolicy::Zeroed))
        .expect("lease");
    assert_eq!(
        pool.update_budgets(budgets(256, 0))
            .expect("update")
            .outstanding_entries(),
        1
    );
    assert_eq!(pool.accounting().outstanding_bytes(), 64);
    drop(lease);
    assert_eq!(pool.accounting().allocated_bytes(), 0);
}
