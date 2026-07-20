use super::{BufferAlignment, HostPoolError};

#[repr(align(64))]
#[derive(Debug, Clone, Copy)]
pub(super) struct Aligned64([u8; 64]);

#[repr(align(128))]
#[derive(Debug, Clone, Copy)]
pub(super) struct Aligned128([u8; 128]);

#[repr(align(256))]
#[derive(Debug, Clone, Copy)]
pub(super) struct Aligned256([u8; 256]);

#[repr(align(512))]
#[derive(Debug, Clone, Copy)]
pub(super) struct Aligned512([u8; 512]);

impl Default for Aligned64 {
    fn default() -> Self {
        Self([0; 64])
    }
}
impl Default for Aligned128 {
    fn default() -> Self {
        Self([0; 128])
    }
}
impl Default for Aligned256 {
    fn default() -> Self {
        Self([0; 256])
    }
}
impl Default for Aligned512 {
    fn default() -> Self {
        Self([0; 512])
    }
}

#[derive(Debug)]
pub(super) enum AlignedStorage {
    A64(Vec<Aligned64>),
    A128(Vec<Aligned128>),
    A256(Vec<Aligned256>),
    A512(Vec<Aligned512>),
}

impl AlignedStorage {
    pub(super) fn allocate(
        alignment: BufferAlignment,
        capacity: usize,
    ) -> Result<Self, HostPoolError> {
        let alignment = alignment.bytes();
        match alignment {
            64 => allocate_blocks::<Aligned64>(capacity, 64).map(Self::A64),
            128 => allocate_blocks::<Aligned128>(capacity, 128).map(Self::A128),
            256 => allocate_blocks::<Aligned256>(capacity, 256).map(Self::A256),
            512 => allocate_blocks::<Aligned512>(capacity, 512).map(Self::A512),
            requested => Err(HostPoolError::UnsupportedAlignment { requested }),
        }
    }

    pub(super) fn capacity(&self) -> usize {
        match self {
            Self::A64(blocks) => blocks.len() * 64,
            Self::A128(blocks) => blocks.len() * 128,
            Self::A256(blocks) => blocks.len() * 256,
            Self::A512(blocks) => blocks.len() * 512,
        }
    }

    pub(super) fn zero(&mut self) {
        match self {
            Self::A64(blocks) => blocks.fill(Aligned64([0; 64])),
            Self::A128(blocks) => blocks.fill(Aligned128([0; 128])),
            Self::A256(blocks) => blocks.fill(Aligned256([0; 256])),
            Self::A512(blocks) => blocks.fill(Aligned512([0; 512])),
        }
    }

    pub(super) fn read(&self, offset: usize) -> Option<u8> {
        match self {
            Self::A64(blocks) => read_blocks(blocks, offset, 64, |block| &block.0),
            Self::A128(blocks) => read_blocks(blocks, offset, 128, |block| &block.0),
            Self::A256(blocks) => read_blocks(blocks, offset, 256, |block| &block.0),
            Self::A512(blocks) => read_blocks(blocks, offset, 512, |block| &block.0),
        }
    }

    pub(super) fn write(&mut self, offset: usize, value: u8) -> bool {
        match self {
            Self::A64(blocks) => write_blocks(blocks, offset, 64, value, |block| &mut block.0),
            Self::A128(blocks) => write_blocks(blocks, offset, 128, value, |block| &mut block.0),
            Self::A256(blocks) => write_blocks(blocks, offset, 256, value, |block| &mut block.0),
            Self::A512(blocks) => write_blocks(blocks, offset, 512, value, |block| &mut block.0),
        }
    }
}

fn allocate_blocks<T: Copy + Default>(
    capacity: usize,
    block_bytes: usize,
) -> Result<Vec<T>, HostPoolError> {
    let count = capacity.div_ceil(block_bytes);
    let mut blocks = Vec::new();
    blocks
        .try_reserve_exact(count)
        .map_err(|_| HostPoolError::AllocationFailed)?;
    blocks.resize(count, T::default());
    Ok(blocks)
}

fn read_blocks<T>(
    blocks: &[T],
    offset: usize,
    block_bytes: usize,
    bytes: impl Fn(&T) -> &[u8],
) -> Option<u8> {
    let block = blocks.get(offset / block_bytes)?;
    bytes(block).get(offset % block_bytes).copied()
}

fn write_blocks<T>(
    blocks: &mut [T],
    offset: usize,
    block_bytes: usize,
    value: u8,
    mut bytes: impl FnMut(&mut T) -> &mut [u8],
) -> bool {
    let Some(block) = blocks.get_mut(offset / block_bytes) else {
        return false;
    };
    let Some(byte) = bytes(block).get_mut(offset % block_bytes) else {
        return false;
    };
    *byte = value;
    true
}
