// Settings panel wiring (BUILD_SPEC §5.2b). Self-initializing ES module.
//
// Opens/closes the settings modal, loads persisted settings via the `get_settings`
// invoke on open, and persists { monthlyBudget, launchAtLogin } via `save_settings`.
// The budget VALUE is only stored here — the budget bar + alerts are a later ticket (#8).

function initSettings() {
    const tauri = window.__TAURI__;
    if (!tauri || !tauri.core) return; // Not running inside Tauri (e.g. plain browser).
    const invoke = tauri.core.invoke;

    const modal = document.getElementById('settings-modal');
    const openBtn = document.getElementById('settings-btn');
    const closeBtn = document.getElementById('settings-close');
    const overlay = document.getElementById('settings-overlay');
    const saveBtn = document.getElementById('settings-save');
    const budgetInput = document.getElementById('settings-budget');
    const autostartInput = document.getElementById('settings-autostart');
    if (!modal || !openBtn) return;

    function isOpen() {
        return !modal.classList.contains('settings-hidden');
    }

    async function open() {
        try {
            const s = await invoke('get_settings');
            budgetInput.value =
                s && s.monthlyBudget != null ? String(s.monthlyBudget) : '';
            autostartInput.checked = !!(s && s.launchAtLogin);
        } catch (e) {
            console.error('munim: failed to load settings', e);
            budgetInput.value = '';
            autostartInput.checked = false;
        }
        modal.classList.remove('settings-hidden');
        budgetInput.focus();
    }

    function close() {
        modal.classList.add('settings-hidden');
    }

    async function save() {
        const raw = budgetInput.value.trim();
        let monthlyBudget = null;
        if (raw !== '') {
            const n = Number(raw);
            monthlyBudget = Number.isFinite(n) && n > 0 ? n : null;
        }
        const settings = {
            monthlyBudget,
            launchAtLogin: !!autostartInput.checked,
        };
        try {
            await invoke('save_settings', { settings });
            window._showExportToast?.('Settings saved');
        } catch (e) {
            console.error('munim: failed to save settings', e);
            window._showExportToast?.('Failed to save settings', true);
        }
        close();
    }

    openBtn.addEventListener('click', open);
    closeBtn?.addEventListener('click', close);
    overlay?.addEventListener('click', close);
    saveBtn?.addEventListener('click', save);

    document.addEventListener('keydown', (e) => {
        if (e.key === 'Escape' && isOpen()) close();
    });
}

if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', initSettings);
} else {
    initSettings();
}
