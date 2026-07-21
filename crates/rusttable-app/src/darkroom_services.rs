//! Explicit unavailable adapters for darkroom services not yet wired to processing.

use rusttable_ui::{
    MaskManagerAction, MaskManagerServiceError, MaskManagerServicePort, MaskManagerSnapshot,
    MultiscaleRetouchAction, MultiscaleRetouchRequest, MultiscaleRetouchServiceError,
    MultiscaleRetouchServicePort, MultiscaleRetouchSnapshot,
};

#[derive(Debug, Default)]
pub(crate) struct UnavailableMaskManagerService;

impl MaskManagerServicePort for UnavailableMaskManagerService {
    fn snapshot(
        &mut self,
        generation: u64,
    ) -> Result<MaskManagerSnapshot, MaskManagerServiceError> {
        Ok(MaskManagerSnapshot::unavailable(
            generation,
            "mask graph and cross-operation consumers are not connected",
        ))
    }

    fn dispatch(
        &mut self,
        _: u64,
        _: &MaskManagerAction,
    ) -> Result<MaskManagerSnapshot, MaskManagerServiceError> {
        Err(MaskManagerServiceError::BackendUnavailable)
    }
}

#[derive(Debug, Default)]
pub(crate) struct UnavailableMultiscaleRetouchService;

impl MultiscaleRetouchServicePort for UnavailableMultiscaleRetouchService {
    fn snapshot(
        &mut self,
        generation: u64,
    ) -> Result<MultiscaleRetouchSnapshot, MultiscaleRetouchServiceError> {
        Ok(MultiscaleRetouchSnapshot::unavailable(
            generation,
            "retouch processing and wavelet lifecycle are not connected",
        ))
    }

    fn update(
        &mut self,
        _: u64,
        _: &MultiscaleRetouchAction,
    ) -> Result<MultiscaleRetouchSnapshot, MultiscaleRetouchServiceError> {
        Err(MultiscaleRetouchServiceError::BackendUnavailable)
    }

    fn start(
        &mut self,
        _: u64,
        _: &MultiscaleRetouchRequest,
    ) -> Result<u64, MultiscaleRetouchServiceError> {
        Err(MultiscaleRetouchServiceError::BackendUnavailable)
    }

    fn cancel(&mut self, _: u64, _: u64) -> Result<(), MultiscaleRetouchServiceError> {
        Err(MultiscaleRetouchServiceError::BackendUnavailable)
    }
}
