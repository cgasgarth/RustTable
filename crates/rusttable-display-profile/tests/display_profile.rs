use rusttable_display_profile::{
    DisplayProfileEvent, DisplayProfileService, DisplayProvider, EventQueue, HdrDescriptor,
    IccProfileError, MAX_QUEUED_EVENTS, ManagedProfileStore, MonitorDescriptor, MonitorGeometry,
    MonitorId, ProfileProbe, ProfileProbeFailure, ProfileSelection, ProfileTransformError,
    ProviderMonitor, SelectionStatus, StaleReason, WindowPresentation,
};

fn profile(seed: u8, device_class: [u8; 4]) -> Vec<u8> {
    let mut bytes = vec![0_u8; 128];
    bytes[0..4].copy_from_slice(&128_u32.to_be_bytes());
    bytes[12..16].copy_from_slice(&device_class);
    bytes[16..20].copy_from_slice(b"RGB ");
    bytes[36..40].copy_from_slice(b"acsp");
    bytes[64] = seed;
    bytes
}

#[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
fn matrix_profile() -> Vec<u8> {
    let tag_count = 4_usize;
    let table_size = 4 + tag_count * 12;
    let profile_size = 128 + table_size + 4 * 20;
    let mut bytes = vec![0_u8; profile_size];
    bytes[0..4].copy_from_slice(&(profile_size as u32).to_be_bytes());
    bytes[8..12].copy_from_slice(b"mntr");
    bytes[12..16].copy_from_slice(b"RGB ");
    bytes[16..20].copy_from_slice(b"RGB ");
    bytes[20..24].copy_from_slice(b"XYZ ");
    bytes[36..40].copy_from_slice(b"acsp");
    bytes[128..132].copy_from_slice(&(tag_count as u32).to_be_bytes());
    for (index, (name, values)) in [
        (b"rXYZ", [0.48657, 0.22897, 0.0]),
        (b"gXYZ", [0.26567, 0.69174, 0.04511]),
        (b"bXYZ", [0.19822, 0.07929, 1.04394]),
        (b"wtpt", [0.95047, 1.0, 1.08883]),
    ]
    .into_iter()
    .enumerate()
    {
        let table = 132 + index * 12;
        let offset = 128 + table_size + index * 20;
        bytes[table..table + 4].copy_from_slice(name);
        bytes[table + 4..table + 8].copy_from_slice(&(offset as u32).to_be_bytes());
        bytes[table + 8..table + 12].copy_from_slice(&20_u32.to_be_bytes());
        bytes[offset..offset + 4].copy_from_slice(b"XYZ ");
        for (channel, value) in values.into_iter().enumerate() {
            bytes[offset + 8 + channel * 4..offset + 12 + channel * 4]
                .copy_from_slice(&((value * 65_536.0) as i32).to_be_bytes());
        }
    }
    bytes
}

fn monitor(seed: &str) -> MonitorDescriptor {
    let id = MonitorId::from_platform_parts("test", Some(seed), Some("maker"), Some("model"), None);
    MonitorDescriptor::new(
        id,
        format!("Display {seed}"),
        MonitorGeometry::new(0, 0, 1920, 1080, 1).expect("geometry"),
        HdrDescriptor {
            supported: false,
            active: false,
        },
    )
    .expect("descriptor")
}

fn provider_monitor(descriptor: MonitorDescriptor, probe: ProfileProbe) -> ProviderMonitor {
    ProviderMonitor::new(descriptor, DisplayProvider::Synthetic, probe)
}

#[test]
fn monitor_identity_excludes_edid_serial_and_user_label() {
    let mut first = vec![0_u8; 128];
    first[8..12].copy_from_slice(&[1, 2, 3, 4]);
    first[54..72].fill(b'P');
    let mut second = first.clone();
    second[12..16].copy_from_slice(&[99, 98, 97, 96]);
    second[54..72].fill(b'A');
    assert_eq!(
        MonitorId::from_platform_parts(
            "x11",
            Some("DP-1"),
            Some("maker"),
            Some("model"),
            Some(&first)
        ),
        MonitorId::from_platform_parts(
            "x11",
            Some("DP-1"),
            Some("maker"),
            Some("model"),
            Some(&second)
        ),
    );
}

#[test]
fn managed_store_validates_and_immutably_hashes_profiles() {
    let mut store = ManagedProfileStore::default();
    let mut bytes = profile(1, *b"mntr");
    let stored = store.insert(&bytes).expect("valid profile");
    bytes[64] = 9;
    assert_eq!(stored.bytes()[64], 1);
    assert_eq!(
        store.get(stored.id()).expect("stored profile").id(),
        stored.id()
    );
    assert!(matches!(
        store.insert(&profile(1, *b"link")),
        Err(IccProfileError::UnsupportedDeviceLink)
    ));
    assert!(matches!(
        store.insert(&vec![0_u8; 65 * 1024 * 1024]),
        Err(IccProfileError::Oversized)
    ));
}

#[test]
fn matrix_profile_builds_a_wide_gamut_presentation_plan() {
    let mut store = ManagedProfileStore::default();
    let stored = store
        .insert(&matrix_profile())
        .expect("valid matrix profile");
    let plan = stored
        .presentation_plan(rusttable_color::RenderingIntent::Relative)
        .expect("matrix presentation plan");
    let transformed = plan
        .apply_rgb([0.8, 0.2, 0.1], || false)
        .expect("finite transformed RGB");
    assert!(
        transformed
            .into_iter()
            .zip([0.8, 0.2, 0.1])
            .any(|(actual, source)| (actual - source).abs() > 0.0001)
    );
    assert!(transformed.into_iter().all(f32::is_finite));
}

#[test]
fn structurally_valid_profile_without_matrix_evidence_is_unusable_for_presentation() {
    let mut store = ManagedProfileStore::default();
    let stored = store
        .insert(&profile(3, *b"mntr"))
        .expect("valid ICC header");
    assert_eq!(
        stored.presentation_plan(rusttable_color::RenderingIntent::Relative),
        Err(ProfileTransformError::InvalidTagTable)
    );
}

#[test]
fn selection_precedence_has_no_implicit_srgb() {
    let descriptor = monitor("one");
    let id = descriptor.id();
    let mut service = DisplayProfileService::new();
    service
        .reconcile([provider_monitor(descriptor.clone(), ProfileProbe::Absent)])
        .expect("inventory");
    assert_eq!(
        service.snapshot(id).expect("snapshot").selection(),
        ProfileSelection::Unprofiled
    );
    assert!(matches!(
        service.snapshot(id).expect("snapshot").status(),
        SelectionStatus::Degraded(_)
    ));

    service
        .set_fallback("explicit sRGB", &profile(2, *b"mntr"))
        .expect("fallback");
    assert_eq!(
        service.snapshot(id).expect("snapshot").selection(),
        ProfileSelection::UserFallback
    );
    service
        .set_override(id, "per-monitor", &profile(3, *b"mntr"))
        .expect("override");
    assert_eq!(
        service.snapshot(id).expect("snapshot").selection(),
        ProfileSelection::Override
    );
    assert!(service.snapshot(id).expect("snapshot").profile().is_some());
}

#[test]
fn invalid_change_keeps_last_valid_profile_as_stale() {
    let descriptor = monitor("one");
    let id = descriptor.id();
    let mut service = DisplayProfileService::new();
    service
        .reconcile([provider_monitor(
            descriptor.clone(),
            ProfileProbe::Current(profile(4, *b"mntr")),
        )])
        .expect("initial");
    let current = service.snapshot(id).expect("current");
    let _ = service.events();
    service
        .reconcile([provider_monitor(
            descriptor,
            ProfileProbe::Failed(ProfileProbeFailure::Invalid),
        )])
        .expect("changed");
    let stale = service.snapshot(id).expect("stale");
    assert_eq!(stale.profile_id(), current.profile_id());
    assert_eq!(
        stale.status(),
        SelectionStatus::Stale(StaleReason::ProfileInvalid)
    );
    assert!(service.events().iter().any(|event| matches!(
        event,
        DisplayProfileEvent::SelectionChanged {
            status: SelectionStatus::Stale(_),
            ..
        }
    )));
}

#[test]
fn profile_hash_hotplug_and_window_generation_events_are_safe() {
    let first = monitor("one");
    let second = monitor("two");
    let first_id = first.id();
    let second_id = second.id();
    let mut service = DisplayProfileService::new();
    service
        .reconcile([
            provider_monitor(first.clone(), ProfileProbe::Current(profile(1, *b"mntr"))),
            provider_monitor(second.clone(), ProfileProbe::Current(profile(2, *b"mntr"))),
        ])
        .expect("initial");
    let _ = service.events();
    let first_snapshot = service.snapshot(first_id).expect("first");
    service
        .reconcile([provider_monitor(
            first,
            ProfileProbe::Current(profile(9, *b"mntr")),
        )])
        .expect("profile update");
    assert!(service.events().iter().any(|event| matches!(event, DisplayProfileEvent::ProfileHashChanged { monitor, .. } if *monitor == first_id)));
    assert!(matches!(
        service.snapshot(second_id),
        Err(rusttable_display_profile::SnapshotRequestError::Removed)
    ));

    let mut windows = WindowPresentation::default();
    windows.move_window("main", &first_snapshot);
    assert!(windows.accepts_transform("main", first_id, first_snapshot.generation()));
    assert!(!windows.accepts_transform(
        "main",
        first_id,
        first_snapshot.generation().saturating_add(1)
    ));
    let second_snapshot = service.snapshot(first_id).expect("new first");
    windows.move_window("secondary", &second_snapshot);
    assert!(windows.accepts_transform("main", first_id, first_snapshot.generation()));
    assert!(windows.accepts_transform("secondary", first_id, second_snapshot.generation()));
}

#[test]
fn event_queue_is_bounded() {
    let mut queue = EventQueue::new();
    for _ in 0..(MAX_QUEUED_EVENTS + 10) {
        queue.push(DisplayProfileEvent::FallbackChanged { generation: 1 });
    }
    assert_eq!(queue.len(), MAX_QUEUED_EVENTS);
    assert_eq!(queue.drain().len(), MAX_QUEUED_EVENTS);
}
