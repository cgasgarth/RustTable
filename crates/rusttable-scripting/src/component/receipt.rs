use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum ReceiptStatus {
    Completed,
    Rejected,
    Cancelled,
    TimedOut,
    Trapped,
    Quarantined,
}

#[must_use]
pub fn digest(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("sha256:{:x}", hasher.finalize())
}
