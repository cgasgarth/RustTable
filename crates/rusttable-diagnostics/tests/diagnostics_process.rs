use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use rusttable_diagnostics::{
    CorrelationContext, DiagnosticCode, DiagnosticEvent, DiagnosticField, Severity, Subsystem,
    install,
};

static COUNTER: AtomicU64 = AtomicU64::new(0);

#[test]
fn diagnostics_process() {
    if let Some(mode) = std::env::var_os("RUSTTABLE_DIAGNOSTICS_MODE") {
        child_mode(mode.to_string_lossy().as_ref());
        return;
    }

    let root = unique_directory("parent");
    fs::create_dir_all(&root).expect("temporary root");
    run_child("normal", &root).assert_success();
    let log = fs::read_to_string(root.join("rusttable.jsonl")).expect("normal JSON log");
    assert_eq!(log.lines().count(), 2);
    assert!(log.starts_with("{\"schema_version\":1,\"sequence\":1,\"timestamp_unix_ms\":"));
    assert!(log.contains("\"code\":\"lifecycle.startup\""));
    assert!(log.contains("\"code\":\"lifecycle.shutdown\""));
    let human = fs::read_to_string(root.join("rusttable.log")).expect("normal human log");
    assert!(human.contains("info lifecycle.startup"));
    assert!(human.contains("info lifecycle.shutdown"));

    let structured = unique_directory("structured");
    run_child("structured", &structured).assert_success();
    let structured_json = fs::read_to_string(structured.join("rusttable.jsonl")).unwrap();
    assert!(structured_json.contains("\"code\":\"import.source.open\""));
    assert!(structured_json.contains("\"severity\":\"warning\""));
    assert!(structured_json.contains("\"privacy\":\"private\",\"value\":\"alias-"));
    assert!(structured_json.contains("\"key\":\"format\""));
    assert!(!structured_json.contains("private/photo.raw"));
    assert!(!structured_json.contains("credential-sentinel"));
    assert!(!structured_json.contains("pixel-sentinel"));

    let selected_preview = unique_directory("selected-preview");
    run_child("selected-preview", &selected_preview).assert_success();
    assert_selected_preview_logs(&selected_preview);

    let concurrent = unique_directory("concurrent");
    run_child("concurrent", &concurrent).assert_success();
    assert_eq!(
        fs::read_to_string(concurrent.join("rusttable.jsonl"))
            .unwrap()
            .lines()
            .count(),
        80
    );

    let rotation = unique_directory("rotation");
    fs::create_dir_all(&rotation).unwrap();
    fs::write(rotation.join("rusttable.log"), vec![b'x'; 10 * 1024 * 1024]).unwrap();
    run_child("rotation", &rotation).assert_success();
    assert_eq!(
        fs::metadata(rotation.join("rusttable.log.1"))
            .unwrap()
            .len(),
        10 * 1024 * 1024
    );
    assert!(
        fs::read_to_string(rotation.join("rusttable.log"))
            .unwrap()
            .contains("startup")
    );

    let invalid = unique_directory("invalid");
    fs::write(&invalid, b"not a directory").unwrap();
    run_child("invalid", &invalid).assert_success();

    let static_crash = unique_directory("static-crash");
    let output = run_child("static-crash", &static_crash);
    assert!(!output.status.success());
    assert_one_bounded_report(&static_crash, "static_str", "static diagnostics panic");

    let dynamic_crash = unique_directory("dynamic-crash");
    let output = run_child("dynamic-crash", &dynamic_crash);
    assert!(!output.status.success());
    assert_one_bounded_report(&dynamic_crash, "dynamic_string", "[redacted]");
    assert!(!report_text(&dynamic_crash).contains("private-dynamic-sentinel"));

    let previous_hook = unique_directory("previous-hook");
    fs::create_dir_all(&previous_hook).unwrap();
    let previous_marker = previous_hook.join("previous-hook.txt");
    let output = run_child("previous-hook", &previous_hook);
    assert!(!output.status.success());
    assert_eq!(fs::read_to_string(previous_marker).unwrap(), "called\n");
    assert_eq!(crash_reports(&previous_hook).len(), 1);

    let retention = unique_directory("retention");
    for _ in 0..7 {
        assert!(!run_child("static-crash", &retention).status.success());
    }
    assert_eq!(crash_reports(&retention).len(), 5);

    #[cfg(unix)]
    {
        let symlink_dir = unique_directory("symlink");
        fs::create_dir_all(&symlink_dir).unwrap();
        let target = symlink_dir.join("target.log");
        fs::write(&target, b"target").unwrap();
        std::os::unix::fs::symlink(&target, symlink_dir.join("rusttable.log")).unwrap();
        assert!(run_child("normal", &symlink_dir).status.success());
        assert_eq!(fs::read(&target).unwrap(), b"target");
    }
}

fn child_mode(mode: &str) {
    match mode {
        "normal" => {
            let guard = install().expect("install");
            guard.record(&DiagnosticEvent::startup()).unwrap();
            guard.record(&DiagnosticEvent::shutdown()).unwrap();
        }
        "concurrent" => {
            let guard = Arc::new(install().expect("install"));
            let mut threads = Vec::new();
            for _ in 0..8 {
                let guard = Arc::clone(&guard);
                threads.push(std::thread::spawn(move || {
                    for _ in 0..10 {
                        guard.record(&DiagnosticEvent::startup()).unwrap();
                    }
                }));
            }
            for thread in threads {
                thread.join().unwrap();
            }
        }
        "rotation" => {
            let guard = install().expect("install");
            guard.record(&DiagnosticEvent::startup()).unwrap();
        }
        "structured" => {
            let guard = install().expect("install");
            let subsystem = Subsystem::new("import").unwrap();
            let code = DiagnosticCode::new(subsystem, "source.open").unwrap();
            let context = CorrelationContext::default().request(guard.redactor(), "request-42");
            let event = DiagnosticEvent::new(code, Severity::Warning, "open")
                .unwrap()
                .with_context(context)
                .with_field(DiagnosticField::public_text("format", "raw").unwrap())
                .unwrap()
                .with_field(DiagnosticField::path("private/photo.raw").unwrap())
                .unwrap()
                .with_field(DiagnosticField::credential("credential-sentinel").unwrap())
                .unwrap()
                .with_field(DiagnosticField::pixel_data(b"pixel-sentinel").unwrap())
                .unwrap();
            guard.record(&event).unwrap();
            assert_eq!(guard.recent_snapshot().unwrap().len(), 1);
        }
        "selected-preview" => {
            let guard = install().expect("install");
            let context = CorrelationContext::default()
                .photo(guard.redactor(), "/private/photo.raw")
                .request(guard.redactor(), "request-secret-preview");
            let metadata = rusttable_diagnostics::SelectedPreviewMetadata::default()
                .with_generation(12)
                .with_expected_generation(13)
                .with_dimensions(1920, 1080)
                .expect("valid dimensions")
                .with_byte_length(8_294_400)
                .with_format("raw")
                .expect("valid format")
                .with_source_kind("raw")
                .expect("valid source kind");
            let event = rusttable_diagnostics::DiagnosticEvent::selected_preview_failure(
                rusttable_diagnostics::SelectedPreviewFailureStage::SourceDecode,
                rusttable_diagnostics::SelectedPreviewFailureCode::UnsupportedFormat,
                rusttable_diagnostics::SelectedPreviewOperation::DecodeSource,
            )
            .with_context(context)
            .with_selected_preview_metadata(metadata)
            .expect("metadata fits");
            guard
                .record(&event)
                .expect("record selected preview failure");
            assert_eq!(guard.recent_snapshot().unwrap().len(), 1);
            assert!(!event.code().as_str().contains("secret-preview-sentinel"));
        }
        "invalid" => assert!(install().is_err()),
        "static-crash" => {
            let _guard = install().expect("install");
            panic!("static diagnostics panic");
        }
        "dynamic-crash" => {
            let _guard = install().expect("install");
            let message = String::from("private-dynamic-sentinel");
            panic!("{message}");
        }
        "previous-hook" => {
            let marker = std::env::var_os("RUSTTABLE_PREVIOUS_HOOK").unwrap();
            std::panic::set_hook(Box::new(move |_| {
                let _ = fs::write(&marker, b"called\n");
            }));
            let _guard = install().expect("install");
            panic!("previous hook panic");
        }
        _ => panic!("unknown diagnostics mode"),
    }
}

fn assert_selected_preview_logs(directory: &Path) {
    let selected_json =
        fs::read_to_string(directory.join("rusttable.jsonl")).expect("selected preview JSON log");
    let selected_line = selected_json
        .lines()
        .next()
        .expect("selected preview event");
    let selected_value: serde_json::Value =
        serde_json::from_str(selected_line).expect("valid selected preview JSONL");
    let selected_object = selected_value.as_object().expect("JSON object schema");
    assert_eq!(selected_object["schema_version"], 1);
    assert_eq!(selected_object["code"], "preview.selected_failure");
    assert_eq!(selected_object["severity"], "error");
    assert_eq!(selected_object["operation"], "decode_source");
    assert_eq!(
        selected_object["context"]["photo"].as_str().unwrap().len(),
        30
    );
    let fields = selected_object["fields"].as_array().expect("field array");
    assert_field(fields, "failure_stage", "source_decode");
    assert_field(fields, "failure_code", "unsupported_format");
    assert_field(fields, "generation", 12);
    assert_field(fields, "expected_generation", 13);
    assert_field(fields, "width", 1920);
    assert_field(fields, "height", 1080);
    assert_field(fields, "format", "raw");
    assert_field(fields, "source_kind", "raw");
    assert_field(fields, "byte_length", 8_294_400);
    assert!(!selected_json.contains("/private/photo.raw"));
    assert!(!selected_json.contains("request-secret-preview"));
    assert!(!selected_json.contains("secret-preview-sentinel"));
    assert!(!selected_json.contains("image-bytes-sentinel"));

    let selected_human =
        fs::read_to_string(directory.join("rusttable.log")).expect("selected preview human log");
    assert!(selected_human.contains("error preview.selected_failure"));
    assert!(selected_human.contains("failure_stage=source_decode"));
    assert!(selected_human.contains("failure_code=unsupported_format"));
    assert!(selected_human.contains("generation=12"));
    assert!(!selected_human.contains("/private/photo.raw"));
    assert!(!selected_human.contains("request-secret-preview"));
    assert!(!selected_human.contains("secret-preview-sentinel"));
    assert!(!selected_human.contains("image-bytes-sentinel"));
}

fn assert_field(fields: &[serde_json::Value], key: &str, value: impl Into<serde_json::Value>) {
    let value = value.into();
    assert!(
        fields
            .iter()
            .any(|field| { field["key"] == key && value == field["value"] })
    );
}

fn run_child(mode: &str, directory: &Path) -> Output {
    Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "diagnostics_process", "--nocapture"])
        .env("RUSTTABLE_DIAGNOSTICS_MODE", mode)
        .env("RUSTTABLE_DIAGNOSTICS_DIR", directory)
        .env("RUSTTABLE_PRIVACY_SENTINEL", "private-environment-sentinel")
        .env(
            "RUSTTABLE_PREVIOUS_HOOK",
            directory.join("previous-hook.txt"),
        )
        .output()
        .expect("child process")
}

fn unique_directory(label: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "rusttable-diagnostics-{label}-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&path);
    path
}

fn crash_reports(directory: &Path) -> Vec<PathBuf> {
    fs::read_dir(directory)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| {
                    name.starts_with("crash-")
                        && Path::new(name)
                            .extension()
                            .is_some_and(|extension| extension.eq_ignore_ascii_case("json"))
                })
        })
        .collect()
}

fn report_text(directory: &Path) -> String {
    fs::read_to_string(crash_reports(directory).into_iter().next().unwrap()).unwrap()
}

fn assert_one_bounded_report(directory: &Path, payload_kind: &str, payload: &str) {
    let reports = crash_reports(directory);
    assert_eq!(reports.len(), 1);
    let report = fs::read_to_string(&reports[0]).unwrap();
    assert!(report.len() <= 256 * 1024);
    assert!(report.ends_with('\n'));
    assert!(report.contains(&format!("\"payload_kind\":\"{payload_kind}\"")));
    assert!(report.contains("\"payload_class\":\"payload\""));
    assert!(report.contains("\"payload_text\":\"[redacted]\""));
    if payload == "private-dynamic-sentinel" {
        assert!(!report.contains(payload));
    }
    assert!(report.contains("\"backtrace_status\":"));
}

trait OutputExt {
    fn assert_success(self);
}

impl OutputExt for Output {
    fn assert_success(self) {
        assert!(
            self.status.success(),
            "child failed: {}",
            String::from_utf8_lossy(&self.stderr)
        );
    }
}
