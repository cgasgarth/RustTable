use rusttable_camera::{
    CameraCommand, CameraEvent, CameraServiceError, CameraServicePort, CapturePolicy, SettingValue,
};

use super::CameraViewModel;

/// Typed user intent emitted by the camera GTK view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CameraAction {
    Discover,
    SelectDevice(String),
    Open,
    Close,
    SetSetting { key: String, value: SettingValue },
    StartLiveView,
    StopLiveView,
    Capture(CapturePolicy),
    ResumeCapture(String),
    ReconcileCapture(String),
}

/// Errors raised before a command can cross the typed camera port.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CameraControllerError {
    NoDeviceSelected,
    NoSession,
    Service(CameraServiceError),
}

impl From<CameraServiceError> for CameraControllerError {
    fn from(error: CameraServiceError) -> Self {
        Self::Service(error)
    }
}

/// Small controller that maps GTK actions to the #469 service port.
pub struct CameraController<P> {
    port: P,
    model: CameraViewModel,
}

impl<P: CameraServicePort> CameraController<P> {
    #[must_use]
    pub fn new(port: P) -> Self {
        Self {
            port,
            model: CameraViewModel::new(),
        }
    }
    #[must_use]
    pub const fn model(&self) -> &CameraViewModel {
        &self.model
    }
    /// Dispatches a typed camera intent and returns the updated projection.
    ///
    /// # Errors
    ///
    /// Returns an error when the intent requires a missing device/session or
    /// when the camera service rejects the command.
    pub fn dispatch(
        &mut self,
        action: CameraAction,
    ) -> Result<&CameraViewModel, CameraControllerError> {
        if let CameraAction::SelectDevice(id) = action {
            self.model.select_device(id);
            return Ok(&self.model);
        }
        let command = self.command(action)?;
        let event = self.port.dispatch(command)?;
        self.apply(event);
        Ok(&self.model)
    }

    fn command(&self, action: CameraAction) -> Result<CameraCommand, CameraControllerError> {
        let generation = self
            .model
            .session()
            .map_or(0, rusttable_camera::CameraSession::generation);
        match action {
            CameraAction::Discover => Ok(CameraCommand::Discover),
            CameraAction::SelectDevice(_) => {
                unreachable!("selection is handled before command mapping")
            }
            CameraAction::Open => self.model.selected_device().map_or(
                Err(CameraControllerError::NoDeviceSelected),
                |device_id| {
                    Ok(CameraCommand::Open {
                        device_id: device_id.to_owned(),
                    })
                },
            ),
            CameraAction::Close => self.session_command(CameraCommand::Close { generation }),
            CameraAction::SetSetting { key, value } => {
                self.session_command(CameraCommand::SetSetting {
                    generation,
                    key,
                    value,
                })
            }
            CameraAction::StartLiveView => {
                self.session_command(CameraCommand::StartLiveView { generation })
            }
            CameraAction::StopLiveView => {
                self.session_command(CameraCommand::StopLiveView { generation })
            }
            CameraAction::Capture(policy) => {
                self.session_command(CameraCommand::Capture { generation, policy })
            }
            CameraAction::ResumeCapture(capture_id) => {
                Ok(CameraCommand::ResumeCapture { capture_id })
            }
            CameraAction::ReconcileCapture(capture_id) => {
                Ok(CameraCommand::ReconcileCapture { capture_id })
            }
        }
    }

    fn session_command(
        &self,
        command: CameraCommand,
    ) -> Result<CameraCommand, CameraControllerError> {
        if self.model.session().is_some() {
            Ok(command)
        } else {
            Err(CameraControllerError::NoSession)
        }
    }

    fn apply(&mut self, event: CameraEvent) {
        match event {
            CameraEvent::Devices(devices) => self.model.set_devices(devices),
            CameraEvent::Session(session) => self.model.set_session(session),
            CameraEvent::Capabilities { values, .. } => self.model.set_capabilities(values),
            CameraEvent::Frame { frame, .. } => self.model.set_frame(frame),
            CameraEvent::Capture(capture) => self.model.set_capture(capture),
            CameraEvent::Receipt(receipt) => self.model.set_receipt(receipt),
            CameraEvent::Error(error) => self.model.set_diagnostic(Some(error.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use rusttable_camera::{
        CameraDevice, CameraDeviceState, CameraEvent, CameraSession, CameraSessionState,
    };

    use super::*;

    struct Fake {
        events: Vec<CameraEvent>,
        commands: Vec<CameraCommand>,
    }
    impl CameraServicePort for Fake {
        fn dispatch(&mut self, command: CameraCommand) -> Result<CameraEvent, CameraServiceError> {
            self.commands.push(command);
            Ok(self.events.remove(0))
        }
    }

    #[test]
    fn controller_keeps_selection_and_session_generation_typed() {
        let device = CameraDevice::new(
            "camera-1",
            "Studio camera",
            "fake",
            CameraDeviceState::Ready,
            None,
        );
        let fake = Fake {
            events: vec![
                CameraEvent::Devices(vec![device]),
                CameraEvent::Session(CameraSession::new("camera-1", 7, CameraSessionState::Ready)),
            ],
            commands: Vec::new(),
        };
        let mut controller = CameraController::new(fake);
        controller
            .dispatch(CameraAction::Discover)
            .expect("discovery");
        controller.dispatch(CameraAction::Open).expect("open");
        assert_eq!(controller.model().selected_device(), Some("camera-1"));
        assert_eq!(
            controller.model().session().map(CameraSession::generation),
            Some(7)
        );
    }
}
