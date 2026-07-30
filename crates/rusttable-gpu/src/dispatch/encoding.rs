use super::model::{
    BindingResource, DispatchError, DispatchFailure, DispatchRegion, EncodingReceipt, GridPlan,
    ParameterBlock, ParityContract, PreparedGpuKernel, ReceiptStatus, digest,
};
use crate::transfer::TransferPlan;

#[derive(Debug, Clone, PartialEq)]
pub struct EncodedDispatch {
    pub identity: [u8; 32],
    pub label: String,
    pub entry_point: String,
    pub parameters: ParameterBlock,
    pub bindings: Vec<BindingResource>,
    pub region: DispatchRegion,
    pub grid: GridPlan,
    pub transfers: Vec<TransferPlan>,
    pub parity: ParityContract,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EncodedBatch {
    pub receipt: EncodingReceipt,
    pub commands: Vec<EncodedDispatch>,
}

impl EncodedBatch {
    pub(super) fn single(command: EncodedDispatch, identity: [u8; 32]) -> Self {
        Self {
            receipt: EncodingReceipt {
                identity,
                status: ReceiptStatus::Encoded,
                command_count: 1,
                submitted: false,
                error: None,
            },
            commands: vec![command],
        }
    }
}

#[derive(Debug, Default)]
pub struct CommandEncoder {
    commands: Vec<EncodedDispatch>,
    reject: bool,
}

impl CommandEncoder {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn commands(&self) -> &[EncodedDispatch] {
        &self.commands
    }

    pub fn reject_next(&mut self) {
        self.reject = true;
    }

    pub(super) fn append(&mut self, commands: &[EncodedDispatch]) -> Result<(), DispatchError> {
        if self.reject {
            self.reject = false;
            return Err(DispatchError::DeviceUnavailable);
        }
        self.commands.extend_from_slice(commands);
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct DispatchBatch {
    kernels: Vec<PreparedGpuKernel>,
}

impl DispatchBatch {
    pub fn new(kernels: Vec<PreparedGpuKernel>) -> Result<Self, DispatchFailure> {
        if kernels.is_empty() {
            return Err(DispatchFailure::failed([0; 32], DispatchError::InvalidGrid));
        }
        let identity = batch_identity(&kernels);
        let generation = kernels[0].generation;
        if kernels.iter().any(|kernel| kernel.generation != generation) {
            return Err(DispatchFailure::failed(
                identity,
                DispatchError::GenerationMismatch,
            ));
        }
        Ok(Self { kernels })
    }

    pub fn encode(self, encoder: &mut CommandEncoder) -> Result<EncodedBatch, DispatchFailure> {
        let identity = batch_identity(&self.kernels);
        if self
            .kernels
            .iter()
            .any(|kernel| kernel.cancellation.is_cancelled())
        {
            return Err(DispatchFailure::cancelled(identity));
        }
        let commands = self
            .kernels
            .iter()
            .map(PreparedGpuKernel::command)
            .collect::<Vec<_>>();
        encoder
            .append(&commands)
            .map_err(|error| DispatchFailure::failed(identity, error))?;
        Ok(EncodedBatch {
            receipt: EncodingReceipt {
                identity,
                status: ReceiptStatus::Encoded,
                command_count: commands.len(),
                submitted: false,
                error: None,
            },
            commands,
        })
    }
}

fn batch_identity(kernels: &[PreparedGpuKernel]) -> [u8; 32] {
    digest(&[
        b"batch",
        &kernels
            .iter()
            .flat_map(|kernel| kernel.identity.digest)
            .collect::<Vec<_>>(),
    ])
}
