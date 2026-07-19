const EXPORT_VERSION = 2;
const EXPORT_FORMAT = 'ai-usage-tracker';
// Pre-rename exports used 'claude-usage-tracker' — still accepted on import.
const ACCEPTED_FORMATS = new Set(['ai-usage-tracker', 'claude-usage-tracker']);

// v1 exports predate the `provider` field — tag those as Claude.
function backfillLegacyProvider(data) {
    const ver = Number(data._version) || 1;
    if (ver >= 2) return;
    for (const s of data.sessions) {
        if (s && !s.provider) s.provider = 'claude';
    }
    data._version = EXPORT_VERSION;
}

export function exportData(summary, sessions) {
    const payload = {
        _format: EXPORT_FORMAT,
        _version: EXPORT_VERSION,
        exported_at: new Date().toISOString(),
        summary,
        sessions,
    };

    const json = JSON.stringify(payload, null, 2);

    try {
        window.webkit.messageHandlers.exportData.postMessage(json);
    } catch {
        const blob = new Blob([json], { type: 'application/json' });
        const url = URL.createObjectURL(blob);
        const dateStr = new Date().toISOString().slice(0, 10);
        const a = document.createElement('a');
        a.href = url;
        a.download = `ai-usage-${dateStr}.json`;
        document.body.appendChild(a);
        a.click();
        document.body.removeChild(a);
        URL.revokeObjectURL(url);
        showToast('Exported ' + sessions.length + ' sessions');
    }
}

export function importData() {
    return new Promise((resolve) => {
        window._importDataResolver = (jsonString) => {
            delete window._importDataResolver;
            if (!jsonString) return resolve(null);

            try {
                const data = JSON.parse(jsonString);
                if (!ACCEPTED_FORMATS.has(data._format) || !Array.isArray(data.sessions)) {
                    showToast('Invalid file format', true);
                    return resolve(null);
                }
                backfillLegacyProvider(data);
                showToast('Imported ' + data.sessions.length + ' sessions');
                resolve(data);
            } catch {
                showToast('Failed to parse file', true);
                resolve(null);
            }
        };

        try {
            window.webkit.messageHandlers.importData.postMessage('');
        } catch {
            delete window._importDataResolver;
            const input = document.createElement('input');
            input.type = 'file';
            input.accept = '.json';
            input.addEventListener('change', () => {
                const file = input.files[0];
                if (!file) return resolve(null);
                const reader = new FileReader();
                reader.onload = () => {
                    try {
                        const data = JSON.parse(reader.result);
                        if (!ACCEPTED_FORMATS.has(data._format) || !Array.isArray(data.sessions)) {
                            showToast('Invalid file format', true);
                            return resolve(null);
                        }
                        backfillLegacyProvider(data);
                        showToast('Imported ' + data.sessions.length + ' sessions');
                        resolve(data);
                    } catch {
                        showToast('Failed to parse file', true);
                        resolve(null);
                    }
                };
                reader.readAsText(file);
            });
            input.addEventListener('cancel', () => resolve(null));
            input.click();
        }
    });
}

// sessionId alone is not unique: Claude Code sub-agents share one sessionId
// across multiple .jsonl files — mirror the collector's full key here.
export function mergeSessions(existing, incoming) {
    const seen = new Set();
    const merged = [];
    const keyOf = (s) => (s.provider || 'claude') + '|' + (s.source || '') + '|' + (s.file || '') + '|' + s.date;

    for (const s of existing) {
        seen.add(keyOf(s));
        merged.push(s);
    }

    for (const s of incoming) {
        const key = keyOf(s);
        if (!seen.has(key)) {
            seen.add(key);
            merged.push(s);
        }
    }

    merged.sort((a, b) => {
        if (a.date !== b.date) return b.date.localeCompare(a.date);
        return (b.time || '').localeCompare(a.time || '');
    });

    return merged;
}

export function recalcSummary(sessions) {
    const today = new Date().toISOString().slice(0, 10);
    const currentMonth = today.slice(0, 7);

    const totals = {};
    let grandTotal = 0;
    const sourceCounts = {};
    let todayCost = 0;
    let monthCost = 0;

    for (const s of sessions) {
        const src = s.source || 'Unknown';
        totals[src] = (totals[src] || 0) + s.cost;
        grandTotal += s.cost;
        sourceCounts[src] = (sourceCounts[src] || 0) + 1;

        if (s.date === today) todayCost += s.cost;
        if (s.date && s.date.startsWith(currentMonth)) monthCost += s.cost;
    }

    totals.grand_total = grandTotal;
    sourceCounts.total = sessions.length;

    return {
        generated_at: new Date().toISOString(),
        today,
        current_month: currentMonth,
        totals,
        today_cost: todayCost,
        month_cost: monthCost,
        session_counts: sourceCounts,
    };
}

let toastTimer = null;

export function showToast(message, isError = false) {
    let toast = document.getElementById('data-transfer-toast');
    if (!toast) {
        toast = document.createElement('div');
        toast.id = 'data-transfer-toast';
        toast.className = 'dt-toast';
        document.body.appendChild(toast);
    }

    toast.textContent = message;
    toast.classList.toggle('dt-toast-error', isError);
    toast.classList.remove('dt-toast-visible');

    void toast.offsetWidth;
    toast.classList.add('dt-toast-visible');

    clearTimeout(toastTimer);
    toastTimer = setTimeout(() => {
        toast.classList.remove('dt-toast-visible');
    }, 2500);
}

window._showExportToast = (msg, isErr) => showToast(msg, isErr);
