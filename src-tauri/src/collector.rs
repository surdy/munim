//! Port of the original `collect-usage.js` (BUILD_SPEC §4). Pure file I/O + parsing.
//!
//! This is the app's whole backend. Keep it testable in isolation: golden-test the cost
//! math and each parser against fixtures + the original Node output (BUILD_SPEC §8, §9).
//!
//! Everything below is a skeleton. Implement per BUILD_SPEC §4.1–§4.8.

use serde::{Deserialize, Serialize};

/// Which provider a record belongs to (BUILD_SPEC §4.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    Claude,
    Codex,
}

/// One aggregated (source, file, date) record — a "session-day" (BUILD_SPEC §4.2).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRecord {
    pub date: String, // local YYYY-MM-DD
    pub time: String, // earliest time seen
    pub provider: Provider,
    pub source: String,
    pub file: String,
    pub cost: f64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
}

/// Resolve per-OS source directories to scan (BUILD_SPEC §4.1).
/// macOS uses ~/Library/Application Support/...; Linux uses XDG ~/.config/...; dotfile
/// roots (~/.claude, ~/.codex, ...) are shared. Write it so Windows can be added later.
pub fn source_dirs() -> Vec<std::path::PathBuf> {
    // TODO(spec §4.1): build the full candidate list per platform; caller skips missing.
    Vec::new()
}

/// The app-data dir where data.js / sessions-cache.json / scan-index.json / settings.json
/// live (BUILD_SPEC §4.7). Prefer Tauri's `app.path().app_data_dir()`; this is a fallback.
pub fn app_data_dir() -> Option<std::path::PathBuf> {
    dirs::data_dir().map(|d| d.join("munim"))
}

/// Full incremental collect (BUILD_SPEC §4). Uses scan-index.json to skip unchanged files,
/// preserves imported/historical records whose files no longer resolve, writes caches
/// atomically, and returns the records + a summary.
pub fn collect() -> Result<Vec<SessionRecord>, String> {
    // TODO(spec §4.2–§4.7): scan source_dirs(), dispatch per-format parsers, aggregate per
    //   (source,file,date), compute cost via crate::pricing, merge with cache, persist.
    Ok(Vec::new())
}

// TODO(spec §4.4): parsers — parse_claude_format, parse_openclaw_format, parse_aider_format,
//   parse_continue_format, parse_codex_format. Codex diffs cumulative token usage and reads
//   the model from turn_context events; Aider timestamps are Unix epoch seconds.
