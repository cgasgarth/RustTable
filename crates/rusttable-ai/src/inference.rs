use std::fmt;
use std::sync::{Arc, Mutex};

use rusttable_ai_native::{
    Cancellation, RuntimeAdapter, RuntimeConfiguration, Session, TensorBuffer,
};
use sha2::{Digest, Sha256};

use crate::CancellationToken;
use crate::cache::{SessionCache, SessionKey};
use crate::package::ModelPackage;
use crate::planning::{InferencePlan, InferenceTile};
use crate::qualification::{QualificationError, QualificationStore};

impl Cancellation for CancellationToken {
    fn is_cancelled(&self) -> bool {
        self.is_cancelled()
    }
}

#[derive(Debug, Clone)]
pub struct InferenceRequest<'a> {
    pub package: &'a ModelPackage,
    pub plan: &'a InferencePlan,
    pub inputs: &'a [TensorBuffer],
    pub cancellation: &'a CancellationToken,
}

#[derive(Debug, Clone, PartialEq)]
pub struct InferenceOutput {
    tile: InferenceTile,
    tensor: TensorBuffer,
}

impl InferenceOutput {
    #[must_use]
    pub const fn tile(&self) -> InferenceTile {
        self.tile
    }
    #[must_use]
    pub const fn tensor(&self) -> &TensorBuffer {
        &self.tensor
    }
}

pub struct InferenceService {
    adapter: Arc<dyn RuntimeAdapter>,
    qualifications: Mutex<QualificationStore>,
    sessions: Mutex<SessionCache<SessionKey, Box<dyn Session>>>,
}

impl InferenceService {
    #[must_use]
    pub fn new(adapter: Arc<dyn RuntimeAdapter>, session_budget: u64) -> Self {
        Self {
            adapter,
            qualifications: Mutex::new(QualificationStore::default()),
            sessions: Mutex::new(SessionCache::new(session_budget)),
        }
    }

    pub fn qualifications(
        &self,
    ) -> Result<std::sync::MutexGuard<'_, QualificationStore>, InferenceError> {
        self.qualifications
            .lock()
            .map_err(|_| InferenceError::PoisonedState)
    }

    pub fn execute(
        &self,
        request: &InferenceRequest<'_>,
    ) -> Result<Vec<InferenceOutput>, InferenceError> {
        if request.inputs.len() != request.plan.tiles().len() {
            return Err(InferenceError::InputTileCount);
        }
        if request.package.identity() != request.plan.model() {
            return Err(InferenceError::ModelPlanMismatch);
        }
        let provider = request.plan.provider();
        if !self.adapter.supports(native_provider(provider)) {
            return Err(InferenceError::ProviderUnavailable);
        }
        let qualification_hash = {
            let qualifications = self
                .qualifications
                .lock()
                .map_err(|_| InferenceError::PoisonedState)?;
            qualifications
                .require(request.package.identity(), provider)
                .map_err(InferenceError::Qualification)?
                .receipt_hash()
        };
        if request.cancellation.is_cancelled() {
            return Err(InferenceError::Cancelled);
        }
        let configuration = RuntimeConfiguration::CANONICAL_CPU;
        let key = SessionKey {
            model: request.package.identity(),
            qualification: qualification_hash,
            runtime_configuration: configuration_hash(&configuration),
        };
        let mut outputs = Vec::with_capacity(request.inputs.len());
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| InferenceError::PoisonedState)?;
        if let Some(session) = sessions.get_mut(&key) {
            run_session(&mut **session, request, &mut outputs)?;
            return Ok(outputs);
        }
        let mut session = self
            .adapter
            .open_session(
                native_provider(provider),
                request.package.model_bytes(),
                &configuration,
            )
            .map_err(InferenceError::Adapter)?;
        let memory = session.memory_bytes();
        run_session(&mut *session, request, &mut outputs)?;
        let _ = sessions.insert(key, session, memory);
        Ok(outputs)
    }

    #[must_use]
    pub fn session_count(&self) -> usize {
        self.sessions.lock().map_or(0, |sessions| sessions.len())
    }
}

const fn native_provider(provider: crate::Provider) -> rusttable_ai_native::Provider {
    match provider {
        crate::Provider::Cpu => rusttable_ai_native::Provider::Cpu,
        crate::Provider::CoreMl => rusttable_ai_native::Provider::CoreMl,
        crate::Provider::DirectMl => rusttable_ai_native::Provider::DirectMl,
        crate::Provider::Cuda => rusttable_ai_native::Provider::Cuda,
    }
}

fn run_session(
    session: &mut dyn Session,
    request: &InferenceRequest<'_>,
    outputs: &mut Vec<InferenceOutput>,
) -> Result<(), InferenceError> {
    for (tile, input) in request
        .plan
        .tiles()
        .iter()
        .copied()
        .zip(request.inputs.iter())
    {
        if request.cancellation.is_cancelled() {
            session.terminate();
            return Err(InferenceError::Cancelled);
        }
        let tensor = match session.run(input, request.cancellation) {
            Ok(tensor) => tensor,
            Err(rusttable_ai_native::AdapterError::Cancelled) => {
                session.terminate();
                return Err(InferenceError::Cancelled);
            }
            Err(error) => return Err(InferenceError::Adapter(error)),
        };
        outputs.push(InferenceOutput { tile, tensor });
    }
    Ok(())
}

fn configuration_hash(configuration: &RuntimeConfiguration) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update([configuration.optimization() as u8]);
    hasher.update(configuration.intra_threads().to_le_bytes());
    hasher.update(configuration.inter_threads().to_le_bytes());
    hasher.update([u8::from(configuration.sequential())]);
    hasher.finalize().into()
}

#[derive(Debug, Clone, PartialEq)]
pub enum InferenceError {
    InputTileCount,
    ModelPlanMismatch,
    ProviderUnavailable,
    Qualification(QualificationError),
    Adapter(rusttable_ai_native::AdapterError),
    Cancelled,
    PoisonedState,
}

impl fmt::Display for InferenceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "bounded inference failed: {self:?}")
    }
}

impl std::error::Error for InferenceError {}
