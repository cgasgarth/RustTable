#[path = "anchors.rs"]
mod anchors;
#[path = "audit.rs"]
mod audit;
#[path = "operations.rs"]
mod operations;
#[path = "policy.rs"]
mod policy;

pub(crate) use operations::run;
