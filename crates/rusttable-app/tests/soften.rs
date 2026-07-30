use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use rusttable_app::gtk_controller::GtkDarkroomEditController;
use rusttable_app::workspace::{
    load_selected_export_render, load_selected_preview, run_raster_import,
};
use rusttable_catalog::{
    EditRepository, HistoryCommand, HistoryOperationKind, HistoryOperationSummary, HistoryPayload,
    HistoryRepository,
};
use rusttable_catalog_store::{RedbEditRepository, RedbHistoryRepository};
use rusttable_core::{
    Edit, EditId, FiniteF64, Operation, OperationId, OperationKey, OperationOpacity, ParameterName,
    ParameterValue, PhotoId, Revision,
};
use rusttable_pixelpipe::{
    CpuPixelpipeOutputMode, CpuPixelpipeSnapshot, RgbaF32ColorEncoding, RgbaF32Descriptor,
    RgbaF32Image, RgbaF32Pixel,
};
use rusttable_processing::{CompiledOperationGraph, RasterDimensions, builtin_registry};
use rusttable_render::RenderTarget;
use rusttable_ui::{DarkroomControlValue, DarkroomModuleAction, DarkroomModuleError};

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);
const MASK_BYTES: &[u8] = b"soften-mask-bundle-v1";
const PIPELINE_BYTES: &[u8] = b"soften-pipeline-bundle-v1";

#[test]
fn soften_app_boundary_preserves_typed_state_order_identity_and_render_receipts() {
    let workspace = TestWorkspace::new();
    let (photo_id, imported_edit) = import_fixture(&workspace);
    let soften_id = OperationId::new(0x501).expect("Soften operation ID");
    let initial = Edit::from_parts(
        imported_edit.id(),
        photo_id,
        imported_edit.base_photo_revision(),
        imported_edit.revision(),
        [
            registry_operation("rusttable.exposure", 0x502),
            registry_operation("rusttable.bloom", 0x503),
            soften_operation(soften_id, false, 64.0, 77.0, -0.25, 32.0, 0.625),
        ],
    )
    .expect("initial Soften edit");
    replace_edit(&workspace.catalog, &imported_edit, &initial);
    append_masked_soften_history(&workspace.catalog, &initial);

    let before = load_selected_preview(&workspace.catalog, &workspace.source_root, photo_id)
        .expect("disabled Soften preview");
    assert_eq!(before.receipt().edit_id(), initial.id());
    assert_eq!(before.receipt().edit_revision(), initial.revision());
    assert_eq!(before.receipt().generation(), 0);
    let before_identity = before.receipt().identity_hash();

    let mut controller = GtkDarkroomEditController::new(Some(workspace.catalog.clone()));
    let modules = controller
        .select_photo(photo_id)
        .expect("select Soften edit");
    let soften = modules
        .module_target("soften", Some(soften_id))
        .expect("exact Soften module");
    assert_eq!(soften.operation_id(), Some(soften_id));
    assert!(!soften.enabled());
    assert_eq!(soften.instance_count(), 1);
    assert_eq!(soften.widget_id(), "soften");
    assert!(soften.has_soften_custom_editor());
    assert!(soften.controls().controls().next().is_none());
    let soften_state = soften
        .soften_editor_state()
        .expect("source-shaped Soften editor state");
    assert_eq!(soften_state.editor().size().to_bits(), 64.0_f32.to_bits());
    assert_eq!(
        soften_state.editor().saturation().to_bits(),
        77.0_f32.to_bits()
    );
    assert_eq!(
        soften_state.editor().brightness().to_bits(),
        (-0.25_f32).to_bits()
    );
    assert_eq!(soften_state.editor().amount().to_bits(), 32.0_f32.to_bits());

    let settled = controller
        .apply(&DarkroomModuleAction::Control {
            module_id: "soften".to_owned(),
            operation_id: Some(soften_id),
            expected_revision: initial.revision(),
            id: "soften-amount".to_owned(),
            value: DarkroomControlValue::Slider(58.0),
        })
        .expect("settle Soften amount");
    let enabled = controller
        .apply(&DarkroomModuleAction::Enable {
            module_id: "soften".to_owned(),
            operation_id: Some(soften_id),
            expected_revision: settled.revision(),
            enabled: true,
        })
        .expect("enable Soften");
    assert!(enabled.processing_changed());

    let persisted = RedbEditRepository::open(&workspace.catalog)
        .expect("open edit repository after Soften action")
        .find_by_edit_id(initial.id())
        .expect("read persisted Soften edit")
        .expect("persisted Soften edit");
    assert_eq!(persisted.photo_id(), photo_id);
    assert_eq!(persisted.revision(), enabled.revision());
    assert_eq!(
        persisted
            .operations()
            .map(|operation| operation.key().as_str())
            .collect::<Vec<_>>(),
        ["rusttable.exposure", "rusttable.bloom", "rusttable.soften",]
    );
    let soften = persisted
        .operations()
        .find(|operation| operation.id() == soften_id)
        .expect("persisted Soften instance");
    assert!(soften.is_enabled());
    assert_eq!(
        soften.opacity(),
        OperationOpacity::new(0.625).expect("opacity")
    );
    assert_scalar_parameter(soften, "size", 64.0);
    assert_scalar_parameter(soften, "saturation", 77.0);
    assert_scalar_parameter(soften, "brightness", -0.25);
    assert_scalar_parameter(soften, "amount", 58.0);

    let after = load_selected_preview(&workspace.catalog, &workspace.source_root, photo_id)
        .expect("enabled Soften preview");
    assert_eq!(after.receipt().edit_id(), persisted.id());
    assert_eq!(after.receipt().edit_revision(), persisted.revision());
    assert_ne!(after.receipt().identity_hash(), before_identity);
    assert_ne!(after.clone().into_parts().2, before.clone().into_parts().2);

    let export = load_selected_export_render(
        &workspace.catalog,
        &workspace.source_root,
        photo_id,
        RenderTarget::FullResolution,
    )
    .expect("Soften export render");
    assert_eq!(export.provenance().source_photo_id(), photo_id);
    assert_eq!(export.provenance().source_edit_id(), persisted.id());
    assert_eq!(export.provenance().edit_revision(), persisted.revision());
    assert_eq!(export.image().dimensions().width(), 2);
    assert_eq!(export.image().dimensions().height(), 1);

    let history = RedbHistoryRepository::open(&workspace.catalog, photo_id)
        .expect("open Soften history after app edit");
    let history = history
        .load()
        .expect("load Soften history")
        .expect("Soften history");
    let current = history
        .current_revision()
        .expect("current history revision");
    assert_eq!(current.payload().edit(), &persisted);
    assert_eq!(current.payload().mask_bytes(), MASK_BYTES);
    assert_eq!(current.payload().pipeline_bytes(), PIPELINE_BYTES);
    assert!(history.revisions().any(|revision| {
        revision.payload().edit() == &initial
            && revision.payload().mask_bytes() == MASK_BYTES
            && revision.payload().pipeline_bytes() == PIPELINE_BYTES
    }));
}

#[test]
fn soften_projection_retains_extra_rows_as_opaque_pending_non_instances() {
    let workspace = TestWorkspace::new();
    let (photo_id, imported_edit) = import_fixture(&workspace);
    let first_id = OperationId::new(0x551).expect("first Soften operation ID");
    let second_id = OperationId::new(0x552).expect("second Soften operation ID");
    let edit = Edit::from_parts(
        imported_edit.id(),
        photo_id,
        imported_edit.base_photo_revision(),
        imported_edit.revision(),
        [
            soften_operation(first_id, true, 50.0, 100.0, 0.33, 50.0, 1.0),
            soften_operation(second_id, false, 12.0, 80.0, -0.5, 25.0, 0.5),
        ],
    )
    .expect("multi-row Soften edit");
    replace_edit(&workspace.catalog, &imported_edit, &edit);

    let mut controller = GtkDarkroomEditController::new(Some(workspace.catalog.clone()));
    let modules = controller
        .select_photo(photo_id)
        .expect("select multi-row Soften edit");
    let instances = modules.instances("soften").collect::<Vec<_>>();

    assert_eq!(instances.len(), 2);
    assert_eq!(instances[0].operation_id(), Some(first_id));
    assert_eq!(instances[1].operation_id(), Some(second_id));
    assert!(instances[0].soften_editor_state().is_some());
    let pending_state = instances[1]
        .soften_editor_state()
        .expect("typed core remains visible while blend/mask state is pending");
    assert_eq!(
        pending_state.editor().amount().to_bits(),
        25.0_f32.to_bits()
    );
    assert!(!pending_state.sensitive());
    assert!(!instances[0].supports_multi_instance());
    assert!(!instances[0].can_add_instance());
    assert!(!instances[1].availability().is_supported());
    assert!(
        instances[1]
            .availability()
            .reason()
            .is_some_and(|reason| { reason.contains("opaque") && reason.contains("pending") })
    );
    assert!(instances[1].controls().controls().next().is_none());

    let error = controller
        .apply(&DarkroomModuleAction::Enable {
            module_id: "soften".to_owned(),
            operation_id: Some(second_id),
            expected_revision: edit.revision(),
            enabled: true,
        })
        .expect_err("pending Soften rows cannot be edited");
    assert!(matches!(
        error,
        DarkroomModuleError::Unsupported { module_id, .. } if module_id == "soften"
    ));
    assert_eq!(
        RedbEditRepository::open(&workspace.catalog)
            .expect("reopen pending Soften catalog")
            .find_by_edit_id(edit.id())
            .expect("read pending Soften edit")
            .expect("pending Soften edit"),
        edit
    );
}

#[test]
fn soften_history_repository_round_trip_retains_mask_and_pipeline_payloads() {
    let workspace = TestWorkspace::new();
    let (photo_id, imported_edit) = import_fixture(&workspace);
    let soften_id = OperationId::new(0x601).expect("Soften operation ID");
    let edit = Edit::from_parts(
        imported_edit.id(),
        photo_id,
        imported_edit.base_photo_revision(),
        imported_edit.revision(),
        [soften_operation(
            soften_id, true, 50.0, 100.0, 0.33, 50.0, 1.0,
        )],
    )
    .expect("Soften history edit");
    replace_edit(&workspace.catalog, &imported_edit, &edit);
    append_masked_soften_history(&workspace.catalog, &edit);

    let repository =
        RedbHistoryRepository::open(&workspace.catalog, photo_id).expect("open Soften history");
    let state = repository
        .load()
        .expect("load Soften history")
        .expect("history state");
    let payload = state
        .current_revision()
        .expect("current history revision")
        .payload();
    assert_eq!(payload.edit(), &edit);
    assert_eq!(payload.mask_bytes(), MASK_BYTES);
    assert_eq!(payload.pipeline_bytes(), PIPELINE_BYTES);
}

#[test]
fn soften_snapshot_identity_changes_for_parameters_enabled_order_and_operation_id() {
    let photo_id = PhotoId::new(700).expect("photo ID");
    let base = soften_edit(701, photo_id, 4, 702, true, [50.0, 100.0, 0.33, 50.0]);
    let changed_parameter = soften_edit(701, photo_id, 4, 702, true, [50.0, 100.0, 0.33, 51.0]);
    let disabled = soften_edit(701, photo_id, 4, 702, false, [50.0, 100.0, 0.33, 50.0]);
    let changed_operation_id = soften_edit(701, photo_id, 4, 703, true, [50.0, 100.0, 0.33, 50.0]);

    let base_snapshot = snapshot(&base);
    assert_ne!(
        base_snapshot.identity(),
        snapshot(&changed_parameter).identity()
    );
    assert_ne!(base_snapshot.identity(), snapshot(&disabled).identity());
    assert_ne!(
        base_snapshot.identity(),
        snapshot(&changed_operation_id).identity()
    );

    let exposure = registry_operation("rusttable.exposure", 704);
    let soften = soften_operation(
        OperationId::new(702).expect("Soften operation ID"),
        true,
        50.0,
        100.0,
        0.33,
        50.0,
        1.0,
    );
    let first_order = Edit::from_parts(
        EditId::new(705).expect("edit ID"),
        photo_id,
        Revision::ZERO,
        Revision::from_u64(4),
        [exposure.clone(), soften.clone()],
    )
    .expect("first operation order");
    let second_order = Edit::from_parts(
        EditId::new(705).expect("edit ID"),
        photo_id,
        Revision::ZERO,
        Revision::from_u64(4),
        [soften, exposure],
    )
    .expect("second operation order");
    assert_ne!(
        snapshot(&first_order).identity(),
        snapshot(&second_order).identity()
    );
}

fn snapshot(edit: &Edit) -> CpuPixelpipeSnapshot {
    let graph = CompiledOperationGraph::compile(edit).expect("Soften graph");
    let dimensions = RasterDimensions::new(2, 1).expect("dimensions");
    let input = RgbaF32Image::new(
        RgbaF32Descriptor::new(dimensions, RgbaF32ColorEncoding::SrgbD65),
        vec![
            RgbaF32Pixel::new(0.1, 0.2, 0.3, 0.4),
            RgbaF32Pixel::new(0.7, 0.6, 0.5, 0.8),
        ],
    )
    .expect("snapshot input");
    CpuPixelpipeSnapshot::try_new(input, graph, CpuPixelpipeOutputMode::FullExport)
        .expect("Soften snapshot")
}

fn soften_edit(
    edit_id: u128,
    photo_id: PhotoId,
    revision: u64,
    operation_id: u128,
    enabled: bool,
    parameters: [f64; 4],
) -> Edit {
    let [size, saturation, brightness, amount] = parameters;
    Edit::from_parts(
        EditId::new(edit_id).expect("edit ID"),
        photo_id,
        Revision::ZERO,
        Revision::from_u64(revision),
        [soften_operation(
            OperationId::new(operation_id).expect("operation ID"),
            enabled,
            size,
            saturation,
            brightness,
            amount,
            1.0,
        )],
    )
    .expect("Soften edit")
}

fn soften_operation(
    operation_id: OperationId,
    enabled: bool,
    size: f64,
    saturation: f64,
    brightness: f64,
    amount: f64,
    opacity: f64,
) -> Operation {
    Operation::new_with_opacity(
        operation_id,
        OperationKey::new("rusttable.soften").expect("Soften key"),
        enabled,
        OperationOpacity::new(opacity).expect("Soften opacity"),
        [
            scalar_parameter("size", size),
            scalar_parameter("saturation", saturation),
            scalar_parameter("brightness", brightness),
            scalar_parameter("amount", amount),
        ],
    )
    .expect("Soften operation")
}

fn registry_operation(key: &str, operation_id: u128) -> Operation {
    builtin_registry()
        .materialize_operation(
            key,
            OperationId::new(operation_id).expect("registry operation ID"),
        )
        .expect("registry operation defaults")
}

fn scalar_parameter(name: &str, value: f64) -> (ParameterName, ParameterValue) {
    (
        ParameterName::new(name).expect("parameter name"),
        ParameterValue::Scalar(FiniteF64::new(value).expect("finite parameter")),
    )
}

fn assert_scalar_parameter(operation: &Operation, name: &str, expected: f64) {
    let parameter = ParameterName::new(name).expect("parameter name");
    let Some(ParameterValue::Scalar(value)) = operation.parameter(&parameter) else {
        panic!("Soften parameter {name} is not scalar");
    };
    assert_eq!(
        value.get().to_bits(),
        expected.to_bits(),
        "parameter {name}"
    );
}

fn replace_edit(catalog: &Path, current: &Edit, replacement: &Edit) {
    let mut repository = RedbEditRepository::open(catalog).expect("open edit repository");
    repository
        .commit_replacement(current.revision(), replacement)
        .expect("replace edit");
}

fn append_masked_soften_history(catalog: &Path, edit: &Edit) {
    let mut repository =
        RedbHistoryRepository::open(catalog, edit.photo_id()).expect("open history");
    let mut state = repository
        .load()
        .expect("load history")
        .expect("history exists");
    let expected = state.version();
    let summary = HistoryOperationSummary::new(
        HistoryOperationKind::Parameter,
        edit.operations()
            .find(|operation| operation.key().as_str() == "rusttable.soften")
            .map(Operation::id),
        Some(OperationKey::new("rusttable.soften").expect("Soften key")),
        "Soften mask payload",
    )
    .expect("history summary");
    state
        .apply(
            expected,
            HistoryCommand::Append {
                payload: HistoryPayload::new(edit.clone(), MASK_BYTES, PIPELINE_BYTES, summary),
            },
        )
        .expect("append masked Soften history");
    repository
        .commit(expected, &state)
        .expect("persist masked Soften history");
}

fn import_fixture(workspace: &TestWorkspace) -> (PhotoId, Edit) {
    let bytes = decode_base64(include_str!(
        "../../rusttable-image-io/tests/fixtures/rgba-2x1.png.b64"
    ));
    fs::write(workspace.source(), bytes).expect("write fixture");
    let batch = run_raster_import(
        &workspace.catalog,
        vec![workspace.source()],
        &rusttable_import::RasterImportCancellation::default(),
        &|_| {},
    );
    let photo_id = batch
        .receipts()
        .next()
        .and_then(|receipt| receipt.photo_id)
        .expect("imported photo");
    let edit = RedbEditRepository::open(&workspace.catalog)
        .expect("open imported edit repository")
        .list()
        .expect("list imported edits")
        .into_iter()
        .find(|edit| edit.photo_id() == photo_id)
        .expect("imported edit");
    (photo_id, edit)
}

fn decode_base64(encoded: &str) -> Vec<u8> {
    let mut output = Vec::new();
    let mut bytes = encoded.bytes().filter(|byte| !byte.is_ascii_whitespace());
    while let Some(first) = bytes.next() {
        let second = bytes.next().expect("complete base64 quartet");
        let third = bytes.next().expect("complete base64 quartet");
        let fourth = bytes.next().expect("complete base64 quartet");
        let values = [
            base64_value(first),
            base64_value(second),
            if third == b'=' {
                0
            } else {
                base64_value(third)
            },
            if fourth == b'=' {
                0
            } else {
                base64_value(fourth)
            },
        ];
        output.push((values[0] << 2) | (values[1] >> 4));
        if third != b'=' {
            output.push((values[1] << 4) | (values[2] >> 2));
        }
        if fourth != b'=' {
            output.push((values[2] << 6) | values[3]);
        }
    }
    output
}

fn base64_value(byte: u8) -> u8 {
    "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/"
        .bytes()
        .position(|candidate| candidate == byte)
        .expect("valid base64 fixture")
        .try_into()
        .expect("base64 alphabet index fits in u8")
}

struct TestWorkspace {
    root: PathBuf,
    source_root: PathBuf,
    source_path: PathBuf,
    catalog: PathBuf,
}

impl TestWorkspace {
    fn new() -> Self {
        let number = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "rusttable-app-soften-boundary-{}-{number}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("source-root")).expect("temporary source root");
        Self {
            source_path: root.join("source-root/soften.png"),
            source_root: root.join("source-root"),
            catalog: root.join("catalog.redb"),
            root,
        }
    }

    fn source(&self) -> PathBuf {
        self.source_path.clone()
    }
}

impl Drop for TestWorkspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
