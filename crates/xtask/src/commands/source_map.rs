//! Pinned darktable source inventory and migration ownership tooling.

mod command;
mod git_inventory;
mod io;
mod model;

pub(super) const PINNED_COMMIT: &str = "cfe57f3bbf5269bfacf31e832267279caa6938ad";
pub(super) const REPOSITORY: &str = "darktable-org/darktable";
pub(super) const INVENTORY_SCHEMA: u32 = 2;
pub(super) const ISSUE_MAP_SCHEMA: &str = "rusttable.darktable-source-map.v1";

pub(crate) use command::run;
