use rusttable_core::numerics::{CompilerFingerprint, NUMERICS_SCHEMA};
use serde::Serialize;

use crate::Result;

const MAX_CHECKS: usize = 64;
const MAX_DETAIL_BYTES: usize = 512;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub(super) enum CheckStatus {
    Passed,
    Blocking,
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(super) struct CheckReceipt {
    pub id: String,
    pub status: CheckStatus,
    pub detail: String,
}

impl CheckReceipt {
    pub fn passed(id: impl Into<String>, detail: impl Into<String>) -> Self {
        Self::new(id, CheckStatus::Passed, detail)
    }

    pub fn blocking(id: impl Into<String>, detail: impl Into<String>) -> Self {
        Self::new(id, CheckStatus::Blocking, detail)
    }

    pub fn unsupported(id: impl Into<String>, detail: impl Into<String>) -> Self {
        Self::new(id, CheckStatus::Unsupported, detail)
    }

    fn new(id: impl Into<String>, status: CheckStatus, detail: impl Into<String>) -> Self {
        let mut detail = detail.into();
        if detail.len() > MAX_DETAIL_BYTES {
            detail.truncate(MAX_DETAIL_BYTES);
        }
        Self {
            id: id.into(),
            status,
            detail,
        }
    }
}

#[derive(Debug, Serialize)]
pub(super) struct NumericsReceipt {
    schema: &'static str,
    kind: &'static str,
    status: CheckStatus,
    checks: Vec<CheckReceipt>,
    active_compiler: Option<CompilerFingerprint>,
    comparison_compilers: Vec<(String, CompilerFingerprint)>,
}

impl NumericsReceipt {
    pub fn verification(
        checks: Vec<CheckReceipt>,
        active_compiler: Option<CompilerFingerprint>,
    ) -> Result<Self> {
        Self::new("verification", checks, active_compiler, Vec::new())
    }

    pub fn comparison(
        checks: Vec<CheckReceipt>,
        comparison_compilers: Vec<(String, CompilerFingerprint)>,
    ) -> Result<Self> {
        Self::new("compiler-comparison", checks, None, comparison_compilers)
    }

    fn new(
        kind: &'static str,
        checks: Vec<CheckReceipt>,
        active_compiler: Option<CompilerFingerprint>,
        comparison_compilers: Vec<(String, CompilerFingerprint)>,
    ) -> Result<Self> {
        if checks.len() > MAX_CHECKS {
            return Err("numerics receipt exceeded its bounded check count".to_owned());
        }
        let status = if checks
            .iter()
            .any(|check| check.status == CheckStatus::Blocking)
        {
            CheckStatus::Blocking
        } else if checks
            .iter()
            .any(|check| check.status == CheckStatus::Unsupported)
        {
            CheckStatus::Unsupported
        } else {
            CheckStatus::Passed
        };
        Ok(Self {
            schema: NUMERICS_SCHEMA,
            kind,
            status,
            checks,
            active_compiler,
            comparison_compilers,
        })
    }

    pub fn emit(&self) -> Result {
        let json = serde_json::to_string(self)
            .map_err(|error| format!("numerics receipt serialization failed: {error}"))?;
        println!("{json}");
        Ok(())
    }

    pub fn blocking_result(&self, label: &str) -> Result {
        let blockers = self
            .checks
            .iter()
            .filter(|check| check.status == CheckStatus::Blocking)
            .map(|check| check.id.as_str())
            .collect::<Vec<_>>();
        if blockers.is_empty() {
            Ok(())
        } else {
            Err(format!(
                "{label}: blocking findings: {}",
                blockers.join(", ")
            ))
        }
    }
}
