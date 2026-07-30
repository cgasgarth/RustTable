/// Lossless history rows read from Darktable's library database.
///
/// These types intentionally contain no SQLite or operation-specific logic;
/// the compatibility crate owns validation and projection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawHistoryRow {
    pub source_row: u64,
    pub image_id: i64,
    pub num: i64,
    pub module: Option<i64>,
    pub operation: Option<Vec<u8>>,
    pub operation_params: Option<Vec<u8>>,
    pub enabled: Option<i64>,
    pub blend_params: Option<Vec<u8>>,
    pub blend_version: Option<i64>,
    pub multi_priority: Option<i64>,
    pub multi_name: Option<Vec<u8>>,
    pub multi_name_hand_edited: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawImageHistoryRow {
    pub source_row: u64,
    pub image_id: i64,
    pub history_end: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawModuleOrderRow {
    pub source_row: u64,
    pub image_id: i64,
    pub version: Option<i64>,
    pub operation_list: Option<Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawHistoryHashRow {
    pub source_row: u64,
    pub image_id: i64,
    pub basic_hash: Option<Vec<u8>>,
    pub auto_hash: Option<Vec<u8>>,
    pub current_hash: Option<Vec<u8>>,
    pub mipmap_hash: Option<Vec<u8>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HistoryRows {
    pub history: Vec<RawHistoryRow>,
    pub images: Vec<RawImageHistoryRow>,
    pub module_orders: Vec<RawModuleOrderRow>,
    pub hashes: Vec<RawHistoryHashRow>,
}
