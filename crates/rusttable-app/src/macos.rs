//! GTK/GIO-facing macOS lifecycle contracts.
//!
//! Darktable receives Finder and Dock callbacks before its normal application startup path is
//! ready.  This module keeps the equivalent policy in owned Rust values so the GTK callback layer
//! only translates native `gio::File` values and forwards typed commands.

use std::collections::BTreeSet;
use std::fs::{self, symlink_metadata};
use std::path::{Path, PathBuf};

use rusttable_image::InputFormat;

/// Maximum number of files accepted from one native event or queued during startup.
pub const MAX_OPEN_FILES: usize = 256;

/// The `RustTable` bundle identity used by native application metadata and the runtime fallback.
pub const BUNDLE_IDENTIFIER: &str = "com.cgasgarth.rusttable";

/// Returns the bundle identifier that owns the running executable.
#[must_use]
pub fn runtime_bundle_identifier() -> String {
    #[cfg(target_os = "macos")]
    {
        let plist_path = std::env::current_exe().ok().and_then(|executable| {
            executable
                .parent()?
                .parent()
                .map(|contents| contents.join("Info.plist"))
        });
        if let Some(plist_path) = plist_path
            && let Ok(plist) = fs::read_to_string(plist_path)
            && let Some(identifier) = bundle_identifier_from_plist(&plist)
        {
            return identifier;
        }
    }
    BUNDLE_IDENTIFIER.to_owned()
}

fn bundle_identifier_from_plist(plist: &str) -> Option<String> {
    let (_, value) = plist.split_once("<key>CFBundleIdentifier</key>")?;
    let value = value.trim_start().strip_prefix("<string>")?;
    let (identifier, _) = value.split_once("</string>")?;
    (!identifier.is_empty()).then(|| identifier.to_owned())
}

/// A native-style application command that the GTK composition root can execute.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MacApplicationCommand {
    About,
    Preferences,
    Services,
    Hide,
    HideOthers,
    ShowAll,
    Window,
    Quit,
}

/// A typed lifecycle event produced by the native GTK/GIO adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MacApplicationEvent {
    Activate,
    Reopen,
    Terminate,
}

/// The target selected by the canonical application capability registry.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum MacOpenTarget {
    Image(PathBuf),
    Catalog(PathBuf),
}

impl MacOpenTarget {
    /// Returns the normalized local path delivered to the application service.
    #[must_use]
    pub fn path(&self) -> &Path {
        match self {
            Self::Image(path) | Self::Catalog(path) => path,
        }
    }
}

/// One bounded, versioned open/import request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MacOpenRequest {
    generation: u64,
    targets: Vec<MacOpenTarget>,
}

impl MacOpenRequest {
    fn new(generation: u64, targets: Vec<MacOpenTarget>) -> Self {
        Self {
            generation,
            targets,
        }
    }

    /// Returns the monotonic request generation used for idempotent delivery receipts.
    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    /// Returns targets in deterministic native-event order.
    #[must_use]
    pub fn targets(&self) -> &[MacOpenTarget] {
        &self.targets
    }

    /// Returns image targets while preserving their original order.
    pub fn image_paths(&self) -> impl Iterator<Item = &Path> {
        self.targets.iter().filter_map(|target| match target {
            MacOpenTarget::Image(path) => Some(path.as_path()),
            MacOpenTarget::Catalog(_) => None,
        })
    }

    /// Returns an explicitly selected catalog target, if present.
    #[must_use]
    pub fn catalog_path(&self) -> Option<&Path> {
        self.targets.iter().find_map(|target| match target {
            MacOpenTarget::Catalog(path) => Some(path.as_path()),
            MacOpenTarget::Image(_) => None,
        })
    }
}

/// Why one native file item was not delivered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MacOpenRejection {
    EventTooLarge,
    InvalidPath,
    Missing,
    SymlinkRejected,
    NotRegularFile,
    UnsupportedType,
    Duplicate,
    StartupQueueFull,
    ShuttingDown,
}

/// Result of translating one native open event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MacOpenDelivery {
    request: Option<MacOpenRequest>,
    rejected: Vec<MacOpenRejection>,
    queued: bool,
}

impl MacOpenDelivery {
    fn from_rejection(rejection: MacOpenRejection) -> Self {
        Self {
            request: None,
            rejected: vec![rejection],
            queued: false,
        }
    }

    /// Returns the request for immediate dispatch, if the application is ready.
    #[must_use]
    pub fn request(&self) -> Option<&MacOpenRequest> {
        self.request.as_ref()
    }

    /// Returns deterministic per-item rejection categories.
    #[must_use]
    pub fn rejected(&self) -> &[MacOpenRejection] {
        &self.rejected
    }

    /// Returns whether valid items were retained for post-startup dispatch.
    #[must_use]
    pub const fn queued(&self) -> bool {
        self.queued
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BridgeState {
    Starting,
    Ready,
    Stopping,
    Stopped,
}

/// Native lifecycle state that is safe to mutate from GTK's application main thread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MacApplicationBridge {
    state: BridgeState,
    generation: u64,
    startup_targets: Vec<MacOpenTarget>,
    seen_paths: BTreeSet<PathBuf>,
}

impl Default for MacApplicationBridge {
    fn default() -> Self {
        Self {
            state: BridgeState::Starting,
            generation: 0,
            startup_targets: Vec::new(),
            seen_paths: BTreeSet::new(),
        }
    }
}

impl MacApplicationBridge {
    /// Marks the application service registry ready and releases one startup batch.
    pub fn mark_ready(&mut self) -> Option<MacOpenRequest> {
        if self.state != BridgeState::Starting {
            return None;
        }
        self.state = BridgeState::Ready;
        let targets = std::mem::take(&mut self.startup_targets);
        self.request_from_targets(targets)
    }

    /// Marks the bridge as shutting down. Later native callbacks are ignored safely.
    pub fn mark_stopping(&mut self) {
        if matches!(self.state, BridgeState::Starting | BridgeState::Ready) {
            self.state = BridgeState::Stopping;
        }
    }

    /// Marks the bridge stopped after the GTK application has left its main loop.
    pub fn mark_stopped(&mut self) {
        self.state = BridgeState::Stopped;
    }

    /// Converts and delivers a bounded list of native local paths.
    pub fn receive_paths<I, P>(&mut self, paths: I) -> MacOpenDelivery
    where
        I: IntoIterator<Item = P>,
        P: Into<PathBuf>,
    {
        self.receive_optional_paths(paths.into_iter().map(|path| Some(path.into())))
    }

    /// Converts native file handles, retaining non-file URLs as explicit rejections.
    pub fn receive_optional_paths<I>(&mut self, paths: I) -> MacOpenDelivery
    where
        I: IntoIterator<Item = Option<PathBuf>>,
    {
        if matches!(self.state, BridgeState::Stopping | BridgeState::Stopped) {
            return MacOpenDelivery::from_rejection(MacOpenRejection::ShuttingDown);
        }
        let paths = paths.into_iter().collect::<Vec<_>>();
        if paths.is_empty() {
            return MacOpenDelivery {
                request: None,
                rejected: Vec::new(),
                queued: false,
            };
        }
        if paths.len() > MAX_OPEN_FILES {
            return MacOpenDelivery {
                request: None,
                rejected: vec![MacOpenRejection::EventTooLarge; paths.len()],
                queued: false,
            };
        }

        let mut targets = Vec::new();
        let mut rejected = Vec::new();
        for path in paths {
            let Some(path) = path else {
                rejected.push(MacOpenRejection::InvalidPath);
                continue;
            };
            let Some(path) = normalize_local_file(&path, &mut rejected) else {
                continue;
            };
            let Some(target) = classify_target(path, &mut rejected) else {
                continue;
            };
            if !self.seen_paths.insert(target.path().to_path_buf()) {
                rejected.push(MacOpenRejection::Duplicate);
                continue;
            }
            targets.push(target);
        }

        if self.state == BridgeState::Starting {
            let remaining = MAX_OPEN_FILES.saturating_sub(self.startup_targets.len());
            let accepted = targets.len().min(remaining);
            let rejected_count = targets.len() - accepted;
            self.startup_targets
                .extend(targets.into_iter().take(accepted));
            rejected.extend(std::iter::repeat_n(
                MacOpenRejection::StartupQueueFull,
                rejected_count,
            ));
            MacOpenDelivery {
                request: None,
                rejected,
                queued: accepted > 0,
            }
        } else {
            let request = self.request_from_targets(targets);
            MacOpenDelivery {
                request,
                rejected,
                queued: false,
            }
        }
    }

    /// Selects the Dock/activation window action without creating duplicate main windows.
    #[must_use]
    pub fn window_action(
        &self,
        event: MacApplicationEvent,
        visible_windows: usize,
    ) -> MacWindowAction {
        if !matches!(
            event,
            MacApplicationEvent::Activate | MacApplicationEvent::Reopen
        ) {
            return MacWindowAction::Ignore;
        }
        if matches!(self.state, BridgeState::Stopping | BridgeState::Stopped) {
            return MacWindowAction::Ignore;
        }
        if visible_windows == 0 {
            MacWindowAction::CreateMainWindow
        } else {
            MacWindowAction::FocusMainWindow
        }
    }

    /// Applies the coordinated termination policy before GTK is allowed to quit.
    pub fn request_termination(
        &mut self,
        durable_jobs: bool,
        user_confirmed: bool,
    ) -> MacTerminationDecision {
        if matches!(self.state, BridgeState::Stopping | BridgeState::Stopped) {
            return MacTerminationDecision::Ignore;
        }
        if durable_jobs && !user_confirmed {
            return MacTerminationDecision::Cancel;
        }
        self.mark_stopping();
        MacTerminationDecision::Proceed
    }

    fn request_from_targets(&mut self, targets: Vec<MacOpenTarget>) -> Option<MacOpenRequest> {
        if targets.is_empty() {
            return None;
        }
        self.generation = self.generation.wrapping_add(1);
        Some(MacOpenRequest::new(self.generation, targets))
    }
}

/// Action taken when a native activation or Dock reopen arrives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MacWindowAction {
    CreateMainWindow,
    FocusMainWindow,
    Ignore,
}

/// Result of the `RustTable` coordinated termination policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MacTerminationDecision {
    Proceed,
    Cancel,
    Ignore,
}

/// The document declarations used by the macOS bundle generator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MacDocumentType {
    pub uti: &'static str,
    pub extensions: &'static [&'static str],
}

/// Returns document declarations derived from the image decoder capability registry and the
/// explicit `RustTable` catalog policy.
#[must_use]
pub const fn document_types() -> [MacDocumentType; 2] {
    [
        MacDocumentType {
            uti: "public.image",
            extensions: &rusttable_image::SUPPORTED_INPUT_EXTENSIONS,
        },
        MacDocumentType {
            uti: "com.cgasgarth.rusttable.catalog",
            extensions: &["redb"],
        },
    ]
}

fn normalize_local_file(path: &Path, rejected: &mut Vec<MacOpenRejection>) -> Option<PathBuf> {
    if !path.is_absolute() {
        rejected.push(MacOpenRejection::InvalidPath);
        return None;
    }
    let metadata = match symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            rejected.push(MacOpenRejection::Missing);
            return None;
        }
        Err(_) => {
            rejected.push(MacOpenRejection::InvalidPath);
            return None;
        }
    };
    if has_symlink_component(path) {
        rejected.push(MacOpenRejection::SymlinkRejected);
        return None;
    }
    if !metadata.is_file() {
        rejected.push(MacOpenRejection::NotRegularFile);
        return None;
    }
    let Ok(canonical) = fs::canonicalize(path) else {
        rejected.push(MacOpenRejection::Missing);
        return None;
    };
    if has_symlink_component(&canonical) {
        rejected.push(MacOpenRejection::SymlinkRejected);
        return None;
    }
    Some(canonical)
}

fn has_symlink_component(path: &Path) -> bool {
    path.ancestors()
        .filter(|ancestor| !ancestor.as_os_str().is_empty())
        .any(|ancestor| {
            symlink_metadata(ancestor).is_ok_and(|metadata| metadata.file_type().is_symlink())
        })
}

fn classify_target(path: PathBuf, rejected: &mut Vec<MacOpenRejection>) -> Option<MacOpenTarget> {
    let extension = path.extension().and_then(|extension| extension.to_str());
    if extension.is_some_and(|extension| extension.eq_ignore_ascii_case("redb")) {
        return Some(MacOpenTarget::Catalog(path));
    }
    if extension.is_some_and(|extension| InputFormat::from_extension(extension).is_some()) {
        return Some(MacOpenTarget::Image(path));
    }
    rejected.push(MacOpenRejection::UnsupportedType);
    None
}

#[cfg(test)]
mod tests {
    use std::fs::{self, File};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::{
        BUNDLE_IDENTIFIER, MAX_OPEN_FILES, MacApplicationBridge, MacApplicationCommand,
        MacApplicationEvent, MacOpenRejection, MacOpenTarget, MacTerminationDecision,
        MacWindowAction, bundle_identifier_from_plist, document_types,
    };

    static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

    struct TempFiles {
        root: PathBuf,
    }

    impl TempFiles {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "rusttable-macos-open-{}-{}",
                std::process::id(),
                TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&root).expect("temporary directory");
            let root = fs::canonicalize(root).expect("canonical temporary directory");
            Self { root }
        }

        fn file(&self, name: &str) -> PathBuf {
            let path = self.root.join(name);
            File::create(&path).expect("temporary file");
            path
        }
    }

    impl Drop for TempFiles {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn cold_start_open_is_bounded_deduplicated_and_released_after_ready() {
        let files = TempFiles::new();
        let image = files.file("one.JPG");
        let catalog = files.file("library.redb");
        let mut bridge = MacApplicationBridge::default();

        let queued = bridge.receive_paths([image.clone(), image.clone(), catalog.clone()]);
        assert!(queued.queued());
        assert_eq!(queued.request(), None);
        assert_eq!(queued.rejected(), &[MacOpenRejection::Duplicate]);

        let request = bridge.mark_ready().expect("startup request");
        assert_eq!(request.generation(), 1);
        assert_eq!(request.targets().len(), 2);
        assert!(matches!(request.targets()[0], MacOpenTarget::Image(_)));
        assert!(matches!(request.targets()[1], MacOpenTarget::Catalog(_)));
    }

    #[test]
    fn ready_image_import_is_delivered_as_a_typed_request() {
        let files = TempFiles::new();
        let image = files.file("import.JPG");
        let mut bridge = MacApplicationBridge::default();
        assert!(bridge.mark_ready().is_none());

        let delivery = bridge.receive_paths([image.clone()]);
        let request = delivery.request().expect("ready import request");
        assert_eq!(
            request.image_paths().collect::<Vec<_>>(),
            vec![image.as_path()]
        );
        assert_eq!(request.catalog_path(), None);
        assert!(!delivery.queued());
    }

    #[test]
    fn invalid_items_do_not_hide_valid_items_and_symlinks_are_rejected() {
        let files = TempFiles::new();
        let image = files.file("valid.png");
        let unsupported = files.file("notes.txt");
        let link = files.root.join("link.png");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&image, &link).expect("symlink");

        let mut bridge = MacApplicationBridge::default();
        let delivery = bridge.receive_paths([image, unsupported, link]);
        assert!(delivery.queued());
        assert_eq!(delivery.rejected()[0], MacOpenRejection::UnsupportedType);
        #[cfg(unix)]
        assert_eq!(delivery.rejected()[1], MacOpenRejection::SymlinkRejected);
    }

    #[test]
    fn startup_queue_and_event_limits_are_explicit() {
        let files = TempFiles::new();
        let paths = (0..=MAX_OPEN_FILES)
            .map(|index| files.file(&format!("image-{index}.png")))
            .collect::<Vec<_>>();
        let mut bridge = MacApplicationBridge::default();
        let delivery = bridge.receive_paths(paths);
        assert_eq!(delivery.rejected().len(), MAX_OPEN_FILES + 1);
        assert!(
            delivery
                .rejected()
                .iter()
                .all(|rejection| *rejection == MacOpenRejection::EventTooLarge)
        );
    }

    #[test]
    fn activation_reopen_and_termination_follow_native_lifecycle_policy() {
        let mut bridge = MacApplicationBridge::default();
        assert_eq!(
            bridge.window_action(MacApplicationEvent::Reopen, 0),
            MacWindowAction::CreateMainWindow
        );
        assert_eq!(
            bridge.window_action(MacApplicationEvent::Activate, 1),
            MacWindowAction::FocusMainWindow
        );
        assert_eq!(
            bridge.request_termination(true, false),
            MacTerminationDecision::Cancel
        );
        assert_eq!(
            bridge.request_termination(true, true),
            MacTerminationDecision::Proceed
        );
        assert_eq!(
            bridge.window_action(MacApplicationEvent::Reopen, 0),
            MacWindowAction::Ignore
        );
    }

    #[test]
    fn native_menu_roles_and_document_declarations_are_stable() {
        assert_eq!(BUNDLE_IDENTIFIER, "com.cgasgarth.rusttable");
        assert_eq!(
            [
                MacApplicationCommand::About,
                MacApplicationCommand::Preferences,
                MacApplicationCommand::Services,
                MacApplicationCommand::Hide,
                MacApplicationCommand::HideOthers,
                MacApplicationCommand::ShowAll,
                MacApplicationCommand::Window,
                MacApplicationCommand::Quit,
            ]
            .len(),
            8
        );
        let declarations = document_types();
        assert_eq!(declarations[0].uti, "public.image");
        assert_eq!(
            declarations[0].extensions,
            &["jpg", "jpeg", "png", "tif", "tiff"]
        );
        assert_eq!(declarations[1].uti, "com.cgasgarth.rusttable.catalog");
    }

    #[test]
    fn installed_bundle_identifier_is_read_without_changing_the_fallback() {
        assert_eq!(
            bundle_identifier_from_plist(
                "<key>CFBundleIdentifier</key><string>com.cgasgarth.rusttable.latest</string>"
            ),
            Some("com.cgasgarth.rusttable.latest".to_owned())
        );
        assert_eq!(
            bundle_identifier_from_plist("<key>CFBundleName</key><string>RustTable</string>"),
            None
        );
        assert_eq!(BUNDLE_IDENTIFIER, "com.cgasgarth.rusttable");
    }
}
