//! Registry seam for censorize while #477's execution capability is unqualified.

use super::{
    DefinitionAvailability, ImplementationIdentity, OperationDefinition, REGISTRY_BUILD_ID,
};
use crate::descriptor::censorize_descriptor;

/// Projects censorize's v1 parameter contract before #477 supplies execution.
///
/// The exact integration point for #477 is this factory: replace the `None`
/// CPU binding and `Unavailable` status with the qualified censorize prepare
/// function and its capability evidence. The descriptor and persisted
/// parameter names remain unchanged so this UI seam needs no second schema.
#[must_use]
pub fn censorize_definition() -> OperationDefinition {
    OperationDefinition::new(
        censorize_descriptor(),
        None,
        None,
        Vec::new(),
        ImplementationIdentity::new(REGISTRY_BUILD_ID, 1, REGISTRY_BUILD_ID),
        vec![
            "iop.censorize.descriptor".to_owned(),
            "iop.censorize.unqualified".to_owned(),
        ],
    )
    .with_availability(DefinitionAvailability::Unavailable {
        reason:
            "censorize backend is unqualified until #477 supplies the CPU/mask pipeline capability"
                .to_owned(),
    })
}
