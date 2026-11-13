//! File collision choices are supplied by the caller; credentials are never involved.
use crate::TransferDirection;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConflictAction {
    Overwrite,
    Skip,
    Cancel,
}
#[derive(Clone, Debug)]
pub struct TransferConflict {
    pub direction: TransferDirection,
    pub source_path: String,
    pub destination_path: String,
    pub source_size: u64,
    pub destination_size: u64,
    pub source_modified: Option<u64>,
    pub destination_modified: Option<u64>,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TransferSummary {
    pub transferred_files: u64,
    pub skipped_files: u64,
}
impl TransferSummary {
    pub(crate) fn add(&mut self, other: Self) {
        self.transferred_files += other.transferred_files;
        self.skipped_files += other.skipped_files;
    }
}
