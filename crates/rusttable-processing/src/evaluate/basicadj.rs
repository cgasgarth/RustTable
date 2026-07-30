use std::collections::BTreeMap;

use rusttable_core::OperationId;

/// Immutable resolved automatic plans keyed by authored operation ID.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BasicAdjPlanSet {
    pub(super) plans: BTreeMap<OperationId, crate::operations::basicadj::BasicAdjPlan>,
    pub(super) identity: [u8; 32],
}

impl BasicAdjPlanSet {
    #[must_use]
    pub fn plan(
        &self,
        operation_id: OperationId,
    ) -> Option<&crate::operations::basicadj::BasicAdjPlan> {
        self.plans.get(&operation_id)
    }

    #[must_use]
    pub const fn identity(&self) -> [u8; 32] {
        self.identity
    }
}
