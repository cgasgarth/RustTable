mod identity;
mod resolution;
mod runner;
mod schema;

pub use identity::{CapabilityProbe, CliReference, ReferenceIdentity, ReferencePin};
pub use resolution::{
    ReferenceIdentityDocument, ReferenceIdentityOverrides, ReferenceProbeError, resolve_reference,
    verify_reference_unchanged,
};
pub use runner::{ReferenceArtifacts, ReferenceError, ReferenceRun, ReferenceRunner};
pub use schema::{
    CancellationToken, ColorProfile, Dimensions, ExecutionMode, ExitStatus, OutputFormat,
    ReferenceIdentityReceipt, ReferenceLimits, ReferenceReceipt, ReferenceRequest, ReferenceStatus,
    ResourceLimits,
};

use std::path::{Path, PathBuf};

/// Builds the pinned CLI's documented positional and option ordering.
#[must_use]
pub fn cli_arguments(
    identity: &ReferenceIdentity,
    request: &ReferenceRequest,
    output: &Path,
) -> Vec<String> {
    let mut arguments = vec![request.source_path.display().to_string()];
    if let Some(xmp) = &request.xmp_path {
        arguments.push(xmp.display().to_string());
    }
    arguments.push(output.display().to_string());
    arguments.extend([
        "--width".to_owned(),
        request.dimensions.width.to_string(),
        "--height".to_owned(),
        request.dimensions.height.to_string(),
    ]);
    arguments.push(identity.cli.core_prefix.clone());
    arguments.extend(
        identity
            .required_flags
            .iter()
            .filter_map(|flag| {
                if matches!(flag.as_str(), "--width" | "--height") {
                    return None;
                }
                let value = match flag.as_str() {
                    "--configdir" => Some(PathBuf::from("<configdir>")),
                    "--cachedir" => Some(PathBuf::from("<cachedir>")),
                    "--datadir" => Some(identity.data_dir.clone()),
                    "--library" => Some(PathBuf::from("<library>")),
                    "--icc-type" | "--icc" => {
                        Some(PathBuf::from(request.output_profile.cli_name()))
                    }
                    "--out-ext" => Some(PathBuf::from(request.output_format.cli_name())),
                    _ => None,
                };
                Some((flag.clone(), value))
            })
            .flat_map(|(flag, value)| {
                std::iter::once(flag)
                    .chain(value.into_iter().map(|path| path.display().to_string()))
            }),
    );
    arguments
}
