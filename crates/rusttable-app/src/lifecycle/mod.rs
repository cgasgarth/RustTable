mod tasks;

pub use tasks::{
    CancellationReason, CancellationToken, ServiceTaskGroup, TaskDrainReceipt, TaskGroupError,
    TaskGroupId, TaskReceipt,
};

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    sync::Arc,
    time::Duration,
};

use futures::{FutureExt, future::BoxFuture};
use rusttable_diagnostics::{DiagnosticEvent, DiagnosticsError, DiagnosticsGuard};
use tokio::time;

/// The application lifecycle states. Startup and shutdown never move backward.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ServiceState {
    Created,
    Starting,
    Ready,
    Degraded,
    Stopping,
    Stopped,
}

impl ServiceState {
    fn can_transition_to(self, next: Self) -> bool {
        matches!(
            (self, next),
            (Self::Created, Self::Starting | Self::Stopped)
                | (
                    Self::Starting,
                    Self::Ready | Self::Degraded | Self::Stopping | Self::Stopped
                )
                | (Self::Ready, Self::Degraded | Self::Stopping)
                | (Self::Degraded, Self::Ready | Self::Stopping)
                | (Self::Stopping, Self::Stopped)
        )
    }
}

/// Stable service identity.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ServiceId(String);

impl ServiceId {
    /// Creates a service identity. Empty identities are rejected by the registry.
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// Returns the stable identity string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for ServiceId {
    fn from(id: &str) -> Self {
        Self::new(id)
    }
}

impl From<String> for ServiceId {
    fn from(id: String) -> Self {
        Self::new(id)
    }
}

impl fmt::Display for ServiceId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Whether a service is required for CPU/library operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceCriticality {
    Required,
    Optional,
}

/// Declarative service dependencies and shutdown policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceDescriptor {
    pub id: ServiceId,
    pub dependencies: Vec<ServiceId>,
    pub criticality: ServiceCriticality,
    pub shutdown_deadline: Duration,
}

impl ServiceDescriptor {
    /// Creates a descriptor with no dependencies and a bounded shutdown deadline.
    #[must_use]
    pub fn new(id: impl Into<ServiceId>, criticality: ServiceCriticality) -> Self {
        Self {
            id: id.into(),
            dependencies: Vec::new(),
            criticality,
            shutdown_deadline: Duration::from_secs(2),
        }
    }

    /// Adds a dependency while keeping descriptor construction fluent.
    #[must_use]
    pub fn with_dependencies(mut self, dependencies: impl IntoIterator<Item = ServiceId>) -> Self {
        self.dependencies = dependencies.into_iter().collect();
        self
    }

    /// Sets the per-service shutdown deadline.
    #[must_use]
    pub const fn with_shutdown_deadline(mut self, deadline: Duration) -> Self {
        self.shutdown_deadline = deadline;
        self
    }
}

/// Typed, privacy-safe service failure categories.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceErrorKind {
    Failed,
    Cancelled,
    TimedOut,
    Panic,
}

/// Service failure without serializing panic payloads or private configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServiceError {
    pub kind: ServiceErrorKind,
}

impl ServiceError {
    #[must_use]
    pub const fn failed() -> Self {
        Self {
            kind: ServiceErrorKind::Failed,
        }
    }

    #[must_use]
    pub const fn cancelled() -> Self {
        Self {
            kind: ServiceErrorKind::Cancelled,
        }
    }

    #[must_use]
    pub const fn timed_out() -> Self {
        Self {
            kind: ServiceErrorKind::TimedOut,
        }
    }

    #[must_use]
    pub const fn panic() -> Self {
        Self {
            kind: ServiceErrorKind::Panic,
        }
    }
}

impl fmt::Display for ServiceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "service {:?}", self.kind)
    }
}

impl std::error::Error for ServiceError {}

/// A service's current health as published to diagnostics and GTK models.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceHealthStatus {
    Unknown,
    Starting,
    Ready,
    Degraded,
    Failed,
    Stopped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceHealthFinding {
    StartFailed,
    StartPanicked,
    StopFailed,
    StopTimedOut,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServiceHealth {
    pub status: ServiceHealthStatus,
    pub finding: Option<ServiceHealthFinding>,
}

/// Immutable, revisioned health view suitable for UI projections.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceHealthSnapshot {
    pub receipt_version: u32,
    pub revision: u64,
    pub state: ServiceState,
    pub services: BTreeMap<ServiceId, ServiceHealth>,
}

/// Published services are immutable after a successful startup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppServices {
    snapshot: ServiceHealthSnapshot,
}

impl AppServices {
    /// Returns the snapshot captured when services were published.
    #[must_use]
    pub const fn snapshot(&self) -> &ServiceHealthSnapshot {
        &self.snapshot
    }

    /// Looks up one established service's health without exposing a locator.
    #[must_use]
    pub fn health(&self, id: &ServiceId) -> Option<ServiceHealth> {
        self.snapshot.services.get(id).copied()
    }
}

/// The future returned by a service lifecycle hook.
pub type ServiceFuture<'a> = BoxFuture<'a, Result<(), ServiceError>>;

/// Context shared only with the service currently being started or stopped.
#[derive(Clone)]
pub struct ServiceContext {
    cancellation: CancellationToken,
    tasks: ServiceTaskGroup,
}

impl ServiceContext {
    /// Returns this service's descendant cancellation token.
    #[must_use]
    pub fn cancellation(&self) -> CancellationToken {
        self.cancellation.clone()
    }

    /// Returns this service's owned task group.
    #[must_use]
    pub const fn tasks(&self) -> &ServiceTaskGroup {
        &self.tasks
    }
}

/// A product service implemented at a Rust-owned boundary.
pub trait ApplicationService: Send + Sync {
    fn descriptor(&self) -> ServiceDescriptor;
    fn start<'a>(&'a self, context: &'a ServiceContext) -> ServiceFuture<'a>;
    fn stop<'a>(&'a self, context: &'a ServiceContext) -> ServiceFuture<'a>;
}

/// Lifecycle events emitted in deterministic dependency order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LifecycleEvent {
    StateChanged {
        from: ServiceState,
        to: ServiceState,
    },
    ServiceStarting(ServiceId),
    ServiceStarted(ServiceId),
    ServiceDegraded(ServiceId, ServiceErrorKind),
    ServiceStartFailed(ServiceId, ServiceErrorKind),
    ServiceStopped(ServiceId),
    ServiceStopFailed(ServiceId, ServiceHealthFinding),
}

/// Versioned evidence for one lifecycle operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LifecycleReceipt {
    pub receipt_version: u32,
    pub revision: u64,
    pub state: ServiceState,
    pub events: Vec<LifecycleEvent>,
    pub task_drains: BTreeMap<ServiceId, TaskDrainReceipt>,
}

/// Errors found before or during required-service startup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LifecycleError {
    InvalidTransition {
        from: ServiceState,
        operation: &'static str,
    },
    EmptyServiceId,
    DuplicateService(ServiceId),
    DuplicateDependency(ServiceId, ServiceId),
    MissingDependency {
        service: ServiceId,
        dependency: ServiceId,
    },
    DependencyCycle(Vec<ServiceId>),
    RequiredServiceFailed {
        service: ServiceId,
        error: ServiceError,
    },
}

impl fmt::Display for LifecycleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "application lifecycle error: {self:?}")
    }
}

impl std::error::Error for LifecycleError {}

struct RegisteredService {
    descriptor: ServiceDescriptor,
    service: Arc<dyn ApplicationService>,
    context: ServiceContext,
    health: ServiceHealth,
}

/// The application composition-root registry and lifecycle state machine.
pub struct ServiceRegistry {
    state: ServiceState,
    revision: u64,
    root_cancellation: CancellationToken,
    services: BTreeMap<ServiceId, RegisteredService>,
    started: Vec<ServiceId>,
    published: Option<AppServices>,
    last_receipt: Option<LifecycleReceipt>,
}

impl Default for ServiceRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ServiceRegistry {
    /// Creates an empty registry at the `Created` state.
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: ServiceState::Created,
            revision: 0,
            root_cancellation: CancellationToken::new(),
            services: BTreeMap::new(),
            started: Vec::new(),
            published: None,
            last_receipt: None,
        }
    }

    /// Registers one service. Registration is legal only before startup.
    ///
    /// # Errors
    ///
    /// Returns the validation errors reported by [`Self::register_arc`].
    pub fn register<S>(&mut self, service: S) -> Result<(), LifecycleError>
    where
        S: ApplicationService + 'static,
    {
        self.register_arc(Arc::new(service))
    }

    /// Registers a dynamically replaceable service implementation.
    ///
    /// # Errors
    ///
    /// Returns a lifecycle error for an invalid state, empty or duplicate identity,
    /// or duplicate dependency declaration.
    pub fn register_arc(
        &mut self,
        service: Arc<dyn ApplicationService>,
    ) -> Result<(), LifecycleError> {
        if self.state != ServiceState::Created {
            return Err(LifecycleError::InvalidTransition {
                from: self.state,
                operation: "register",
            });
        }
        let descriptor = service.descriptor();
        if descriptor.id.as_str().is_empty() {
            return Err(LifecycleError::EmptyServiceId);
        }
        if self.services.contains_key(&descriptor.id) {
            return Err(LifecycleError::DuplicateService(descriptor.id));
        }
        let mut dependencies = BTreeSet::new();
        for dependency in &descriptor.dependencies {
            if !dependencies.insert(dependency) {
                return Err(LifecycleError::DuplicateDependency(
                    descriptor.id,
                    dependency.clone(),
                ));
            }
        }
        let id = descriptor.id.clone();
        let cancellation = self.root_cancellation.child_token();
        let context = ServiceContext {
            cancellation: cancellation.clone(),
            tasks: ServiceTaskGroup::new(id.as_str(), cancellation, 4),
        };
        self.services.insert(
            id,
            RegisteredService {
                descriptor,
                service,
                context,
                health: ServiceHealth {
                    status: ServiceHealthStatus::Unknown,
                    finding: None,
                },
            },
        );
        Ok(())
    }

    /// Returns the current application state.
    #[must_use]
    pub const fn state(&self) -> ServiceState {
        self.state
    }

    /// Returns the immutable published service view, if startup completed.
    #[must_use]
    pub const fn app_services(&self) -> Option<&AppServices> {
        self.published.as_ref()
    }

    /// Returns the latest lifecycle evidence.
    #[must_use]
    pub const fn last_receipt(&self) -> Option<&LifecycleReceipt> {
        self.last_receipt.as_ref()
    }

    /// Returns a fresh concurrent-reader-friendly health snapshot.
    #[must_use]
    pub fn health_snapshot(&self) -> ServiceHealthSnapshot {
        ServiceHealthSnapshot {
            receipt_version: 1,
            revision: self.revision,
            state: self.state,
            services: self
                .services
                .iter()
                .map(|(id, service)| (id.clone(), service.health))
                .collect(),
        }
    }

    /// Starts services in deterministic topological order.
    ///
    /// # Errors
    ///
    /// Returns a lifecycle error for an invalid state, invalid dependency graph,
    /// or required-service failure after rollback.
    ///
    /// # Panics
    ///
    /// Panics only if the registry's internal validated service map is corrupted.
    pub async fn start(&mut self) -> Result<LifecycleReceipt, LifecycleError> {
        if self.state != ServiceState::Created {
            return Err(LifecycleError::InvalidTransition {
                from: self.state,
                operation: "start",
            });
        }
        let order = self.start_order()?;
        let mut events = Vec::new();
        self.transition(ServiceState::Starting, &mut events)?;
        let mut task_drains = BTreeMap::new();
        let mut degraded = false;

        for id in order {
            let (service, context, descriptor) = self.service_parts(&id);
            self.services
                .get_mut(&id)
                .expect("service parts came from registry")
                .health = ServiceHealth {
                status: ServiceHealthStatus::Starting,
                finding: None,
            };
            events.push(LifecycleEvent::ServiceStarting(id.clone()));
            let result = std::panic::AssertUnwindSafe(service.start(&context))
                .catch_unwind()
                .await;
            let result = match result {
                Ok(result) => result,
                Err(_) => Err(ServiceError::panic()),
            };
            if let Err(error) = result {
                let finding = if error.kind == ServiceErrorKind::Panic {
                    ServiceHealthFinding::StartPanicked
                } else {
                    ServiceHealthFinding::StartFailed
                };
                let criticality = descriptor.criticality;
                let drain = context.tasks.shutdown(descriptor.shutdown_deadline).await;
                task_drains.insert(id.clone(), drain);
                self.services
                    .get_mut(&id)
                    .expect("service parts came from registry")
                    .health = ServiceHealth {
                    status: if criticality == ServiceCriticality::Optional {
                        ServiceHealthStatus::Degraded
                    } else {
                        ServiceHealthStatus::Failed
                    },
                    finding: Some(finding),
                };
                if criticality == ServiceCriticality::Optional {
                    degraded = true;
                    events.push(LifecycleEvent::ServiceDegraded(id, error.kind));
                    continue;
                }
                events.push(LifecycleEvent::ServiceStartFailed(id.clone(), error.kind));
                self.transition(ServiceState::Stopping, &mut events)?;
                self.rollback(&mut events, &mut task_drains).await;
                self.transition(ServiceState::Stopped, &mut events)?;
                self.revision = self.revision.saturating_add(1);
                let receipt = self.receipt(events, task_drains);
                self.last_receipt = Some(receipt);
                return Err(LifecycleError::RequiredServiceFailed { service: id, error });
            }
            self.services
                .get_mut(&id)
                .expect("service parts came from registry")
                .health = ServiceHealth {
                status: ServiceHealthStatus::Ready,
                finding: None,
            };
            self.started.push(id.clone());
            events.push(LifecycleEvent::ServiceStarted(id));
        }

        let final_state = if degraded {
            ServiceState::Degraded
        } else {
            ServiceState::Ready
        };
        self.transition(final_state, &mut events)?;
        self.revision = self.revision.saturating_add(1);
        let receipt = self.receipt(events, task_drains);
        self.published = Some(AppServices {
            snapshot: self.health_snapshot(),
        });
        self.last_receipt = Some(receipt.clone());
        Ok(receipt)
    }

    /// Stops services in reverse startup order. Cleanup continues after errors.
    ///
    /// # Errors
    ///
    /// Returns a lifecycle error when called from a non-stoppable transitional state.
    pub async fn stop(&mut self) -> Result<LifecycleReceipt, LifecycleError> {
        if self.state == ServiceState::Stopped {
            return Ok(self
                .last_receipt
                .clone()
                .unwrap_or_else(|| self.receipt(Vec::new(), BTreeMap::new())));
        }
        if self.state == ServiceState::Created {
            let mut events = Vec::new();
            self.transition(ServiceState::Stopped, &mut events)?;
            self.revision = self.revision.saturating_add(1);
            let receipt = self.receipt(events, BTreeMap::new());
            self.last_receipt = Some(receipt.clone());
            return Ok(receipt);
        }
        if self.state == ServiceState::Starting || self.state == ServiceState::Stopping {
            return Err(LifecycleError::InvalidTransition {
                from: self.state,
                operation: "stop",
            });
        }

        let mut events = Vec::new();
        self.transition(ServiceState::Stopping, &mut events)?;
        self.root_cancellation.cancel(CancellationReason::Shutdown);
        let mut task_drains = BTreeMap::new();
        let started = std::mem::take(&mut self.started);
        for id in started.into_iter().rev() {
            self.stop_one(&id, &mut events, &mut task_drains).await;
        }
        self.transition(ServiceState::Stopped, &mut events)?;
        self.published = None;
        self.revision = self.revision.saturating_add(1);
        let receipt = self.receipt(events, task_drains);
        self.last_receipt = Some(receipt.clone());
        Ok(receipt)
    }

    fn service_parts(
        &self,
        id: &ServiceId,
    ) -> (
        Arc<dyn ApplicationService>,
        ServiceContext,
        ServiceDescriptor,
    ) {
        let service = self.services.get(id).expect("service exists");
        (
            Arc::clone(&service.service),
            service.context.clone(),
            service.descriptor.clone(),
        )
    }

    fn start_order(&self) -> Result<Vec<ServiceId>, LifecycleError> {
        let mut indegree = self
            .services
            .keys()
            .map(|id| (id.clone(), 0usize))
            .collect::<BTreeMap<_, _>>();
        let mut dependents = BTreeMap::<ServiceId, Vec<ServiceId>>::new();
        for (id, service) in &self.services {
            for dependency in &service.descriptor.dependencies {
                if !self.services.contains_key(dependency) {
                    return Err(LifecycleError::MissingDependency {
                        service: id.clone(),
                        dependency: dependency.clone(),
                    });
                }
                *indegree.get_mut(id).expect("service is in indegree") += 1;
                dependents
                    .entry(dependency.clone())
                    .or_default()
                    .push(id.clone());
            }
        }
        let mut ready = indegree
            .iter()
            .filter(|(_, count)| **count == 0)
            .map(|(id, _)| id.clone())
            .collect::<BTreeSet<_>>();
        let mut order = Vec::with_capacity(self.services.len());
        while let Some(id) = ready.pop_first() {
            order.push(id.clone());
            if let Some(children) = dependents.get(&id) {
                for child in children {
                    let count = indegree.get_mut(child).expect("child is in indegree");
                    *count -= 1;
                    if *count == 0 {
                        ready.insert(child.clone());
                    }
                }
            }
        }
        if order.len() != self.services.len() {
            let cycle = indegree
                .into_iter()
                .filter(|(_, count)| *count != 0)
                .map(|(id, _)| id)
                .collect();
            return Err(LifecycleError::DependencyCycle(cycle));
        }
        Ok(order)
    }

    fn transition(
        &mut self,
        next: ServiceState,
        events: &mut Vec<LifecycleEvent>,
    ) -> Result<(), LifecycleError> {
        if !self.state.can_transition_to(next) {
            return Err(LifecycleError::InvalidTransition {
                from: self.state,
                operation: "transition",
            });
        }
        let from = self.state;
        self.state = next;
        events.push(LifecycleEvent::StateChanged { from, to: next });
        Ok(())
    }

    async fn rollback(
        &mut self,
        events: &mut Vec<LifecycleEvent>,
        task_drains: &mut BTreeMap<ServiceId, TaskDrainReceipt>,
    ) {
        let started = std::mem::take(&mut self.started);
        for id in started.into_iter().rev() {
            self.stop_one(&id, events, task_drains).await;
        }
    }

    async fn stop_one(
        &mut self,
        id: &ServiceId,
        events: &mut Vec<LifecycleEvent>,
        task_drains: &mut BTreeMap<ServiceId, TaskDrainReceipt>,
    ) {
        let (service, context, descriptor) = self.service_parts(id);
        context.cancellation.cancel(CancellationReason::Shutdown);
        let deadline = time::Instant::now() + descriptor.shutdown_deadline;
        let result = time::timeout_at(
            deadline,
            std::panic::AssertUnwindSafe(service.stop(&context)).catch_unwind(),
        )
        .await;
        let finding = match result {
            Ok(Ok(Ok(()))) => None,
            Ok(Ok(Err(_)) | Err(_)) => Some(ServiceHealthFinding::StopFailed),
            Err(_) => Some(ServiceHealthFinding::StopTimedOut),
        };
        let remaining = deadline.saturating_duration_since(time::Instant::now());
        let drain = context.tasks.shutdown(remaining).await;
        task_drains.insert(id.clone(), drain);
        let health = self.services.get_mut(id).expect("service exists");
        health.health = ServiceHealth {
            status: ServiceHealthStatus::Stopped,
            finding,
        };
        if let Some(finding) = finding {
            events.push(LifecycleEvent::ServiceStopFailed(id.clone(), finding));
        } else {
            events.push(LifecycleEvent::ServiceStopped(id.clone()));
        }
    }

    fn receipt(
        &self,
        events: Vec<LifecycleEvent>,
        task_drains: BTreeMap<ServiceId, TaskDrainReceipt>,
    ) -> LifecycleReceipt {
        LifecycleReceipt {
            receipt_version: 1,
            revision: self.revision,
            state: self.state,
            events,
            task_drains,
        }
    }
}

/// The result returned by a desktop framework's application runner.
///
/// The lifecycle layer only needs to distinguish successful completion from a
/// framework-owned error. Keeping that error generic lets GTK own its runtime
/// error type without leaking a UI framework into bootstrap behavior.
pub(crate) type ApplicationRunResult<Error> = Result<(), Error>;

pub(crate) trait Recorder {
    fn record(&self, event: &DiagnosticEvent) -> Result<(), DiagnosticsError>;
}

impl Recorder for DiagnosticsGuard {
    fn record(&self, event: &DiagnosticEvent) -> Result<(), DiagnosticsError> {
        DiagnosticsGuard::record(self, event)
    }
}

pub(crate) fn run_with_bootstrap<I, G, A, W, Error>(
    installer: I,
    application: A,
    mut warn: W,
) -> ApplicationRunResult<Error>
where
    I: FnOnce() -> Result<G, DiagnosticsError>,
    G: Recorder,
    A: FnOnce() -> ApplicationRunResult<Error>,
    W: FnMut(&str),
{
    let guard = if let Ok(guard) = installer() {
        Some(guard)
    } else {
        warn("RustTable diagnostics installation failed; continuing");
        None
    };

    if let Some(ref guard) = guard
        && guard.record(&DiagnosticEvent::startup()).is_err()
    {
        warn("RustTable diagnostics startup record failed; continuing");
    }

    let result = application();
    if let Some(ref guard) = guard {
        let event = result.as_ref().map_or(
            DiagnosticEvent::application_failure(
                rusttable_diagnostics::ApplicationFailureCode::DesktopUiRun,
            ),
            |()| DiagnosticEvent::shutdown(),
        );
        if guard.record(&event).is_err() {
            warn("RustTable diagnostics final record failed; continuing");
        }
    }
    result
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod diagnostics_tests {
    use std::cell::RefCell;
    use std::fmt;
    use std::rc::Rc;

    use super::{ApplicationRunResult, Recorder, run_with_bootstrap};
    use rusttable_diagnostics::{ApplicationFailureCode, DiagnosticEvent, DiagnosticsError};

    struct FakeGuard {
        events: Rc<RefCell<Vec<DiagnosticEvent>>>,
        failure: Option<DiagnosticEvent>,
    }

    #[derive(Debug, Eq, PartialEq)]
    enum TestRunError {
        Backend,
    }

    impl fmt::Display for TestRunError {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("test application runner error")
        }
    }

    impl std::error::Error for TestRunError {}

    impl Recorder for FakeGuard {
        fn record(&self, event: &DiagnosticEvent) -> Result<(), DiagnosticsError> {
            self.events.borrow_mut().push(event.clone());
            if self.failure == Some(event.clone()) {
                Err(DiagnosticsError::DirectoryUnavailable)
            } else {
                Ok(())
            }
        }
    }

    fn run(
        install: Result<FakeGuard, DiagnosticsError>,
        app: ApplicationRunResult<TestRunError>,
    ) -> (ApplicationRunResult<TestRunError>, Vec<String>, u8) {
        let installer = || install;
        let warnings = RefCell::new(Vec::new());
        let calls = RefCell::new(0);
        let result = run_with_bootstrap(
            installer,
            || {
                *calls.borrow_mut() += 1;
                app
            },
            |message| warnings.borrow_mut().push(message.to_owned()),
        );
        (result, warnings.into_inner(), *calls.borrow())
    }

    #[test]
    fn install_failure_still_runs_application_once() {
        let (result, warnings, calls) = run(Err(DiagnosticsError::DirectoryUnavailable), Ok(()));
        assert!(result.is_ok());
        assert_eq!(calls, 1);
        assert_eq!(
            warnings,
            ["RustTable diagnostics installation failed; continuing"]
        );
    }

    #[test]
    fn startup_and_shutdown_surround_successful_application() {
        let events = Rc::new(RefCell::new(Vec::new()));
        let guard = FakeGuard {
            events: Rc::clone(&events),
            failure: None,
        };
        let result: ApplicationRunResult<TestRunError> =
            run_with_bootstrap(|| Ok(guard), || Ok(()), |_| {});
        assert!(result.is_ok());
        assert_eq!(
            events.borrow().as_slice(),
            &[DiagnosticEvent::startup(), DiagnosticEvent::shutdown()]
        );
    }

    #[test]
    fn application_failure_is_returned_without_replacement() {
        let events = Rc::new(RefCell::new(Vec::new()));
        let result: ApplicationRunResult<TestRunError> = run_with_bootstrap(
            || {
                Ok(FakeGuard {
                    events: Rc::clone(&events),
                    failure: None,
                })
            },
            || Err(TestRunError::Backend),
            |_| {},
        );
        assert_eq!(result, Err(TestRunError::Backend));
        assert_eq!(
            events.borrow().as_slice(),
            &[
                DiagnosticEvent::startup(),
                DiagnosticEvent::application_failure(ApplicationFailureCode::DesktopUiRun),
            ]
        );
    }

    #[test]
    fn startup_record_failure_does_not_skip_application() {
        let calls = RefCell::new(0);
        let warnings = RefCell::new(0);
        let result: ApplicationRunResult<TestRunError> = run_with_bootstrap(
            || {
                Ok(FakeGuard {
                    events: Rc::new(RefCell::new(Vec::new())),
                    failure: Some(DiagnosticEvent::startup()),
                })
            },
            || {
                *calls.borrow_mut() += 1;
                Ok(())
            },
            |_| *warnings.borrow_mut() += 1,
        );
        assert!(result.is_ok());
        assert_eq!(*calls.borrow(), 1);
        assert_eq!(*warnings.borrow(), 1);
    }

    #[test]
    fn final_record_failure_does_not_replace_application_result() {
        let warnings = RefCell::new(Vec::new());
        let result: ApplicationRunResult<TestRunError> = run_with_bootstrap(
            || {
                Ok(FakeGuard {
                    events: Rc::new(RefCell::new(Vec::new())),
                    failure: Some(DiagnosticEvent::shutdown()),
                })
            },
            || Ok(()),
            |message| warnings.borrow_mut().push(message.to_owned()),
        );
        assert!(result.is_ok());
        assert_eq!(
            warnings.into_inner(),
            ["RustTable diagnostics final record failed; continuing"]
        );
    }
}
