use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use rusttable_core::template::{Template, TemplateContext, VariableId};
use rusttable_core::{ContentHash, EditId, PhotoId, Revision};
use rusttable_export::copy::{SidecarSettings, SourceDescriptor};
use rusttable_export::queue::{
    CopyQueueRequest, ExportQueue, FileCopySourceProvider, LocalBundleDestination,
};
use rusttable_export::{
    ArtifactKind, Dependency, DependencySnapshot, DestinationSettings, ExportRequest,
};
use rusttable_import::{FileSourceSnapshotReader, ImportSourceLimits, SourceSnapshotReader};

static NEXT: AtomicU64 = AtomicU64::new(0);

fn temp_dir(label: &str) -> PathBuf {
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "rusttable-queue-{label}-{}-{id}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).unwrap();
    path
}

fn copy_request() -> ExportRequest {
    let request = ExportRequest::for_edit(
        PhotoId::new(1).unwrap(),
        EditId::new(2).unwrap(),
        Revision::from_u64(3),
        ArtifactKind::Image,
        Template::parse("${source_stem}").unwrap(),
        {
            let mut context = TemplateContext::new();
            context.set_text(VariableId::SourceStem, "photo");
            context
        },
        DependencySnapshot::new(Revision::from_u64(4), Revision::from_u64(3))
            .with_asset(Dependency::new("primary", ContentHash::Sha256([9; 32]))),
    )
    .with_destination(DestinationSettings::new(
        "local",
        rusttable_export::CollisionPolicy::CreateNew,
    ));
    request.with_metadata_policy(rusttable_export::MetadataPolicy {
        exif: rusttable_export::MetadataAction::Include,
        iptc: rusttable_export::MetadataAction::Include,
        xmp: rusttable_export::MetadataAction::Include,
        gps: rusttable_export::MetadataAction::Include,
        faces_and_regions: rusttable_export::MetadataAction::Include,
        ratings_labels_tags: rusttable_export::MetadataAction::Include,
        history: rusttable_export::MetadataAction::Include,
        thumbnail: rusttable_export::MetadataAction::Include,
        icc_and_cicp: rusttable_export::MetadataAction::Include,
        software_and_version: rusttable_export::MetadataAction::Include,
        user_fields: rusttable_export::MetadataAction::Include,
    })
}

#[test]
fn queue_snapshots_opaque_source_and_commits_one_idempotent_bundle() {
    let root = temp_dir("end-to-end");
    let source_path = root.join("original.jpg");
    fs::write(&source_path, b"original bytes that must not change").unwrap();
    let snapshot = FileSourceSnapshotReader
        .read_snapshot(&source_path, ImportSourceLimits::new(1024).unwrap())
        .unwrap();
    let source = SourceDescriptor::from_snapshot(&snapshot).unwrap();
    let request = copy_request();
    let sidecar = SidecarSettings::new(3, request.request_hash().unwrap()).with_history(b"history");
    let queued =
        CopyQueueRequest::from_request(&request, "asset-primary", source, "photo", Some(sidecar))
            .unwrap();
    let database = root.join("catalog.redb");
    let staging = root.join("staging");
    let destination_root = root.join("exports");
    let queue = ExportQueue::open(&database, &staging).unwrap();
    let job = queue.enqueue_copy(queued).unwrap();
    let duplicate = queue
        .enqueue_copy(
            CopyQueueRequest::from_request(
                &request,
                "asset-primary",
                SourceDescriptor::from_snapshot(&snapshot).unwrap(),
                "photo",
                Some(
                    SidecarSettings::new(3, request.request_hash().unwrap())
                        .with_history(b"history"),
                ),
            )
            .unwrap(),
        )
        .unwrap();
    assert_eq!(duplicate.id(), job.id());
    let record = queue.get(job.id()).unwrap().unwrap();
    assert_eq!(record.state(), rusttable_export::ExportJobState::Queued);
    assert!(
        !String::from_utf8_lossy(record.snapshot())
            .contains(source_path.to_string_lossy().as_ref())
    );

    let mut provider = FileCopySourceProvider::new(1024).unwrap();
    provider.register("asset-primary", &source_path).unwrap();
    let destination = LocalBundleDestination::new(&destination_root).unwrap();
    let receipt = queue
        .execute_copy(job.id(), &provider, &destination)
        .unwrap();
    assert_eq!(receipt.primary_bytes, 35);
    assert_eq!(
        queue.get(job.id()).unwrap().unwrap().state(),
        rusttable_export::ExportJobState::Succeeded
    );
    assert_eq!(
        fs::read(destination_root.join("photo/primary.jpg")).unwrap(),
        b"original bytes that must not change"
    );
    assert!(destination_root.join("photo/primary.xmp").exists());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn queue_rejects_incompatible_copy_requests() {
    let mut request = copy_request();
    request = request.with_dither_policy(rusttable_export::DitherPolicy::Ordered8x8);
    let source = SourceDescriptor::new([1; 32], 4, "jpg");
    let error =
        CopyQueueRequest::from_request(&request, "asset", source, "photo", None).unwrap_err();
    assert!(error.to_string().contains("dither"));
}
