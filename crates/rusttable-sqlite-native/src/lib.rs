#![forbid(unsafe_code)]
#![doc = "Typed rows and explicit query plans at the Darktable SQLite boundary."]

mod organization;
mod schema;

pub use organization::{
    OrganizationRows, RawColorLabelRow, RawImageRow, RawTagAssignmentRow, RawTagRow,
};
pub use schema::{
    CURRENT_DATA_SCHEMA, CURRENT_LIBRARY_SCHEMA, DarktableSchema, OrganizationQueryPlans,
    QueryPlan, SchemaError,
};
