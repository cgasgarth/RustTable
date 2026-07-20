use rusttable_diagnostics::{DiagnosticEvent, DiagnosticsError, DiagnosticsGuard};

/// The result returned by a desktop framework's application runner.
///
/// The lifecycle layer only needs to distinguish successful completion from a
/// framework-owned error. Keeping that error generic lets GTK own its runtime
/// error type without leaking a UI framework into bootstrap behavior.
pub(crate) type ApplicationRunResult<Error> = Result<(), Error>;

pub(crate) trait Recorder {
    fn record(&self, event: DiagnosticEvent) -> Result<(), DiagnosticsError>;
}

impl Recorder for DiagnosticsGuard {
    fn record(&self, event: DiagnosticEvent) -> Result<(), DiagnosticsError> {
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
        && guard.record(DiagnosticEvent::Startup).is_err()
    {
        warn("RustTable diagnostics startup record failed; continuing");
    }

    let result = application();
    if let Some(ref guard) = guard {
        let event = result.as_ref().map_or(
            DiagnosticEvent::ApplicationFailure(
                rusttable_diagnostics::ApplicationFailureCode::DesktopUiRun,
            ),
            |()| DiagnosticEvent::Shutdown,
        );
        if guard.record(event).is_err() {
            warn("RustTable diagnostics final record failed; continuing");
        }
    }
    result
}

#[cfg(test)]
mod tests {
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
        fn record(&self, event: DiagnosticEvent) -> Result<(), DiagnosticsError> {
            self.events.borrow_mut().push(event);
            if self.failure == Some(event) {
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
            &[DiagnosticEvent::Startup, DiagnosticEvent::Shutdown]
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
                DiagnosticEvent::Startup,
                DiagnosticEvent::ApplicationFailure(ApplicationFailureCode::DesktopUiRun),
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
                    failure: Some(DiagnosticEvent::Startup),
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
                    failure: Some(DiagnosticEvent::Shutdown),
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
