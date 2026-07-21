//! Unavailable registry definition for imported-history-only CLAHE.

use crate::clahe_compatibility::CLAHE_OPERATION_KEY;
use crate::registry::{
    DefinitionAvailability, ImplementationIdentity, OperationDefinition, REGISTRY_BUILD_ID,
};

/// Publishes the v1 descriptor without an executor until #473 qualifies the CPU backend.
#[must_use]
pub fn clahe_definition() -> OperationDefinition {
    let identity = format!("{REGISTRY_BUILD_ID}.clahe");
    OperationDefinition::new(
        crate::descriptor::clahe_descriptor(),
        None,
        None,
        Vec::new(),
        ImplementationIdentity::new(identity.clone(), 1, identity),
        vec![
            "iop.clahe.params.v1".to_owned(),
            "iop.clahe.deprecated-visibility".to_owned(),
            "iop.clahe.typed-seam".to_owned(),
        ],
    )
    .with_availability(DefinitionAvailability::Unavailable {
        reason: format!(
            "backend qualification is pending #473; {CLAHE_OPERATION_KEY} is read-only"
        ),
    })
}
