#![forbid(unsafe_code)]
#![doc = "Mask graph boundary contracts for the `RustTable` rewrite."]
#![allow(
    clippy::cast_precision_loss,
    clippy::double_must_use,
    clippy::large_enum_variant,
    clippy::many_single_char_names,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc
)]

mod graph;
mod model;
pub mod purpose;
mod raster;

pub use graph::{GraphBuildError, GraphNode, MaskGraph, MaskGraphBuilder, MaskGroup, MaskNode};
pub use model::{
    CombinationMode, GeometryAncestry, GeometryStep, MaskGeometry, MaskIdentity, MaskModelError,
    MaskModifier, MaskReference, MaskRoi, MaskSource, ProducerIdentity, RasterMaskDescriptor,
};
pub use raster::{
    MaskExecutionError, MaskLease, MaskRaster, RasterMaskPublication, RasterMaskStore,
};
