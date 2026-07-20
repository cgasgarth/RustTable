//! Non-destructive darkroom presentation diagnostics.

mod common;
mod overexposed;
mod raw_overexposed;

pub use common::{
    DiagnosticBackend, DiagnosticDescriptor, DiagnosticFinding, DiagnosticFrame,
    DiagnosticFrameError, DiagnosticGeometry, DiagnosticPath, RgbaPixel,
};
pub use overexposed::{
    OverexposedColorScheme, OverexposedMode, OverexposedPlan, OverexposedReceipt,
    OverexposedResult, OverexposedState,
};
pub use raw_overexposed::{
    RawOverexposedPlan, RawOverexposedReceipt, RawOverexposedResult, RawOverexposedState,
    RawOverlayMode, RawSolidColor,
};
