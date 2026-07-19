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
}

function hideSplash() {
    const splash = document.getElementById('munim-splash');
    if (!splash) return;
    splash.classList.add('hide');
    setTimeout(() => splash.remove(), 400);
}

// Menu "Refresh" (Cmd/Ctrl+R) emits `menu-refresh` from the native shell; the reload FAB
// falls back to location.reload() on its own. Either way a reload re-runs this bootstrap,
// which re-invokes get_usage_data (an incremental re-scan).
if (tauri?.event?.listen) {
    tauri.event.listen('menu-refresh', () => location.reload());
}

await loadUsage();
await import('./main.js');
// main.js self-inits on import (DOM already loaded); give it a frame to paint, then reveal.
requestAnimationFrame(hideSplash);
