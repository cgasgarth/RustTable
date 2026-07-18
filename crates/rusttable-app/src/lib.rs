#![forbid(unsafe_code)]
#![doc = "The `RustTable` Iced application composition root."]

mod application;
mod composition;
mod library;
mod lifecycle;

pub use composition::run;
