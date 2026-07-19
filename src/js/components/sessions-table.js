import { formatNumber } from '../utils/formatters.js';
import { getWeekStart, getWeekEnd, formatWeekLabel } from '../utils/date-utils.js';
import { getModelInfo } from '../utils/model-utils.js';
import { costClass, sourceClass } from '../utils/class-utils.js';
import { loadSessionConversation } from '../utils/session-detail-loader.js';

let mostExpensiveFile = null;
let mostExpensiveDate = null;

let _sessionDetailStore = [];
const _builtDays = new Set();
let _daySessionsMap = {};

export function resetSessionStore() {
    _sessionDetailStore = [];
}

export function pushToSessionStore(session) {
    _sessionDetailStore.push(session);
    return _sessionDetailStore.length - 1;
}

export function setMostExpensive(file, date) {
    mostExpensiveFile = file;
    mostExpensiveDate = date;
}

export function toggleDay(date) {
    const row = document.getElementById('day-' + date);
    const detailWrapper = document.getElementById('detail-wrapper-' + date);

    if (row.classList.contains('expanded')) {
        row.classList.remove('expanded');
        detailWrapper.classList.remove('open');
    } else {
        if (!_builtDays.has(date) && _daySessionsMap[date]) {
            detailWrapper.innerHTML = buildDayDetail(date, _daySessionsMap[date]);
            _builtDays.add(date);
        }

        row.classList.add('expanded');
        detailWrapper.classList.add('open');

        setTimeout(() => {
            detailWrapper.querySelectorAll('.cost-bar-fill').forEach(bar => {
                bar.style.transform = `scaleX(${parseFloat(bar.dataset.width) / 100})`;
            });
        }, 50);
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
                if (!_builtDays.has(date) && _daySessionsMap[date]) {
                    detailWrapper.innerHTML = buildDayDetail(date, _daySessionsMap[date]);
                    _builtDays.add(date);
                }

                row.classList.add('expanded');
                detailWrapper.classList.add('open');

                setTimeout(() => {
                    detailWrapper.querySelectorAll('.cost-bar-fill').forEach(bar => {
                        bar.style.transform = `scaleX(${parseFloat(bar.dataset.width) / 100})`;
                    });
                }, 50);
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
        const sessionIdx = pushToSessionStore(s);
        subTableHTML += `<tr class="session-clickable${isExpensive ? ' expensive-session-row' : ''}" onclick="showSessionDetail(${sessionIdx})">
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

export function renderSessionTable(sessions) {
    resetSessionStore();
    _builtDays.clear();

    const byDate = {};
    sessions.forEach(s => {
        if (!byDate[s.date]) byDate[s.date] = [];
        byDate[s.date].push(s);
    });
    _daySessionsMap = byDate;

    const sortedDates = Object.keys(byDate).sort().reverse();

    const tbody = document.getElementById('sessions-body');
    if (sortedDates.length === 0) {
        tbody.innerHTML = '<tr><td colspan="8" class="no-data">No sessions match the current filters.</td></tr>';
        updateTotalsRow([]);
        return;
    }

    const weekGroups = {};
    for (const date of sortedDates) {
        const ws = getWeekStart(date);
        if (!weekGroups[ws]) weekGroups[ws] = [];
        weekGroups[ws].push(date);
    }

    const sortedWeeks = Object.keys(weekGroups).sort().reverse();

    let html = '';
    for (const weekStart of sortedWeeks) {
        const weekDates = weekGroups[weekStart];

        let weekTotalCost = 0;
        let weekTotalInput = 0;
        let weekTotalOutput = 0;
        let weekTotalCacheRead = 0;
        let weekTotalCacheWrite = 0;
        let weekTotalSessions = 0;
        const weekModels = new Set();

        for (const date of weekDates) {
            const daySessions = byDate[date];
            let dayCost = 0, dayInput = 0, dayOutput = 0, dayCacheRead = 0, dayCacheWrite = 0;
            const modelSet = new Set();
            for (const x of daySessions) {
                dayCost += x.cost;
                dayInput += x.input_tokens || 0;
                dayOutput += x.output_tokens || 0;
                dayCacheRead += x.cache_read || 0;
                dayCacheWrite += x.cache_write || 0;
                if (x.model) modelSet.add(x.model);
            }
            const models = [...modelSet];
            const modelBadges = models.map(m => {
                const mi = getModelInfo(m);
                return `<span class="model-badge ${mi.cls}">${mi.name}</span>`;
            }).join(' ');

            weekTotalCost += dayCost;
            weekTotalInput += dayInput;
            weekTotalOutput += dayOutput;
            weekTotalCacheRead += dayCacheRead;
            weekTotalCacheWrite += dayCacheWrite;
            weekTotalSessions += daySessions.length;
            models.forEach(m => weekModels.add(m));

            const dateLabel = new Date(date + 'T00:00:00').toLocaleDateString('en-US', { weekday: 'short', month: 'short', day: 'numeric' });

            html += `<tr class="day-row" id="day-${date}" onclick="toggleDay('${date}')">
                <td><span class="chevron">\u25B6</span>${dateLabel}</td>
                <td>${daySessions.length}</td>
                <td>${modelBadges}</td>
                <td class="token-cell">${formatNumber(dayInput)}</td>
                <td class="token-cell">${formatNumber(dayOutput)}</td>
                <td class="token-cell">${formatNumber(dayCacheRead)}</td>
                <td class="token-cell">${formatNumber(dayCacheWrite)}</td>
                <td style="text-align:right"><span class="cost-badge ${costClass(dayCost)}">$${dayCost.toFixed(2)}</span></td>
            </tr>`;

            html += `<tr class="day-detail-row"><td colspan="8">
                <div class="day-detail-wrapper" id="detail-wrapper-${date}"></div>
            </td></tr>`;
        }

        const weekLabel = formatWeekLabel(weekStart);
        html += `<tr class="week-row">
            <td colspan="8">
                <div class="week-strip">
                    <div class="week-strip-left">
                        <span class="week-strip-icon">\u03A3</span>
                        <span class="week-strip-label">${weekLabel}</span>
                    </div>
                    <div class="week-strip-stats">
                        <span class="week-stat"><span class="week-stat-label">Sessions</span><span class="week-stat-value">${weekTotalSessions}</span></span>
                        <span class="week-stat-divider"></span>
                        <span class="week-stat"><span class="week-stat-label">In</span><span class="week-stat-value">${formatNumber(weekTotalInput)}</span></span>
                        <span class="week-stat"><span class="week-stat-label">Out</span><span class="week-stat-value">${formatNumber(weekTotalOutput)}</span></span>
                        <span class="week-stat-divider"></span>
                        <span class="week-strip-cost">$${weekTotalCost.toFixed(2)}</span>
                    </div>
                </div>
            </td>
        </tr>`;
    }
    tbody.innerHTML = html;
    updateTotalsRow(sessions);
    updateToggleAllButton(false);
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

export function showSessionDetail(idx) {
    const s = _sessionDetailStore[idx];
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
            <button class="session-modal-close" onclick="closeSessionDetail()">&times;</button>
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
                    <button class="resume-copy-btn" onclick="copySessionCmd('${resumeCmd}', this)">Copy</button>
                </div>
                ${s.cwd ? `<div class="resume-cmd-row" style="margin-top:6px;">
                    <code class="resume-cmd">cd ${escapeHTML(s.cwd)} && ${escapeHTML(resumeCmd)}</code>
                    <button class="resume-copy-btn" onclick="copySessionCmd('cd ${s.cwd.replace(/'/g, "\\\\'")} && ${resumeCmd}', this)">Copy</button>
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

window.toggleDay = toggleDay;
window.showSessionDetail = showSessionDetail;
window.closeSessionDetail = closeSessionDetail;
window.copySessionCmd = copySessionCmd;
