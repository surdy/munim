//! Secure on-demand read of a session file for the detail modal's conversation history
//! (BUILD_SPEC §6, issue #10). The frontend parses the returned raw text; this module's job
//! is purely the security boundary: only files under known AI-tool roots, size-capped,
//! non-directory, with symlinks resolved so nothing escapes the allowlist.

use std::path::{Path, PathBuf};

/// Hard cap on a single session file we'll read (matches the original's 8 MB limit).
pub const MAX_SESSION_BYTES: u64 = 8 * 1024 * 1024;

/// macOS uses `~/Library/Application Support`; other platforms use XDG `~/.config`.
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

/// The allowlisted root directories a session file may live under (BUILD_SPEC §4.1/§6).
/// Only roots that exist are returned, canonicalized (symlinks resolved) for prefix checks.
fn allowed_roots(home: &Path) -> Vec<PathBuf> {
    let app = app_support(home);
    let candidates = [
        home.join(".claude"),
        home.join(".codex"),
        home.join(".openclaw"),
        home.join(".clawdbot"),
        home.join(".aider"),
        home.join(".continue"),
        home.join(".cursor"),
        home.join(".windsurf"),
        home.join(".cline"),
        home.join(".roo-code"),
        app.join("Claude"),
        app.join("Cursor"),
        app.join("Windsurf"),
        app.join("Code"),
    ];
    candidates
        .into_iter()
        .filter_map(|p| std::fs::canonicalize(p).ok())
        .collect()
}

/// Validate `file_path` against the allowlist and read it. Returns the raw file text, or an
/// error string safe to surface to the UI. Rejects: non-existent, directories, oversized
/// files, and anything (after symlink resolution) not under an allowlisted root.
pub fn load_session_file(home: &Path, file_path: &str) -> Result<String, String> {
    let canonical = std::fs::canonicalize(file_path).map_err(|_| "File not found.".to_string())?;

    let meta = std::fs::metadata(&canonical).map_err(|e| e.to_string())?;
    if meta.is_dir() {
        return Err("Not a file.".to_string());
    }
    if meta.len() > MAX_SESSION_BYTES {
        return Err("Session file too large to display.".to_string());
    }

    let roots = allowed_roots(home);
    if !roots.iter().any(|root| canonical.starts_with(root)) {
        return Err("File is outside the allowed session directories.".to_string());
    }

    std::fs::read_to_string(&canonical).map_err(|e| e.to_string())
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
            let d = std::env::temp_dir().join(format!("munim-detail-{}-{}", std::process::id(), n));
            std::fs::create_dir_all(&d).unwrap();
            // Canonicalize so tests comparing under `home` aren't broken by /var -> /private
            // symlinks on macOS.
            Tmp(std::fs::canonicalize(&d).unwrap())
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
        std::fs::File::create(path)
            .unwrap()
            .write_all(body.as_bytes())
            .unwrap();
    }

    #[test]
    fn reads_file_under_allowed_root() {
        let home = Tmp::new();
        let f = home.path().join(".claude/projects/p/s.jsonl");
        write(&f, "{\"hello\":true}\n");
        let out = load_session_file(home.path(), f.to_str().unwrap()).unwrap();
        assert!(out.contains("hello"));
    }

    #[test]
    fn rejects_outside_allowlist() {
        let home = Tmp::new();
        let other = Tmp::new(); // not under home
        let f = other.path().join("secret.jsonl");
        write(&f, "nope");
        let err = load_session_file(home.path(), f.to_str().unwrap()).unwrap_err();
        assert!(err.contains("outside"), "got: {err}");
    }

    #[test]
    fn rejects_symlink_escape() {
        let home = Tmp::new();
        let outside = Tmp::new();
        let secret = outside.path().join("secret.jsonl");
        write(&secret, "top secret");
        // A symlink INSIDE an allowed root pointing OUTSIDE must be rejected (canonicalize
        // resolves it, so the prefix check fails).
        let link_dir = home.path().join(".claude/projects");
        std::fs::create_dir_all(&link_dir).unwrap();
        let link = link_dir.join("link.jsonl");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&secret, &link).unwrap();
        #[cfg(unix)]
        {
            let err = load_session_file(home.path(), link.to_str().unwrap()).unwrap_err();
            assert!(err.contains("outside"), "symlink escape not blocked: {err}");
        }
    }

    #[test]
    fn rejects_directory_and_missing() {
        let home = Tmp::new();
        let dir = home.path().join(".claude/projects/p");
        std::fs::create_dir_all(&dir).unwrap();
        assert!(load_session_file(home.path(), dir.to_str().unwrap()).is_err());
        assert!(load_session_file(home.path(), "/no/such/file.jsonl").is_err());
    }

    #[test]
    fn rejects_oversized() {
        let home = Tmp::new();
        let f = home.path().join(".claude/projects/p/big.jsonl");
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        let big = vec![b'x'; (MAX_SESSION_BYTES + 1) as usize];
        std::fs::write(&f, big).unwrap();
        let err = load_session_file(home.path(), f.to_str().unwrap()).unwrap_err();
        assert!(err.contains("too large"), "got: {err}");
    }
}
