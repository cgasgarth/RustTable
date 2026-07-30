use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::{
    ApplicationRunResult, ApplicationService, Recorder, ServiceContext, ServiceCriticality,
    ServiceDescriptor, ServiceError, ServiceFuture, ServiceHealthStatus, ServiceId,
    ServiceRegistry, ServiceState, run_with_bootstrap,
};
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
        |_| {
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
        run_with_bootstrap(|| Ok(guard), |_| Ok(()), |_| {});
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
        |_| Err(TestRunError::Backend),
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
        |_| {
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
        |_| Ok(()),
        |message| warnings.borrow_mut().push(message.to_owned()),
    );
    assert!(result.is_ok());
    assert_eq!(
        warnings.into_inner(),
        ["RustTable diagnostics final record failed; continuing"]
    );
}

struct FakeService {
    descriptor: ServiceDescriptor,
    log: Arc<Mutex<Vec<String>>>,
    start_result: Result<(), ServiceError>,
    stop_result: Result<(), ServiceError>,
    stop_pending: bool,
}

impl ApplicationService for FakeService {
    fn descriptor(&self) -> ServiceDescriptor {
        self.descriptor.clone()
    }

    fn start<'a>(&'a self, _context: &'a ServiceContext) -> ServiceFuture<'a> {
        let log = Arc::clone(&self.log);
        let id = self.descriptor.id.clone();
        let result = self.start_result;
        Box::pin(async move {
            log.lock()
                .expect("fake log lock")
                .push(format!("start:{id}"));
            result
        })
    }

    fn stop<'a>(&'a self, _context: &'a ServiceContext) -> ServiceFuture<'a> {
        let log = Arc::clone(&self.log);
        let id = self.descriptor.id.clone();
        let stop_result = self.stop_result;
        let stop_pending = self.stop_pending;
        Box::pin(async move {
            log.lock()
                .expect("fake log lock")
                .push(format!("stop:{id}"));
            if stop_pending {
                std::future::pending::<()>().await;
            }
            stop_result
        })
    }
}

fn fake(
    id: &str,
    dependencies: &[&str],
    criticality: ServiceCriticality,
    log: &Arc<Mutex<Vec<String>>>,
    start_result: Result<(), ServiceError>,
) -> FakeService {
    FakeService {
        descriptor: ServiceDescriptor::new(id, criticality).with_dependencies(
            dependencies
                .iter()
                .copied()
                .map(ServiceId::from)
                .collect::<Vec<_>>(),
        ),
        log: Arc::clone(log),
        start_result,
        stop_result: Ok(()),
        stop_pending: false,
    }
}

#[tokio::test]
async fn startup_is_topological_and_shutdown_is_reverse_order() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let mut registry = ServiceRegistry::new();
    registry
        .register(fake(
            "catalog",
            &["diagnostics"],
            ServiceCriticality::Required,
            &log,
            Ok(()),
        ))
        .expect("register catalog");
    registry
        .register(fake(
            "ui",
            &["catalog"],
            ServiceCriticality::Required,
            &log,
            Ok(()),
        ))
        .expect("register ui");
    registry
        .register(fake(
            "diagnostics",
            &[],
            ServiceCriticality::Required,
            &log,
            Ok(()),
        ))
        .expect("register diagnostics");

    let start = registry.start().await.expect("start");
    assert_eq!(registry.state(), ServiceState::Ready);
    assert_eq!(
        start.events,
        vec![
            super::LifecycleEvent::StateChanged {
                from: ServiceState::Created,
                to: ServiceState::Starting,
            },
            super::LifecycleEvent::ServiceStarting(ServiceId::from("diagnostics")),
            super::LifecycleEvent::ServiceStarted(ServiceId::from("diagnostics")),
            super::LifecycleEvent::ServiceStarting(ServiceId::from("catalog")),
            super::LifecycleEvent::ServiceStarted(ServiceId::from("catalog")),
            super::LifecycleEvent::ServiceStarting(ServiceId::from("ui")),
            super::LifecycleEvent::ServiceStarted(ServiceId::from("ui")),
            super::LifecycleEvent::StateChanged {
                from: ServiceState::Starting,
                to: ServiceState::Ready,
            },
        ]
    );
    registry.stop().await.expect("stop");
    assert_eq!(
        *log.lock().expect("fake log lock"),
        [
            "start:diagnostics",
            "start:catalog",
            "start:ui",
            "stop:ui",
            "stop:catalog",
            "stop:diagnostics",
        ]
    );
}

#[tokio::test]
async fn required_failure_rolls_back_started_services_and_never_publishes_ready() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let mut registry = ServiceRegistry::new();
    registry
        .register(fake(
            "diagnostics",
            &[],
            ServiceCriticality::Required,
            &log,
            Ok(()),
        ))
        .expect("register diagnostics");
    registry
        .register(fake(
            "catalog",
            &["diagnostics"],
            ServiceCriticality::Required,
            &log,
            Err(ServiceError::failed()),
        ))
        .expect("register catalog");

    let error = registry.start().await.expect_err("required failure");
    assert!(matches!(
        error,
        super::LifecycleError::RequiredServiceFailed { service, .. }
            if service == ServiceId::from("catalog")
    ));
    assert_eq!(registry.state(), ServiceState::Stopped);
    assert!(registry.app_services().is_none());
    assert_eq!(
        *log.lock().expect("fake log lock"),
        ["start:diagnostics", "start:catalog", "stop:diagnostics"]
    );
    let events = registry
        .last_receipt()
        .expect("rollback receipt")
        .events
        .as_slice();
    assert!(events.contains(&super::LifecycleEvent::StateChanged {
        from: ServiceState::Starting,
        to: ServiceState::Stopping,
    }));
}

#[tokio::test]
async fn optional_failure_publishes_degraded_cpu_capable_services() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let mut registry = ServiceRegistry::new();
    registry
        .register(fake(
            "catalog",
            &[],
            ServiceCriticality::Required,
            &log,
            Ok(()),
        ))
        .expect("register catalog");
    registry
        .register(fake(
            "gpu",
            &["catalog"],
            ServiceCriticality::Optional,
            &log,
            Err(ServiceError::failed()),
        ))
        .expect("register gpu");
    registry
        .register(fake(
            "render",
            &["catalog", "gpu"],
            ServiceCriticality::Required,
            &log,
            Ok(()),
        ))
        .expect("register render");

    registry.start().await.expect("degraded startup");
    assert_eq!(registry.state(), ServiceState::Degraded);
    let services = registry.app_services().expect("published services");
    assert_eq!(
        services
            .health(&ServiceId::from("gpu"))
            .expect("gpu health")
            .status,
        ServiceHealthStatus::Degraded
    );
    assert_eq!(
        services
            .health(&ServiceId::from("render"))
            .expect("render health")
            .status,
        ServiceHealthStatus::Ready
    );
}

#[tokio::test]
async fn cleanup_failures_are_recorded_without_skipping_reverse_cleanup() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let mut registry = ServiceRegistry::new();
    registry
        .register(fake(
            "first",
            &[],
            ServiceCriticality::Required,
            &log,
            Ok(()),
        ))
        .expect("register first");
    registry
        .register(FakeService {
            descriptor: ServiceDescriptor::new("second", ServiceCriticality::Required)
                .with_dependencies([ServiceId::from("first")]),
            log: Arc::clone(&log),
            start_result: Ok(()),
            stop_result: Err(ServiceError::failed()),
            stop_pending: false,
        })
        .expect("register second");
    registry.start().await.expect("start");
    let receipt = registry.stop().await.expect("stop continues");
    assert!(
        receipt
            .events
            .contains(&super::LifecycleEvent::ServiceStopFailed(
                ServiceId::from("second"),
                super::ServiceHealthFinding::StopFailed,
            ))
    );
    assert_eq!(
        *log.lock().expect("fake log lock"),
        ["start:first", "start:second", "stop:second", "stop:first"]
    );
}

#[tokio::test]
async fn service_stop_and_owned_task_drain_share_one_deadline() {
    let log = Arc::new(Mutex::new(Vec::new()));
    let mut registry = ServiceRegistry::new();
    registry
        .register(FakeService {
            descriptor: ServiceDescriptor::new("worker", ServiceCriticality::Required)
                .with_shutdown_deadline(Duration::from_millis(1)),
            log: Arc::clone(&log),
            start_result: Ok(()),
            stop_result: Ok(()),
            stop_pending: false,
        })
        .expect("register worker");
    registry.start().await.expect("start");
    let receipt = registry.stop().await.expect("stop");
    assert_eq!(
        receipt.task_drains[&ServiceId::from("worker")].forced_aborts,
        0
    );
}

#[tokio::test]
async fn cancellation_is_hierarchical_and_task_groups_reject_work_after_stop() {
    let root = super::CancellationToken::new();
    let child = root.child_token();
    let grandchild = child.child_token();
    root.cancel(super::CancellationReason::Requested);
    assert!(child.is_cancelled());
    assert_eq!(
        grandchild.reason(),
        Some(super::CancellationReason::Requested)
    );

    let group = super::tasks::ServiceTaskGroup::new("test", super::CancellationToken::new(), 1);
    group.shutdown(Duration::ZERO).await;
    assert_eq!(
        group.spawn(async {}).await,
        Err(super::TaskGroupError::Stopped)
    );
}
