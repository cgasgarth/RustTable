#![allow(clippy::missing_errors_doc, clippy::too_many_lines)]

use rusttable_color::{BlackPointCompensation, ColorEncoding};
use rusttable_core::RenderSizeRequest;
use serde_json::Value;

use crate::recipe::{ExportRecipe, format_from_encoder};
use crate::{
    AlphaPolicy, DitherPolicy, EncoderSettings, Interpolation, MetadataAction, MetadataPolicy,
    OutputProfileSpec, PipelineQuality, PixelEncoding, PostSuccessAction, RecipeDestination,
    RecipeError, RecipeId, RecipeRevision, RecipeTemplate,
};

pub(crate) fn parse_recipe(value: &Value) -> Result<ExportRecipe, RecipeError> {
    let id = RecipeId::new(string(value, "id")?)?;
    let revision = RecipeRevision::new(
        value
            .get("revision")
            .and_then(Value::as_u64)
            .ok_or(RecipeError::MalformedJson)?,
    )
    .ok_or(RecipeError::MalformedJson)?;
    let encoder_id = string(value, "encoder_id")?;
    let settings = value
        .get("encoder_settings")
        .ok_or(RecipeError::MalformedJson)?;
    let format = format_from_encoder(&string(settings, "format")?)
        .ok_or(RecipeError::UnsupportedValue { field: "format" })?;
    let mut encoder_settings = EncoderSettings::new(format);
    if let Some(parameters) = settings.get("parameters").and_then(Value::as_object) {
        for (name, value) in parameters {
            encoder_settings = encoder_settings.with_parameter(
                name.clone(),
                value.as_str().ok_or(RecipeError::MalformedJson)?,
            );
        }
    }
    let destination_value = value.get("destination").ok_or(RecipeError::MalformedJson)?;
    let mut destination = RecipeDestination::new(
        string(destination_value, "id")?,
        parse_collision(&string(destination_value, "collision")?)?,
    )?;
    if let Some(reference) = destination_value
        .get("credential_ref")
        .and_then(Value::as_str)
    {
        destination = destination.with_credential_ref(reference);
    }
    if let Some(parameters) = destination_value
        .get("parameters")
        .and_then(Value::as_object)
    {
        for (name, value) in parameters {
            destination = destination.with_parameter(
                name.clone(),
                value.as_str().ok_or(RecipeError::MalformedJson)?,
            );
        }
    }
    let size = parse_size(value.get("size").ok_or(RecipeError::MalformedJson)?)?;
    let output_profile = parse_profile(
        value
            .get("output_profile")
            .ok_or(RecipeError::MalformedJson)?,
    )?;
    let pixel_encoding = parse_pixel_encoding(
        value
            .get("pixel_encoding")
            .ok_or(RecipeError::MalformedJson)?,
    )?;
    let template_value = value
        .get("filename_template")
        .ok_or(RecipeError::MalformedJson)?;
    let template = RecipeTemplate::new(
        string(template_value, "id")?,
        template_value
            .get("version")
            .and_then(Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
            .ok_or(RecipeError::MalformedJson)?,
    )?;
    let mut actions = Vec::new();
    if let Some(values) = value.get("post_success").and_then(Value::as_array) {
        for value in values {
            let mut action = PostSuccessAction::new(string(value, "id")?)?;
            if let Some(parameters) = value.get("parameters").and_then(Value::as_object) {
                for (name, value) in parameters {
                    action = action.with_parameter(
                        name.clone(),
                        value.as_str().ok_or(RecipeError::MalformedJson)?,
                    );
                }
            }
            actions.push(action);
        }
    }
    Ok(ExportRecipe {
        id,
        revision,
        name: string(value, "name")?,
        description: string(value, "description")?,
        encoder_id,
        encoder_settings,
        destination,
        size,
        quality: parse_quality(&string(value, "quality")?)?,
        interpolation: parse_interpolation(&string(value, "interpolation")?)?,
        output_profile,
        intent: parse_intent(&string(value, "intent")?)?,
        black_point_compensation: parse_bpc(&string(value, "black_point_compensation")?)?,
        pixel_encoding,
        alpha: parse_alpha(&string(value, "alpha")?)?,
        dither: parse_dither(&string(value, "dither")?)?,
        metadata: parse_metadata(value.get("metadata").ok_or(RecipeError::MalformedJson)?)?,
        filename_template: template,
        post_success: actions,
        enabled: value
            .get("enabled")
            .and_then(Value::as_bool)
            .ok_or(RecipeError::MalformedJson)?,
        built_in: value
            .get("built_in")
            .and_then(Value::as_bool)
            .ok_or(RecipeError::MalformedJson)?,
        content_hash: parse_hash(&string(value, "content_hash")?)?,
    })
}

pub(crate) fn string(value: &Value, field: &'static str) -> Result<String, RecipeError> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or(RecipeError::MalformedJson)
}
fn parse_hash(value: &str) -> Result<[u8; 32], RecipeError> {
    if value.len() != 64 {
        return Err(RecipeError::MalformedJson);
    }
    let mut hash = [0; 32];
    for (index, chunk) in value.as_bytes().chunks(2).enumerate() {
        hash[index] = u8::from_str_radix(
            std::str::from_utf8(chunk).map_err(|_| RecipeError::MalformedJson)?,
            16,
        )
        .map_err(|_| RecipeError::MalformedJson)?;
    }
    Ok(hash)
}
fn parse_collision(value: &str) -> Result<crate::CollisionPolicy, RecipeError> {
    match value {
        "create_new" => Ok(crate::CollisionPolicy::CreateNew),
        "replace_existing" => Ok(crate::CollisionPolicy::ReplaceExisting),
        _ => Err(RecipeError::UnsupportedValue { field: "collision" }),
    }
}
fn parse_quality(value: &str) -> Result<PipelineQuality, RecipeError> {
    match value {
        "draft" => Ok(PipelineQuality::Draft),
        "standard" => Ok(PipelineQuality::Standard),
        "high" => Ok(PipelineQuality::High),
        "reference" => Ok(PipelineQuality::Reference),
        _ => Err(RecipeError::UnsupportedValue { field: "quality" }),
    }
}
fn parse_interpolation(value: &str) -> Result<Interpolation, RecipeError> {
    match value {
        "nearest" => Ok(Interpolation::Nearest),
        "bilinear" => Ok(Interpolation::Bilinear),
        "bicubic" => Ok(Interpolation::Bicubic),
        "lanczos" => Ok(Interpolation::Lanczos),
        _ => Err(RecipeError::UnsupportedValue {
            field: "interpolation",
        }),
    }
}
fn parse_intent(value: &str) -> Result<rusttable_color::RenderingIntent, RecipeError> {
    match value {
        "perceptual" => Ok(rusttable_color::RenderingIntent::Perceptual),
        "relative" => Ok(rusttable_color::RenderingIntent::Relative),
        "saturation" => Ok(rusttable_color::RenderingIntent::Saturation),
        "absolute" => Ok(rusttable_color::RenderingIntent::Absolute),
        _ => Err(RecipeError::UnsupportedValue { field: "intent" }),
    }
}
fn parse_bpc(value: &str) -> Result<BlackPointCompensation, RecipeError> {
    match value {
        "disabled" => Ok(BlackPointCompensation::Disabled),
        "enabled" => Ok(BlackPointCompensation::Enabled),
        _ => Err(RecipeError::UnsupportedValue {
            field: "black_point_compensation",
        }),
    }
}
fn parse_alpha(value: &str) -> Result<AlphaPolicy, RecipeError> {
    match value {
        "preserve" => Ok(AlphaPolicy::Preserve),
        "replace_opaque" => Ok(AlphaPolicy::ReplaceOpaque),
        "require" => Ok(AlphaPolicy::Require),
        "ignore" => Ok(AlphaPolicy::Ignore),
        _ => Err(RecipeError::UnsupportedValue { field: "alpha" }),
    }
}
fn parse_dither(value: &str) -> Result<DitherPolicy, RecipeError> {
    match value {
        "none" => Ok(DitherPolicy::None),
        "ordered_8x8" => Ok(DitherPolicy::Ordered8x8),
        "error_diffusion" => Ok(DitherPolicy::ErrorDiffusion),
        _ => Err(RecipeError::UnsupportedValue { field: "dither" }),
    }
}
fn parse_action(value: &str) -> Result<MetadataAction, RecipeError> {
    match value {
        "include" => Ok(MetadataAction::Include),
        "exclude" => Ok(MetadataAction::Exclude),
        "redact" => Ok(MetadataAction::Redact),
        _ => Err(RecipeError::UnsupportedValue { field: "metadata" }),
    }
}
fn parse_metadata(value: &Value) -> Result<MetadataPolicy, RecipeError> {
    Ok(MetadataPolicy {
        exif: parse_action(&string(value, "exif")?)?,
        iptc: parse_action(&string(value, "iptc")?)?,
        xmp: parse_action(&string(value, "xmp")?)?,
        gps: parse_action(&string(value, "gps")?)?,
        faces_and_regions: parse_action(&string(value, "faces_and_regions")?)?,
        ratings_labels_tags: parse_action(&string(value, "ratings_labels_tags")?)?,
        history: parse_action(&string(value, "history")?)?,
        thumbnail: parse_action(&string(value, "thumbnail")?)?,
        icc_and_cicp: parse_action(&string(value, "icc_and_cicp")?)?,
        software_and_version: parse_action(&string(value, "software_and_version")?)?,
        user_fields: parse_action(&string(value, "user_fields")?)?,
    })
}
fn parse_size(value: &Value) -> Result<RenderSizeRequest, RecipeError> {
    match string(value, "mode")?.as_str() {
        "source" => Ok(RenderSizeRequest::Source),
        "exact" => {
            RenderSizeRequest::exact(u32_value(value, "width")?, u32_value(value, "height")?)
                .map_err(|_| RecipeError::InvalidSize)
        }
        "fit" => RenderSizeRequest::fit(
            u32_value(value, "max_width")?,
            u32_value(value, "max_height")?,
        )
        .map_err(|_| RecipeError::InvalidSize),
        "long_edge" => RenderSizeRequest::long_edge(u32_value(value, "edge")?)
            .map_err(|_| RecipeError::InvalidSize),
        _ => Err(RecipeError::UnsupportedValue { field: "size" }),
    }
}
fn u32_value(value: &Value, field: &'static str) -> Result<u32, RecipeError> {
    value
        .get(field)
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .ok_or(RecipeError::MalformedJson)
}
fn parse_profile(value: &Value) -> Result<OutputProfileSpec, RecipeError> {
    let encoding = parse_color(&string(value, "encoding")?)?;
    let Some(reference) = value.get("reference") else {
        return Ok(OutputProfileSpec::builtin(encoding));
    };
    Ok(OutputProfileSpec::external(
        encoding,
        parse_hash(&string(reference, "sha256")?)?,
        reference
            .get("size")
            .and_then(Value::as_u64)
            .ok_or(RecipeError::MalformedJson)?,
    ))
}
fn parse_pixel_encoding(value: &Value) -> Result<PixelEncoding, RecipeError> {
    let channels = parse_channels(&string(value, "channels")?)?;
    let color = parse_color(&string(value, "color")?)?;
    match string(value, "kind")?.as_str() {
        "f16" => Ok(PixelEncoding::Float16 { channels, color }),
        "f32" => Ok(PixelEncoding::Float32 { channels, color }),
        "integer" => Ok(PixelEncoding::Integer {
            channels,
            depth: parse_depth(&string(value, "depth")?)?,
            color,
        }),
        _ => Err(RecipeError::UnsupportedValue {
            field: "pixel_encoding",
        }),
    }
}
fn parse_color(value: &str) -> Result<ColorEncoding, RecipeError> {
    [
        ColorEncoding::Unspecified,
        ColorEncoding::SrgbD65,
        ColorEncoding::DisplayP3D65,
        ColorEncoding::LinearSrgbD65,
        ColorEncoding::LinearDisplayP3D65,
        ColorEncoding::Rec2020D65,
        ColorEncoding::LinearRec2020D65,
        ColorEncoding::AcesCgD60,
        ColorEncoding::XyzD50,
        ColorEncoding::XyzD65,
        ColorEncoding::LabD50,
        ColorEncoding::LchD50,
    ]
    .into_iter()
    .find(|candidate| format!("{candidate:?}") == value)
    .ok_or(RecipeError::UnsupportedValue { field: "color" })
}
fn parse_channels(value: &str) -> Result<crate::ChannelLayout, RecipeError> {
    match value {
        "Gray" => Ok(crate::ChannelLayout::Gray),
        "Rgb" => Ok(crate::ChannelLayout::Rgb),
        "Rgba" => Ok(crate::ChannelLayout::Rgba),
        _ => Err(RecipeError::UnsupportedValue { field: "channels" }),
    }
}
fn parse_depth(value: &str) -> Result<crate::BitDepth, RecipeError> {
    match value {
        "Eight" => Ok(crate::BitDepth::Eight),
        "Ten" => Ok(crate::BitDepth::Ten),
        "Twelve" => Ok(crate::BitDepth::Twelve),
        "Sixteen" => Ok(crate::BitDepth::Sixteen),
        _ => Err(RecipeError::UnsupportedValue { field: "depth" }),
    }
}
