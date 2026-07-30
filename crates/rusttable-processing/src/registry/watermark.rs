//! Registry identity for the managed watermark backend seam.

use super::{
    DefinitionAvailability, ImplementationIdentity, MigrationBinding, OperationDefinition,
};
use crate::descriptor::watermark_descriptor;

#[must_use]
pub fn watermark_definition() -> OperationDefinition {
    let descriptor = watermark_descriptor();
    OperationDefinition::new(
        descriptor,
        None,
        None,
        (1..7)
            .map(|version| {
                MigrationBinding::new(
                    version,
                    version + 1,
                    format!("watermark.migration.v{version}-v{}", version + 1),
                )
            })
            .collect(),
        ImplementationIdentity::new(
            "rusttable-processing.watermark",
            crate::operations::watermark::WATERMARK_IMPLEMENTATION_VERSION,
            "rusttable-processing.watermark.managed-svg.v1",
        ),
        vec![
            "iop.watermark.descriptor".to_owned(),
            "iop.watermark.managed-svg".to_owned(),
            "iop.watermark.context-expansion".to_owned(),
            "iop.watermark.cpu-composite".to_owned(),
        ],
    )
    .with_availability(DefinitionAvailability::Unavailable {
        reason: "generic operation payload lacks a managed asset/context port; use WatermarkPlan until that seam lands".to_owned(),
    })
}
