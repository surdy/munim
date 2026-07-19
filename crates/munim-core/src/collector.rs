//! Usage collector — port of the original `collect-usage.js` (BUILD_SPEC §4).
//!
//! Ticket #1 implements the Claude Code slice end-to-end: resolve paths → parse JSONL →
//! aggregate per (source, file, date) → price → summarize. Later tickets (#2/#3) add the
//! other sources, caching, and the incremental scan; `collect()` is the extension point.

use std::collections::BTreeMap;
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

// ─── file discovery (port of findJsonl) ───

/// Recursively collect `*.jsonl` files, skipping `.git*` dirs and any filename containing
/// "audit", capped at depth 10.
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

fn u64_field(usage: &Value, key: &str) -> u64 {
    usage.get(key).and_then(Value::as_u64).unwrap_or(0)
}

/// Parse a Claude-Code-shaped JSONL file into per-date aggregates (port of
/// parseClaudeCodeFormat). Also handles Desktop/Cursor/Windsurf/Cline/Roo in later tickets.
fn parse_claude_format(path: &Path, pricing: &Pricing) -> BTreeMap<String, DayEntry> {
    let mut days: BTreeMap<String, DayEntry> = BTreeMap::new();
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return days,
    };
    let fallback_date = std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .and_then(|d| to_local_date(d.as_millis() as i64));

    for line in content.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let entry: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
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
        let date = match ts.and_then(to_local_date).or_else(|| fallback_date.clone()) {
            Some(d) => d,
            None => continue,
        };
        let time = ts
            .and_then(to_local_time)
            .unwrap_or_else(|| "00:00".to_string());

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

// ─── session meta (port of extractSessionMeta/extractText/cleanMessageText) ───

#[derive(Default)]
struct Meta {
    title: Option<String>,
    session_id: Option<String>,
    cwd: Option<String>,
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

fn extract_session_meta(path: &Path) -> Meta {
    let mut meta = Meta::default();
    if let Ok(content) = std::fs::read_to_string(path) {
        let mut found_title = false;
        for line in content.lines() {
            if line.trim().is_empty() {
                continue;
            }
            let entry: Value = match serde_json::from_str(line) {
                Ok(v) => v,
                Err(_) => continue,
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
                meta.title = Some(if text.chars().count() > 80 {
                    let truncated: String = text.chars().take(77).collect();
                    format!("{truncated}...")
                } else {
                    text
                });
                found_title = true;
            }
        }
    }
    if meta.session_id.is_none() {
        if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
            if is_uuid(stem) {
                meta.session_id = Some(stem.to_string());
            }
        }
    }
    meta
}

// ─── emit records (port of pushSessions) ───

fn push_sessions(
    out: &mut Vec<SessionRecord>,
    days: BTreeMap<String, DayEntry>,
    source: &str,
    file_name: &str,
    file_path: &Path,
    provider: Provider,
    meta: &Meta,
) {
    for (date, data) in days {
        if data.cost < 0.0001 {
            continue;
        }
        let mut times = data.times;
        times.sort();
        let time = times
            .first()
            .cloned()
            .unwrap_or_else(|| "00:00".to_string());
        out.push(SessionRecord {
            date,
            time,
            provider,
            source: source.to_string(),
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

// ─── collectors ───

/// Claude Code CLI: `~/.claude/projects` (cross-platform dotfile root). BUILD_SPEC §4.1.
pub fn collect_claude_code(home: &Path, pricing: &Pricing) -> Vec<SessionRecord> {
    let dir = home.join(".claude/projects");
    let mut out = Vec::new();
    if !dir.exists() {
        return out;
    }
    for path in find_jsonl(&dir) {
        let days = parse_claude_format(&path, pricing);
        let meta = extract_session_meta(&path);
        let file_name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        push_sessions(
            &mut out,
            days,
            "Claude Code",
            &file_name,
            &path,
            Provider::Claude,
            &meta,
        );
    }
    out
}

/// Build the summary + split sessions into the three dashboard buckets (BUILD_SPEC §4.6).
pub fn build_output(sessions: Vec<SessionRecord>) -> CollectOutput {
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

    for s in &sessions {
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

/// Full collect. Ticket #1 scans only Claude Code; #2 adds the remaining sources here.
pub fn collect(home: &Path, pricing: &Pricing) -> CollectOutput {
    let mut sessions = Vec::new();
    sessions.extend(collect_claude_code(home, pricing));
    build_output(sessions)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Write a minimal Claude Code JSONL fixture into a temp `~/.claude/projects` tree.
    fn fixture_home() -> (tempdir::Guard, PathBuf) {
        let guard = tempdir::Guard::new();
        let proj = guard.path().join(".claude/projects/demo");
        std::fs::create_dir_all(&proj).unwrap();
        let file = proj.join("11111111-2222-3333-4444-555555555555.jsonl");
        let mut f = std::fs::File::create(&file).unwrap();
        // A user turn (for title/cwd/sessionId) + two assistant usage rows.
        let today = Local::now().format("%Y-%m-%dT%H:%M:%S%.3f%:z").to_string();
        writeln!(
            f,
            r#"{{"sessionId":"11111111-2222-3333-4444-555555555555","cwd":"/home/u/proj","timestamp":"{today}","message":{{"role":"user","content":"Hello <thinking>ignore</thinking> world"}}}}"#
        )
        .unwrap();
        writeln!(
            f,
            r#"{{"timestamp":"{today}","message":{{"role":"assistant","model":"claude-sonnet-4-5-20250929","usage":{{"input_tokens":1000,"output_tokens":500,"cache_read_input_tokens":300,"cache_creation_input_tokens":200}}}}}}"#
        )
        .unwrap();
        writeln!(
            f,
            r#"{{"timestamp":"{today}","message":{{"role":"assistant","model":"claude-opus-4-1-20250805","usage":{{"input_tokens":100,"output_tokens":50}}}}}}"#
        )
        .unwrap();
        let home = guard.path().to_path_buf();
        (guard, home)
    }

    #[test]
    fn collects_claude_code_end_to_end() {
        let pricing = Pricing::embedded_default();
        let (_guard, home) = fixture_home();
        let out = collect(&home, &pricing);

        assert_eq!(out.claude.len(), 1, "one session-day");
        let rec = &out.claude[0];
        assert_eq!(rec.source, "Claude Code");
        assert_eq!(rec.provider, Provider::Claude);
        assert_eq!(rec.input_tokens, 1100);
        assert_eq!(rec.output_tokens, 550);
        assert_eq!(rec.cache_read, 300);
        assert_eq!(rec.cache_write, 200);
        assert_eq!(
            rec.session_id.as_deref(),
            Some("11111111-2222-3333-4444-555555555555")
        );
        assert_eq!(rec.cwd.as_deref(), Some("/home/u/proj"));
        assert_eq!(rec.title.as_deref(), Some("Hello  world")); // tag stripped

        // cost = sonnet(1000,500,200,300) + opus4.1(100,50,0,0)
        let want = (1000.0 * 3.0
            + 500.0 * 15.0
            + 200.0 * 3.75
            + 300.0 * 0.30
            + 100.0 * 15.0
            + 50.0 * 75.0)
            / 1_000_000.0;
        assert!((rec.cost - round(want, 4)).abs() < 1e-9);

        // summary
        assert_eq!(out.summary.session_counts.get("total"), Some(&1));
        assert_eq!(
            out.summary.totals.get("Claude Code").copied(),
            Some(round(want, 2))
        );
        assert!(out.codex.is_empty() && out.openclaw.is_empty());
    }

    #[test]
    fn missing_dir_is_empty() {
        let pricing = Pricing::embedded_default();
        let out = collect(Path::new("/nonexistent/munim-home"), &pricing);
        assert!(out.claude.is_empty());
        assert_eq!(out.summary.totals.get("grand_total").copied(), Some(0.0));
    }

    /// Tiny self-contained temp-dir helper (avoids an extra dev-dependency).
    mod tempdir {
        use std::path::{Path, PathBuf};
        pub struct Guard(PathBuf);
        impl Guard {
            pub fn new() -> Self {
                // Deterministic-enough unique dir without Instant/rand: use pid + a counter.
                use std::sync::atomic::{AtomicU32, Ordering};
                static N: AtomicU32 = AtomicU32::new(0);
                let n = N.fetch_add(1, Ordering::Relaxed);
                let dir =
                    std::env::temp_dir().join(format!("munim-test-{}-{}", std::process::id(), n));
                std::fs::create_dir_all(&dir).unwrap();
                Guard(dir)
            }
            pub fn path(&self) -> &Path {
                &self.0
            }
        }
        impl Drop for Guard {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
    }
}
