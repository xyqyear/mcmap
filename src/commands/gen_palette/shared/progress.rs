// Shared progress-event types used by both modern and legacy pack loaders.

use std::path::PathBuf;

/// One entry per archive processed by `load_packs` / `load_texture_packs`.
/// `error` is `Some` when the archive failed to load (in which case the
/// added-counts are zero); otherwise `None`.
///
/// `index` / `total` are 1-based position and total-archive count so
/// consumers can render "loading pack 3 / 7" without tracking state.
pub struct PackLoadReport {
    pub path: PathBuf,
    pub index: usize,
    pub total: usize,
    pub blockstates_added: usize,
    pub models_added: usize,
    pub textures_added: usize,
    pub error: Option<String>,
}
