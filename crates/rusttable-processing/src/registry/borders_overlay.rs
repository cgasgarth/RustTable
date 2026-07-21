//! Registry identities for the borders and managed raster overlay backends.

use super::{
    DefinitionAvailability, ImplementationIdentity, MigrationBinding, OperationDefinition,
};
use crate::descriptor::{borders_descriptor, overlay_descriptor};

#[must_use]
pub fn borders_definition() -> OperationDefinition {
    let descriptor = borders_descriptor();
    OperationDefinition::new(
        descriptor,
        None,
        None,
        (1..4)
            .map(|version| {
                MigrationBinding::new(
                    version,
                    version + 1,
                    format!("borders.migration.v{version}-v{}", version + 1),
                )
            })
            .collect(),
        ImplementationIdentity::new(
            "rusttable-processing.borders",
            crate::operations::borders::BORDERS_IMPLEMENTATION_VERSION,
            "rusttable-processing.borders.frame-line.v1",
        ),
        vec![
            "iop.borders.descriptor".to_owned(),
            "iop.borders.geometry".to_owned(),
            "iop.borders.frame-line".to_owned(),
        ],
    )
    .with_availability(DefinitionAvailability::Unavailable {
        reason: "generic operation payload lacks a geometry-output port; use BordersPlan until that seam lands".to_owned(),
    })
}

#[must_use]
pub fn overlay_definition() -> OperationDefinition {
    let descriptor = overlay_descriptor();
    OperationDefinition::new(
        descriptor,
        None,
        None,
        Vec::new(),
        ImplementationIdentity::new(
            "rusttable-processing.overlay",
            crate::operations::overlay::OVERLAY_IMPLEMENTATION_VERSION,
            "rusttable-processing.overlay.managed-raster.v1",
        ),
        vec![
            "iop.overlay.descriptor".to_owned(),
            "iop.overlay.managed-raster".to_owned(),
            "iop.overlay.cpu-composite".to_owned(),
            "iop.overlay.wgpu-composite".to_owned(),
        ],
    )
    .with_availability(DefinitionAvailability::Unavailable {
        reason: "generic operation payload lacks a managed asset/context port; use OverlayPlan until that seam lands".to_owned(),
    })
}
