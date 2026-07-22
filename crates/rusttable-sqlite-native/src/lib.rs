#![forbid(unsafe_code)]
#![doc = "Typed rows and explicit query plans at the Darktable SQLite boundary."]

mod history;
mod organization;
mod schema;

pub use history::{
    HistoryRows, RawHistoryHashRow, RawHistoryRow, RawImageHistoryRow, RawModuleOrderRow,
};
pub use organization::{
    OrganizationRows, RawColorLabelRow, RawImageRow, RawTagAssignmentRow, RawTagRow,
};
pub use schema::{
    CURRENT_DATA_SCHEMA, CURRENT_LIBRARY_SCHEMA, DarktableSchema, HistoryQueryPlans,
    OrganizationQueryPlans, QueryPlan, SchemaError,
};
