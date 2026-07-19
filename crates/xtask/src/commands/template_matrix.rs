use rusttable_core::template::{
    BuiltinTemplate, EncoderDescriptor, Template, TemplateContext, TemplateDateTime, TemplateValue,
    VariableId, VariableValue,
};
use serde_json::json;

use crate::cli::TemplateMatrixArgs;
use crate::commands::{Result, report};
use crate::root::RepositoryRoot;

pub(crate) fn run(root: &RepositoryRoot, args: &TemplateMatrixArgs) -> Result {
    if args.repeat == 0 {
        return Err("template matrix repeat count must be positive".to_owned());
    }
    let mut context = TemplateContext::new();
    context.set_text(VariableId::SourceStem, "matrix-photo");
    context.set_text(VariableId::Recipe, "recipe");
    context.set_integer(VariableId::VirtualCopy, 1);
    context.set_integer(VariableId::Sequence, 12);
    context.set_integer(VariableId::CaptureYear, 2026);
    context.set_integer(VariableId::CaptureMonth, 7);
    context.set_integer(VariableId::CaptureDay, 18);
    context.set(
        VariableId::CaptureDate,
        VariableValue::available(TemplateValue::DateTime(
            TemplateDateTime::new(2026, 7, 18, 12, 0, 0, 0).map_err(str::to_owned)?,
        )),
    );
    context.set_text(VariableId::Title, "private title");
    let encoder = EncoderDescriptor::new("jpeg", "jpg", &["jpg", "jpeg"]);
    let builtins = if args.all_builtins {
        BuiltinTemplate::all().to_vec()
    } else {
        vec![BuiltinTemplate::SourceStem]
    };
    let mut rows = Vec::new();
    for builtin in builtins {
        let template = builtin.template().map_err(|error| error.to_string())?;
        let mut hashes = Vec::new();
        for _ in 0..args.repeat {
            let (_, receipt) = template
                .evaluate(&context, Some(&encoder))
                .map_err(|error| error.to_string())?;
            hashes.push(receipt.receipt_hash());
        }
        let stable = hashes.windows(2).all(|pair| pair[0] == pair[1]);
        if !stable {
            return Err(format!("builtin {} was not deterministic", builtin.name()));
        }
        rows.push(json!({
            "name": builtin.name(),
            "ast_hash": template.ast_hash(),
            "content_hash": builtin.content_hash(),
            "evaluation_hash": hashes.first(),
            "stable": stable,
        }));
    }
    let privacy = if args.verify_privacy {
        let template = Template::parse("${title}").map_err(|error| error.to_string())?;
        let (_, receipt) = template
            .evaluate(&context, None)
            .map_err(|error| error.to_string())?;
        receipt.privacy_redacted && receipt.display_path == "[redacted]"
    } else {
        false
    };
    if args.verify_privacy && !privacy {
        return Err("privacy receipt verification failed".to_owned());
    }
    Ok(report(
        root,
        "template-matrix",
        json!({
            "builtins": rows,
            "platforms": if args.all_platforms { json!(["linux", "macos", "windows"]) } else { json!([]) },
            "privacy_verified": privacy,
            "repeat": args.repeat,
            "receipts": "component-hashes-and-redacted-display-only",
        }),
    ))
}
