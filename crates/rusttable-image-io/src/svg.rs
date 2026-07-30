//! Bounded, managed SVG parsing and rasterization for backend assets.

#![forbid(unsafe_code)]

use resvg::tiny_skia::{Pixmap, Transform};
use sha2::{Digest, Sha256};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

pub const SVG_SCHEMA_VERSION: u16 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SvgLimits {
    pub max_source_bytes: usize,
    pub max_nodes: usize,
    pub max_path_segments: usize,
    pub max_text_bytes: usize,
    pub max_embedded_image_bytes: usize,
    pub max_rendered_width: u32,
    pub max_rendered_height: u32,
    pub max_rendered_pixels: u64,
}

impl Default for SvgLimits {
    fn default() -> Self {
        Self {
            max_source_bytes: 8 * 1024 * 1024,
            max_nodes: 32_768,
            max_path_segments: 262_144,
            max_text_bytes: 256 * 1024,
            max_embedded_image_bytes: 16 * 1024 * 1024,
            max_rendered_width: 16_384,
            max_rendered_height: 16_384,
            max_rendered_pixels: 16_384 * 16_384,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SvgError {
    SourceTooLarge { actual: usize, limit: usize },
    InvalidUtf8,
    CompressedSourceUnsupported,
    ForbiddenContent(&'static str),
    ExternalResource,
    Parse(String),
    NodeLimit,
    PathLimit,
    TextLimit,
    EmbeddedImageLimit,
    RenderDimensionLimit,
    Render(String),
}

impl std::fmt::Display for SvgError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SourceTooLarge { actual, limit } => {
                write!(f, "SVG source is {actual} bytes; limit is {limit}")
            }
            Self::InvalidUtf8 => f.write_str("SVG source is not UTF-8"),
            Self::CompressedSourceUnsupported => f.write_str("compressed SVG is not supported"),
            Self::ForbiddenContent(kind) => write!(f, "SVG contains forbidden {kind}"),
            Self::ExternalResource => f.write_str("SVG references an external resource"),
            Self::Parse(error) => write!(f, "SVG parsing failed: {error}"),
            Self::NodeLimit => f.write_str("SVG node limit exceeded"),
            Self::PathLimit => f.write_str("SVG path limit exceeded"),
            Self::TextLimit => f.write_str("SVG text limit exceeded"),
            Self::EmbeddedImageLimit => f.write_str("SVG embedded image limit exceeded"),
            Self::RenderDimensionLimit => {
                f.write_str("SVG render dimensions exceed the managed limit")
            }
            Self::Render(error) => write!(f, "SVG rendering failed: {error}"),
        }
    }
}

impl std::error::Error for SvgError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SvgRaster {
    width: u32,
    height: u32,
    /// Premultiplied sRGB RGBA bytes, row-major, as returned by tiny-skia.
    pixels: Vec<u8>,
}

impl SvgRaster {
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }

    #[must_use]
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }
}

#[derive(Clone)]
pub struct ManagedSvgAsset {
    source: Arc<[u8]>,
    source_hash: [u8; 32],
    tree_hash: [u8; 32],
    tree: usvg::Tree,
    limits: SvgLimits,
}

impl std::fmt::Debug for ManagedSvgAsset {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ManagedSvgAsset")
            .field("source_hash", &self.source_hash)
            .field("tree_hash", &self.tree_hash)
            .field("source_bytes", &self.source.len())
            .field("size", &self.tree.size())
            .field("limits", &self.limits)
            .finish()
    }
}

impl ManagedSvgAsset {
    /// Parses an immutable SVG source with no filesystem or network resource access.
    ///
    /// # Errors
    ///
    /// Returns a bounded parse or safety error when the source is malformed,
    /// exceeds a managed limit, or attempts a forbidden resource/content type.
    pub fn parse(bytes: impl Into<Vec<u8>>, limits: SvgLimits) -> Result<Self, SvgError> {
        let bytes = bytes.into();
        if bytes.len() > limits.max_source_bytes {
            return Err(SvgError::SourceTooLarge {
                actual: bytes.len(),
                limit: limits.max_source_bytes,
            });
        }
        if bytes.starts_with(&[0x1f, 0x8b]) {
            return Err(SvgError::CompressedSourceUnsupported);
        }
        let source = std::str::from_utf8(&bytes).map_err(|_| SvgError::InvalidUtf8)?;
        reject_unsafe_markup(source)?;
        let external = Arc::new(AtomicBool::new(false));
        let image_limit = limits.max_embedded_image_bytes;
        let external_for_resolver = Arc::clone(&external);
        let options = usvg::Options {
            fontdb: Arc::new(usvg::fontdb::Database::new()),
            image_href_resolver: usvg::ImageHrefResolver {
                resolve_data: Box::new(move |mime, data, _| {
                    if data.len() > image_limit {
                        return None;
                    }
                    match mime {
                        "image/png" => Some(usvg::ImageKind::PNG(data)),
                        "image/jpeg" | "image/jpg" => Some(usvg::ImageKind::JPEG(data)),
                        _ => None,
                    }
                }),
                resolve_string: Box::new(move |_, _| {
                    external_for_resolver.store(true, Ordering::Relaxed);
                    None
                }),
            },
            ..usvg::Options::default()
        };
        let tree =
            usvg::Tree::from_data(&bytes, &options).map_err(|e| SvgError::Parse(e.to_string()))?;
        if external.load(Ordering::Relaxed) {
            return Err(SvgError::ExternalResource);
        }
        validate_tree(&tree, limits)?;
        let source_hash = Sha256::digest(&bytes).into();
        let tree_hash = hash_tree(&tree);
        Ok(Self {
            source: Arc::from(bytes),
            source_hash,
            tree_hash,
            tree,
            limits,
        })
    }

    #[must_use]
    pub fn source_bytes(&self) -> &[u8] {
        &self.source
    }

    #[must_use]
    pub const fn source_hash(&self) -> [u8; 32] {
        self.source_hash
    }

    #[must_use]
    pub const fn tree_hash(&self) -> [u8; 32] {
        self.tree_hash
    }

    #[must_use]
    pub fn size(&self) -> usvg::Size {
        self.tree.size()
    }

    /// Rasterizes the frozen tree into premultiplied sRGB RGBA bytes.
    ///
    /// # Errors
    ///
    /// Returns an error when the requested dimensions exceed the managed
    /// limits or the rasterizer cannot allocate the target.
    #[allow(clippy::cast_precision_loss)]
    pub fn rasterize(&self, width: u32, height: u32) -> Result<SvgRaster, SvgError> {
        validate_dimensions(width, height, self.limits)?;
        let mut pixmap = Pixmap::new(width, height).ok_or(SvgError::RenderDimensionLimit)?;
        let scale_x = width as f32 / self.tree.size().width();
        let scale_y = height as f32 / self.tree.size().height();
        resvg::render(
            &self.tree,
            Transform::from_scale(scale_x, scale_y),
            &mut pixmap.as_mut(),
        );
        Ok(SvgRaster {
            width,
            height,
            pixels: pixmap.take(),
        })
    }
}

fn reject_unsafe_markup(source: &str) -> Result<(), SvgError> {
    let lower = source.to_ascii_lowercase();
    for (needle, kind) in [
        ("<script", "script"),
        ("<animate", "animation"),
        ("<set", "animation"),
        ("<!doctype", "doctype"),
        ("<!entity", "entity"),
        ("javascript:", "script URL"),
    ] {
        if lower.contains(needle) {
            return Err(SvgError::ForbiddenContent(kind));
        }
    }
    for attribute in ["href=", "xlink:href="] {
        let mut rest = lower.as_str();
        while let Some(index) = rest.find(attribute) {
            rest = &rest[index + attribute.len()..];
            let value = rest.trim_start_matches([' ', '\t', '\r', '\n']);
            let quoted = value.strip_prefix('"').or_else(|| value.strip_prefix('\''));
            if let Some(quoted) = quoted
                && !quoted.starts_with("data:")
            {
                return Err(SvgError::ExternalResource);
            }
        }
    }
    Ok(())
}

fn validate_tree(tree: &usvg::Tree, limits: SvgLimits) -> Result<(), SvgError> {
    if tree.has_text_nodes() {
        return Err(SvgError::ForbiddenContent(
            "text without a managed bundled font set",
        ));
    }
    if !tree.filters().is_empty() {
        return Err(SvgError::ForbiddenContent("filters"));
    }
    let mut counts = (0_usize, 0_usize, 0_usize, 0_usize);
    validate_group(tree.root(), limits, &mut counts)?;
    Ok(())
}

fn validate_group(
    group: &usvg::Group,
    limits: SvgLimits,
    counts: &mut (usize, usize, usize, usize),
) -> Result<(), SvgError> {
    for node in group.children() {
        counts.0 = counts.0.checked_add(1).ok_or(SvgError::NodeLimit)?;
        if counts.0 > limits.max_nodes {
            return Err(SvgError::NodeLimit);
        }
        match node {
            usvg::Node::Group(group) => validate_group(group, limits, counts)?,
            usvg::Node::Path(path) => {
                counts.1 = counts
                    .1
                    .checked_add(path.data().segments().count())
                    .ok_or(SvgError::PathLimit)?;
                if counts.1 > limits.max_path_segments {
                    return Err(SvgError::PathLimit);
                }
            }
            usvg::Node::Image(image) => {
                let bytes = match image.kind() {
                    usvg::ImageKind::JPEG(data)
                    | usvg::ImageKind::PNG(data)
                    | usvg::ImageKind::GIF(data)
                    | usvg::ImageKind::WEBP(data) => data.len(),
                    usvg::ImageKind::SVG(tree) => {
                        validate_group(tree.root(), limits, counts)?;
                        0
                    }
                };
                counts.2 = counts
                    .2
                    .checked_add(bytes)
                    .ok_or(SvgError::EmbeddedImageLimit)?;
                if counts.2 > limits.max_embedded_image_bytes {
                    return Err(SvgError::EmbeddedImageLimit);
                }
            }
            usvg::Node::Text(text) => {
                counts.3 = counts
                    .3
                    .checked_add(format!("{text:?}").len())
                    .ok_or(SvgError::TextLimit)?;
                if counts.3 > limits.max_text_bytes {
                    return Err(SvgError::TextLimit);
                }
            }
        }
    }
    Ok(())
}

fn validate_dimensions(width: u32, height: u32, limits: SvgLimits) -> Result<(), SvgError> {
    if width == 0
        || height == 0
        || width > limits.max_rendered_width
        || height > limits.max_rendered_height
        || u64::from(width)
            .checked_mul(u64::from(height))
            .is_none_or(|pixels| pixels > limits.max_rendered_pixels)
    {
        return Err(SvgError::RenderDimensionLimit);
    }
    Ok(())
}

fn hash_tree(tree: &usvg::Tree) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"rusttable.managed-svg.tree.v1");
    hasher.update(format!("{tree:?}").as_bytes());
    hasher.finalize().into()
}
