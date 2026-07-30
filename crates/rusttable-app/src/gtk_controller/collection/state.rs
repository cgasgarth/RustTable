use std::path::PathBuf;

use rusttable_catalog::{
    ActiveLighttableProperty, ActiveLighttableSort, ActiveLighttableSortDirection, ImportRecord,
};
use rusttable_import::decode_reference_source;
use rusttable_ui::{CollectionItem, CollectionProperty, LighttableSort, LighttableSortDirection};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ActiveLighttableRestoreReport {
    pub discarded_missing: usize,
    pub discarded_hidden: usize,
}

pub(super) fn active_property(property: CollectionProperty) -> ActiveLighttableProperty {
    match property {
        CollectionProperty::Filmroll => ActiveLighttableProperty::Filmroll,
        CollectionProperty::Folders => ActiveLighttableProperty::Folders,
        CollectionProperty::Rating => ActiveLighttableProperty::Rating,
        CollectionProperty::ColorLabel => ActiveLighttableProperty::ColorLabel,
        CollectionProperty::Filename => ActiveLighttableProperty::Filename,
    }
}

pub(super) fn ui_property(property: ActiveLighttableProperty) -> CollectionProperty {
    match property {
        ActiveLighttableProperty::Filmroll => CollectionProperty::Filmroll,
        ActiveLighttableProperty::Folders => CollectionProperty::Folders,
        ActiveLighttableProperty::Rating => CollectionProperty::Rating,
        ActiveLighttableProperty::ColorLabel => CollectionProperty::ColorLabel,
        ActiveLighttableProperty::Filename => CollectionProperty::Filename,
    }
}

pub(super) fn active_sort(sort: LighttableSort) -> ActiveLighttableSort {
    match sort {
        LighttableSort::Filename => ActiveLighttableSort::Filename,
        LighttableSort::CaptureTime => ActiveLighttableSort::CaptureTime,
        LighttableSort::Rating => ActiveLighttableSort::Rating,
    }
}

pub(super) fn ui_sort(sort: ActiveLighttableSort) -> LighttableSort {
    match sort {
        ActiveLighttableSort::Filename => LighttableSort::Filename,
        ActiveLighttableSort::CaptureTime => LighttableSort::CaptureTime,
        ActiveLighttableSort::Rating => LighttableSort::Rating,
    }
}

pub(super) fn active_sort_direction(
    direction: LighttableSortDirection,
) -> ActiveLighttableSortDirection {
    match direction {
        LighttableSortDirection::Ascending => ActiveLighttableSortDirection::Ascending,
        LighttableSortDirection::Descending => ActiveLighttableSortDirection::Descending,
    }
}

pub(super) fn ui_sort_direction(
    direction: ActiveLighttableSortDirection,
) -> LighttableSortDirection {
    match direction {
        ActiveLighttableSortDirection::Ascending => LighttableSortDirection::Ascending,
        ActiveLighttableSortDirection::Descending => LighttableSortDirection::Descending,
    }
}

pub(super) fn collection_item(record: &ImportRecord) -> CollectionItem {
    let path = decode_reference_source(record.source())
        .map_or_else(|_| record.source().as_str().to_owned(), path_string);
    CollectionItem::new(record.photo().id(), path).with_capture_metadata(record.metadata())
}

fn path_string(path: PathBuf) -> String {
    path.into_os_string().to_string_lossy().into_owned()
}
