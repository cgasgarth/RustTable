#![forbid(unsafe_code)]
#![doc = "Camera capture and import boundary contracts for the `RustTable` rewrite."]

pub mod purpose;

mod contract;

pub use contract::{
    CameraCapability, CameraCommand, CameraDevice, CameraDeviceState, CameraErrorCode, CameraEvent,
    CameraFrame, CameraFrameOrientation, CameraReceipt, CameraServiceError, CameraServicePort,
    CameraSession, CameraSessionState, CapturePolicy, CaptureProgress, CaptureStage,
    MAX_FRAME_BYTES, SettingKind, SettingValue,
};
