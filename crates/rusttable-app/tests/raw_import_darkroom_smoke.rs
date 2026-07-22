use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;

use rusttable_app::gtk_controller::{
    GtkCatalogController, GtkCatalogState, GtkDarkroomPanelController,
};
use rusttable_app::gtk_preview_controller::{GtkPreviewController, GtkPreviewState};
use rusttable_app::gtk_thumbnail_controller::{GtkThumbnailController, GtkThumbnailSource};
use rusttable_app::workspace::run_raster_import;
use rusttable_image::{
    CancellationToken, ColorEncoding, DecodedImage, ImageInput, InputFormat, SampleType,
};
use rusttable_image_io::{FileImageInput, ImageDecoderRegistry};
use rusttable_import::RasterImportCancellation;
use rusttable_render::{MipmapLevel, ThumbnailGenerator, ThumbnailRequest, ThumbnailSize};
use rusttable_testkit::fixtures::deterministic_compressed_raf;
#[cfg(target_os = "linux")]
use rusttable_ui::HistogramData;
use rusttable_ui::gtk_shell::{DARKTABLE_DESKTOP_SPEC, DesktopRegion};
#[cfg(target_os = "linux")]
use rusttable_ui::gtk_shell::{DarkroomSelectionState, GtkShell, WorkspaceRole};
use rusttable_ui::presentation::{
    PresentationText, PreviewDimensions, Rgba8PreviewMetadata, SelectedPreviewState,
};
use rusttable_ui::{
    DarkroomPanelTarget, ImportAction, ImportRequest, ImportSourceModel, ViewportGeneration,
};

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

struct SmokeDirectory(PathBuf);

impl SmokeDirectory {
    fn new() -> Self {
        let number = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "rusttable-raw-import-darkroom-smoke-{}-{number}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("smoke directory");
        Self(path)
    }

    fn source(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }

    fn catalog(&self) -> PathBuf {
        self.0.join("catalog.redb")
    }
}

impl Drop for SmokeDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn persisted_xpro2_catalog_selection_decodes_and_publishes_the_selected_edit() {
    let fixture = deterministic_compressed_raf();
    let directory = SmokeDirectory::new();
    let source = directory.source(fixture.source_name());
    fs::write(&source, fixture.bytes()).expect("X-Pro2 RAF regression fixture");
    let catalog_path = directory.catalog();

    let batch = run_raster_import(
        &catalog_path,
        vec![source],
        &RasterImportCancellation::default(),
        &|_| {},
    );
    let photo_id = batch
        .first_selected_photo()
        .expect("catalog persisted the imported RAF selection");
    drop(batch);

    let mut catalog = GtkCatalogController::load_catalog_at(catalog_path);
    assert!(matches!(catalog.state(), GtkCatalogState::Ready(_)));
    assert!(catalog.select_photo(photo_id));
    let selected_edit = catalog
        .current_edit(photo_id)
        .expect("persisted edit lookup")
        .expect("selected RAF edit");

    let state = GtkPreviewController::new().render_selected(&catalog);
    let GtkPreviewState::Ready(preview) = state else {
        panic!("persisted X-Pro2 RAF selection must publish a preview");
    };
    let receipt = preview
        .receipt()
        .expect("published catalog preview receipt");
    assert_eq!(preview.photo_id(), photo_id);
    assert_eq!(receipt.edit_id(), selected_edit.id());
    assert_eq!(receipt.edit_revision(), selected_edit.revision());
    assert_eq!(preview.dimensions().width(), fixture.expected_width());
    assert_eq!(preview.dimensions().height(), fixture.expected_height());
    assert!(!preview.pixels().is_empty());
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "the focused smoke test keeps the import-to-darkroom evidence in one ordered path"
)]
fn cold_launch_main_preview_and_filmstrip_converge_on_neutral_raw_presentation() {
    let fixture = deterministic_compressed_raf();
    let directory = SmokeDirectory::new();
    let source = directory.source(fixture.source_name());
    fs::write(&source, fixture.bytes()).expect("deterministic RAW fixture");

    let mut source_model = ImportSourceModel::default();
    source_model.begin(1);
    source_model.replace_rows(1, [(source.clone(), true)]);
    source_model.set_ignore_nonraws(true);
    assert!(
        source_model
            .rows()
            .next()
            .is_some_and(rusttable_ui::ImportSourceRow::is_raw)
    );
    assert_eq!(source_model.effective_paths(), vec![source.clone()]);

    let request = ImportRequest::new(
        source_model.effective_paths(),
        source_model.recursive(),
        source_model.select_new(),
        source_model.ignore_nonraws(),
        source_model.generation(),
    )
    .expect("selected RAW source");
    let action = ImportAction::Import(request.clone());
    let ImportAction::Import(request) = action else {
        panic!("source selection must cross the typed import action boundary");
    };
    assert_eq!(request.paths(), std::slice::from_ref(&source));
    assert!(request.select_new());
    assert!(request.ignore_nonraws());

    let catalog_path = directory.catalog();
    let batch = run_raster_import(
        &catalog_path,
        request.paths().to_vec(),
        &RasterImportCancellation::default(),
        &|_| {},
    );
    let receipt = batch.receipts().next().expect("import receipt");
    assert_eq!(
        receipt.status,
        rusttable_import::RasterImportStatus::Imported
    );
    let photo_id = receipt.photo_id.expect("registered photo");

    let decoder = ImageDecoderRegistry::standard()
        .select(fixture.bytes())
        .expect("RAW decoder identity");
    assert_eq!(decoder.format(), InputFormat::Raw);
    assert_eq!(decoder.identity().id(), fixture.expected_decoder_id());
    assert_eq!(
        decoder.identity().version(),
        fixture.expected_decoder_version()
    );
    assert_eq!(
        decoder.identity().implementation(),
        fixture.expected_decoder_implementation()
    );
    let frame = FileImageInput::new(
        rusttable_image::DecodeLimits::new(64 * 1024 * 1024, 2_000, 2_000, 4_000_000, 8_000_000)
            .expect("typed frame limits"),
    )
    .decode_frame_bytes(fixture.bytes())
    .expect("typed RAF frame");
    assert_eq!(frame.sample_type(), SampleType::U8);
    assert_eq!(
        frame.image().descriptor().dimensions().width(),
        fixture.expected_width()
    );
    assert_eq!(
        frame.image().descriptor().dimensions().height(),
        fixture.expected_height()
    );

    let repository = rusttable_catalog_store::RedbCatalogRepository::open(&catalog_path)
        .expect("catalog registration");
    let details = repository
        .find_import_details_by_photo_id(photo_id)
        .expect("import details")
        .expect("durable import details");
    assert_eq!(details.summary().format(), InputFormat::Raw);
    assert_eq!(
        details.summary().dimensions(),
        decoder_probe(fixture.bytes()).dimensions()
    );
    assert_eq!(details.receipt().source_alias(), fixture.source_name());
    assert!(!format!("{details:?}").contains(directory.0.to_str().expect("UTF-8 path")));
    drop(repository);

    let mut thumbnails = GtkThumbnailController::open(
        &catalog_path,
        &directory.0,
        directory.0.join("thumbnail-cache"),
    )
    .expect("GTK thumbnail controller");
    let thumbnail = thumbnails.render(photo_id).expect("RAW thumbnail");
    assert_eq!(thumbnail.photo_id(), photo_id);
    assert_eq!(thumbnail.source(), GtkThumbnailSource::Render);
    assert!(!thumbnail.metadata().pixels().is_empty());

    let mut catalog = GtkCatalogController::load_catalog_at(catalog_path.clone());
    assert!(matches!(catalog.state(), GtkCatalogState::Ready(_)));
    assert!(catalog.select_photo(photo_id));

    let concurrent_preview_catalog = catalog.clone();
    let concurrent_thumbnail_catalog = catalog_path.clone();
    let concurrent_thumbnail_source_root = directory.0.clone();
    let concurrent_thumbnail_cache = directory.0.join("thumbnail-concurrent-cache");
    let (concurrent_preview, concurrent_thumbnail) = thread::scope(|scope| {
        let preview = scope.spawn(move || {
            GtkPreviewController::new().render_selected(&concurrent_preview_catalog)
        });
        let thumbnail = scope.spawn(move || {
            let mut controller = GtkThumbnailController::open(
                concurrent_thumbnail_catalog,
                concurrent_thumbnail_source_root,
                concurrent_thumbnail_cache,
            )
            .expect("concurrent RAW thumbnail controller");
            controller.render(photo_id)
        });
        (
            preview.join().expect("concurrent RAW preview worker"),
            thumbnail.join().expect("concurrent RAW thumbnail worker"),
        )
    });
    assert!(
        matches!(&concurrent_preview, GtkPreviewState::Ready(_)),
        "concurrent preview state: {concurrent_preview:?}"
    );
    let concurrent_thumbnail = concurrent_thumbnail.expect("concurrent RAW thumbnail worker");
    assert_eq!(
        concurrent_thumbnail.render_receipt_identity(),
        preview_receipt_identity(&concurrent_preview),
        "concurrent views must share the same persisted render receipt"
    );

    let preview = GtkPreviewController::new().render_selected(&catalog);
    let GtkPreviewState::Ready(preview) = preview else {
        panic!("selected RAW preview must render");
    };
    assert_eq!(preview.photo_id(), photo_id);
    assert_eq!(preview.dimensions().width(), fixture.expected_width());
    assert_eq!(preview.dimensions().height(), fixture.expected_height());
    let sensor_dimensions = decoder_probe(fixture.bytes()).dimensions();
    assert!(preview.dimensions().width() <= sensor_dimensions.width());
    assert!(preview.dimensions().height() <= sensor_dimensions.height());
    assert_eq!(
        preview.pixels().len(),
        usize::try_from(
            preview
                .dimensions()
                .decoded_byte_count()
                .expect("preview byte count"),
        )
        .expect("preview bytes fit usize")
    );
    let mean_rgb = preview
        .pixels()
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|pixel| &pixel[..3])
        .map(|channel| f64::from(*channel))
        .sum::<f64>()
        / f64::from(preview.dimensions().width() * preview.dimensions().height() * 3);
    assert!(
        mean_rgb >= 32.0,
        "selected RAW preview must not be near-black; mean RGB was {mean_rgb:.2}"
    );
    let channel_means = preview
        .pixels()
        .as_chunks::<4>()
        .0
        .iter()
        .fold([0.0_f64; 3], |mut sum, pixel| {
            for channel in 0..3 {
                sum[channel] += f64::from(pixel[channel]);
            }
            sum
        })
        .map(|value| {
            value / f64::from(preview.dimensions().width() * preview.dimensions().height())
        });
    let spread = channel_means
        .iter()
        .copied()
        .fold(f64::NEG_INFINITY, f64::max)
        - channel_means.iter().copied().fold(f64::INFINITY, f64::min);
    assert!(
        spread <= 8.0,
        "cold-launch neutral blue-sky/gray-building presentation is green/cyan: means {channel_means:?}"
    );

    let reselected = GtkPreviewController::new().render_selected(&catalog);
    let GtkPreviewState::Ready(reselected) = reselected else {
        panic!("reselecting the same RAW photo must reuse a ready preview");
    };
    assert_eq!(reselected.photo_id(), photo_id);
    assert_eq!(reselected.dimensions(), preview.dimensions());
    assert_eq!(reselected.pixels(), preview.pixels());
    assert_eq!(
        reselected
            .receipt()
            .map(rusttable_app::CatalogPreviewReceipt::identity_hash),
        preview
            .receipt()
            .map(rusttable_app::CatalogPreviewReceipt::identity_hash)
    );

    let source = DecodedImage::new_with_color_encoding(
        preview.dimensions(),
        preview.pixels().to_vec(),
        ColorEncoding::Srgb,
    )
    .expect("darkroom output is a valid thumbnail source");
    let request = ThumbnailRequest::new(
        MipmapLevel::zero(),
        ThumbnailSize::fit(180, 120).expect("bounded thumbnail request"),
    );
    let expected =
        ThumbnailGenerator::generate(&source, request, 2 * 1024 * 1024, &CancellationToken::new())
            .expect("downsample edited RAW render");
    assert_eq!(thumbnail.metadata().pixels(), expected.pixels());
    assert_eq!(
        thumbnail.render_receipt_identity(),
        preview
            .receipt()
            .map(rusttable_app::CatalogPreviewReceipt::identity_hash)
    );

    let mut darkroom_panels = GtkDarkroomPanelController::new(Some(catalog_path.clone()));
    let projections = darkroom_panels
        .select_photo(DarkroomPanelTarget::new(
            photo_id,
            ViewportGeneration::new(1),
            rusttable_core::Revision::ZERO,
        ))
        .expect("successful RAW open keeps darkroom rails available");
    assert!(matches!(
        projections.history().state(),
        rusttable_ui::DarkroomPanelState::Ready(_)
    ));

    let dimensions =
        PreviewDimensions::new(preview.dimensions().width(), preview.dimensions().height())
            .expect("GTK preview dimensions");
    let metadata = Rgba8PreviewMetadata::new(
        dimensions,
        PresentationText::new("rendered").expect("status text"),
        preview.pixels().to_vec(),
    )
    .expect("GTK preview payload");
    let GtkCatalogState::Ready(ready) = catalog.state() else {
        panic!("catalog remains ready after selection");
    };
    let loading_workspace = ready.workspace().clone();
    let workspace = loading_workspace
        .clone()
        .with_selected_preview(photo_id, SelectedPreviewState::Ready(metadata))
        .expect("selected detail");
    let detail = workspace
        .detail(photo_id)
        .expect("selected darkroom detail");
    assert!(matches!(
        detail.selected_preview(),
        SelectedPreviewState::Ready(_)
    ));

    assert_eq!(
        DARKTABLE_DESKTOP_SPEC.regions,
        &[
            DesktopRegion::Header,
            DesktopRegion::LeftPanel,
            DesktopRegion::CenterWorkspace,
            DesktopRegion::RightPanel,
            DesktopRegion::BottomFilmstrip,
        ]
    );

    #[cfg(target_os = "linux")]
    assert_gtk_darkroom_state(&loading_workspace, detail, photo_id);
}

#[cfg(target_os = "linux")]
fn assert_gtk_darkroom_state(
    loading_workspace: &rusttable_ui::PhotoWorkspaceViewModel,
    detail: &rusttable_ui::PhotoDetailViewModel,
    photo_id: rusttable_core::PhotoId,
) {
    if gtk4::init().is_err() {
        return;
    }
    let application = gtk4::Application::new(
        Some("com.cgasgarth.rusttable.test.raw-preview-lifecycle"),
        Default::default(),
    );
    let shell = GtkShell::new(&application);
    let generation = ViewportGeneration::new(1);
    shell.set_photo_workspace(loading_workspace);
    shell.begin_darkroom_selection(photo_id, generation);
    shell.set_darkroom_preview_loading(generation);
    let SelectedPreviewState::Ready(metadata) = detail.selected_preview() else {
        panic!("successful RAW projection must be ready");
    };
    let histogram = HistogramData::from_rgba8(metadata.dimensions(), metadata.pixels())
        .expect("RAW preview histogram");
    shell
        .set_darkroom_preview_result(generation, metadata, Ok(histogram))
        .expect("install RAW preview result");

    assert_eq!(
        shell.darkroom_preview().selection_state(),
        DarkroomSelectionState::Selected(photo_id)
    );
    assert!(shell.darkroom_preview().texture().is_some());
    assert_eq!(shell.darkroom_preview().status_label().text(), "rendered");

    shell.show_workspace(WorkspaceRole::Lighttable);
    assert!(shell.open_photo(photo_id));
    assert!(shell.darkroom_preview().texture().is_some());
    assert_eq!(shell.darkroom_preview().status_label().text(), "rendered");
}

fn preview_receipt_identity(state: &GtkPreviewState) -> Option<[u8; 32]> {
    match state {
        GtkPreviewState::Ready(preview) => preview
            .receipt()
            .map(rusttable_app::CatalogPreviewReceipt::identity_hash),
        GtkPreviewState::Failed(_) => None,
    }
}

fn decoder_probe(bytes: &[u8]) -> rusttable_image::ImageProbe {
    FileImageInput::new(
        rusttable_image::DecodeLimits::new(1_000_000, 2_000, 2_000, 10_000, 40_000)
            .expect("smoke decode limits"),
    )
    .probe_bytes(bytes)
    .expect("RAW probe")
}
