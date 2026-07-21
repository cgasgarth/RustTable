//! Unavailable registry definition for the imported-history-only defringe seam.

use crate::defringe_compatibility::DEFRINGE_OPERATION_KEY;
use crate::registry::{
    DefinitionAvailability, ImplementationIdentity, OperationDefinition, REGISTRY_BUILD_ID,
};

/// Publishes the v1 defringe descriptor without an executor until #475 lands.
#[must_use]
pub fn defringe_definition() -> OperationDefinition {
    let identity = format!("{REGISTRY_BUILD_ID}.defringe");
    OperationDefinition::new(
        crate::descriptor::defringe_descriptor(),
        None,
        None,
        Vec::new(),
        ImplementationIdentity::new(identity.clone(), 1, identity),
        vec![
            "iop.defringe.params.v1".to_owned(),
            "iop.defringe.deprecated-visibility".to_owned(),
            "iop.defringe.typed-seam".to_owned(),
        ],
    )
    .with_availability(DefinitionAvailability::Unavailable {
        reason: format!(
            "backend qualification is pending #475; {DEFRINGE_OPERATION_KEY} is read-only"
        ),
    })
}
