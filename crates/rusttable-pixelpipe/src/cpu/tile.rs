use super::{
    CpuPixelpipeError, CpuTileAssemblyError, RgbaF32Descriptor, RgbaF32Image, RgbaF32Pixel,
};

pub(super) fn assemble_tile(
    assembled: &mut [RgbaF32Pixel],
    output_descriptor: RgbaF32Descriptor,
    tile: crate::CpuPixelpipeTile,
    tile_output: &RgbaF32Image,
) -> Result<(), CpuPixelpipeError> {
    if tile_output.descriptor().dimensions() != tile.dimensions() {
        return Err(CpuPixelpipeError::TileAssembly {
            source: CpuTileAssemblyError::TileOutputDimensionsMismatch,
        });
    }
    for local_y in 0..tile.dimensions().height() {
        let output_y =
            tile.origin_y()
                .checked_add(local_y)
                .ok_or(CpuPixelpipeError::TileAssembly {
                    source: CpuTileAssemblyError::PixelIndexOverflow,
                })?;
        let destination_start = pixel_index(output_descriptor, tile.origin_x(), output_y)?;
        let destination_end = checked_row_end(destination_start, tile.dimensions().width())?;
        let destination = assembled
            .get_mut(destination_start..destination_end)
            .ok_or(CpuPixelpipeError::TileAssembly {
                source: CpuTileAssemblyError::DestinationRowOutsideOutput,
            })?;
        let source_start = pixel_index(tile_output.descriptor(), 0, local_y)?;
        let source_end = checked_row_end(source_start, tile.dimensions().width())?;
        let source = tile_output.pixels().get(source_start..source_end).ok_or(
            CpuPixelpipeError::TileAssembly {
                source: CpuTileAssemblyError::SourceRowOutsideInput,
            },
        )?;
        destination.copy_from_slice(source);
    }
    Ok(())
}

pub(super) fn tile_pixel_count(tile: crate::CpuPixelpipeTile) -> Result<usize, CpuPixelpipeError> {
    usize::try_from(tile.dimensions().pixel_count()).map_err(|_| CpuPixelpipeError::TileAssembly {
        source: CpuTileAssemblyError::PixelIndexExceedsPlatform {
            index: tile.dimensions().pixel_count(),
        },
    })
}

pub(super) fn pixel_index(
    descriptor: RgbaF32Descriptor,
    x: u32,
    y: u32,
) -> Result<usize, CpuPixelpipeError> {
    let offset = u64::from(y)
        .checked_mul(u64::from(descriptor.dimensions().width()))
        .and_then(|row_offset| row_offset.checked_add(u64::from(x)))
        .ok_or(CpuPixelpipeError::TileAssembly {
            source: CpuTileAssemblyError::PixelIndexOverflow,
        })?;
    usize::try_from(offset).map_err(|_| CpuPixelpipeError::TileAssembly {
        source: CpuTileAssemblyError::PixelIndexExceedsPlatform { index: offset },
    })
}

pub(super) fn checked_row_end(start: usize, width: u32) -> Result<usize, CpuPixelpipeError> {
    let width = usize::try_from(width).map_err(|_| CpuPixelpipeError::TileAssembly {
        source: CpuTileAssemblyError::PixelIndexExceedsPlatform {
            index: u64::from(width),
        },
    })?;
    start
        .checked_add(width)
        .ok_or(CpuPixelpipeError::TileAssembly {
            source: CpuTileAssemblyError::RowEndOverflow,
        })
}
