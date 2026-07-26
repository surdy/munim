import { formatNumber } from '../utils/formatters.js';
import { getWeekStart, getWeekEnd, formatWeekLabel } from '../utils/date-utils.js';
import { getModelInfo } from '../utils/model-utils.js';
import { costClass, sourceClass } from '../utils/class-utils.js';
import { loadSessionConversation } from '../utils/session-detail-loader.js';

let mostExpensiveFile = null;
let mostExpensiveDate = null;

// Keyed by a stable per-session identity rather than by array position. Incremental
// re-renders rebuild only the days that actually changed, and a positional index would
// either go stale or grow without bound as those rebuilds re-register their rows.
// Re-registering a session overwrites its entry, so the store stays bounded by the
// number of distinct sessions no matter how often a day is patched.
const _sessionDetailStore = new Map();
const _builtDays = new Set();
let _daySessionsMap = {};

export function resetSessionStore() {
    _sessionDetailStore.clear();
}

export function registerSession(session) {
    const id = session.sessionId || session.filePath || session.file || 'unknown';
    const key = `${session.provider || 'claude'}|${id}|${session.date || ''}|${session.time || ''}`;
    _sessionDetailStore.set(key, session);
    return key;
}

// \u0001 / \u0002 are the ASCII unit/record separators. They cannot occur in any of the
// fields below, so the joined signature is unambiguous.
const FIELD_SEP = '\u0001';
const RECORD_SEP = '\u0002';

// Every field the day/project detail rows render. Two renders with equal signatures
// produce byte-identical markup, which is what lets an unchanged day be skipped outright.
export function sessionsSignature(list) {
    const parts = [String(list.length)];
    for (const x of list) {
        parts.push([
            x.sessionId || x.file || '', x.date || '', x.time || '',
            x.source || '', x.model || '', x.cost,
            x.input_tokens || 0, x.output_tokens || 0, x.cache_read || 0, x.cache_write || 0,
            x.title || '', x.cwd || '',
        ].join(FIELD_SEP));
    }
    return parts.join(RECORD_SEP);
}

export function setMostExpensive(file, date) {
    mostExpensiveFile = file;
    mostExpensiveDate = date;
}

function aggregate(list) {
    let cost = 0, input = 0, output = 0, cacheRead = 0, cacheWrite = 0;
    const modelSet = new Set();
    for (const x of list) {
        cost += x.cost;
        input += x.input_tokens || 0;
        output += x.output_tokens || 0;
        cacheRead += x.cache_read || 0;
        cacheWrite += x.cache_write || 0;
        if (x.model) modelSet.add(x.model);
    }
    return { cost, input, output, cacheRead, cacheWrite, count: list.length, models: [...modelSet] };
}

// Cells only — the caller owns the <tr>, so patching a day in place preserves its id,
// data-day and `expanded` class (and therefore the open detail panel below it).
function dayRowCells(date, agg) {
    const dateLabel = new Date(date + 'T00:00:00').toLocaleDateString('en-US', { weekday: 'short', month: 'short', day: 'numeric' });
    const modelBadges = agg.models.map(m => {
        const mi = getModelInfo(m);
        return `<span class="model-badge ${mi.cls}">${mi.name}</span>`;
    }).join(' ');

    return `<td><span class="chevron">▶</span>${dateLabel}</td>
                <td>${agg.count}</td>
                <td>${modelBadges}</td>
                <td class="token-cell">${formatNumber(agg.input)}</td>
                <td class="token-cell">${formatNumber(agg.output)}</td>
                <td class="token-cell">${formatNumber(agg.cacheRead)}</td>
                <td class="token-cell">${formatNumber(agg.cacheWrite)}</td>
                <td style="text-align:right"><span class="cost-badge ${costClass(agg.cost)}">$${agg.cost.toFixed(2)}</span></td>`;
}

function weekRowCells(weekStart, agg) {
    return `<td colspan="8">
                <div class="week-strip">
                    <div class="week-strip-left">
                        <span class="week-strip-icon">Σ</span>
                        <span class="week-strip-label">${formatWeekLabel(weekStart)}</span>
                    </div>
                    <div class="week-strip-stats">
                        <span class="week-stat"><span class="week-stat-label">Sessions</span><span class="week-stat-value">${agg.count}</span></span>
                        <span class="week-stat-divider"></span>
                        <span class="week-stat"><span class="week-stat-label">In</span><span class="week-stat-value">${formatNumber(agg.input)}</span></span>
                        <span class="week-stat"><span class="week-stat-label">Out</span><span class="week-stat-value">${formatNumber(agg.output)}</span></span>
                        <span class="week-stat-divider"></span>
                        <span class="week-strip-cost">$${agg.cost.toFixed(2)}</span>
                    </div>
                </div>
            </td>`;
}

// Lazily materialize a day's detail markup the first time it is opened.
function buildDayDetailOnce(date, detailWrapper) {
    if (!_builtDays.has(date) && _daySessionsMap[date]) {
        detailWrapper.innerHTML = buildDayDetail(date, _daySessionsMap[date]);
        _builtDays.add(date);
    }
}

function setCostBarWidths(detailWrapper) {
    detailWrapper.querySelectorAll('.cost-bar-fill').forEach(bar => {
        bar.style.transform = `scaleX(${parseFloat(bar.dataset.width) / 100})`;
    });
}

// `animate: false` is for state restored after a silent re-render. The detail markup is
// rebuilt from scratch every time, so without this the source cards replay their
// fade-in and the cost bars re-grow from zero on every auto-refresh — a visible flash on
// a row the user never touched. Suppressed via CSS (.no-intro) so the restored rows just
// stay as they were.
function expandDay(date, row, detailWrapper, { animate = true } = {}) {
    if (!animate) detailWrapper.classList.add('no-intro');
    buildDayDetailOnce(date, detailWrapper);
    row.classList.add('expanded');
    detailWrapper.classList.add('open');

    if (animate) {
        setTimeout(() => setCostBarWidths(detailWrapper), 50);
    } else {
        setCostBarWidths(detailWrapper);
    }
}

export function toggleDay(date) {
    const row = document.getElementById('day-' + date);
    const detailWrapper = document.getElementById('detail-wrapper-' + date);
    if (!row || !detailWrapper) return;

    if (row.classList.contains('expanded')) {
        row.classList.remove('expanded');
        detailWrapper.classList.remove('open');
    } else {
        expandDay(date, row, detailWrapper);
    }

    const anyExpanded = document.querySelectorAll('.day-row.expanded').length > 0;
    updateToggleAllButton(anyExpanded);
}

export function toggleAllDays() {
    const dayRows = document.querySelectorAll('.day-row');
    if (dayRows.length === 0) return;

    const anyExpanded = document.querySelectorAll('.day-row.expanded').length > 0;
    const shouldExpand = !anyExpanded;

    dayRows.forEach((row, index) => {
        const date = row.id.replace('day-', '');
        const detailWrapper = document.getElementById('detail-wrapper-' + date);
        if (!detailWrapper) return;

        setTimeout(() => {
            if (shouldExpand && !row.classList.contains('expanded')) {
                expandDay(date, row, detailWrapper);
            } else if (!shouldExpand && row.classList.contains('expanded')) {
                row.classList.remove('expanded');
                detailWrapper.classList.remove('open');
            }
        }, index * 10);
    });

    updateToggleAllButton(shouldExpand);
}

export function updateToggleAllButton(anyExpanded) {
    const btn = document.getElementById('toggle-all-btn');
    if (!btn) return;

    if (anyExpanded) {
        btn.innerHTML = 'Collapse All<span class="arrow">&#9660;</span><span class="kbd-hint">Shift+E</span>';
        btn.classList.add('is-expanded');
    } else {
        btn.innerHTML = 'Expand All<span class="arrow">&#9660;</span><span class="kbd-hint">Shift+E</span>';
        btn.classList.remove('is-expanded');
    }
}

export function updateTotalsRow(sessions) {
    const tfoot = document.getElementById('sessions-tfoot');
    if (!tfoot) return;

    if (!sessions || sessions.length === 0) {
        tfoot.innerHTML = '';
        return;
    }

    const totalSessions = sessions.length;
    let totalInput = 0, totalOutput = 0, totalCacheRead = 0, totalCacheWrite = 0, totalCost = 0;
    for (const s of sessions) {
        totalInput += s.input_tokens || 0;
        totalOutput += s.output_tokens || 0;
        totalCacheRead += s.cache_read || 0;
        totalCacheWrite += s.cache_write || 0;
        totalCost += s.cost;
    }

    tfoot.innerHTML = `<tr>
        <td>TOTAL</td>
        <td><span class="totals-session-count">${totalSessions}</span></td>
        <td><span class="totals-models-placeholder">--</span></td>
        <td class="token-cell">${formatNumber(totalInput)}</td>
        <td class="token-cell">${formatNumber(totalOutput)}</td>
        <td class="token-cell">${formatNumber(totalCacheRead)}</td>
        <td class="token-cell">${formatNumber(totalCacheWrite)}</td>
        <td style="text-align:right"><span class="cost-badge ${costClass(totalCost)}">$${totalCost.toFixed(2)}</span></td>
    </tr>`;
}

export function buildDayDetail(date, sessions) {
    const bySource = {};
    sessions.forEach(s => {
        if (!bySource[s.source]) bySource[s.source] = [];
        bySource[s.source].push(s);
    });

    const totalCost = sessions.reduce((sum, s) => sum + s.cost, 0);
    const maxSourceCost = Math.max(...Object.values(bySource).map(arr => arr.reduce((s, x) => s + x.cost, 0)));

    let sourceCardsHTML = '';
    for (const [source, items] of Object.entries(bySource)) {
        const sCost = items.reduce((s, x) => s + x.cost, 0);
        const sInput = items.reduce((s, x) => s + (x.input_tokens || 0), 0);
        const sOutput = items.reduce((s, x) => s + (x.output_tokens || 0), 0);
        const sCacheRead = items.reduce((s, x) => s + (x.cache_read || 0), 0);
        const sCacheWrite = items.reduce((s, x) => s + (x.cache_write || 0), 0);
        const models = [...new Set(items.map(x => x.model).filter(Boolean))];
        const sc = sourceClass(source);
        const barPct = maxSourceCost > 0 ? (sCost / maxSourceCost * 100).toFixed(1) : 0;

        sourceCardsHTML += `
            <div class="source-card border-${sc}">
                <div class="source-card-header">
                    <span class="source-name">
                        <span class="source-badge source-${sc}">${source}</span>
                        <span style="margin-left:6px;font-size:0.7rem;color:var(--text-muted);">${items.length} session${items.length > 1 ? 's' : ''}</span>
                    </span>
                    <span class="source-cost ${costClass(sCost) + '-text'}">${'$' + sCost.toFixed(2)}</span>
                </div>
                <div class="source-stats">
                    <div class="source-stat"><span class="stat-label">Input</span><span class="stat-value">${formatNumber(sInput)}</span></div>
                    <div class="source-stat"><span class="stat-label">Output</span><span class="stat-value">${formatNumber(sOutput)}</span></div>
                    <div class="source-stat"><span class="stat-label">Cache Read</span><span class="stat-value">${formatNumber(sCacheRead)}</span></div>
                    <div class="source-stat"><span class="stat-label">Cache Write</span><span class="stat-value">${formatNumber(sCacheWrite)}</span></div>
                    <div class="source-stat"><span class="stat-label">Models</span><span class="stat-value">${models.map(m => getModelInfo(m).name).join(', ') || '—'}</span></div>
                    <div class="source-stat"><span class="stat-label">% of Day</span><span class="stat-value">${totalCost > 0 ? (sCost / totalCost * 100).toFixed(1) : 0}%</span></div>
                </div>
                <div class="cost-bar-container">
                    <div class="cost-bar-bg">
                        <div class="cost-bar-fill fill-${sc}" data-width="${barPct}%"></div>
                    </div>
                </div>
            </div>`;
    }

    let subTableHTML = `
        <table class="session-subtable">
            <thead><tr>
                <th>Time</th><th>Title</th><th>Source</th><th>Model</th>
                <th>Input</th><th>Output</th><th>Cache R</th><th>Cache W</th><th style="text-align:right">Cost</th>
            </tr></thead><tbody>`;
    sessions.sort((a, b) => (b.time || '').localeCompare(a.time || ''));
    for (const s of sessions) {
        const mi = getModelInfo(s.model);
        const sc = sourceClass(s.source);
        const isExpensive = (s.file === mostExpensiveFile && date === mostExpensiveDate);
        const titleText = s.title ? s.title.replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;') : '—';
        const sessionKey = registerSession(s);
        subTableHTML += `<tr class="session-clickable${isExpensive ? ' expensive-session-row' : ''}" data-session-key="${escapeHTML(sessionKey)}">
            <td style="font-family:'JetBrains Mono',monospace;font-size:0.72rem;">${s.time || '—'}</td>
            <td class="session-title-cell" title="${titleText}">${titleText}</td>
            <td><span class="source-badge source-${sc}">${s.source}</span></td>
            <td><span class="model-badge ${mi.cls}">${mi.name}</span></td>
            <td class="token-cell">${formatNumber(s.input_tokens || 0)}</td>
            <td class="token-cell">${formatNumber(s.output_tokens || 0)}</td>
            <td class="token-cell">${formatNumber(s.cache_read || 0)}</td>
            <td class="token-cell">${formatNumber(s.cache_write || 0)}</td>
            <td style="text-align:right"><span class="cost-badge ${costClass(s.cost)}">$${s.cost.toFixed(2)}</span></td>
        </tr>`;
    }
    subTableHTML += '</tbody></table>';

    return `
        <div class="day-detail">
            <div class="source-breakdown">${sourceCardsHTML}</div>
            ${subTableHTML}
        </div>`;
}

// Last rendered shape, so a re-render can tell what (if anything) actually moved.
// `structureKey` covers which weeks/days exist and in what order; a change there means
// rows appear, vanish or reorder, so we fall back to rebuilding the whole tbody.
let _tableState = null;

function tableGroups(sessions) {
    const byDate = {};
    for (const s of sessions) {
        if (!byDate[s.date]) byDate[s.date] = [];
        byDate[s.date].push(s);
    }

    const sortedDates = Object.keys(byDate).sort().reverse();
    const weekGroups = {};
    for (const date of sortedDates) {
        const ws = getWeekStart(date);
        if (!weekGroups[ws]) weekGroups[ws] = [];
        weekGroups[ws].push(date);
    }
    const sortedWeeks = Object.keys(weekGroups).sort().reverse();

    const dayAggs = new Map();
    const daySigs = new Map();
    for (const date of sortedDates) {
        dayAggs.set(date, aggregate(byDate[date]));
        daySigs.set(date, sessionsSignature(byDate[date]));
    }

    const weekAggs = new Map();
    const weekSigs = new Map();
    for (const weekStart of sortedWeeks) {
        const agg = { cost: 0, input: 0, output: 0, cacheRead: 0, cacheWrite: 0, count: 0, models: [] };
        for (const date of weekGroups[weekStart]) {
            const d = dayAggs.get(date);
            agg.cost += d.cost;
            agg.input += d.input;
            agg.output += d.output;
            agg.cacheRead += d.cacheRead;
            agg.cacheWrite += d.cacheWrite;
            agg.count += d.count;
        }
        weekAggs.set(weekStart, agg);
        weekSigs.set(weekStart, [
            agg.count, agg.input, agg.output, agg.cacheRead, agg.cacheWrite, agg.cost,
        ].join(FIELD_SEP));
    }

    // The most-expensive marker decorates one detail row, so a change to it has to
    // invalidate the whole table rather than just the day whose numbers moved.
    const structureKey = JSON.stringify({
        weeks: sortedWeeks.map(w => [w, weekGroups[w]]),
        expensive: [mostExpensiveFile, mostExpensiveDate],
    });

    return { byDate, sortedDates, weekGroups, sortedWeeks, dayAggs, daySigs, weekAggs, weekSigs, structureKey };
}

// Re-render only the days and week strips whose numbers moved. Rows left alone keep their
// DOM untouched, so an open detail panel, its painted cost bars and the scroll position
// all survive. Returns false if the DOM isn't the shape we recorded, so the caller can
// fall back to a full rebuild.
function patchSessionTable(g, changedDays, changedWeeks) {
    for (const date of changedDays) {
        const row = document.getElementById('day-' + date);
        if (!row) return false;
        row.innerHTML = dayRowCells(date, g.dayAggs.get(date));

        // Only days whose detail is already materialized need their panel refreshed;
        // the rest rebuild lazily from _daySessionsMap the next time they're opened.
        if (_builtDays.has(date)) {
            const wrapper = document.getElementById('detail-wrapper-' + date);
            if (!wrapper) return false;
            // This is a background update, so suppress the source-card intro even if the
            // user had opened this row by hand (which leaves .no-intro off).
            wrapper.classList.add('no-intro');
            wrapper.innerHTML = buildDayDetail(date, g.byDate[date]);
            setCostBarWidths(wrapper);
        }
    }

    for (const weekStart of changedWeeks) {
        const row = document.getElementById('week-' + weekStart);
        if (!row) return false;
        row.innerHTML = weekRowCells(weekStart, g.weekAggs.get(weekStart));
    }

    return true;
}

export function renderSessionTable(sessions) {
    const tbody = document.getElementById('sessions-body');
    const g = tableGroups(sessions);

    if (g.sortedDates.length === 0) {
        resetSessionStore();
        _builtDays.clear();
        _daySessionsMap = g.byDate;
        _tableState = null;
        tbody.innerHTML = '<tr><td colspan="8" class="no-data">No sessions match the current filters.</td></tr>';
        updateTotalsRow([]);
        updateToggleAllButton(false);
        return;
    }

    // Fast paths. A silent auto-refresh normally changes one day (today) or nothing at
    // all; rebuilding the entire tbody for that is what made expanded rows collapse,
    // flash, and lose their place. The getElementById guard also catches the case where
    // the projects view owns the tbody, in which case we must do a full rebuild.
    if (_tableState
        && _tableState.structureKey === g.structureKey
        && document.getElementById('day-' + g.sortedDates[0])) {

        const changedDays = g.sortedDates.filter(d => _tableState.days.get(d) !== g.daySigs.get(d));
        const changedWeeks = g.sortedWeeks.filter(w => _tableState.weeks.get(w) !== g.weekSigs.get(w));

        // Sessions are re-registered per day as details are rebuilt, so the store only
        // needs the fresh objects for days we touch.
        _daySessionsMap = g.byDate;

        if (changedDays.length === 0 && changedWeeks.length === 0) {
            updateTotalsRow(sessions);
            return;                                     // nothing moved — touch nothing
        }

        if (patchSessionTable(g, changedDays, changedWeeks)) {
            _tableState = { structureKey: g.structureKey, days: g.daySigs, weeks: g.weekSigs };
            updateTotalsRow(sessions);
            return;
        }
        // DOM didn't match what we recorded — fall through and rebuild from scratch.
    }

    // Full rebuild. Remember which days were open so they can be restored afterwards.
    const wasExpanded = new Set(
        Array.from(document.querySelectorAll('.day-row.expanded')).map(r => r.dataset.day)
    );

    resetSessionStore();
    _builtDays.clear();
    _daySessionsMap = g.byDate;

    let html = '';
    for (const weekStart of g.sortedWeeks) {
        for (const date of g.weekGroups[weekStart]) {
            html += `<tr class="day-row" id="day-${date}" data-day="${date}">${dayRowCells(date, g.dayAggs.get(date))}</tr>`;
            html += `<tr class="day-detail-row"><td colspan="8">
                <div class="day-detail-wrapper" id="detail-wrapper-${date}"></div>
            </td></tr>`;
        }
        html += `<tr class="week-row" id="week-${weekStart}">${weekRowCells(weekStart, g.weekAggs.get(weekStart))}</tr>`;
    }
    tbody.innerHTML = html;

    let restored = 0;
    for (const date of wasExpanded) {
        const row = document.getElementById('day-' + date);
        const detailWrapper = document.getElementById('detail-wrapper-' + date);
        if (!row || !detailWrapper) continue;   // day filtered out of the new render
        expandDay(date, row, detailWrapper, { animate: false });
        restored++;
    }

    _tableState = { structureKey: g.structureKey, days: g.daySigs, weeks: g.weekSigs };
    updateTotalsRow(sessions);
    updateToggleAllButton(restored > 0);
}

// Incremented on every modal open so out-of-order async loads can't paint
// stale conversation history into a modal for a different session.
let _sessionDetailRequestId = 0;

function escapeHTML(str) {
    return String(str == null ? '' : str)
        .replace(/&/g, '&amp;')
        .replace(/</g, '&lt;')
        .replace(/>/g, '&gt;')
        .replace(/"/g, '&quot;');
}

function renderHistoryItems(turns, truncated, assistantLabel) {
    if (!turns || turns.length === 0) {
        return `<div class="history-empty">No conversation content recorded for this session.</div>`;
    }
    const aiLabel = assistantLabel || 'Claude';
    const items = turns.map((h, i) => `
        <div class="history-msg history-${h.role} history-entering" style="animation-delay:${Math.min(i * 28, 420)}ms">
            <div class="history-role">${h.role === 'user' ? 'You' : aiLabel}</div>
            <div class="history-text">${escapeHTML(h.text)}</div>
        </div>`).join('');
    const tail = truncated ? '<div class="history-truncated">… conversation continues</div>' : '';
    return items + tail;
}

export function showSessionDetail(key) {
    const s = _sessionDetailStore.get(key);
    if (!s) return;

    const requestId = ++_sessionDetailRequestId;

    const mi = getModelInfo(s.model);
    const sc = sourceClass(s.source);
    const titleText = s.title || '(untitled session)';
    const sessionId = s.sessionId || s.file?.replace('.jsonl', '') || '—';
    const hasSessionId = s.sessionId || (s.file && s.file.endsWith('.jsonl'));
    const isCodex = s.provider === 'codex';
    const canResume = isCodex || s.source === 'Claude Code';
    const resumeCmd = isCodex
        ? `codex resume ${sessionId}`
        : `claude --resume ${sessionId}`;

    const skeletonHTML = `
        <div class="history-skeleton" aria-hidden="true">
            <div class="history-msg history-user skeleton-msg"><div class="skeleton-line w-30"></div><div class="skeleton-line w-80"></div></div>
            <div class="history-msg history-ai skeleton-msg"><div class="skeleton-line w-20"></div><div class="skeleton-line w-90"></div><div class="skeleton-line w-60"></div></div>
            <div class="history-msg history-user skeleton-msg"><div class="skeleton-line w-30"></div><div class="skeleton-line w-70"></div></div>
        </div>`;

    const modalHTML = `
        <div class="session-modal-header">
            <div class="session-modal-title">${escapeHTML(titleText)}</div>
            <button class="session-modal-close" data-action="close-session-detail">&times;</button>
        </div>
        <div class="session-modal-body">
            <div class="session-modal-meta">
                <div class="session-meta-row">
                    <span class="meta-label">Date</span>
                    <span class="meta-value">${s.date} ${s.time || ''}</span>
                </div>
                <div class="session-meta-row">
                    <span class="meta-label">Source</span>
                    <span class="meta-value"><span class="source-badge source-${sc}">${s.source}</span></span>
                </div>
                <div class="session-meta-row">
                    <span class="meta-label">Model</span>
                    <span class="meta-value"><span class="model-badge ${mi.cls}">${mi.name}</span></span>
                </div>
                ${s.cwd ? `<div class="session-meta-row">
                    <span class="meta-label">Project</span>
                    <span class="meta-value meta-mono">${escapeHTML(s.cwd)}</span>
                </div>` : ''}
                <div class="session-meta-row">
                    <span class="meta-label">Session ID</span>
                    <span class="meta-value meta-mono">${escapeHTML(sessionId)}</span>
                </div>
            </div>
            <div class="session-modal-tokens">
                <div class="token-stat"><span class="token-stat-label">Input</span><span class="token-stat-value">${formatNumber(s.input_tokens || 0)}</span></div>
                <div class="token-stat"><span class="token-stat-label">Output</span><span class="token-stat-value">${formatNumber(s.output_tokens || 0)}</span></div>
                <div class="token-stat"><span class="token-stat-label">Cache Read</span><span class="token-stat-value">${formatNumber(s.cache_read || 0)}</span></div>
                <div class="token-stat"><span class="token-stat-label">Cache Write</span><span class="token-stat-value">${formatNumber(s.cache_write || 0)}</span></div>
                ${s.reasoning_tokens > 0 ? `<div class="token-stat"><span class="token-stat-label">Reasoning</span><span class="token-stat-value">${formatNumber(s.reasoning_tokens)}</span></div>` : ''}
                <div class="token-stat"><span class="token-stat-label">Cost</span><span class="token-stat-value cost-value ${costClass(s.cost)}">$${s.cost.toFixed(2)}</span></div>
            </div>
            <div class="session-modal-history" data-request-id="${requestId}">
                <div class="history-label">Conversation History</div>
                <div class="history-timeline" id="session-history-timeline">
                    ${skeletonHTML}
                </div>
            </div>
            ${hasSessionId && canResume ? `
            <div class="session-modal-resume">
                <div class="resume-label">Resume this session</div>
                <div class="resume-cmd-row">
                    <code class="resume-cmd">${escapeHTML(resumeCmd)}</code>
                    <button class="resume-copy-btn" data-copy="${escapeHTML(resumeCmd)}">Copy</button>
                </div>
                ${s.cwd ? `<div class="resume-cmd-row" style="margin-top:6px;">
                    <code class="resume-cmd">cd ${escapeHTML(s.cwd)} && ${escapeHTML(resumeCmd)}</code>
                    <button class="resume-copy-btn" data-copy="${escapeHTML(`cd ${s.cwd} && ${resumeCmd}`)}">Copy</button>
                </div>` : ''}
            </div>` : ''}
        </div>`;

    let overlay = document.getElementById('session-modal-overlay');
    let modal = document.getElementById('session-modal');
    if (!overlay) {
        overlay = document.createElement('div');
        overlay.id = 'session-modal-overlay';
        overlay.className = 'session-modal-overlay';
        overlay.onclick = closeSessionDetail;
        document.body.appendChild(overlay);
    }
    if (!modal) {
        modal = document.createElement('div');
        modal.id = 'session-modal';
        modal.className = 'session-modal';
        document.body.appendChild(modal);
    }
    modal.innerHTML = modalHTML;

    // The app CSP is `script-src 'self'` (no 'unsafe-inline'), so inline on* attributes
    // never fire — every handler has to be bound in JS. One delegated listener on the
    // modal shell survives the innerHTML swap above.
    if (!modal.dataset.wired) {
        modal.addEventListener('click', e => {
            if (e.target.closest('[data-action="close-session-detail"]')) {
                closeSessionDetail();
                return;
            }
            const copyBtn = e.target.closest('[data-copy]');
            if (copyBtn) copySessionCmd(copyBtn.dataset.copy, copyBtn);
        });
        modal.dataset.wired = '1';
    }

    requestAnimationFrame(() => {
        overlay.classList.add('visible');
        modal.classList.add('visible');
    });

    // Race guard via requestId — quick session switches always show the latest.
    const assistantLabel = s.provider === 'codex' ? 'Codex' : 'Claude';
    loadSessionConversation(s.filePath, { provider: s.provider }).then(result => {
        if (requestId !== _sessionDetailRequestId) return;
        const section = modal.querySelector('.session-modal-history');
        if (!section || section.dataset.requestId !== String(requestId)) return;
        const timeline = section.querySelector('.history-timeline');
        if (!timeline) return;

        if (result.error && result.turns.length === 0) {
            timeline.innerHTML = `<div class="history-empty">${escapeHTML(result.error)}</div>`;
            return;
        }

        timeline.classList.add('is-swapping');
        setTimeout(() => {
            if (requestId !== _sessionDetailRequestId) return;
            timeline.innerHTML = renderHistoryItems(result.turns, result.truncated, assistantLabel);
            timeline.classList.remove('is-swapping');
        }, 140);
    });
}

export function closeSessionDetail() {
    const overlay = document.getElementById('session-modal-overlay');
    const modal = document.getElementById('session-modal');
    if (overlay) overlay.classList.remove('visible');
    if (modal) modal.classList.remove('visible');
}

export function copySessionCmd(cmd, btn) {
    navigator.clipboard.writeText(cmd).then(() => {
        const orig = btn.textContent;
        btn.textContent = 'Copied!';
        btn.classList.add('copied');
        setTimeout(() => { btn.textContent = orig; btn.classList.remove('copied'); }, 1500);
    });
}

export function initKeyboardShortcuts(toggleAllFn) {
    const toggleAll = toggleAllFn || toggleAllDays;
    document.addEventListener('keydown', function(e) {
        if (e.key === 'Escape') {
            closeSessionDetail();
            return;
        }
        if (e.shiftKey && e.key === 'E') {
            const tag = document.activeElement.tagName.toLowerCase();
            if (tag === 'input' || tag === 'textarea' || tag === 'select') return;
            e.preventDefault();
            toggleAll();
        }
    });
}
