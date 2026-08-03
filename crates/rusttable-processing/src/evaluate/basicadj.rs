//! Automatic `BasicAdj` plan publication and identity boundary.
//!
//! Source lineage: `src/iop/basicadj.c` automatic-exposure state.

use std::collections::BTreeMap;

use rusttable_core::OperationId;
use sha2::{Digest, Sha256};

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

    #[must_use]
    pub fn is_published(&self) -> bool {
        self.plans.is_empty() || self.identity != [0; 32]
    }

    /// Builds the immutable publication identity after every automatic plan
    /// has resolved successfully. A non-empty set never publishes a zero
    /// identity, so cancellation or an earlier operation error cannot be
    /// mistaken for reusable analysis.
    pub(super) fn published(
        plans: BTreeMap<OperationId, crate::operations::basicadj::BasicAdjPlan>,
    ) -> Self {
        let identity = if plans.is_empty() {
            [0; 32]
        } else {
            let mut hasher = Sha256::new();
            hasher.update(b"rusttable.basicadj.plan-set.v2");
            for (operation_id, plan) in &plans {
                hasher.update(operation_id.get().to_le_bytes());
                hasher.update(plan.identity());
            }
            hasher.finalize().into()
        };
        Self { plans, identity }
    }

    pub(super) const fn transient(
        plans: BTreeMap<OperationId, crate::operations::basicadj::BasicAdjPlan>,
    ) -> Self {
        Self {
            plans,
            identity: [0; 32],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn automatic_plan_set_publishes_only_a_nonempty_immutable_identity() {
        let operation_id = OperationId::new(1).expect("operation ID");
        let plan = crate::BasicAdjPlan::new(crate::BasicAdjConfig::defaults()).expect("plan");
        let plans = std::iter::once((operation_id, plan.clone())).collect();
        let transient = BasicAdjPlanSet::transient(plans);
        assert!(!transient.is_published());
        assert_eq!(transient.identity(), [0; 32]);

        let plans = std::iter::once((operation_id, plan)).collect();
        let published = BasicAdjPlanSet::published(plans);
        assert!(published.is_published());
        assert_ne!(published.identity(), [0; 32]);
        assert!(published.plan(operation_id).is_some());
    }
}
