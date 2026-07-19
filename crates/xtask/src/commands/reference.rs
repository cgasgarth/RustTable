use super::{Result, report};
use crate::cli::ReferenceCommand;
use crate::root::RepositoryRoot;
use rusttable_testkit::reference::{
    ColorProfile, Dimensions, ExecutionMode, OutputFormat, ReferenceIdentityOverrides,
    ReferenceLimits, ReferenceRequest, ReferenceRunner, resolve_reference,
};

pub(super) fn run(
    root: &RepositoryRoot,
    command: &ReferenceCommand,
    _runner: &crate::process::ProcessRunner,
) -> Result {
    let (name, arguments) = match command {
        ReferenceCommand::Probe(arguments) => ("reference.probe", arguments),
        ReferenceCommand::Render(arguments) => ("reference.render", arguments),
    };
    let identity_path = root.join(&arguments.identity);
    let identity = resolve_reference(
        &identity_path,
        &ReferenceIdentityOverrides {
            source_path: arguments.source.as_ref().map(|path| root.join(path)),
            executable_path: arguments.executable.as_ref().map(|path| root.join(path)),
            data_dir: arguments.data_dir.as_ref().map(|path| root.join(path)),
        },
    )
    .map_err(|error| error.to_string())?;
    if matches!(command, ReferenceCommand::Probe(_)) {
        return Ok(report(
            root,
            name,
            serde_json::json!({ "identity": identity.receipt() }),
        ));
    }
    let input = arguments
        .input
        .as_ref()
        .map(|path| root.join(path))
        .ok_or_else(|| "reference render requires --input".to_owned())?;
    let request = ReferenceRequest {
        source_fixture_id: arguments.fixture_id.clone(),
        source_path: input,
        xmp_path: arguments.xmp.as_ref().map(|path| root.join(path)),
        config_path: None,
        output_format: OutputFormat::Png,
        output_profile: ColorProfile::Srgb,
        dimensions: Dimensions {
            width: arguments.width,
            height: arguments.height,
        },
        timeout_ms: 30_000,
        execution_mode: if arguments.gpu {
            ExecutionMode::Gpu
        } else {
            ExecutionMode::Cpu
        },
    };
    let receipt = ReferenceRunner::new(identity, ReferenceLimits::default())
        .run(&request)
        .map_err(|error| error.to_string())?;
    Ok(report(
        root,
        name,
        serde_json::json!({ "receipt": receipt }),
    ))
}
