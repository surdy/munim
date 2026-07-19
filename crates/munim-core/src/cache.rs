//! On-disk caches (BUILD_SPEC §4.7): `sessions-cache.json` (merged records) and
//! `scan-index.json` (mtime/size fingerprints). Both use atomic tmp+rename writes.
//! IO is parameterized by `dir` so it unit-tests against a temp directory.

use std::path::Path;

use serde_json::Value;

use crate::collector::{collect, Caches, CollectOutput, ScanIndex, SessionRecord};
use crate::pricing::Pricing;

const CACHE_FILE: &str = "sessions-cache.json";
const SCAN_INDEX_FILE: &str = "scan-index.json";

fn atomic_write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

/// Load + normalize the session cache: validate shape, strip legacy `history`, backfill
/// `provider` from the source (port of loadCache). Malformed entries are dropped.
pub fn load_cache(dir: &Path) -> Vec<SessionRecord> {
    let raw = match std::fs::read_to_string(dir.join(CACHE_FILE)) {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };
    let Ok(Value::Array(items)) = serde_json::from_str::<Value>(&raw) else {
        return Vec::new();
    };
    let mut out = Vec::with_capacity(items.len());
    for mut v in items {
        let Some(obj) = v.as_object_mut() else {
            continue;
        };
        // Required fields with the right types.
        let ok = obj.get("source").and_then(Value::as_str).is_some()
            && obj.get("file").and_then(Value::as_str).is_some()
            && obj.get("date").and_then(Value::as_str).is_some()
            && obj.get("cost").and_then(Value::as_f64).is_some();
        if !ok {
            continue;
        }
        obj.remove("history"); // pre-v4 caches embedded full history
        if !obj.contains_key("provider") {
            let source = obj.get("source").and_then(Value::as_str).unwrap_or("");
            let provider = if source.starts_with("Codex") {
                "codex"
            } else {
                "claude"
            };
            obj.insert("provider".to_string(), Value::String(provider.to_string()));
        }
        if let Ok(rec) = serde_json::from_value::<SessionRecord>(v) {
            out.push(rec);
        }
    }
    out
}

pub fn save_cache(dir: &Path, sessions: &[SessionRecord]) -> std::io::Result<()> {
    let bytes = serde_json::to_vec(sessions)?;
    atomic_write(&dir.join(CACHE_FILE), &bytes)
}

pub fn load_scan_index(dir: &Path) -> ScanIndex {
    match std::fs::read_to_string(dir.join(SCAN_INDEX_FILE)) {
        Ok(raw) => serde_json::from_str(&raw).unwrap_or_default(),
        Err(_) => ScanIndex::new(),
    }
}

pub fn save_scan_index(dir: &Path, index: &ScanIndex) -> std::io::Result<()> {
    let bytes = serde_json::to_vec(index)?;
    atomic_write(&dir.join(SCAN_INDEX_FILE), &bytes)
}

/// Load caches from `data_dir`, run an incremental collect, persist the results, and
/// return the dashboard payload. This is the entrypoint the desktop command calls.
pub fn collect_and_persist(
    home: &Path,
    pricing: &Pricing,
    data_dir: &Path,
) -> std::io::Result<CollectOutput> {
    std::fs::create_dir_all(data_dir)?;
    let caches = Caches {
        scan_index: load_scan_index(data_dir),
        cached_sessions: load_cache(data_dir),
    };
    let res = collect(home, pricing, &caches);
    // Best-effort persistence: a failed cache write shouldn't fail the whole request.
    let _ = save_cache(data_dir, &res.sessions);
    let _ = save_scan_index(data_dir, &res.scan_index);
    Ok(res.output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collector::Provider;

    struct Tmp(std::path::PathBuf);
    impl Tmp {
        fn new() -> Self {
            use std::sync::atomic::{AtomicU32, Ordering};
            static N: AtomicU32 = AtomicU32::new(0);
            let n = N.fetch_add(1, Ordering::Relaxed);
            let d = std::env::temp_dir().join(format!("munim-cache-{}-{}", std::process::id(), n));
            std::fs::create_dir_all(&d).unwrap();
            Tmp(d)
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

    #[test]
    fn cache_round_trips_and_backfills() {
        let dir = Tmp::new();
        // Legacy entry: no `provider`, plus a `history` blob to strip.
        let legacy = r#"[
          {"date":"2026-07-01","time":"09:00","source":"Codex","file":"r.jsonl","cost":2.5,
           "input_tokens":10,"output_tokens":5,"cache_read":0,"cache_write":0,"model":"gpt-5.4",
           "history":[{"role":"user"}]},
          {"date":"2026-07-02","time":"09:00","source":"Claude Code","file":"c.jsonl","cost":1.0,
           "input_tokens":10,"output_tokens":5,"cache_read":0,"cache_write":0,"model":"claude-sonnet-4-5"},
          {"garbage":true}
        ]"#;
        std::fs::write(dir.path().join(CACHE_FILE), legacy).unwrap();

        let loaded = load_cache(dir.path());
        assert_eq!(loaded.len(), 2, "malformed entry dropped");
        assert_eq!(
            loaded[0].provider,
            Provider::Codex,
            "provider backfilled from source"
        );
        assert_eq!(loaded[1].provider, Provider::Claude);

        // Re-save and reload is stable.
        save_cache(dir.path(), &loaded).unwrap();
        let reloaded = load_cache(dir.path());
        assert_eq!(reloaded.len(), 2);
    }

    #[test]
    fn missing_files_are_empty() {
        let dir = Tmp::new();
        assert!(load_cache(dir.path()).is_empty());
        assert!(load_scan_index(dir.path()).is_empty());
    }
}
