use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

use rawler::cfa::CFAColor;
use rawler::decoders::{Orientation, RawDecodeParams};
use rawler::formats::tiff::Value;
use rawler::imgop::Rect;
use rawler::imgop::xyz::Illuminant;
use rawler::rawimage::RawPhotometricInterpretation;
use rawler::rawsource::RawSource;
use rawler::{RawImage, RawImageData, RawlerError};
use rusttable_image::{
    ColorEncoding, DecodeLimits, DecodedImage, DecodedImageError, ImageDimensions, ImageInputError,
    ImageProbe, InputFormat,
};
use sha2::{Digest, Sha256};

use super::{
    RAWLER_BACKEND_ID, RawCameraIdentity, RawCancellationToken, RawCapabilityError,
    RawCapabilityKind, RawChannel, RawColorMatrix, RawContainerKind, RawContainerProbe,
    RawContainerRegistry, RawDecodeError, RawDecodeLimits, RawDecodeReceipt, RawDecodeRequest,
    RawDecodeResult, RawDimensions, RawFrame, RawFrameParts, RawFrameValidationError, RawHeader,
    RawIlluminant, RawLevelPattern, RawOpcodeDescriptor, RawOpcodeStage, RawOrientation, RawPlane,
    RawPlaneLayout, RawPreviewDescriptor, RawPreviewFormat, RawPreviewKind, RawProbeOutcome,
    RawRect, RawSourceError, RawSourceReceipt,
};

const COPY_CHUNK_BYTES: usize = 64 * 1024;

/// Bounded random-access source accepted by the RAW adapter.
pub trait RawByteSource: Send + Sync {
    /// Returns the current source length.
    ///
    /// # Errors
    ///
    /// Returns [`RawSourceError`] when the source cannot report its length.
    fn len(&self) -> Result<u64, RawSourceError>;

    /// Returns a stable revision token for mutation detection.
    ///
    /// # Errors
    ///
    /// Returns [`RawSourceError`] when the source cannot produce a revision.
    fn revision(&self) -> Result<[u8; 32], RawSourceError>;

    /// Reads exactly `buffer.len()` bytes at `offset`.
    ///
    /// # Errors
    ///
    /// Returns [`RawSourceError`] when the requested range cannot be read.
    fn read_exact_at(&self, offset: u64, buffer: &mut [u8]) -> Result<(), RawSourceError>;

    /// Reports whether the source is empty.
    ///
    /// # Errors
    ///
    /// Returns [`RawSourceError`] when the source length cannot be read.
    fn is_empty(&self) -> Result<bool, RawSourceError> {
        Ok(self.len()? == 0)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct SliceRawSource<'a> {
    bytes: &'a [u8],
}

impl<'a> SliceRawSource<'a> {
    #[must_use]
    pub const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes }
    }
}

impl RawByteSource for SliceRawSource<'_> {
    fn len(&self) -> Result<u64, RawSourceError> {
        u64::try_from(self.bytes.len()).map_err(|_| RawSourceError::LengthConversion)
    }

    fn revision(&self) -> Result<[u8; 32], RawSourceError> {
        Ok(Sha256::digest(self.bytes).into())
    }

    fn read_exact_at(&self, offset: u64, buffer: &mut [u8]) -> Result<(), RawSourceError> {
        let start = usize::try_from(offset).map_err(|_| RawSourceError::Read {
            offset,
            requested: buffer.len(),
        })?;
        let end = start
            .checked_add(buffer.len())
            .ok_or(RawSourceError::Read {
                offset,
                requested: buffer.len(),
            })?;
        let source = self.bytes.get(start..end).ok_or(RawSourceError::Read {
            offset,
            requested: buffer.len(),
        })?;
        buffer.copy_from_slice(source);
        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
pub struct RawlerRawDecoder {
    registry: RawContainerRegistry,
}

impl RawlerRawDecoder {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            registry: RawContainerRegistry::standard(),
        }
    }

    /// Decodes an immutable byte slice into an owned, validated RAW frame.
    ///
    /// # Errors
    ///
    /// Returns [`RawDecodeError`] for unsupported, malformed, cancelled, or over-limit input.
    pub fn decode_bytes(
        &self,
        bytes: &[u8],
        request: &RawDecodeRequest,
    ) -> Result<RawDecodeResult, RawDecodeError> {
        self.decode_source(&SliceRawSource::new(bytes), request)
    }

    /// Probes an immutable byte slice without allocating decoded pixel planes.
    ///
    /// # Errors
    ///
    /// Returns [`RawDecodeError`] for unsupported, malformed, cancelled, or over-limit input.
    pub fn probe_bytes(
        &self,
        bytes: &[u8],
        request: &RawDecodeRequest,
    ) -> Result<RawHeader, RawDecodeError> {
        self.probe_source(&SliceRawSource::new(bytes), request)
    }

    /// Probes container and camera metadata without asking the backend to allocate pixel planes.
    ///
    /// # Errors
    ///
    /// Returns [`RawDecodeError`] for unsupported, malformed, cancelled, mutated, or over-limit
    /// input.
    pub fn probe_source<S: RawByteSource + ?Sized>(
        &self,
        source: &S,
        request: &RawDecodeRequest,
    ) -> Result<RawHeader, RawDecodeError> {
        check_cancel(&request.cancellation)?;
        let copy = StableSourceCopy::read(source, request)?;
        let probe = selected_probe(self.registry, copy.bytes.as_slice())?;
        let raw_source = RawSource::new_from_shared_vec(Arc::clone(&copy.bytes));
        let dummy = backend_decode(
            &raw_source,
            &RawDecodeParams {
                image_index: request.image_index,
            },
            true,
        )
        .map_err(|error| map_backend_error(error, &probe))?;
        validate_backend_declaration(&dummy, request.limits, &probe)?;
        check_cancel(&request.cancellation)?;
        header_from_backend(&dummy, &probe)
    }

    /// Copies one immutable source through bounded random access and publishes no partial frame.
    ///
    /// # Errors
    ///
    /// Returns [`RawDecodeError`] for unsupported, malformed, cancelled, mutated, or over-limit
    /// input.
    pub fn decode_source<S: RawByteSource + ?Sized>(
        &self,
        source: &S,
        request: &RawDecodeRequest,
    ) -> Result<RawDecodeResult, RawDecodeError> {
        check_cancel(&request.cancellation)?;
        let copy = StableSourceCopy::read(source, request)?;
        check_cancel(&request.cancellation)?;
        let probe = selected_probe(self.registry, copy.bytes.as_slice())?;
        let raw_source = RawSource::new_from_shared_vec(Arc::clone(&copy.bytes));
        let params = RawDecodeParams {
            image_index: request.image_index,
        };
        let dummy = backend_decode(&raw_source, &params, true)
            .map_err(|error| map_backend_error(error, &probe))?;
        validate_backend_declaration(&dummy, request.limits, &probe)?;
        check_cancel(&request.cancellation)?;
        let decoded = backend_decode(&raw_source, &params, false)
            .map_err(|error| map_backend_error(error, &probe))?;
        check_cancel(&request.cancellation)?;
        let frame = convert_frame(decoded, &probe, request.limits, copy.bytes.as_slice())?;
        let sample_count = frame
            .parts()
            .planes
            .iter()
            .try_fold(0_u64, |total, plane| {
                total.checked_add(u64::try_from(plane.samples.len()).ok()?)
            })
            .ok_or(RawDecodeError::InvalidFrame(
                RawFrameValidationError::ArithmeticOverflow,
            ))?;
        let plane_count = u8::try_from(frame.parts().planes.len())
            .map_err(|_| RawDecodeError::InvalidFrame(RawFrameValidationError::PlaneCount))?;
        let receipt = RawDecodeReceipt {
            backend: RAWLER_BACKEND_ID.to_owned(),
            container: probe.container,
            camera: frame.parts().camera.clone(),
            compression: frame.parts().compression.clone(),
            bit_depth: frame.parts().bit_depth,
            source: RawSourceReceipt {
                source_bytes: copy.source_bytes,
                source_sha256: copy.sha256,
                stable_copy_used: true,
            },
            plane_count,
            sample_count,
        };
        Ok(RawDecodeResult { frame, receipt })
    }
}

fn selected_probe(
    registry: RawContainerRegistry,
    bytes: &[u8],
) -> Result<RawContainerProbe, RawDecodeError> {
    match registry.probe_bytes(bytes) {
        RawProbeOutcome::NoMatch => Err(RawDecodeError::UnsupportedSignature {
            signature: bytes.iter().copied().take(24).collect(),
        }),
        RawProbeOutcome::MalformedRecognized { container, message } => {
            Err(RawDecodeError::Malformed {
                container: Some(container),
                message,
            })
        }
        RawProbeOutcome::Match(probe) => Ok(probe),
    }
}

fn header_from_backend(
    image: &RawImage,
    probe: &RawContainerProbe,
) -> Result<RawHeader, RawDecodeError> {
    let dimensions = dimensions(image.width, image.height)?;
    let full_area = RawRect::new(0, 0, dimensions.width, dimensions.height).map_err(invalid)?;
    let active_area = image
        .active_area
        .map(convert_rect)
        .transpose()?
        .unwrap_or(full_area);
    let crop_area = image
        .crop_area
        .map(convert_rect)
        .transpose()?
        .unwrap_or(active_area);
    let bit_depth = probe
        .evidence
        .bit_depth
        .or_else(|| image.camera.bps.and_then(|value| u8::try_from(value).ok()))
        .unwrap_or_else(|| u8::try_from(image.bps).unwrap_or_default());
    Ok(RawHeader {
        container: probe.container,
        dimensions,
        active_area,
        crop_area,
        orientation: orientation(image.orientation),
        camera: RawCameraIdentity {
            maker: safe_text(&image.make),
            model: safe_text(&image.model),
            normalized_maker: safe_text(&image.clean_make),
            normalized_model: safe_text(&image.clean_model),
            mode: safe_text(&image.camera.mode),
        },
        compression: probe.evidence.compression.clone(),
        bit_depth,
    })
}

impl Default for RawlerRawDecoder {
    fn default() -> Self {
        Self::new()
    }
}

struct StableSourceCopy {
    bytes: Arc<Vec<u8>>,
    source_bytes: u64,
    sha256: [u8; 32],
}

impl StableSourceCopy {
    fn read<S: RawByteSource + ?Sized>(
        source: &S,
        request: &RawDecodeRequest,
    ) -> Result<Self, RawDecodeError> {
        let length = source.len().map_err(RawDecodeError::Source)?;
        if length == 0 {
            return Err(RawDecodeError::Source(RawSourceError::Empty));
        }
        if length > request.limits.max_source_bytes {
            return Err(RawDecodeError::Source(RawSourceError::TooLarge {
                actual: length,
                limit: request.limits.max_source_bytes,
            }));
        }
        let revision = source.revision().map_err(RawDecodeError::Source)?;
        let length_usize = usize::try_from(length)
            .map_err(|_| RawDecodeError::Source(RawSourceError::LengthConversion))?;
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(length_usize)
            .map_err(|_| RawDecodeError::Source(RawSourceError::AllocationFailure))?;
        bytes.resize(length_usize, 0);
        for (index, chunk) in bytes.chunks_mut(COPY_CHUNK_BYTES).enumerate() {
            check_cancel(&request.cancellation)?;
            let offset = index
                .checked_mul(COPY_CHUNK_BYTES)
                .and_then(|value| u64::try_from(value).ok())
                .ok_or(RawDecodeError::Source(RawSourceError::LengthConversion))?;
            source
                .read_exact_at(offset, chunk)
                .map_err(RawDecodeError::Source)?;
        }
        if source.revision().map_err(RawDecodeError::Source)? != revision {
            return Err(RawDecodeError::Source(RawSourceError::Changed));
        }
        let sha256 = Sha256::digest(bytes.as_slice()).into();
        Ok(Self {
            bytes: Arc::new(bytes),
            source_bytes: length,
            sha256,
        })
    }
}

fn backend_decode(
    source: &RawSource,
    params: &RawDecodeParams,
    dummy: bool,
) -> Result<RawImage, RawlerError> {
    catch_unwind(AssertUnwindSafe(|| {
        if dummy {
            rawler::decode_dummy(source)
        } else {
            rawler::decode(source, params)
        }
    }))
    .map_err(|_| RawlerError::DecoderFailed("backend panic was contained".to_owned()))?
}

fn validate_backend_declaration(
    image: &RawImage,
    limits: RawDecodeLimits,
    probe: &RawContainerProbe,
) -> Result<(), RawDecodeError> {
    let dimensions = dimensions(image.width, image.height)?;
    if dimensions.width > limits.max_width || dimensions.height > limits.max_height {
        return Err(invalid(RawFrameValidationError::DimensionLimit {
            width: dimensions.width,
            height: dimensions.height,
        }));
    }
    if image.cpp == 0 || image.cpp > 4 {
        return Err(capability(
            RawCapabilityKind::Layout,
            probe,
            &image.make,
            &image.model,
            &image.camera.mode,
            "backend declared an unsupported component layout",
        ));
    }
    let samples = u64::from(dimensions.width)
        .checked_mul(u64::from(dimensions.height))
        .and_then(|pixels| pixels.checked_mul(u64::try_from(image.cpp).ok()?))
        .ok_or_else(|| invalid(RawFrameValidationError::ArithmeticOverflow))?;
    if samples > limits.max_samples {
        return Err(invalid(RawFrameValidationError::SampleLimit {
            actual: samples,
            limit: limits.max_samples,
        }));
    }
    let bytes = samples
        .checked_mul(2)
        .ok_or_else(|| invalid(RawFrameValidationError::ArithmeticOverflow))?;
    if bytes > limits.max_decoded_bytes {
        return Err(invalid(RawFrameValidationError::DecodedByteLimit {
            actual: bytes,
            limit: limits.max_decoded_bytes,
        }));
    }
    let bit_depth = probe
        .evidence
        .bit_depth
        .map(usize::from)
        .or(image.camera.bps)
        .unwrap_or(image.bps);
    if bit_depth == 0 || bit_depth > 16 || image.bps == 0 || image.bps > 16 {
        return Err(invalid(RawFrameValidationError::BitDepth));
    }
    validate_backend_metadata(image, dimensions, limits)
}

fn validate_backend_metadata(
    image: &RawImage,
    dimensions: RawDimensions,
    limits: RawDecodeLimits,
) -> Result<(), RawDecodeError> {
    if let Some(area) = image.active_area {
        convert_rect(area)?
            .validate_within(dimensions)
            .map_err(invalid)?;
    }
    if let Some(area) = image.crop_area {
        convert_rect(area)?
            .validate_within(dimensions)
            .map_err(invalid)?;
    }
    if image.blackareas.len() > usize::from(limits.max_metadata_items) {
        return Err(invalid(RawFrameValidationError::MetadataLimit));
    }
    for area in &image.blackareas {
        convert_rect(*area)?
            .validate_within(dimensions)
            .map_err(invalid)?;
    }
    let black_values = image.blacklevel.as_vec();
    let white_values = image.whitelevel.as_vec();
    if black_values.len() > usize::from(limits.max_level_values)
        || white_values.len() > usize::from(limits.max_level_values)
        || black_values
            .iter()
            .any(|value| !value.is_finite() || *value < 0.0)
        || white_values
            .iter()
            .any(|value| !value.is_finite() || *value <= 0.0)
    {
        return Err(invalid(RawFrameValidationError::InvalidLevels));
    }
    for matrix in image.color_matrix.values() {
        if matrix.len() > usize::from(limits.max_level_values)
            || matrix.iter().any(|value| !value.is_finite())
        {
            return Err(invalid(RawFrameValidationError::InvalidMatrix));
        }
    }
    match &image.photometric {
        RawPhotometricInterpretation::Cfa(config) => {
            let cells = config
                .cfa
                .width
                .checked_mul(config.cfa.height)
                .ok_or_else(|| invalid(RawFrameValidationError::ArithmeticOverflow))?;
            if cells == 0 || cells > usize::from(limits.max_cfa_cells) {
                return Err(invalid(RawFrameValidationError::InvalidCfa));
            }
        }
        RawPhotometricInterpretation::LinearRaw | RawPhotometricInterpretation::BlackIsZero => {}
    }
    Ok(())
}

fn convert_frame(
    image: RawImage,
    probe: &RawContainerProbe,
    limits: RawDecodeLimits,
    source: &[u8],
) -> Result<RawFrame, RawDecodeError> {
    validate_backend_declaration(&image, limits, probe)?;
    let dimensions = dimensions(image.width, image.height)?;
    let layout = convert_layout(&image.photometric, image.cpp, probe, &image)?;
    let samples = match image.data {
        RawImageData::Integer(samples) => samples,
        RawImageData::Float(_) => {
            return Err(capability(
                RawCapabilityKind::SampleType,
                probe,
                &image.make,
                &image.model,
                &image.camera.mode,
                "the RustTable RawFrame contract requires U16 sensor planes",
            ));
        }
    };
    let full_area = RawRect::new(0, 0, dimensions.width, dimensions.height).map_err(invalid)?;
    let active_area = image
        .active_area
        .map(convert_rect)
        .transpose()?
        .unwrap_or(full_area);
    let crop_area = image
        .crop_area
        .map(convert_rect)
        .transpose()?
        .unwrap_or(active_area);
    let masked_areas = image
        .blackareas
        .iter()
        .copied()
        .map(convert_rect)
        .collect::<Result<Vec<_>, _>>()?;
    let channels =
        u8::try_from(image.cpp).map_err(|_| invalid(RawFrameValidationError::PlaneLayout))?;
    let camera = RawCameraIdentity {
        maker: safe_text(&image.make),
        model: safe_text(&image.model),
        normalized_maker: safe_text(&image.clean_make),
        normalized_model: safe_text(&image.clean_model),
        mode: safe_text(&image.camera.mode),
    };
    let white_balance = white_balance(image.wb_coeffs)?;
    let color_matrices = color_matrices(image.color_matrix)?;
    let opcodes = opcode_descriptors(&image.dng_tags, limits)?;
    let previews = preview_descriptors(source, probe.container);
    let repeat_width = u8::try_from(image.blacklevel.width)
        .map_err(|_| invalid(RawFrameValidationError::InvalidLevels))?;
    let repeat_height = u8::try_from(image.blacklevel.height)
        .map_err(|_| invalid(RawFrameValidationError::InvalidLevels))?;
    let level_channels = u8::try_from(image.blacklevel.cpp)
        .map_err(|_| invalid(RawFrameValidationError::InvalidLevels))?;
    let bit_depth = probe
        .evidence
        .bit_depth
        .or_else(|| image.camera.bps.and_then(|value| u8::try_from(value).ok()))
        .unwrap_or_else(|| u8::try_from(image.bps).unwrap_or_default());
    RawFrame::try_new(
        RawFrameParts {
            planes: vec![RawPlane {
                dimensions,
                channels_per_pixel: channels,
                layout,
                samples,
            }],
            active_area,
            crop_area,
            masked_areas,
            black_levels: RawLevelPattern {
                repeat_width,
                repeat_height,
                channels: level_channels,
                values: image.blacklevel.as_vec(),
            },
            white_levels: image.whitelevel.as_vec(),
            orientation: orientation(image.orientation),
            camera,
            color_matrices,
            white_balance,
            compression: probe.evidence.compression.clone(),
            bit_depth,
            opcodes,
            previews,
        },
        limits,
    )
    .map_err(invalid)
}

fn convert_layout(
    photometric: &RawPhotometricInterpretation,
    cpp: usize,
    probe: &RawContainerProbe,
    image: &RawImage,
) -> Result<RawPlaneLayout, RawDecodeError> {
    match photometric {
        RawPhotometricInterpretation::Cfa(config) => {
            let width = u8::try_from(config.cfa.width)
                .map_err(|_| invalid(RawFrameValidationError::InvalidCfa))?;
            let height = u8::try_from(config.cfa.height)
                .map_err(|_| invalid(RawFrameValidationError::InvalidCfa))?;
            Ok(RawPlaneLayout::Mosaic(super::RawCfa {
                width,
                height,
                phase_x: 0,
                phase_y: 0,
                pattern: config
                    .cfa
                    .flat_pattern()
                    .into_iter()
                    .map(channel_from_index)
                    .collect(),
            }))
        }
        RawPhotometricInterpretation::LinearRaw | RawPhotometricInterpretation::BlackIsZero => {
            let mut channels: Vec<_> = image
                .camera
                .plane_color
                .colors
                .iter()
                .copied()
                .map(channel)
                .take(cpp)
                .collect();
            if channels.len() != cpp {
                channels = match cpp {
                    1 => vec![RawChannel::Unknown],
                    3 => vec![RawChannel::Red, RawChannel::Green, RawChannel::Blue],
                    4 => vec![
                        RawChannel::Red,
                        RawChannel::Green,
                        RawChannel::Blue,
                        RawChannel::Green,
                    ],
                    _ => {
                        return Err(capability(
                            RawCapabilityKind::Layout,
                            probe,
                            &image.make,
                            &image.model,
                            &image.camera.mode,
                            "linear channel mapping is unavailable",
                        ));
                    }
                };
            }
            Ok(RawPlaneLayout::Linear { channels })
        }
    }
}

fn channel(value: CFAColor) -> RawChannel {
    match value {
        CFAColor::RED => RawChannel::Red,
        CFAColor::GREEN => RawChannel::Green,
        CFAColor::BLUE => RawChannel::Blue,
        CFAColor::CYAN => RawChannel::Cyan,
        CFAColor::MAGENTA => RawChannel::Magenta,
        CFAColor::YELLOW => RawChannel::Yellow,
        CFAColor::WHITE => RawChannel::White,
        CFAColor::FUJI_GREEN => RawChannel::FujiGreen,
        CFAColor::END | CFAColor::UNKNOWN => RawChannel::Unknown,
    }
}

fn channel_from_index(value: u8) -> RawChannel {
    CFAColor::try_from(value).map_or(RawChannel::Unknown, channel)
}

fn color_matrices(
    matrices: std::collections::HashMap<Illuminant, Vec<f32>>,
) -> Result<Vec<RawColorMatrix>, RawDecodeError> {
    let mut output = Vec::with_capacity(matrices.len());
    for (illuminant, coefficients) in matrices {
        let rows = match coefficients.len() {
            9 => 3,
            12 => 4,
            _ => return Err(invalid(RawFrameValidationError::InvalidMatrix)),
        };
        output.push(RawColorMatrix {
            illuminant: convert_illuminant(illuminant),
            rows,
            columns: 3,
            coefficients,
        });
    }
    output.sort_by_key(|matrix| matrix.illuminant);
    Ok(output)
}

fn convert_illuminant(value: Illuminant) -> RawIlluminant {
    match value {
        Illuminant::Unknown => RawIlluminant::Unknown,
        Illuminant::Daylight => RawIlluminant::Daylight,
        Illuminant::Fluorescent => RawIlluminant::Fluorescent,
        Illuminant::Tungsten => RawIlluminant::Tungsten,
        Illuminant::Flash => RawIlluminant::Flash,
        Illuminant::FineWeather => RawIlluminant::FineWeather,
        Illuminant::CloudyWeather => RawIlluminant::CloudyWeather,
        Illuminant::Shade => RawIlluminant::Shade,
        Illuminant::DaylightFluorescent => RawIlluminant::DaylightFluorescent,
        Illuminant::DaylightWhiteFluorescent => RawIlluminant::DaylightWhiteFluorescent,
        Illuminant::CoolWhiteFluorescent => RawIlluminant::CoolWhiteFluorescent,
        Illuminant::WhiteFluorescent => RawIlluminant::WhiteFluorescent,
        Illuminant::A => RawIlluminant::A,
        Illuminant::B => RawIlluminant::B,
        Illuminant::C => RawIlluminant::C,
        Illuminant::D50 => RawIlluminant::D50,
        Illuminant::D55 => RawIlluminant::D55,
        Illuminant::D65 => RawIlluminant::D65,
        Illuminant::D75 => RawIlluminant::D75,
        Illuminant::IsoStudioTungsten => RawIlluminant::IsoStudioTungsten,
    }
}

fn white_balance(values: [f32; 4]) -> Result<Vec<Option<f32>>, RawDecodeError> {
    if values.iter().any(|value| value.is_infinite()) {
        return Err(invalid(RawFrameValidationError::NonFiniteMetadata));
    }
    Ok(values
        .into_iter()
        .map(|value| (value.is_finite() && value > 0.0).then_some(value))
        .collect())
}

fn opcode_descriptors(
    tags: &std::collections::HashMap<u16, Value>,
    limits: RawDecodeLimits,
) -> Result<Vec<RawOpcodeDescriptor>, RawDecodeError> {
    let mut output = Vec::new();
    for (tag, stage) in [
        (51_008_u16, RawOpcodeStage::BeforeDemosaic),
        (51_009_u16, RawOpcodeStage::AfterDemosaic),
        (51_022_u16, RawOpcodeStage::AfterGainMap),
    ] {
        let Some(value) = tags.get(&tag) else {
            continue;
        };
        let (Value::Byte(bytes) | Value::Undefined(bytes) | Value::Unknown(_, bytes)) = value
        else {
            return Err(invalid(RawFrameValidationError::MetadataByteLimit));
        };
        if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > limits.max_metadata_bytes {
            return Err(invalid(RawFrameValidationError::MetadataByteLimit));
        }
        output.push(RawOpcodeDescriptor {
            stage,
            byte_length: u32::try_from(bytes.len())
                .map_err(|_| invalid(RawFrameValidationError::MetadataByteLimit))?,
            sha256: Sha256::digest(bytes).into(),
        });
    }
    Ok(output)
}

fn preview_descriptors(source: &[u8], container: RawContainerKind) -> Vec<RawPreviewDescriptor> {
    if container != RawContainerKind::Raf || source.len() < 92 {
        return Vec::new();
    }
    let offset = u64::from(u32::from_be_bytes(
        source[84..88].try_into().expect("fixed range"),
    ));
    let byte_length = u64::from(u32::from_be_bytes(
        source[88..92].try_into().expect("fixed range"),
    ));
    if byte_length == 0
        || offset
            .checked_add(byte_length)
            .is_none_or(|end| end > u64::try_from(source.len()).unwrap_or(u64::MAX))
    {
        return Vec::new();
    }
    vec![RawPreviewDescriptor {
        kind: RawPreviewKind::FullSize,
        format: RawPreviewFormat::Jpeg,
        offset,
        byte_length,
        dimensions: None,
    }]
}

fn dimensions(width: usize, height: usize) -> Result<RawDimensions, RawDecodeError> {
    RawDimensions::new(
        u32::try_from(width).map_err(|_| invalid(RawFrameValidationError::ArithmeticOverflow))?,
        u32::try_from(height).map_err(|_| invalid(RawFrameValidationError::ArithmeticOverflow))?,
    )
    .map_err(invalid)
}

fn convert_rect(rect: Rect) -> Result<RawRect, RawDecodeError> {
    RawRect::new(
        u32::try_from(rect.p.x)
            .map_err(|_| invalid(RawFrameValidationError::ArithmeticOverflow))?,
        u32::try_from(rect.p.y)
            .map_err(|_| invalid(RawFrameValidationError::ArithmeticOverflow))?,
        u32::try_from(rect.d.w)
            .map_err(|_| invalid(RawFrameValidationError::ArithmeticOverflow))?,
        u32::try_from(rect.d.h)
            .map_err(|_| invalid(RawFrameValidationError::ArithmeticOverflow))?,
    )
    .map_err(invalid)
}

fn orientation(value: Orientation) -> RawOrientation {
    match value {
        Orientation::Normal => RawOrientation::Normal,
        Orientation::HorizontalFlip => RawOrientation::HorizontalFlip,
        Orientation::Rotate180 => RawOrientation::Rotate180,
        Orientation::VerticalFlip => RawOrientation::VerticalFlip,
        Orientation::Transpose => RawOrientation::Transpose,
        Orientation::Rotate90 => RawOrientation::Rotate90,
        Orientation::Transverse => RawOrientation::Transverse,
        Orientation::Rotate270 => RawOrientation::Rotate270,
        Orientation::Unknown => RawOrientation::Unknown,
    }
}

fn map_backend_error(error: RawlerError, probe: &RawContainerProbe) -> RawDecodeError {
    match error {
        RawlerError::Unsupported {
            what,
            model,
            make,
            mode,
        } => capability(
            classify_capability(&what),
            probe,
            &make,
            &model,
            &mode,
            &what,
        ),
        RawlerError::DecoderFailed(message) => {
            let lowered = message.to_ascii_lowercase();
            if ["unsupported", "don't know", "unknown camera", "no decoder"]
                .iter()
                .any(|needle| lowered.contains(needle))
            {
                capability(
                    classify_capability(&message),
                    probe,
                    probe.evidence.camera.maker.as_deref().unwrap_or_default(),
                    probe.evidence.camera.model.as_deref().unwrap_or_default(),
                    "",
                    &message,
                )
            } else {
                RawDecodeError::Malformed {
                    container: Some(probe.container),
                    message: safe_text(&message),
                }
            }
        }
    }
}

fn classify_capability(message: &str) -> RawCapabilityKind {
    let message = message.to_ascii_lowercase();
    if message.contains("compression") || message.contains("compressed") {
        RawCapabilityKind::Compression
    } else if message.contains("bps") || message.contains("bit") || message.contains("cpp") {
        RawCapabilityKind::Layout
    } else if message.contains("camera") || message.contains("model") {
        RawCapabilityKind::Camera
    } else {
        RawCapabilityKind::Container
    }
}

fn capability(
    missing: RawCapabilityKind,
    probe: &RawContainerProbe,
    maker: &str,
    model: &str,
    mode: &str,
    detail: &str,
) -> RawDecodeError {
    RawDecodeError::Capability(RawCapabilityError {
        missing,
        container: Some(probe.container),
        maker: safe_text(maker),
        model: safe_text(model),
        mode: safe_text(mode),
        detail: safe_text(detail),
    })
}

fn invalid(error: RawFrameValidationError) -> RawDecodeError {
    RawDecodeError::InvalidFrame(error)
}

fn safe_text(value: &str) -> String {
    value
        .chars()
        .filter(|character| !character.is_control())
        .take(256)
        .collect()
}

fn check_cancel(token: &RawCancellationToken) -> Result<(), RawDecodeError> {
    if token.is_cancelled() {
        Err(RawDecodeError::Cancelled)
    } else {
        Ok(())
    }
}

pub(super) fn probe_legacy(
    bytes: &[u8],
    limits: DecodeLimits,
) -> Result<ImageProbe, ImageInputError> {
    let request = RawDecodeRequest::new(raw_limits(limits)?);
    let header = RawlerRawDecoder::new()
        .probe_bytes(bytes, &request)
        .map_err(raw_input_error)?;
    let dimensions = oriented_header_dimensions(&header)?;
    Ok(ImageProbe::new(InputFormat::Raw, dimensions))
}

fn oriented_header_dimensions(header: &RawHeader) -> Result<ImageDimensions, ImageInputError> {
    let transpose = matches!(
        header.orientation,
        RawOrientation::Transpose
            | RawOrientation::Rotate90
            | RawOrientation::Transverse
            | RawOrientation::Rotate270
    );
    ImageDimensions::new(
        if transpose {
            header.dimensions.height
        } else {
            header.dimensions.width
        },
        if transpose {
            header.dimensions.width
        } else {
            header.dimensions.height
        },
    )
    .map_err(|_| ImageInputError::ArithmeticOverflow)
}

pub(super) fn decode_legacy_preview(
    bytes: &[u8],
    limits: DecodeLimits,
) -> Result<DecodedImage, ImageInputError> {
    let request = RawDecodeRequest::new(raw_limits(limits)?);
    let result = RawlerRawDecoder::new()
        .decode_bytes(bytes, &request)
        .map_err(raw_input_error)?;
    sensor_compatibility_preview(&result.frame, limits)
}

fn raw_limits(limits: DecodeLimits) -> Result<RawDecodeLimits, ImageInputError> {
    RawDecodeLimits::new(
        limits.max_source_bytes(),
        limits.max_width(),
        limits.max_height(),
        limits.max_pixel_count(),
        limits.max_decoded_bytes(),
    )
    .map_err(|_| ImageInputError::ArithmeticOverflow)
}

fn oriented_dimensions(parts: &RawFrameParts) -> Result<ImageDimensions, ImageInputError> {
    let dimensions = parts.planes[0].dimensions;
    let transpose = matches!(
        parts.orientation,
        RawOrientation::Transpose
            | RawOrientation::Rotate90
            | RawOrientation::Transverse
            | RawOrientation::Rotate270
    );
    ImageDimensions::new(
        if transpose {
            dimensions.height
        } else {
            dimensions.width
        },
        if transpose {
            dimensions.width
        } else {
            dimensions.height
        },
    )
    .map_err(|_| ImageInputError::ArithmeticOverflow)
}

fn sensor_compatibility_preview(
    frame: &RawFrame,
    limits: DecodeLimits,
) -> Result<DecodedImage, ImageInputError> {
    let parts = frame.parts();
    let plane = &parts.planes[0];
    let dimensions = oriented_dimensions(parts)?;
    let output_bytes = dimensions
        .decoded_byte_count()
        .map_err(|_| ImageInputError::ArithmeticOverflow)?;
    if output_bytes > limits.max_decoded_bytes() {
        return Err(ImageInputError::DecodedByteLimit {
            actual: output_bytes,
            limit: limits.max_decoded_bytes(),
        });
    }
    let capacity =
        usize::try_from(output_bytes).map_err(|_| ImageInputError::ArithmeticOverflow)?;
    let mut output = Vec::new();
    output
        .try_reserve_exact(capacity)
        .map_err(|_| ImageInputError::AllocationFailure)?;
    let shift = u32::from(parts.bit_depth.saturating_sub(8));
    let cpp = usize::from(plane.channels_per_pixel);
    for pixel in plane.samples.chunks_exact(cpp) {
        let channels = if cpp >= 3 {
            [pixel[0], pixel[1], pixel[2]]
        } else {
            [pixel[0]; 3]
        };
        for value in channels {
            output.push(u8::try_from((u32::from(value) >> shift).min(255)).unwrap_or(255));
        }
        output.push(255);
    }
    DecodedImage::new_with_color_encoding(dimensions, output, ColorEncoding::Unspecified).map_err(
        |error| match error {
            DecodedImageError::ArithmeticOverflow => ImageInputError::ArithmeticOverflow,
            DecodedImageError::ByteLengthMismatch { expected, actual } => {
                ImageInputError::DecodedBufferInvariant { expected, actual }
            }
        },
    )
}

fn raw_input_error(error: RawDecodeError) -> ImageInputError {
    match error {
        RawDecodeError::Source(RawSourceError::TooLarge { actual, limit }) => {
            ImageInputError::SourceTooLarge { limit, actual }
        }
        RawDecodeError::UnsupportedSignature { signature } => {
            ImageInputError::UnsupportedSignature { signature }
        }
        RawDecodeError::InvalidFrame(RawFrameValidationError::DimensionLimit { width, .. }) => {
            ImageInputError::WidthLimit {
                actual: width,
                limit: width.saturating_sub(1),
            }
        }
        RawDecodeError::InvalidFrame(RawFrameValidationError::PixelLimit { actual, limit }) => {
            ImageInputError::PixelLimit { actual, limit }
        }
        RawDecodeError::InvalidFrame(RawFrameValidationError::DecodedByteLimit {
            actual,
            limit,
        }) => ImageInputError::DecodedByteLimit { actual, limit },
        RawDecodeError::Source(RawSourceError::AllocationFailure) => {
            ImageInputError::AllocationFailure
        }
        RawDecodeError::InvalidFrame(RawFrameValidationError::ArithmeticOverflow)
        | RawDecodeError::Source(RawSourceError::LengthConversion) => {
            ImageInputError::ArithmeticOverflow
        }
        other => ImageInputError::MalformedInput {
            format: InputFormat::Raw,
            message: safe_text(&other.to_string()),
        },
    }
}
