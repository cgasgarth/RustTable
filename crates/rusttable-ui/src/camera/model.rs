use rusttable_camera::{
    CameraCapability, CameraDevice, CameraFrame, CameraReceipt, CameraSession, CaptureProgress,
};

/// Controller-owned projection for the camera panel.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CameraViewModel {
    devices: Vec<CameraDevice>,
    selected_device: Option<String>,
    session: Option<CameraSession>,
    capabilities: Vec<CameraCapability>,
    latest_frame: Option<CameraFrame>,
    capture: Option<CaptureProgress>,
    receipt: Option<CameraReceipt>,
    diagnostic: Option<String>,
}

impl CameraViewModel {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            devices: Vec::new(),
            selected_device: None,
            session: None,
            capabilities: Vec::new(),
            latest_frame: None,
            capture: None,
            receipt: None,
            diagnostic: None,
        }
    }
    #[must_use]
    pub fn devices(&self) -> &[CameraDevice] {
        &self.devices
    }
    #[must_use]
    pub fn selected_device(&self) -> Option<&str> {
        self.selected_device.as_deref()
    }
    #[must_use]
    pub const fn session(&self) -> Option<&CameraSession> {
        self.session.as_ref()
    }
    #[must_use]
    pub fn capabilities(&self) -> &[CameraCapability] {
        &self.capabilities
    }
    #[must_use]
    pub const fn latest_frame(&self) -> Option<&CameraFrame> {
        self.latest_frame.as_ref()
    }
    #[must_use]
    pub const fn capture(&self) -> Option<&CaptureProgress> {
        self.capture.as_ref()
    }
    #[must_use]
    pub const fn receipt(&self) -> Option<&CameraReceipt> {
        self.receipt.as_ref()
    }
    #[must_use]
    pub fn diagnostic(&self) -> Option<&str> {
        self.diagnostic.as_deref()
    }

    pub(super) fn set_devices(&mut self, devices: Vec<CameraDevice>) {
        if self
            .selected_device
            .as_ref()
            .is_none_or(|id| !devices.iter().any(|device| device.id() == id))
        {
            self.selected_device = devices.first().map(|device| device.id().to_owned());
        }
        self.devices = devices;
    }
    pub(super) fn select_device(&mut self, id: String) {
        self.selected_device = Some(id);
    }
    pub(super) fn set_session(&mut self, session: CameraSession) {
        if self
            .session
            .as_ref()
            .is_none_or(|current| session.generation() >= current.generation())
        {
            self.session = Some(session);
        }
    }
    pub(super) fn set_capabilities(&mut self, values: Vec<CameraCapability>) {
        self.capabilities = values;
    }
    pub(super) fn set_frame(&mut self, frame: CameraFrame) {
        if self
            .latest_frame
            .as_ref()
            .is_none_or(|current| frame.sequence() >= current.sequence())
        {
            self.latest_frame = Some(frame);
        }
    }
    pub(super) fn set_capture(&mut self, capture: CaptureProgress) {
        self.capture = Some(capture);
    }
    pub(super) fn set_receipt(&mut self, receipt: CameraReceipt) {
        self.receipt = Some(receipt);
    }
    pub(super) fn set_diagnostic(&mut self, diagnostic: Option<String>) {
        self.diagnostic = diagnostic;
    }
}
