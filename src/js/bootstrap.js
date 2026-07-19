// munim frontend bootstrap.
//
// The ported dashboard (main.js) expects the usage payload on window globals, exactly as
// the original macOS app injected them. In Tauri we instead fetch it over the invoke()
// bridge (get_usage_data — BUILD_SPEC §3), set the globals, wire refresh, then hand off to
// main.js. Runs before main.js because it is the module entry point in index.html.
//
// Not shimmed here: export/import and session-detail bridges. The original main.js already
// has browser fallbacks for those, so they degrade gracefully until #9/#10 wire real
// Tauri commands.

const tauri = window.__TAURI__;

async function loadUsage() {
    if (!tauri?.core?.invoke) {
        // Browser preview (no Tauri): leave globals unset; main.js shows its "no data" state.
        console.warn('[munim] Tauri API not present — running without live data.');
        return;
    }
    try {
        const data = await tauri.core.invoke('get_usage_data');
        window.__SUMMARY__ = data.summary;
        window.__CLAUDE_SESSIONS__ = data.claude || [];
        window.__CODEX_SESSIONS__ = data.codex || [];
        window.__OPENCLAW_SESSIONS__ = data.openclaw || [];
    } catch (err) {
        console.error('[munim] get_usage_data failed:', err);
    }
    // Settings power the budget bar (issue #8); refresh alongside usage so the bar stays
    // current on load and on auto-refresh. Best-effort — absence just hides the bar.
    try {
        window.__SETTINGS__ = await tauri.core.invoke('get_settings');
    } catch (err) {
        console.error('[munim] get_settings failed:', err);
    }
}

function hideSplash() {
    const splash = document.getElementById('munim-splash');
    if (!splash) return;
    splash.classList.add('hide');
    setTimeout(() => splash.remove(), 400);
}

// Compatibility bridge (issues #9/#10). The original components talked to the native macOS
// bridge via window.webkit.messageHandlers.*. In a WKWebView that object is READ-ONLY (and
// Tauri's own IPC lives on it), so we must NOT mutate it. Instead we expose a `window.__munim`
// namespace and point the (lightly edited) component call-sites at it.
function installBridge(invoke) {
    window.__munim = {
        // Export: frontend passes the assembled JSON; Rust shows a save dialog + writes it.
        exportData: async (json) => {
            try {
                const res = await invoke('export_data', { json });
                if (res?.saved) {
                    window._showExportToast?.('Exported ' + (res.count ?? '') + ' sessions');
                }
            } catch (e) {
                console.error('[munim] export failed:', e);
                window._showExportToast?.('Export failed', true);
            }
        },
        // Import: Rust shows an open dialog + returns the file text; resolve the JS awaiter.
        importData: async () => {
            let text = null;
            try {
                text = await invoke('import_data');
            } catch (e) {
                console.error('[munim] import failed:', e);
            }
            window._importDataResolver?.(text || '');
        },
        // Persist frontend-merged records to the session cache.
        saveImportedData: async (records) => {
            try {
                await invoke('save_imported_data', { records });
            } catch (e) {
                console.error('[munim] saveImportedData failed:', e);
            }
        },
        // Session detail: returns the raw file text (Rust enforces the allowlist).
        loadSessionDetail: (filePath) => invoke('load_session_detail', { filePath }),
    };
}

// Menu "Refresh" (Cmd/Ctrl+R) emits `menu-refresh` from the native shell; the reload FAB
// falls back to location.reload() on its own. Either way a reload re-runs this bootstrap,
// which re-invokes get_usage_data (an incremental re-scan).
if (tauri?.event?.listen) {
    tauri.event.listen('menu-refresh', () => location.reload());
}

// Auto-refresh (BUILD_SPEC §4.8): the native shell emits `usage-updated` when a source
// file changes on disk (debounced) or every 60s. Handle it SILENTLY — re-fetch the data,
// refresh the window.__* globals, and re-render in place via main.js's __munimRefresh hook.
// No splash, no toast, no location.reload(). A simple in-flight flag prevents overlap.
if (tauri?.event?.listen && tauri?.core?.invoke) {
    let refreshing = false;
    tauri.event.listen('usage-updated', async () => {
        if (refreshing) return;
        refreshing = true;
        try {
            await loadUsage();
            window.__munimRefresh?.();
        } catch (err) {
            console.error('[munim] auto-refresh failed:', err);
        } finally {
            refreshing = false;
        }
    });
}
if (tauri?.core?.invoke) {
    installBridge(tauri.core.invoke);
}

await loadUsage();
await import('./main.js');
// main.js self-inits on import (DOM already loaded); give it a frame to paint, then reveal.
requestAnimationFrame(hideSplash);
