use std::fmt;

/// Bounded maximum for a frame payload crossing the service boundary.
pub const MAX_FRAME_BYTES: usize = 64 * 1024 * 1024;

/// A camera discovered by a backend without exposing a native handle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CameraDevice {
    id: String,
    label: String,
    backend: String,
    state: CameraDeviceState,
    capability_hash: Option<String>,
}

impl CameraDevice {
    #[must_use]
    pub fn new(
        id: impl Into<String>,
        label: impl Into<String>,
        backend: impl Into<String>,
        state: CameraDeviceState,
        capability_hash: Option<String>,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            backend: backend.into(),
            state,
            capability_hash,
        }
    }

    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }
    #[must_use]
    pub fn label(&self) -> &str {
        &self.label
    }
    #[must_use]
    pub fn backend(&self) -> &str {
        &self.backend
    }
    #[must_use]
    pub const fn state(&self) -> CameraDeviceState {
        self.state
    }
    #[must_use]
    pub fn capability_hash(&self) -> Option<&str> {
        self.capability_hash.as_deref()
    }
}

/// Truthful discovery/session availability shown by the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CameraDeviceState {
    Discovering,
    Ready,
    PermissionDenied,
    Busy,
    Disconnected,
    Unsupported,
}

impl CameraDeviceState {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Discovering => "Discovering",
            Self::Ready => "Ready",
            Self::PermissionDenied => "Permission denied",
            Self::Busy => "Busy",
            Self::Disconnected => "Disconnected",
            Self::Unsupported => "Unsupported",
        }
    }
}

/// A capability descriptor returned after a session is opened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CameraCapability {
    key: String,
    label: String,
    kind: SettingKind,
    unit: Option<String>,
    choices: Vec<String>,
    minimum: Option<i32>,
    maximum: Option<i32>,
    confirmed: SettingValue,
}

impl CameraCapability {
    #[expect(
        clippy::too_many_arguments,
        reason = "capability descriptors keep every service-confirmed field explicit"
    )]
    #[must_use]
    pub fn new(
        key: impl Into<String>,
        label: impl Into<String>,
        kind: SettingKind,
        unit: Option<String>,
        choices: Vec<String>,
        minimum: Option<i32>,
        maximum: Option<i32>,
        confirmed: SettingValue,
    ) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
            kind,
            unit,
            choices,
            minimum,
            maximum,
            confirmed,
        }
    }

    #[must_use]
    pub fn key(&self) -> &str {
        &self.key
    }
    #[must_use]
    pub fn label(&self) -> &str {
        &self.label
    }
    #[must_use]
    pub const fn kind(&self) -> SettingKind {
        self.kind
    }
    #[must_use]
    pub fn unit(&self) -> Option<&str> {
        self.unit.as_deref()
    }
    #[must_use]
    pub fn choices(&self) -> &[String] {
        &self.choices
    }
    #[must_use]
    pub const fn minimum(&self) -> Option<i32> {
        self.minimum
    }
    #[must_use]
    pub const fn maximum(&self) -> Option<i32> {
        self.maximum
    }
    #[must_use]
    pub const fn confirmed(&self) -> &SettingValue {
        &self.confirmed
    }
}

/// The shape of a camera setting control.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingKind {
    Toggle,
    Choice,
    Range,
    Action,
}

/// A validated setting value; no native property pointers cross the port.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingValue {
    Toggle(bool),
    Choice(String),
    Number(i32),
    Action,
}

/// An exclusive camera lease identified by a generation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CameraSession {
    device_id: String,
    generation: u64,
    state: CameraSessionState,
}

impl CameraSession {
    #[must_use]
    pub fn new(device_id: impl Into<String>, generation: u64, state: CameraSessionState) -> Self {
        Self {
            device_id: device_id.into(),
            generation,
            state,
        }
    }
    #[must_use]
    pub fn device_id(&self) -> &str {
        &self.device_id
    }
    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }
    #[must_use]
    pub const fn state(&self) -> CameraSessionState {
        self.state
    }
}

/// Session lifecycle including recoverable device loss.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CameraSessionState {
    Closed,
    Opening,
    Ready,
    LiveView,
    Capturing,
    Lost,
    Busy,
}

impl CameraSessionState {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Closed => "No active session",
            Self::Opening => "Opening camera",
            Self::Ready => "Connected",
            Self::LiveView => "Live view",
            Self::Capturing => "Capturing",
            Self::Lost => "Camera disconnected",
            Self::Busy => "Session busy",
        }
    }
}

/// Immutable, service-provided RGBA pixels for a bounded latest-frame view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CameraFrame {
    width: u32,
    height: u32,
    stride: u32,
    rgba8: Vec<u8>,
    sequence: u64,
    camera_timestamp_ms: u64,
    orientation: CameraFrameOrientation,
    dropped_frames: u32,
}

impl CameraFrame {
    #[expect(
        clippy::too_many_arguments,
        reason = "frame metadata and bounded pixel payload form one immutable service value"
    )]
    #[must_use]
    pub fn new(
        width: u32,
        height: u32,
        stride: u32,
        rgba8: Vec<u8>,
        sequence: u64,
        camera_timestamp_ms: u64,
        orientation: CameraFrameOrientation,
        dropped_frames: u32,
    ) -> Self {
        Self {
            width,
            height,
            stride,
            rgba8,
            sequence,
            camera_timestamp_ms,
            orientation,
            dropped_frames,
        }
    }
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }
    #[must_use]
    pub const fn stride(&self) -> u32 {
        self.stride
    }
    #[must_use]
    pub fn rgba8(&self) -> &[u8] {
        &self.rgba8
    }
    #[must_use]
    pub const fn sequence(&self) -> u64 {
        self.sequence
    }
    #[must_use]
    pub const fn camera_timestamp_ms(&self) -> u64 {
        self.camera_timestamp_ms
    }
    #[must_use]
    pub const fn orientation(&self) -> CameraFrameOrientation {
        self.orientation
    }
    #[must_use]
    pub const fn dropped_frames(&self) -> u32 {
        self.dropped_frames
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CameraFrameOrientation {
    Normal,
    Rotate90,
    Rotate180,
    Rotate270,
}

/// Explicit transfer policy; delete-after-import is never implicit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapturePolicy {
    RetainOnCamera,
    DeleteAfterVerifiedImport,
}

/// Typed capture workflow progress.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureProgress {
    pub capture_id: String,
    pub stage: CaptureStage,
    pub completed: u32,
    pub total: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureStage {
    Triggering,
    Transferring,
    Verifying,
    Importing,
    Complete,
    Ambiguous,
    Failed,
}

impl CaptureStage {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Triggering => "Triggering shutter",
            Self::Transferring => "Transferring",
            Self::Verifying => "Verifying transfer",
            Self::Importing => "Registering import",
            Self::Complete => "Imported",
            Self::Ambiguous => "Capture needs reconciliation",
            Self::Failed => "Capture failed",
        }
    }
}

/// Privacy-safe receipt summary suitable for the UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CameraReceipt {
    pub receipt_id: String,
    pub summary: String,
    pub retained_on_camera: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CameraErrorCode {
    Unavailable,
    PermissionDenied,
    Busy,
    Disconnected,
    Unsupported,
    StaleSession,
    Rejected,
    TransferFailed,
    ImportFailed,
}

/// Bounded service failure without native diagnostics or handles.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CameraServiceError {
    pub code: CameraErrorCode,
    pub detail: String,
}

impl fmt::Display for CameraServiceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "camera service {}: {}",
            self.code.label(),
            self.detail
        )
    }
}

impl std::error::Error for CameraServiceError {}

impl CameraErrorCode {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Unavailable => "unavailable",
            Self::PermissionDenied => "permission denied",
            Self::Busy => "busy",
            Self::Disconnected => "disconnected",
            Self::Unsupported => "unsupported",
            Self::StaleSession => "stale session",
            Self::Rejected => "rejected",
            Self::TransferFailed => "transfer failed",
            Self::ImportFailed => "import failed",
        }
    }
}

/// Commands emitted by GTK controllers and handled by the #469 service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CameraCommand {
    Discover,
    Open {
        device_id: String,
    },
    Close {
        generation: u64,
    },
    SetSetting {
        generation: u64,
        key: String,
        value: SettingValue,
    },
    StartLiveView {
        generation: u64,
    },
    StopLiveView {
        generation: u64,
    },
    Capture {
        generation: u64,
        policy: CapturePolicy,
    },
    ResumeCapture {
        capture_id: String,
    },
    ReconcileCapture {
        capture_id: String,
    },
}

/// Events projected back to the GTK controller by the service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CameraEvent {
    Devices(Vec<CameraDevice>),
    Session(CameraSession),
    Capabilities {
        generation: u64,
        values: Vec<CameraCapability>,
    },
    Frame {
        generation: u64,
        frame: CameraFrame,
    },
    Capture(CaptureProgress),
    Receipt(CameraReceipt),
    Error(CameraServiceError),
}

/// Typed port implemented by the application camera service or a deterministic fake.
pub trait CameraServicePort {
    /// Dispatch one command and return the immediate state projection.
    ///
    /// # Errors
    ///
    /// Returns a bounded service error when the device is unavailable, busy,
    /// disconnected, or rejects the typed command.
    fn dispatch(&mut self, command: CameraCommand) -> Result<CameraEvent, CameraServiceError>;
}
