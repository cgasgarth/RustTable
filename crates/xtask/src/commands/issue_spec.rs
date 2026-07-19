//! Semantic validation for the rusttable.issue-spec.v2 issue-body contract.

mod checks;
mod commands;
mod extract;
mod helpers;
mod types;
mod validate;

pub(crate) use commands::run;
#[allow(unused_imports)]
pub use extract::{
    canonical_body_hash, canonical_spec_hash, extract_sections, normalize_text,
    normalized_body_fingerprint, parse_dependencies,
};
#[cfg(test)]
mod tests;
