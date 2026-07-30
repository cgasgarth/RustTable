use super::inspect::is_complete_group;
use super::types::{
    ExrChannel, ExrChannelMapping, ExrChannelRole, ExrDecodeError, ExrDecodeRequest, ExrHeader,
    ExrLayerView, ExrPart, ExrSampleType,
};

pub(crate) struct Selection<'a> {
    pub part: &'a ExrPart,
    pub group: ExrLayerView,
    pub mapping: ExrChannelMapping,
    pub channels: Vec<&'a ExrChannel>,
    pub sample_type: ExrSampleType,
}

pub(crate) fn select<'a>(
    header: &'a ExrHeader,
    request: &ExrDecodeRequest,
) -> Result<Selection<'a>, ExrDecodeError> {
    let part_index = request
        .part
        .or(header.default_part)
        .ok_or_else(|| ExrDecodeError::InvalidSelection("no RGB or Y part exists".to_owned()))?;
    let part = header
        .parts
        .get(part_index)
        .ok_or(ExrDecodeError::InvalidPart(part_index))?;
    if part.deep {
        return Err(ExrDecodeError::UnsupportedDeepData { part: part_index });
    }
    let (group, mapping) = if let Some(mapping) = &request.channels {
        let group = ExrLayerView {
            layer: request.layer.clone().unwrap_or_default(),
            view: request.view.clone().unwrap_or_default(),
            channels: mapping_names(mapping),
            has_rgb: matches!(mapping, ExrChannelMapping::Rgb { .. }),
            has_luminance: matches!(mapping, ExrChannelMapping::Gray { .. }),
            has_alpha: mapping_alpha(mapping).is_some(),
        };
        (group, mapping.clone())
    } else {
        let group = choose_group(part, request)?;
        let mapping = mapping_for_group(part, &group)?;
        (group, mapping)
    };
    let channels = mapping_names(&mapping)
        .iter()
        .map(|name| {
            part.channels
                .iter()
                .find(|channel| channel.name == *name)
                .ok_or_else(|| {
                    ExrDecodeError::InvalidSelection(format!("channel {name} is missing"))
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    if let Some(channel) = channels
        .iter()
        .find(|channel| channel.sample_type == ExrSampleType::U32)
    {
        return Err(ExrDecodeError::UnsupportedSampleType {
            channel: channel.name.clone(),
        });
    }
    if let Some(channel) = channels
        .iter()
        .find(|channel| channel.x_sampling != 1 || channel.y_sampling != 1)
    {
        return Err(ExrDecodeError::InvalidSelection(format!(
            "selected channel {} is subsampled",
            channel.name
        )));
    }
    let sample_type = if channels
        .iter()
        .all(|channel| channel.sample_type == ExrSampleType::F16)
    {
        ExrSampleType::F16
    } else {
        ExrSampleType::F32
    };
    Ok(Selection {
        part,
        group,
        mapping,
        channels,
        sample_type,
    })
}

fn choose_group(
    part: &ExrPart,
    request: &ExrDecodeRequest,
) -> Result<ExrLayerView, ExrDecodeError> {
    let mut candidates = part
        .layers
        .iter()
        .filter(|group| is_complete_group(group))
        .filter(|group| {
            request
                .layer
                .as_ref()
                .is_none_or(|layer| group.layer == *layer)
        })
        .filter(|group| request.view.as_ref().is_none_or(|view| group.view == *view))
        .cloned()
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| preference(left).cmp(&preference(right)));
    candidates.into_iter().next().ok_or_else(|| {
        ExrDecodeError::InvalidSelection(
            "requested layer/view has no complete RGB or Y channels".to_owned(),
        )
    })
}

fn preference(group: &ExrLayerView) -> (u8, &str, &str) {
    let rank = if group.layer.is_empty() && group.view.is_empty() {
        0
    } else if group.layer.eq_ignore_ascii_case("beauty") {
        1
    } else {
        2
    };
    (rank, &group.layer, &group.view)
}

fn mapping_for_group(
    part: &ExrPart,
    group: &ExrLayerView,
) -> Result<ExrChannelMapping, ExrDecodeError> {
    let find = |role| {
        part.channels
            .iter()
            .find(|channel| {
                channel.layer == group.layer
                    && channel.view == group.view
                    && channel.role == Some(role)
            })
            .map(|channel| channel.name.clone())
    };
    let alpha = find(ExrChannelRole::Alpha);
    if group.has_rgb {
        Ok(ExrChannelMapping::Rgb {
            red: find(ExrChannelRole::Red).ok_or_else(incomplete)?,
            green: find(ExrChannelRole::Green).ok_or_else(incomplete)?,
            blue: find(ExrChannelRole::Blue).ok_or_else(incomplete)?,
            alpha,
        })
    } else {
        Ok(ExrChannelMapping::Gray {
            gray: find(ExrChannelRole::Luminance).ok_or_else(incomplete)?,
            alpha,
        })
    }
}

fn incomplete() -> ExrDecodeError {
    ExrDecodeError::InvalidSelection("normalized channel group became incomplete".to_owned())
}

pub(crate) fn mapping_names(mapping: &ExrChannelMapping) -> Vec<String> {
    match mapping {
        ExrChannelMapping::Gray { gray, alpha } => {
            let mut names = vec![gray.clone()];
            names.extend(alpha.iter().cloned());
            names
        }
        ExrChannelMapping::Rgb {
            red,
            green,
            blue,
            alpha,
        } => {
            let mut names = vec![red.clone(), green.clone(), blue.clone()];
            names.extend(alpha.iter().cloned());
            names
        }
    }
}

pub(crate) fn mapping_alpha(mapping: &ExrChannelMapping) -> Option<&str> {
    match mapping {
        ExrChannelMapping::Gray { alpha, .. } | ExrChannelMapping::Rgb { alpha, .. } => {
            alpha.as_deref()
        }
    }
}
