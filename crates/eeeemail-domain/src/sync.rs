use serde::{Deserialize, Serialize};

/// An opaque per-folder sync checkpoint.
///
/// Every provider advances state differently — Graph uses a `deltaLink`, Gmail
/// uses `historyId`, IMAP uses `UIDVALIDITY` + `UIDNEXT` (+ `MODSEQ` with
/// CONDSTORE). Storing `cursor_type` alongside the value keeps the storage layer
/// from assuming `nextPageToken` semantics everywhere.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncCursor {
    pub provider: String,
    pub account_id: String,
    pub folder_id: String,
    /// e.g. `uid`, `delta_link`, `history_id`.
    pub cursor_type: String,
    pub cursor: String,
}

/// A flag-only change discovered during incremental sync.
#[derive(Debug, Clone)]
pub struct FlagUpdate {
    pub remote_id: String,
    pub is_read: bool,
    pub is_starred: bool,
}

/// Outcome of syncing one folder.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SyncResult {
    pub folder_id: String,
    pub folder_name: String,
    /// Messages inserted or updated locally.
    pub upserted: usize,
    /// Messages that disappeared remotely.
    pub removed: usize,
    /// Messages that changed flags only.
    pub flagged: usize,
    pub cursor: Option<SyncCursor>,
    pub has_more: bool,
}

impl SyncResult {
    pub fn new(folder_id: &str) -> Self {
        Self {
            folder_id: folder_id.to_string(),
            ..Default::default()
        }
    }

    pub fn total_touched(&self) -> usize {
        self.upserted + self.removed + self.flagged
    }
}
