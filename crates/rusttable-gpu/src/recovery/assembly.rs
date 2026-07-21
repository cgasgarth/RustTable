use std::collections::{BTreeMap, BTreeSet};

use sha2::{Digest, Sha256};

use super::{AttemptId, RecoveryContext, RecoveryError};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CoverageRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl CoverageRect {
    pub const fn new(x: u32, y: u32, width: u32, height: u32) -> Result<Self, CoverageError> {
        if width == 0
            || height == 0
            || x.checked_add(width).is_none()
            || y.checked_add(height).is_none()
        {
            return Err(CoverageError::InvalidRectangle);
        }
        Ok(Self {
            x,
            y,
            width,
            height,
        })
    }

    #[must_use]
    pub const fn right(self) -> u32 {
        self.x.saturating_add(self.width)
    }

    #[must_use]
    pub const fn bottom(self) -> u32 {
        self.y.saturating_add(self.height)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AssemblyTile {
    pub id: u32,
    pub coverage: CoverageRect,
    pub output_bytes: u64,
}

impl AssemblyTile {
    pub const fn new(
        id: u32,
        coverage: CoverageRect,
        output_bytes: u64,
    ) -> Result<Self, CoverageError> {
        if output_bytes == 0 {
            return Err(CoverageError::InvalidOutputSize);
        }
        Ok(Self {
            id,
            coverage,
            output_bytes,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssemblyPlan {
    context: RecoveryContext,
    width: u32,
    height: u32,
    tiles: Vec<AssemblyTile>,
    pub(super) coverage: CoverageReceipt,
}

impl AssemblyPlan {
    pub fn new(
        context: RecoveryContext,
        width: u32,
        height: u32,
        mut tiles: Vec<AssemblyTile>,
    ) -> Result<Self, CoverageError> {
        if width == 0 || height == 0 || tiles.is_empty() {
            return Err(CoverageError::InvalidBounds);
        }
        let mut ids = BTreeSet::new();
        for tile in &tiles {
            if tile.coverage.width == 0
                || tile.coverage.height == 0
                || tile.output_bytes == 0
                || tile.coverage.x.checked_add(tile.coverage.width).is_none()
                || tile.coverage.y.checked_add(tile.coverage.height).is_none()
            {
                return Err(CoverageError::InvalidRectangle);
            }
            if !ids.insert(tile.id) {
                return Err(CoverageError::DuplicateTile(tile.id));
            }
            if tile.coverage.right() > width || tile.coverage.bottom() > height {
                return Err(CoverageError::OutOfBounds(tile.id));
            }
        }
        tiles.sort_by_key(|tile| (tile.coverage.y, tile.coverage.x, tile.id));
        validate_exact_cover(width, height, &tiles)?;
        let coverage = CoverageReceipt::for_plan(context, width, height, &tiles);
        Ok(Self {
            context,
            width,
            height,
            tiles,
            coverage,
        })
    }

    #[must_use]
    pub const fn context(&self) -> RecoveryContext {
        self.context
    }

    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }

    #[must_use]
    pub fn tiles(&self) -> &[AssemblyTile] {
        &self.tiles
    }

    #[must_use]
    pub const fn coverage(&self) -> &CoverageReceipt {
        &self.coverage
    }

    fn tile(&self, id: u32) -> Option<AssemblyTile> {
        self.tiles.iter().copied().find(|tile| tile.id == id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CoverageReceipt {
    pub context: RecoveryContext,
    pub width: u32,
    pub height: u32,
    pub tile_count: usize,
    pub pixels: u64,
    pub identity: [u8; 32],
}

impl CoverageReceipt {
    fn for_plan(context: RecoveryContext, width: u32, height: u32, tiles: &[AssemblyTile]) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(b"rusttable.gpu.coverage.v1");
        hasher.update(context.snapshot.bytes());
        hasher.update(context.plan.bytes());
        hasher.update(context.generation.value().to_le_bytes());
        hasher.update(width.to_le_bytes());
        hasher.update(height.to_le_bytes());
        for tile in tiles {
            hasher.update(tile.id.to_le_bytes());
            hasher.update(tile.coverage.x.to_le_bytes());
            hasher.update(tile.coverage.y.to_le_bytes());
            hasher.update(tile.coverage.width.to_le_bytes());
            hasher.update(tile.coverage.height.to_le_bytes());
            hasher.update(tile.output_bytes.to_le_bytes());
        }
        Self {
            context,
            width,
            height,
            tile_count: tiles.len(),
            pixels: u64::from(width) * u64::from(height),
            identity: hasher.finalize().into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OutputFragment {
    pub attempt: AttemptId,
    pub context: RecoveryContext,
    pub tile_id: u32,
    pub coverage: CoverageRect,
    pub output_bytes: u64,
    pub output_identity: [u8; 32],
}

impl OutputFragment {
    #[must_use]
    pub const fn new(
        attempt: AttemptId,
        context: RecoveryContext,
        tile_id: u32,
        coverage: CoverageRect,
        output_bytes: u64,
        output_identity: [u8; 32],
    ) -> Self {
        Self {
            attempt,
            context,
            tile_id,
            coverage,
            output_bytes,
            output_identity,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssemblyReceipt {
    pub context: RecoveryContext,
    pub attempt: AttemptId,
    pub coverage: CoverageReceipt,
    pub output_identity: [u8; 32],
    pub fragment_count: usize,
    pub output_bytes: u64,
}

#[derive(Debug)]
pub(super) struct Assembly {
    pub(super) plan: AssemblyPlan,
    attempt: AttemptId,
    fragments: BTreeMap<u32, OutputFragment>,
}

impl Assembly {
    pub(super) fn new(plan: AssemblyPlan, attempt: AttemptId) -> Self {
        Self {
            plan,
            attempt,
            fragments: BTreeMap::new(),
        }
    }

    pub(super) const fn plan(&self) -> &AssemblyPlan {
        &self.plan
    }

    pub(super) fn accept(&mut self, fragment: OutputFragment) -> Result<(), RecoveryError> {
        if fragment.attempt != self.attempt {
            return Err(RecoveryError::StaleAttempt {
                expected: self.attempt,
                actual: fragment.attempt,
            });
        }
        if fragment.context != self.plan.context() {
            return Err(RecoveryError::StaleContext);
        }
        let expected = self
            .plan
            .tile(fragment.tile_id)
            .ok_or(RecoveryError::UnknownTile(fragment.tile_id))?;
        if expected.coverage != fragment.coverage {
            return Err(RecoveryError::Coverage(CoverageError::TileMismatch(
                fragment.tile_id,
            )));
        }
        if expected.output_bytes != fragment.output_bytes {
            return Err(RecoveryError::OutputSizeMismatch {
                tile: fragment.tile_id,
                expected: expected.output_bytes,
                actual: fragment.output_bytes,
            });
        }
        if self.fragments.contains_key(&fragment.tile_id) {
            return Err(RecoveryError::DuplicateOutput(fragment.tile_id));
        }
        self.fragments.insert(fragment.tile_id, fragment);
        Ok(())
    }

    pub(super) fn finish(&self) -> Result<AssemblyReceipt, RecoveryError> {
        if self.fragments.len() != self.plan.tiles().len() {
            return Err(RecoveryError::Coverage(CoverageError::Incomplete {
                expected: self.plan.tiles().len(),
                actual: self.fragments.len(),
            }));
        }
        let mut hasher = Sha256::new();
        hasher.update(b"rusttable.gpu.assembly.v1");
        hasher.update(self.plan.coverage.identity);
        let mut output_bytes = 0_u64;
        for tile in self.plan.tiles() {
            let fragment = self
                .fragments
                .get(&tile.id)
                .ok_or(RecoveryError::UnknownTile(tile.id))?;
            hasher.update(tile.id.to_le_bytes());
            hasher.update(fragment.output_identity);
            output_bytes = output_bytes
                .checked_add(fragment.output_bytes)
                .ok_or(RecoveryError::Coverage(CoverageError::OutputSizeOverflow))?;
        }
        Ok(AssemblyReceipt {
            context: self.plan.context(),
            attempt: self.attempt,
            coverage: self.plan.coverage.clone(),
            output_identity: hasher.finalize().into(),
            fragment_count: self.fragments.len(),
            output_bytes,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoverageError {
    InvalidBounds,
    InvalidRectangle,
    InvalidOutputSize,
    OutputSizeOverflow,
    DuplicateTile(u32),
    OutOfBounds(u32),
    Gap { x: u32, y: u32 },
    Overlap { x: u32, y: u32 },
    TileMismatch(u32),
    Incomplete { expected: usize, actual: usize },
}

impl std::fmt::Display for CoverageError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "GPU coverage error: {self:?}")
    }
}

impl std::error::Error for CoverageError {}

fn validate_exact_cover(
    width: u32,
    height: u32,
    tiles: &[AssemblyTile],
) -> Result<(), CoverageError> {
    let mut y_edges = vec![0, height];
    y_edges.extend(
        tiles
            .iter()
            .flat_map(|tile| [tile.coverage.y, tile.coverage.bottom()]),
    );
    y_edges.sort_unstable();
    y_edges.dedup();
    for pair in y_edges.windows(2) {
        let y = pair[0];
        let next_y = pair[1];
        if y == next_y {
            continue;
        }
        let mut active = tiles
            .iter()
            .filter(|tile| tile.coverage.y <= y && tile.coverage.bottom() >= next_y)
            .collect::<Vec<_>>();
        active.sort_by_key(|tile| (tile.coverage.x, tile.id));
        let mut cursor = 0;
        for tile in active {
            if tile.coverage.x < cursor {
                return Err(CoverageError::Overlap {
                    x: tile.coverage.x,
                    y,
                });
            }
            if tile.coverage.x > cursor {
                return Err(CoverageError::Gap { x: cursor, y });
            }
            cursor = tile.coverage.right();
        }
        if cursor < width {
            return Err(CoverageError::Gap { x: cursor, y });
        }
    }
    Ok(())
}
