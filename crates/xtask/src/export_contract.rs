use std::fs;
use std::path::Path;

use rusttable_export::EXPORT_CONTRACT_SCHEMA;
use serde_json::json;

use crate::Result;

const CONTRACT_PATH: &str = "architecture/rusttable-export-contract.json";

pub(crate) fn run(root: &Path, check: bool) -> Result {
    let path = root.join(CONTRACT_PATH);
    let expected = serde_json::to_string_pretty(&json!({
        "schema": EXPORT_CONTRACT_SCHEMA,
        "version": 1,
        "request": [
            "photo_id", "edit_id", "edit_revision", "style_hash", "quality", "size",
            "interpolation", "output_profile", "rendering_intent", "black_point_compensation",
            "pixel_encoding", "alpha_policy", "dither_policy", "metadata_policy",
            "encoder_settings", "destination", "dependency_snapshot", "priority"
        ],
        "artifact": [
            "buffer", "metadata_packet", "filename_context", "content_hash", "dependency_hash",
            "request_hash", "render_receipt"
        ],
        "publish_rule": "artifact must be complete and validated before publication"
    }))
    .map_err(|error| format!("export contract: serialize schema: {error}"))?
        + "\n";
    if check {
        let actual = fs::read_to_string(&path)
            .map_err(|error| format!("export contract: read {}: {error}", path.display()))?;
        if actual != expected {
            return Err(format!("export contract: {} is stale", path.display()));
        }
    } else {
        fs::write(&path, expected)
            .map_err(|error| format!("export contract: write {}: {error}", path.display()))?;
    }
    Ok(())
}
