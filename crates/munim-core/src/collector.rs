//! Usage collector — port of the original `collect-usage.js` (BUILD_SPEC §4).
//!
//! Covers all sources (Claude Code/Desktop, Cursor, Windsurf, Cline, Roo, OpenClaw,
//! Aider, Continue, Codex) with per-format parsers, plus the incremental-scan cache
//! (BUILD_SPEC §4.7): unchanged files are skipped via an mtime/size fingerprint, and
//! historical/imported records whose files no longer resolve are preserved. Cache
//! persistence lives in `crate::cache`.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Local, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use walkdir::WalkDir;

use crate::pricing::{self, Pricing};

/// Provider a record belongs to. Serializes lowercase to match the dashboard payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    Claude,
    Codex,
}

/// One aggregated (source, file, date) record — a "session-day" (BUILD_SPEC §4.2).
/// Field names/casing match the original JSON so the ported dashboard reads it unchanged.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRecord {
    pub date: String,
    pub time: String,
    pub provider: Provider,
    pub source: String,
    pub file: String,
    pub cost: f64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub reasoning_tokens: Option<u64>,
    #[serde(rename = "filePath", skip_serializing_if = "Option::is_none", default)]
    pub file_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub title: Option<String>,
    #[serde(rename = "sessionId", skip_serializing_if = "Option::is_none", default)]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub cwd: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProviderTotals {
    pub claude: f64,
    pub codex: f64,
}

/// Summary payload (BUILD_SPEC §4.6). `totals` carries per-source costs plus `grand_total`;
/// `session_counts` carries per-source counts plus `total`.
#[derive(Debug, Clone, Serialize)]
pub struct Summary {
    pub generated_at: String,
    pub today: String,
    pub current_month: String,
    pub totals: BTreeMap<String, f64>,
    pub provider_totals: ProviderTotals,
    pub today_cost: f64,
    pub month_cost: f64,
    pub session_counts: BTreeMap<String, u64>,
}

/// What the dashboard consumes: the summary plus the three session buckets that map to
/// `window.__CLAUDE_SESSIONS__` / `__CODEX_SESSIONS__` / `__OPENCLAW_SESSIONS__`.
#[derive(Debug, Clone, Serialize)]
pub struct CollectOutput {
    pub summary: Summary,
    pub claude: Vec<SessionRecord>,
    pub codex: Vec<SessionRecord>,
    pub openclaw: Vec<SessionRecord>,
}

/// Per-file fingerprint for the incremental scan index (BUILD_SPEC §4.7).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Fingerprint {
    pub mtime: i64, // epoch millis
    pub size: u64,
}

/// path → fingerprint. BTreeMap for deterministic serialization.
pub type ScanIndex = BTreeMap<String, Fingerprint>;

/// Inputs carried over from the previous run (BUILD_SPEC §4.7).
#[derive(Debug, Clone, Default)]
pub struct Caches {
    pub scan_index: ScanIndex,
    pub cached_sessions: Vec<SessionRecord>,
}

impl Caches {
    pub fn empty() -> Self {
        Self::default()
    }
}

/// Scan counters for logging/telemetry.
#[derive(Debug, Clone, Copy, Default)]
pub struct ScanStats {
    pub parsed: usize,
    pub skipped: usize,
    pub preserved: usize,
}

/// Result of a collect run: the dashboard payload plus what to persist for next time.
#[derive(Debug, Clone)]
pub struct CollectResult {
    pub output: CollectOutput,
    pub sessions: Vec<SessionRecord>,
    pub scan_index: ScanIndex,
    pub stats: ScanStats,
}

// ─── date/time helpers (port of toLocalDate/toLocalTime/parseTimestamp) ───

fn to_local_date(ms: i64) -> Option<String> {
    Local
        .timestamp_millis_opt(ms)
        .single()
        .map(|dt| dt.format("%Y-%m-%d").to_string())
}

fn to_local_time(ms: i64) -> Option<String> {
    Local
        .timestamp_millis_opt(ms)
        .single()
        .map(|dt| dt.format("%H:%M").to_string())
}

/// Accepts an epoch-ms number or an ISO string (BUILD_SPEC §4.3).
fn parse_timestamp_ms(v: Option<&Value>) -> Option<i64> {
    match v {
        Some(Value::Number(n)) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)),
        Some(Value::String(s)) => DateTime::parse_from_rfc3339(s.trim())
            .ok()
            .map(|dt| dt.timestamp_millis()),
        _ => None,
    }
}

fn round(x: f64, dp: i32) -> f64 {
    let f = 10f64.powi(dp);
    (x * f).round() / f
}

fn now_local_date() -> String {
    Local::now().format("%Y-%m-%d").to_string()
}

// ─── file discovery ───

/// Recursively collect `*.jsonl` files, skipping `.git*` dirs and any filename containing
/// "audit", capped at depth 10 (port of findJsonl).
fn find_jsonl(root: &Path) -> Vec<PathBuf> {
    WalkDir::new(root)
        .max_depth(10)
        .into_iter()
        .filter_entry(|e| {
            !(e.file_type().is_dir() && e.file_name().to_string_lossy().starts_with(".git"))
        })
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .map(|e| e.into_path())
        .filter(|p| {
            let name = p.file_name().and_then(|s| s.to_str()).unwrap_or("");
            name.ends_with(".jsonl") && !name.contains("audit")
        })
        .collect()
}

/// Non-recursive listing of files in `dir` whose name ends with one of `exts`.
fn read_dir_files(dir: &Path, exts: &[&str]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for entry in rd.flatten() {
            let path = entry.path();
            if path.is_file() {
                let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
                if exts.iter().any(|e| name.ends_with(e)) {
                    out.push(path);
                }
            }
        }
    }
    out
}

/// macOS uses `~/Library/Application Support`; other platforms use XDG `~/.config`
/// (BUILD_SPEC §4.1). Windows (`%APPDATA%`) is a future seam.
fn app_support(home: &Path) -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        home.join("Library/Application Support")
    }
    #[cfg(not(target_os = "macos"))]
    {
        home.join(".config")
    }
}

// ─── per-day accumulator (port of makeDayEntry) ───

#[derive(Default)]
struct DayEntry {
    cost: f64,
    input: u64,
    output: u64,
    cache_read: u64,
    cache_write: u64,
    reasoning: u64,
    models: Vec<String>, // insertion order preserved; last() == last model seen
    times: Vec<String>,
}

impl DayEntry {
    fn add_model(&mut self, model: &str) {
        if !model.is_empty() && !self.models.iter().any(|m| m == model) {
            self.models.push(model.to_string());
        }
    }
}

type DayMap = BTreeMap<String, DayEntry>;

fn u64_field(v: &Value, key: &str) -> u64 {
    v.get(key).and_then(Value::as_u64).unwrap_or(0)
}

fn file_fallback_date(path: &Path) -> Option<String> {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .and_then(|d| to_local_date(d.as_millis() as i64))
}

fn read_lines(path: &Path) -> Option<String> {
    std::fs::read_to_string(path).ok()
}

// ─── parsers ───

/// Claude-Code-shaped JSONL (also Desktop/Cursor/Windsurf/Cline/Roo). Port of
/// parseClaudeCodeFormat.
fn parse_claude_format(path: &Path, pricing: &Pricing) -> DayMap {
    let mut days = DayMap::new();
    let Some(content) = read_lines(path) else {
        return days;
    };
    let fallback = file_fallback_date(path);
    for line in content.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let Ok(entry): Result<Value, _> = serde_json::from_str(line) else {
            continue;
        };
        let message = entry.get("message");
        let usage = message
            .and_then(|m| m.get("usage"))
            .or_else(|| entry.get("usage"));
        let usage = match usage {
            Some(u) if u.is_object() => u,
            _ => continue,
        };
        let input = u64_field(usage, "input_tokens");
        let output = u64_field(usage, "output_tokens");
        let cache_write = u64_field(usage, "cache_creation_input_tokens");
        let cache_read = u64_field(usage, "cache_read_input_tokens");
        if input == 0 && output == 0 && cache_read == 0 && cache_write == 0 {
            continue;
        }
        let ts = parse_timestamp_ms(entry.get("timestamp"))
            .or_else(|| parse_timestamp_ms(message.and_then(|m| m.get("timestamp"))));
        let Some(date) = ts.and_then(to_local_date).or_else(|| fallback.clone()) else {
            continue;
        };
        let time = ts.and_then(to_local_time).unwrap_or_else(|| "00:00".into());
        let model = message
            .and_then(|m| m.get("model"))
            .or_else(|| entry.get("model"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();

        let dd = days.entry(date).or_default();
        dd.times.push(time);
        if model.starts_with("claude") {
            dd.add_model(&model);
        }
        dd.input += input;
        dd.output += output;
        dd.cache_read += cache_read;
        dd.cache_write += cache_write;
        let rate = pricing.claude_rate(&model);
        dd.cost += pricing::claude_cost(rate, input, output, cache_write, cache_read);
    }
    days
}

/// OpenClaw / Clawdbot. Skips non-Claude models; uses precomputed `usage.cost.total` when
/// present. Port of parseOpenClawFormat.
fn parse_openclaw_format(path: &Path, pricing: &Pricing) -> DayMap {
    let mut days = DayMap::new();
    let Some(content) = read_lines(path) else {
        return days;
    };
    let fallback = file_fallback_date(path);
    for line in content.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let Ok(entry): Result<Value, _> = serde_json::from_str(line) else {
            continue;
        };
        let message = entry.get("message");
        let usage = message
            .and_then(|m| m.get("usage"))
            .or_else(|| entry.get("usage"));
        let usage = match usage {
            Some(u) if u.is_object() => u,
            _ => continue,
        };
        let input = u64_field(usage, "input");
        let output = u64_field(usage, "output");
        let cache_read = u64_field(usage, "cacheRead");
        let cache_write = u64_field(usage, "cacheWrite");
        let cost_total = usage
            .get("cost")
            .and_then(|c| c.get("total"))
            .and_then(Value::as_f64);
        let has_cost = usage.get("cost").map(|c| !c.is_null()).unwrap_or(false);
        if !has_cost && input == 0 && output == 0 {
            continue;
        }
        let model = message
            .and_then(|m| m.get("model"))
            .or_else(|| entry.get("model"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        if !model.starts_with("claude") {
            continue;
        }
        let ts = parse_timestamp_ms(entry.get("timestamp"))
            .or_else(|| parse_timestamp_ms(message.and_then(|m| m.get("timestamp"))));
        let Some(date) = ts.and_then(to_local_date).or_else(|| fallback.clone()) else {
            continue;
        };
        let time = ts.and_then(to_local_time).unwrap_or_else(|| "00:00".into());

        let dd = days.entry(date).or_default();
        dd.times.push(time);
        dd.add_model(&model);
        if let Some(total) = cost_total.filter(|t| *t != 0.0) {
            dd.cost += total;
        } else {
            let rate = pricing.claude_rate(&model);
            dd.cost += pricing::claude_cost(rate, input, output, cache_write, cache_read);
        }
        dd.input += input;
        dd.output += output;
        dd.cache_read += cache_read;
        dd.cache_write += cache_write;
    }
    days
}

/// Aider (litellm/OpenAI-shape; Unix-epoch-seconds timestamps). Port of parseAiderFormat.
fn parse_aider_format(path: &Path, pricing: &Pricing) -> DayMap {
    let mut days = DayMap::new();
    let Some(content) = read_lines(path) else {
        return days;
    };
    let fallback = file_fallback_date(path);
    for line in content.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let Ok(entry): Result<Value, _> = serde_json::from_str(line) else {
            continue;
        };
        let usage = entry
            .get("usage")
            .or_else(|| entry.get("response").and_then(|r| r.get("usage")));
        let usage = match usage {
            Some(u) if u.is_object() => u,
            _ => continue,
        };
        let input = usage
            .get("prompt_tokens")
            .and_then(Value::as_u64)
            .unwrap_or_else(|| u64_field(usage, "input_tokens"));
        let output = usage
            .get("completion_tokens")
            .and_then(Value::as_u64)
            .unwrap_or_else(|| u64_field(usage, "output_tokens"));
        let cache_read = u64_field(usage, "cache_read_input_tokens");
        let cache_write = u64_field(usage, "cache_creation_input_tokens");
        if input == 0 && output == 0 {
            continue;
        }
        let mut ts = parse_timestamp_ms(entry.get("timestamp"))
            .or_else(|| parse_timestamp_ms(entry.get("created")));
        // Aider uses Unix epoch SECONDS.
        if let Some(created) = entry.get("created") {
            if let Some(secs) = created.as_f64() {
                if created.is_number() && secs < 2_000_000_000.0 {
                    ts = Some((secs * 1000.0) as i64);
                }
            }
        }
        let Some(date) = ts.and_then(to_local_date).or_else(|| fallback.clone()) else {
            continue;
        };
        let time = ts.and_then(to_local_time).unwrap_or_else(|| "00:00".into());
        let model = entry
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();

        let dd = days.entry(date).or_default();
        dd.times.push(time);
        if model.contains("claude") {
            dd.add_model(&model);
        }
        dd.input += input;
        dd.output += output;
        dd.cache_read += cache_read;
        dd.cache_write += cache_write;
        let rate = pricing.claude_rate(&model);
        dd.cost += pricing::claude_cost(rate, input, output, cache_write, cache_read);
    }
    days
}

/// Continue.dev (single JSON with `steps`/`history`). Port of parseContinueFormat.
fn parse_continue_format(path: &Path, pricing: &Pricing) -> DayMap {
    let mut days = DayMap::new();
    let Some(content) = read_lines(path) else {
        return days;
    };
    let Ok(data): Result<Value, _> = serde_json::from_str(&content) else {
        return days;
    };
    let steps = data
        .get("steps")
        .or_else(|| data.get("history"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for step in &steps {
        let input = step
            .get("promptTokens")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let output = step
            .get("completionTokens")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        if input == 0 && output == 0 {
            continue;
        }
        let ts = parse_timestamp_ms(step.get("timestamp"))
            .or_else(|| parse_timestamp_ms(data.get("dateCreated")));
        let date = match ts.and_then(to_local_date) {
            Some(d) => d,
            None => match file_fallback_date(path) {
                Some(d) => d,
                None => continue,
            },
        };
        let time = ts.and_then(to_local_time).unwrap_or_else(|| "00:00".into());
        let model = step
            .get("model")
            .or_else(|| data.get("model"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();

        let dd = days.entry(date).or_default();
        dd.times.push(time);
        if model.contains("claude") {
            dd.add_model(&model);
        }
        dd.input += input;
        dd.output += output;
        let rate = pricing.claude_rate(&model);
        dd.cost += pricing::claude_cost(rate, input, output, 0, 0);
    }
    days
}

/// Codex CLI rollout logs. Tracks model from `turn_context`, diffs cumulative token usage
/// on mid-session resets, bills non-cached input. Port of parseCodexFormat.
fn parse_codex_format(path: &Path, pricing: &Pricing) -> DayMap {
    let mut days = DayMap::new();
    let Some(content) = read_lines(path) else {
        return days;
    };
    let fallback = file_fallback_date(path);
    let mut current_model = String::new();
    let mut last_cumulative: Option<Value> = None;

    for line in content.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let Ok(entry): Result<Value, _> = serde_json::from_str(line) else {
            continue;
        };
        let payload = match entry.get("payload") {
            Some(p) if p.is_object() => p,
            _ => continue,
        };
        let etype = entry.get("type").and_then(Value::as_str).unwrap_or("");
        if etype == "turn_context" {
            if let Some(m) = payload.get("model").and_then(Value::as_str) {
                current_model = m.to_string();
            }
            continue;
        }
        if etype != "event_msg"
            || payload.get("type").and_then(Value::as_str) != Some("token_count")
        {
            continue;
        }
        let Some(info) = payload.get("info") else {
            continue;
        };

        // Resolve the per-turn usage: prefer last_token_usage, else diff the cumulative.
        let last = info.get("last_token_usage").filter(|v| v.is_object());
        let total = info.get("total_token_usage").filter(|v| v.is_object());
        let usage: Option<(u64, u64, u64, u64)> = if let Some(u) = last {
            Some((
                u64_field(u, "input_tokens"),
                u64_field(u, "cached_input_tokens"),
                u64_field(u, "output_tokens"),
                u64_field(u, "reasoning_output_tokens"),
            ))
        } else if let Some(t) = total {
            let diff = |k: &str| -> u64 {
                let cur = u64_field(t, k);
                let prev = last_cumulative
                    .as_ref()
                    .map(|lc| u64_field(lc, k))
                    .unwrap_or(0);
                cur.saturating_sub(prev)
            };
            let u = (
                diff("input_tokens"),
                diff("cached_input_tokens"),
                diff("output_tokens"),
                diff("reasoning_output_tokens"),
            );
            last_cumulative = Some(t.clone());
            Some(u)
        } else {
            None
        };
        // Keep cumulative fresh even when last_token_usage was used.
        if last.is_some() {
            if let Some(t) = total {
                last_cumulative = Some(t.clone());
            }
        }
        let Some((input, cached, output, reasoning)) = usage else {
            continue;
        };
        if input == 0 && output == 0 && cached == 0 {
            continue;
        }
        let ts = parse_timestamp_ms(entry.get("timestamp"));
        let Some(date) = ts.and_then(to_local_date).or_else(|| fallback.clone()) else {
            continue;
        };
        let time = ts.and_then(to_local_time).unwrap_or_else(|| "00:00".into());

        let dd = days.entry(date).or_default();
        dd.times.push(time);
        if !current_model.is_empty() {
            dd.add_model(&current_model);
        }
        dd.input += input;
        dd.output += output;
        dd.cache_read += cached;
        dd.reasoning += reasoning;
        let rate = pricing.codex_rate(&current_model);
        dd.cost += pricing::codex_cost(rate, input, cached, output);
    }
    days
}

/// The parser + provider selected per source.
#[derive(Clone, Copy)]
enum Format {
    Claude,
    OpenClaw,
    Aider,
    Continue,
    Codex,
}

impl Format {
    fn provider(self) -> Provider {
        match self {
            Format::Codex => Provider::Codex,
            _ => Provider::Claude,
        }
    }

    fn parse(self, path: &Path, pricing: &Pricing) -> DayMap {
        match self {
            Format::Claude => parse_claude_format(path, pricing),
            Format::OpenClaw => parse_openclaw_format(path, pricing),
            Format::Aider => parse_aider_format(path, pricing),
            Format::Continue => parse_continue_format(path, pricing),
            Format::Codex => parse_codex_format(path, pricing),
        }
    }
}

// ─── session meta (port of extractSessionMeta/extractCodexMeta) ───

#[derive(Default)]
struct Meta {
    title: Option<String>,
    session_id: Option<String>,
    cwd: Option<String>,
    /// Some formats (Codex) override the display source per-file.
    source: Option<String>,
}

fn clean_message_text(text: &str) -> String {
    // Port of the original's tag-stripping (regex-free): first remove `<tag>…</tag>` blocks
    // (non-nested, non-greedy — matches the first closing tag), then remove any lone tags.
    // Cosmetic (title only). '<' and '>' are ASCII, so byte indices are char boundaries.
    let mut s = text.to_string();
    while let Some(open) = s.find('<') {
        let Some(open_end) = s[open..].find('>').map(|r| open + r) else {
            break; // dangling '<' with no '>'; leave it
        };
        let is_closing = s[open + 1..].starts_with('/');
        if !is_closing {
            if let Some(close_rel) = s[open_end + 1..].find("</") {
                let close_start = open_end + 1 + close_rel;
                if let Some(close_end) = s[close_start..].find('>').map(|r| close_start + r) {
                    s.replace_range(open..=close_end, "");
                    continue;
                }
            }
        }
        s.replace_range(open..=open_end, ""); // lone/closing tag
    }
    s.trim().to_string()
}

fn extract_text(msg: &Value) -> String {
    match msg.get("content") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(blocks)) => {
            if blocks
                .iter()
                .any(|b| b.get("type").and_then(Value::as_str) == Some("tool_result"))
            {
                return String::new();
            }
            blocks
                .iter()
                .find(|b| {
                    b.get("type").and_then(Value::as_str) == Some("text")
                        && b.get("text")
                            .and_then(Value::as_str)
                            .is_some_and(|t| !t.trim().is_empty())
                })
                .and_then(|b| b.get("text").and_then(Value::as_str))
                .unwrap_or("")
                .to_string()
        }
        _ => String::new(),
    }
}

fn is_uuid(s: &str) -> bool {
    let groups = [8usize, 4, 4, 4, 12];
    let parts: Vec<&str> = s.split('-').collect();
    parts.len() == 5
        && parts
            .iter()
            .zip(groups)
            .all(|(p, n)| p.len() == n && p.chars().all(|c| c.is_ascii_hexdigit()))
}

fn truncate_title(text: &str) -> String {
    if text.chars().count() > 80 {
        let head: String = text.chars().take(77).collect();
        format!("{head}...")
    } else {
        text.to_string()
    }
}

/// UUID suffix from the filename stem (Codex uses a trailing UUID; Claude uses the whole stem).
fn uuid_from_stem(path: &Path, suffix_only: bool) -> Option<String> {
    let stem = path.file_stem().and_then(|s| s.to_str())?;
    if is_uuid(stem) {
        return Some(stem.to_string());
    }
    if suffix_only {
        let tail: String = stem
            .chars()
            .rev()
            .take(36)
            .collect::<String>()
            .chars()
            .rev()
            .collect();
        if is_uuid(&tail) {
            return Some(tail);
        }
    }
    None
}

fn extract_session_meta(path: &Path) -> Meta {
    let mut meta = Meta::default();
    if let Some(content) = read_lines(path) {
        let mut found_title = false;
        for line in content.lines() {
            if line.trim().is_empty() {
                continue;
            }
            let Ok(entry): Result<Value, _> = serde_json::from_str(line) else {
                continue;
            };
            if meta.session_id.is_none() {
                if let Some(id) = entry.get("sessionId").and_then(Value::as_str) {
                    meta.session_id = Some(id.to_string());
                }
            }
            if meta.cwd.is_none() {
                if let Some(cwd) = entry.get("cwd").and_then(Value::as_str) {
                    meta.cwd = Some(cwd.to_string());
                }
            }
            let msg = match entry.get("message") {
                Some(m) if m.is_object() => m,
                _ => continue,
            };
            let role = msg.get("role").and_then(Value::as_str).unwrap_or("");
            if role != "user" && role != "assistant" {
                continue;
            }
            if found_title && meta.session_id.is_some() && meta.cwd.is_some() {
                break;
            }
            if !found_title && role == "user" {
                let raw = extract_text(msg);
                if raw.is_empty() {
                    continue;
                }
                let text = clean_message_text(&raw);
                if text.is_empty() {
                    continue;
                }
                meta.title = Some(truncate_title(&text));
                found_title = true;
            }
        }
    }
    if meta.session_id.is_none() {
        meta.session_id = uuid_from_stem(path, false);
    }
    meta
}

fn extract_codex_meta(path: &Path) -> Meta {
    let mut meta = Meta {
        source: Some("Codex".to_string()),
        ..Default::default()
    };
    if let Some(content) = read_lines(path) {
        let mut found_title = false;
        for line in content.lines() {
            if line.trim().is_empty() {
                continue;
            }
            let Ok(entry): Result<Value, _> = serde_json::from_str(line) else {
                continue;
            };
            let Some(payload) = entry.get("payload") else {
                continue;
            };
            let etype = entry.get("type").and_then(Value::as_str).unwrap_or("");
            if etype == "session_meta" {
                if meta.session_id.is_none() {
                    if let Some(id) = payload.get("id").and_then(Value::as_str) {
                        meta.session_id = Some(id.to_string());
                    }
                }
                if meta.cwd.is_none() {
                    if let Some(cwd) = payload.get("cwd").and_then(Value::as_str) {
                        meta.cwd = Some(cwd.to_string());
                    }
                }
                meta.source = Some(codex_source(payload).to_string());
            }
            if !found_title
                && etype == "response_item"
                && payload.get("type").and_then(Value::as_str) == Some("message")
                && payload.get("role").and_then(Value::as_str) == Some("user")
            {
                if let Some(blocks) = payload.get("content").and_then(Value::as_array) {
                    for block in blocks {
                        let text = match block {
                            Value::String(s) => s.clone(),
                            _ => block
                                .get("text")
                                .and_then(Value::as_str)
                                .unwrap_or("")
                                .to_string(),
                        };
                        let t = text.trim();
                        if t.is_empty()
                            || t.to_lowercase().starts_with("<environment_context>")
                            || t.to_lowercase().starts_with("<permissions instructions>")
                        {
                            continue;
                        }
                        let cleaned = clean_message_text(&text);
                        if cleaned.is_empty() {
                            continue;
                        }
                        meta.title = Some(truncate_title(&cleaned));
                        found_title = true;
                        break;
                    }
                }
            }
            if found_title && meta.session_id.is_some() && meta.cwd.is_some() {
                break;
            }
        }
    }
    if meta.session_id.is_none() {
        meta.session_id = uuid_from_stem(path, true);
    }
    meta
}

/// Codex sub-source classification from a `session_meta` payload.
fn codex_source(payload: &Value) -> &'static str {
    let src = payload.get("source");
    let subagent = src
        .and_then(|s| s.get("subagent"))
        .or_else(|| payload.get("subagent"))
        .and_then(Value::as_str);
    if subagent == Some("review") {
        return "Codex Review";
    }
    let kind = src.and_then(|s| s.as_str());
    let kind_obj = src.and_then(|s| s.get("kind")).and_then(Value::as_str);
    match (kind, kind_obj) {
        (Some("cli"), _) | (_, Some("cli")) => "Codex CLI",
        (Some("exec"), _) | (_, Some("exec")) => "Codex Exec",
        _ => "Codex",
    }
}

// ─── emit records (port of pushSessions) ───

fn push_sessions(
    out: &mut Vec<SessionRecord>,
    days: DayMap,
    source: &str,
    file_name: &str,
    file_path: &Path,
    provider: Provider,
    meta: &Meta,
) {
    let effective_source = meta.source.as_deref().unwrap_or(source);
    for (date, data) in days {
        if data.cost < 0.0001 {
            continue;
        }
        let mut times = data.times;
        times.sort();
        let time = times.first().cloned().unwrap_or_else(|| "00:00".into());
        out.push(SessionRecord {
            date,
            time,
            provider,
            source: effective_source.to_string(),
            file: file_name.to_string(),
            cost: round(data.cost, 4),
            input_tokens: data.input,
            output_tokens: data.output,
            cache_read: data.cache_read,
            cache_write: data.cache_write,
            model: data.models.last().cloned().unwrap_or_default(),
            reasoning_tokens: (data.reasoning > 0).then_some(data.reasoning),
            file_path: file_path.to_str().map(|s| s.to_string()),
            title: meta.title.clone(),
            session_id: meta.session_id.clone(),
            cwd: meta.cwd.clone(),
        });
    }
}

// ─── scan context (port of processJsonlFile + scan-index state) ───

struct Ctx<'a> {
    pricing: &'a Pricing,
    prev_index: &'a ScanIndex,
    cached_by_filepath: HashMap<String, Vec<SessionRecord>>,
    new_index: ScanIndex,
    seen: HashSet<String>,
    parsed: usize,
    skipped: usize,
}

fn file_fingerprint(path: &Path) -> Option<Fingerprint> {
    let md = std::fs::metadata(path).ok()?;
    let mtime = md
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_millis() as i64;
    Some(Fingerprint {
        mtime,
        size: md.len(),
    })
}

impl<'a> Ctx<'a> {
    fn process_file(
        &mut self,
        out: &mut Vec<SessionRecord>,
        source: &str,
        path: &Path,
        format: Format,
    ) {
        let Some(fp) = file_fingerprint(path) else {
            return;
        };
        let key = path.to_string_lossy().to_string();
        self.seen.insert(key.clone());

        // Skip unchanged files: replay their cached records (BUILD_SPEC §4.7).
        if let (Some(prev), Some(cached)) =
            (self.prev_index.get(&key), self.cached_by_filepath.get(&key))
        {
            if *prev == fp {
                out.extend(cached.iter().cloned());
                self.new_index.insert(key, fp);
                self.skipped += 1;
                return;
            }
        }

        let days = format.parse(path, self.pricing);
        let meta = match format {
            Format::Codex => extract_codex_meta(path),
            _ => extract_session_meta(path),
        };
        let file_name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        push_sessions(
            out,
            days,
            source,
            &file_name,
            path,
            format.provider(),
            &meta,
        );
        self.new_index.insert(key, fp);
        self.parsed += 1;
    }
}

// ─── source collectors ───

fn collect_openclaw(ctx: &mut Ctx, home: &Path, out: &mut Vec<SessionRecord>) {
    let mut seen_files: HashSet<String> = HashSet::new();
    for (dir_name, source) in [("openclaw", "OpenClaw"), ("clawdbot", "Clawdbot")] {
        let dir = home.join(format!(".{dir_name}/agents/main/sessions"));
        if !dir.exists() {
            continue;
        }
        for path in read_dir_files(&dir, &[".jsonl"]) {
            let name = path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();
            if !seen_files.insert(name) {
                continue;
            }
            ctx.process_file(out, source, &path, Format::OpenClaw);
        }
    }
}

fn collect_recursive(ctx: &mut Ctx, out: &mut Vec<SessionRecord>, source: &str, dirs: &[PathBuf]) {
    for dir in dirs {
        if !dir.exists() {
            continue;
        }
        for path in find_jsonl(dir) {
            ctx.process_file(out, source, &path, Format::Claude);
        }
    }
}

fn collect_codex(ctx: &mut Ctx, home: &Path, out: &mut Vec<SessionRecord>) {
    let dir = home.join(".codex/sessions");
    if !dir.exists() {
        return;
    }
    for path in find_jsonl(&dir) {
        let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
        if !name.starts_with("rollout-") {
            continue;
        }
        ctx.process_file(out, "Codex", &path, Format::Codex);
    }
}

fn dedup(sessions: Vec<SessionRecord>) -> Vec<SessionRecord> {
    // Key provider|source|file|date; last write wins (port of the dedupedMap logic).
    let mut order: Vec<String> = Vec::new();
    let mut map: HashMap<String, SessionRecord> = HashMap::new();
    for s in sessions {
        let provider = match s.provider {
            Provider::Claude => "claude",
            Provider::Codex => "codex",
        };
        let key = format!("{}|{}|{}|{}", provider, s.source, s.file, s.date);
        if !map.contains_key(&key) {
            order.push(key.clone());
        }
        map.insert(key, s);
    }
    order.into_iter().filter_map(|k| map.remove(&k)).collect()
}

/// Build the summary + split sessions into the three dashboard buckets (BUILD_SPEC §4.6).
pub fn build_output(sessions: &[SessionRecord]) -> CollectOutput {
    let today = now_local_date();
    let current_month = today[..7].to_string();

    let mut totals: BTreeMap<String, f64> = BTreeMap::new();
    let mut counts: BTreeMap<String, u64> = BTreeMap::new();
    let mut provider_totals = ProviderTotals {
        claude: 0.0,
        codex: 0.0,
    };
    let mut grand_total = 0.0;
    let mut today_cost = 0.0;
    let mut month_cost = 0.0;

    for s in sessions {
        *totals.entry(s.source.clone()).or_insert(0.0) += s.cost;
        *counts.entry(s.source.clone()).or_insert(0) += 1;
        match s.provider {
            Provider::Claude => provider_totals.claude += s.cost,
            Provider::Codex => provider_totals.codex += s.cost,
        }
        grand_total += s.cost;
        if s.date == today {
            today_cost += s.cost;
        }
        if s.date.starts_with(&current_month) {
            month_cost += s.cost;
        }
    }

    for v in totals.values_mut() {
        *v = round(*v, 2);
    }
    totals.insert("grand_total".to_string(), round(grand_total, 2));
    provider_totals.claude = round(provider_totals.claude, 2);
    provider_totals.codex = round(provider_totals.codex, 2);
    counts.insert("total".to_string(), sessions.len() as u64);

    let summary = Summary {
        generated_at: Utc::now().to_rfc3339(),
        today,
        current_month,
        totals,
        provider_totals,
        today_cost: round(today_cost, 2),
        month_cost: round(month_cost, 2),
        session_counts: counts,
    };

    let is_openclaw = |s: &SessionRecord| s.source == "OpenClaw" || s.source == "Clawdbot";
    let codex = sessions
        .iter()
        .filter(|s| s.provider == Provider::Codex)
        .cloned()
        .collect();
    let openclaw = sessions
        .iter()
        .filter(|s| s.provider != Provider::Codex && is_openclaw(s))
        .cloned()
        .collect();
    let claude = sessions
        .iter()
        .filter(|s| s.provider != Provider::Codex && !is_openclaw(s))
        .cloned()
        .collect();

    CollectOutput {
        summary,
        claude,
        codex,
        openclaw,
    }
}

/// Full collect across all sources with incremental caching (BUILD_SPEC §4).
pub fn collect(home: &Path, pricing: &Pricing, caches: &Caches) -> CollectResult {
    let mut cached_by_filepath: HashMap<String, Vec<SessionRecord>> = HashMap::new();
    for s in &caches.cached_sessions {
        if let Some(fp) = &s.file_path {
            cached_by_filepath
                .entry(fp.clone())
                .or_default()
                .push(s.clone());
        }
    }

    let mut ctx = Ctx {
        pricing,
        prev_index: &caches.scan_index,
        cached_by_filepath,
        new_index: ScanIndex::new(),
        seen: HashSet::new(),
        parsed: 0,
        skipped: 0,
    };

    let mut sessions: Vec<SessionRecord> = Vec::new();
    let app = app_support(home);

    // Order mirrors the original source list.
    collect_openclaw(&mut ctx, home, &mut sessions);
    collect_recursive(
        &mut ctx,
        &mut sessions,
        "Claude Code",
        &[home.join(".claude/projects")],
    );
    collect_recursive(
        &mut ctx,
        &mut sessions,
        "Claude Desktop",
        &[app.join("Claude/local-agent-mode-sessions")],
    );
    collect_recursive(
        &mut ctx,
        &mut sessions,
        "Cursor",
        &[
            home.join(".cursor/projects"),
            app.join("Cursor/User/workspaceStorage"),
        ],
    );
    collect_recursive(
        &mut ctx,
        &mut sessions,
        "Windsurf",
        &[
            home.join(".windsurf/projects"),
            home.join(".windsurf"),
            app.join("Windsurf/User/workspaceStorage"),
        ],
    );
    collect_recursive(
        &mut ctx,
        &mut sessions,
        "Cline",
        &[
            home.join(".cline"),
            app.join("Code/User/globalStorage/saoudrizwan.claude-dev"),
            app.join("Code/User/globalStorage/cline.cline"),
        ],
    );
    collect_recursive(
        &mut ctx,
        &mut sessions,
        "Roo Code",
        &[
            home.join(".roo-code"),
            app.join("Code/User/globalStorage/rooveterinaryinc.roo-cline"),
        ],
    );

    // Aider: single-level, *.jsonl or *.json.
    for dir in [home.join(".aider"), home.join(".aider/logs")] {
        if dir.exists() {
            for path in read_dir_files(&dir, &[".jsonl", ".json"]) {
                ctx.process_file(&mut sessions, "Aider", &path, Format::Aider);
            }
        }
    }
    // Continue: single-level, *.json.
    let cont = home.join(".continue/sessions");
    if cont.exists() {
        for path in read_dir_files(&cont, &[".json"]) {
            ctx.process_file(&mut sessions, "Continue", &path, Format::Continue);
        }
    }
    collect_codex(&mut ctx, home, &mut sessions);

    // Preserve historical / imported records whose file didn't resolve this run.
    let mut preserved = 0;
    for s in &caches.cached_sessions {
        let resolved = s
            .file_path
            .as_ref()
            .map(|fp| ctx.seen.contains(fp))
            .unwrap_or(false);
        if !resolved {
            sessions.push(s.clone());
            preserved += 1;
        }
    }

    let deduped = dedup(sessions);
    let output = build_output(&deduped);
    let stats = ScanStats {
        parsed: ctx.parsed,
        skipped: ctx.skipped,
        preserved,
    };
    CollectResult {
        output,
        sessions: deduped,
        scan_index: ctx.new_index,
        stats,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    struct Tmp(PathBuf);
    impl Tmp {
        fn new() -> Self {
            use std::sync::atomic::{AtomicU32, Ordering};
            static N: AtomicU32 = AtomicU32::new(0);
            let n = N.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!("munim-col-{}-{}", std::process::id(), n));
            std::fs::create_dir_all(&dir).unwrap();
            Tmp(dir)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn write(path: &Path, body: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut f = std::fs::File::create(path).unwrap();
        f.write_all(body.as_bytes()).unwrap();
    }

    fn ts() -> String {
        Local::now().format("%Y-%m-%dT%H:%M:%S%.3f%:z").to_string()
    }

    #[test]
    fn claude_code_and_meta() {
        let home = Tmp::new();
        let now = ts();
        write(
            &home.path().join(".claude/projects/p/aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee.jsonl"),
            &format!(
                "{{\"sessionId\":\"aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee\",\"cwd\":\"/w\",\"timestamp\":\"{now}\",\"message\":{{\"role\":\"user\",\"content\":\"hi\"}}}}\n\
                 {{\"timestamp\":\"{now}\",\"message\":{{\"role\":\"assistant\",\"model\":\"claude-sonnet-4-5\",\"usage\":{{\"input_tokens\":1000,\"output_tokens\":500}}}}}}\n"
            ),
        );
        let res = collect(home.path(), &Pricing::embedded_default(), &Caches::empty());
        assert_eq!(res.output.claude.len(), 1);
        assert_eq!(res.output.claude[0].cwd.as_deref(), Some("/w"));
        assert_eq!(res.stats.parsed, 1);
    }

    #[test]
    fn openclaw_split_and_codex_provider() {
        let home = Tmp::new();
        let now = ts();
        // OpenClaw record (claude model, precomputed cost).
        write(
            &home.path().join(".openclaw/agents/main/sessions/s1.jsonl"),
            &format!(
                "{{\"timestamp\":\"{now}\",\"message\":{{\"model\":\"claude-opus-4-1\",\"usage\":{{\"input\":10,\"output\":20,\"cost\":{{\"total\":1.5}}}}}}}}\n"
            ),
        );
        // Codex rollout with turn_context model + token_count event.
        write(
            &home.path().join(".codex/sessions/2026/rollout-2026-07-19T10-00-00.jsonl"),
            &format!(
                "{{\"type\":\"session_meta\",\"payload\":{{\"id\":\"11111111-2222-3333-4444-555555555555\",\"cwd\":\"/c\",\"source\":\"cli\"}}}}\n\
                 {{\"type\":\"turn_context\",\"payload\":{{\"model\":\"gpt-5.4\"}}}}\n\
                 {{\"type\":\"event_msg\",\"timestamp\":\"{now}\",\"payload\":{{\"type\":\"token_count\",\"info\":{{\"last_token_usage\":{{\"input_tokens\":1000,\"cached_input_tokens\":200,\"output_tokens\":500,\"reasoning_output_tokens\":100}}}}}}}}\n"
            ),
        );
        let res = collect(home.path(), &Pricing::embedded_default(), &Caches::empty());

        assert_eq!(res.output.openclaw.len(), 1, "openclaw bucket");
        assert!(
            (res.output.openclaw[0].cost - 1.5).abs() < 1e-9,
            "precomputed cost.total used"
        );

        assert_eq!(res.output.codex.len(), 1, "codex bucket");
        let cx = &res.output.codex[0];
        assert_eq!(cx.provider, Provider::Codex);
        assert_eq!(cx.source, "Codex CLI", "sub-source from session_meta");
        assert_eq!(cx.reasoning_tokens, Some(100));
        assert_eq!(cx.cache_read, 200);
        // cost = non-cached(800)*2.5 + cached(200)*0.25 + out(500)*15 per 1e6
        let want = (800.0 * 2.50 + 200.0 * 0.25 + 500.0 * 15.0) / 1_000_000.0;
        assert!((cx.cost - round(want, 4)).abs() < 1e-9);

        assert!(
            res.output.claude.is_empty(),
            "openclaw is its own bucket, not claude"
        );
        assert_eq!(res.output.summary.provider_totals.codex, round(want, 2));
    }

    #[test]
    fn incremental_scan_skips_unchanged() {
        let home = Tmp::new();
        let now = ts();
        write(
            &home.path().join(".claude/projects/p/aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee.jsonl"),
            &format!(
                "{{\"timestamp\":\"{now}\",\"message\":{{\"role\":\"assistant\",\"model\":\"claude-sonnet-4-5\",\"usage\":{{\"input_tokens\":1000,\"output_tokens\":500}}}}}}\n"
            ),
        );
        let pricing = Pricing::embedded_default();
        let first = collect(home.path(), &pricing, &Caches::empty());
        assert_eq!(first.stats.parsed, 1);
        assert_eq!(first.stats.skipped, 0);

        // Feed the first run's outputs back as caches: the file is unchanged → skipped.
        let caches = Caches {
            scan_index: first.scan_index.clone(),
            cached_sessions: first.sessions.clone(),
        };
        let second = collect(home.path(), &pricing, &caches);
        assert_eq!(second.stats.parsed, 0, "unchanged file not re-parsed");
        assert_eq!(second.stats.skipped, 1);
        assert_eq!(second.output.claude.len(), 1, "cached record replayed");
        assert_eq!(second.output.summary.totals, first.output.summary.totals);
    }

    #[test]
    fn preserves_historical_when_file_gone() {
        let home = Tmp::new();
        // A cached record whose filePath does not exist on disk this run.
        let ghost = SessionRecord {
            date: "2020-01-01".into(),
            time: "00:00".into(),
            provider: Provider::Claude,
            source: "Claude Code".into(),
            file: "old.jsonl".into(),
            cost: 4.2,
            input_tokens: 1,
            output_tokens: 1,
            cache_read: 0,
            cache_write: 0,
            model: "claude-sonnet-4-5".into(),
            reasoning_tokens: None,
            file_path: Some("/nope/old.jsonl".into()),
            title: None,
            session_id: None,
            cwd: None,
        };
        let caches = Caches {
            scan_index: ScanIndex::new(),
            cached_sessions: vec![ghost],
        };
        let res = collect(home.path(), &Pricing::embedded_default(), &caches);
        assert_eq!(res.stats.preserved, 1);
        assert_eq!(res.output.claude.len(), 1);
        assert!(
            (res.output
                .summary
                .totals
                .get("grand_total")
                .copied()
                .unwrap()
                - 4.2)
                .abs()
                < 1e-9
        );
    }
}
