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

/// On-disk shape of `scan-index.json`.
///
/// The `pricing` fingerprint is what makes a rate change take effect: cached records
/// carry a precomputed cost and unchanged files are replayed verbatim, so without it an
/// edit to `pricing.toml` only affects sessions that happen to be rewritten afterwards.
///
/// Older builds wrote a bare `{path: fingerprint}` map. That fails to deserialize here,
/// which `load_scan_index` turns into an empty index — a one-time full re-price on
/// upgrade, which is exactly what a pricing correction needs.
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct PersistedIndex {
    #[serde(default)]
    pricing: String,
    #[serde(default)]
    files: ScanIndex,
}

/// Returns an empty index when the file is missing, unreadable, written by an older
/// build, or was produced under different pricing — every case means "re-scan".
pub fn load_scan_index(dir: &Path, pricing_fingerprint: &str) -> ScanIndex {
    let Ok(raw) = std::fs::read_to_string(dir.join(SCAN_INDEX_FILE)) else {
        return ScanIndex::new();
    };
    let Ok(parsed) = serde_json::from_str::<PersistedIndex>(&raw) else {
        return ScanIndex::new();
    };
    if parsed.pricing != pricing_fingerprint {
        return ScanIndex::new();
    }
    parsed.files
}

pub fn save_scan_index(
    dir: &Path,
    index: &ScanIndex,
    pricing_fingerprint: &str,
) -> std::io::Result<()> {
    let bytes = serde_json::to_vec(&PersistedIndex {
        pricing: pricing_fingerprint.to_string(),
        files: index.clone(),
    })?;
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
    let fingerprint = pricing.fingerprint();
    let caches = Caches {
        // An empty index here (new install, or rates changed) re-parses every file, which
        // re-prices every session against the current table.
        scan_index: load_scan_index(data_dir, &fingerprint),
        cached_sessions: load_cache(data_dir),
    };
    let res = collect(home, pricing, &caches);
    // Best-effort persistence: a failed cache write shouldn't fail the whole request.
    let _ = save_cache(data_dir, &res.sessions);
    let _ = save_scan_index(data_dir, &res.scan_index, &fingerprint);
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
        assert!(load_scan_index(dir.path(), "abc").is_empty());
    }

    #[test]
    fn scan_index_round_trips_under_the_same_pricing() {
        let dir = Tmp::new();
        let mut idx = ScanIndex::new();
        idx.insert(
            "/tmp/a.jsonl".into(),
            crate::collector::Fingerprint {
                mtime: 1,
                size: 100,
            },
        );
        save_scan_index(dir.path(), &idx, "rates-v1").unwrap();
        assert_eq!(load_scan_index(dir.path(), "rates-v1").len(), 1);
    }

    /// The whole point of the fingerprint: changed rates must drop the index so every
    /// file is re-parsed and re-priced instead of replaying a stale cached cost.
    #[test]
    fn changed_pricing_discards_the_scan_index() {
        let dir = Tmp::new();
        let mut idx = ScanIndex::new();
        idx.insert(
            "/tmp/a.jsonl".into(),
            crate::collector::Fingerprint {
                mtime: 1,
                size: 100,
            },
        );
        save_scan_index(dir.path(), &idx, "rates-v1").unwrap();
        assert!(load_scan_index(dir.path(), "rates-v2").is_empty());
    }

    /// Index files from builds before the fingerprint existed (a bare path→fingerprint
    /// map) must be treated as stale rather than parsed as valid.
    #[test]
    fn legacy_bare_map_index_is_discarded() {
        let dir = Tmp::new();
        std::fs::write(
            dir.path().join(SCAN_INDEX_FILE),
            br#"{"/tmp/a.jsonl":{"mtime":1,"size":100}}"#,
        )
        .unwrap();
        assert!(load_scan_index(dir.path(), "rates-v1").is_empty());
    }
}
