use rusttable_diagnostics::{DiagnosticEvent, DiagnosticsError, DiagnosticsGuard};

pub(crate) trait Recorder {
    fn record(&self, event: DiagnosticEvent) -> Result<(), DiagnosticsError>;
}

impl Recorder for DiagnosticsGuard {
    fn record(&self, event: DiagnosticEvent) -> Result<(), DiagnosticsError> {
        DiagnosticsGuard::record(self, event)
    }
}

pub(crate) fn run_with_bootstrap<I, G, A, W>(
    installer: I,
    application: A,
    mut warn: W,
) -> iced::Result
where
    I: FnOnce() -> Result<G, DiagnosticsError>,
    G: Recorder,
    A: FnOnce() -> iced::Result,
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
                rusttable_diagnostics::ApplicationFailureCode::IcedRun,
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

    use super::{Recorder, run_with_bootstrap};
    use rusttable_diagnostics::{DiagnosticEvent, DiagnosticsError};

    struct FakeGuard {
        events: Rc<RefCell<Vec<DiagnosticEvent>>>,
        failure: Option<DiagnosticEvent>,
    }

    #[derive(Debug)]
    struct TestError;

    impl fmt::Display for TestError {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("test error")
        }
    }

    impl std::error::Error for TestError {}

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
        app: iced::Result,
    ) -> (iced::Result, Vec<DiagnosticEvent>, Vec<String>, u8) {
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
        (result, Vec::new(), warnings.into_inner(), *calls.borrow())
    }

    #[test]
    fn install_failure_still_runs_application_once() {
        let (result, _, warnings, calls) = run(Err(DiagnosticsError::DirectoryUnavailable), Ok(()));
        assert!(result.is_ok());
        assert_eq!(calls, 1);
        assert_eq!(warnings.len(), 1);
    }

    #[test]
    fn startup_and_shutdown_surround_successful_application() {
        let events = Rc::new(RefCell::new(Vec::new()));
        let guard = FakeGuard {
            events,
            failure: None,
        };
        let result = run_with_bootstrap(|| Ok(guard), || Ok(()), |_| {});
        assert!(result.is_ok());
    }

    #[test]
    fn iced_failure_is_returned_without_replacement() {
        let result = run_with_bootstrap(
            || {
                Ok(FakeGuard {
                    events: Rc::new(RefCell::new(Vec::new())),
                    failure: None,
                })
            },
            || Err(iced::Error::WindowCreationFailed(Box::new(TestError))),
            |_| {},
        );
        assert!(result.is_err());
    }

    #[test]
    fn record_failures_do_not_skip_application() {
        let calls = RefCell::new(0);
        let warnings = RefCell::new(0);
        let result = run_with_bootstrap(
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
}
