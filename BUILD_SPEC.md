# munim — Build Spec

> **Purpose of this document:** a complete, self-contained specification for an AI agent to build **munim**, a **cross-platform (macOS + Linux)** AI-coding usage/cost tracker modeled on [`658jjh/claude-usage-tracker`](https://github.com/658jjh/claude-usage-tracker) (v3.0.0, MIT, "AI Usage Tracker"). It captures everything the original does — data collection, cost math, and UI/UX — plus munim's added features and every settled build decision (see **§0.5 Locked decisions**). The original was reverse-engineered from source; file/line refs point at the original repo. Where §0.5 and prose ever seem to differ, **§0.5 wins.**

---

## 0. Important corrections to common assumptions

- **This is NOT a menu bar / system-tray app.** The original is a single **windowed** app: a native macOS Swift shell hosting a `WKWebView` that renders a web dashboard. There is **no `NSStatusItem`**, no tray icon, no "live number in the menu bar." (The `// MARK: - Menu Bar` in the source is the standard top-of-screen *application menu*, not a status item.)
- **There is no autostart / launch-at-login** anywhere in the original.
- **There are no usage limits, quotas, plan tiers, or 5-hour reset windows.** It tracks **cost only**, not consumption against a cap. (This differs from tools like `ccusage`.) The only forward-looking logic is a monthly-cost *projection*.
- **No network/API calls in the backend.** All data is read from local files. The only runtime network deps are Chart.js + Google Fonts loaded from CDNs (should be vendored locally for offline/Linux).
- If you *want* a tray with live text and/or launch-at-login, treat them as **net-new features**, not a port. (See §7 for the cross-platform reality — inline tray text only works well on macOS.)

---

## 0.5 Locked decisions (authoritative)

Every decision below is settled — build to these; don't re-open them. The rest of the document elaborates.

| # | Decision | Choice |
|---|---|---|
| 1 | **Product name** | **munim** (bookkeeper). Bundle id `com.munim.app`; app-data dir `munim`. |
| 2 | **Framework** | **Tauri v2** (Rust core + system WebView). |
| 3 | **Collector** | **Rust rewrite** of `collect-usage.js` (`serde_json` + `walkdir`). No Node runtime shipped. |
| 4 | **Frontend** | **Keep vanilla JS + ES modules + Chart.js** — reuse the original dashboard almost as-is; build new settings/budget UI in the same style. |
| 5 | **Platforms (v1)** | **macOS 12+ and Linux.** Windows deferred (write the path resolver so Windows can slot in later, but don't build/test it now). |
| 6 | **System tray** | **Icon + menu-on-click** (details dropdown, §6.1). No inline live-text (not portable). |
| 7 | **Close button** | **Hide to tray**; app keeps running + auto-refreshing; **Quit** menu item exits. |
| 8 | **Auto-refresh** | **Required** — `notify` file-watch + 60 s interval fallback, debounced, silent (§4.8). |
| 9 | **Launch at login** | **Off by default**; toggleable from tray **and** settings panel (`tauri-plugin-autostart`). |
| 10 | **Budget / alerts** | **Required feature.** Single **monthly $ budget**; native notification + tray highlight at **80% and 100%** (fire once each per calendar month). Uses `tauri-plugin-notification`. |
| 11 | **Settings UI** | **In-app settings panel** (view/modal in the dashboard window), styled like the rest of the UI. Holds budget, launch-at-login, etc. |
| 12 | **Pricing tables** | **Editable bundled config file** (TOML/JSON) read at startup — single source of truth for the Rust cost math. |
| 13 | **Data migration** | **Manual JSON import only** (reuse the parity Export/Import). No auto-detection of an existing install. munim rebuilds history from source files on first scan anyway. |
| 14 | **License / repo** | **MIT, public.** |
| 15 | **Icon / brand** | **Design a fresh munim logo** (keep the original's dark visual language + palette). |
| 16 | **macOS signing** | **Developer ID sign + notarize** (needs a paid Apple Developer account; creds as CI secrets). |
| 17 | **macOS updates** | **`tauri-plugin-updater`**, manifest (`latest.json`) + `.dmg`/bundles hosted as **GitHub Release assets**. Needs a **minisign updater keypair** (public key in config, private key + pw in CI). No website. |
| 18 | **Linux packaging** | **Flatpak**, sandbox granted **`--filesystem=home:ro`** (+ writable app-data dir). |
| 19 | **Linux updates** | **Self-hosted flatpak repo on GitHub Pages** — users add the remote once, Flatpak auto-updates from it. **`tauri-plugin-updater` is disabled on Linux** (Flatpak owns updates there). |
| 20 | **Distribution/CI** | GitHub Actions builds mac (`.dmg`, signed+notarized) and Linux (Flatpak → published to the GitHub Pages repo), generates/signs the updater manifest, attaches assets to the release. |

> **New surfaces this adds beyond the original** (none exist in `claude-usage-tracker`): system tray, close-to-tray + Quit, auto-refresh, launch-at-login, budget + notifications, an in-app settings panel, an editable pricing config, and the two update channels.

---

## 1. What it does (one paragraph)

A **local-first desktop app** that auto-discovers and aggregates your AI-coding **token usage & dollar cost** across 10+ dev tools (Anthropic Claude tools + OpenAI Codex), by scanning each tool's on-disk session logs (JSONL/JSON). It attributes tokens to calendar dates from message timestamps, computes per-model cost from hardcoded pricing tables, and renders a dark-themed interactive dashboard (Chart.js): stat cards, a daily stacked bar chart, source/model donuts, an activity heatmap, and an expandable, filterable session log with a per-session detail modal. No cloud, no accounts, no telemetry — all data stays on the machine.

---

## 2. Chosen tech stack: Tauri (v2)

**Decision (locked): Tauri v2** (Rust core + system WebView). Rationale: the entire UI is already a plain web app that drops straight into a Tauri window, it produces a small single-binary app with low RAM, it has built-in cross-platform system-tray + dialog + autostart plugins, and the human building this is already fluent in Tauri across other apps.

**Tray model (locked): icon + menu-on-click** (no inline live-text). This is the tray behavior that works consistently on **both macOS and Linux** — Tauri's `TrayIconBuilder` renders a status icon and shows a native menu on click. (Inline text next to the icon is *not* portable and is intentionally **not** used; see §7.)

**The collector (locked): rewrite in Rust.** The original is dependency-free Node (`collect-usage.js`); port it to a Rust module so distribution is a single binary with **no Node runtime dependency**, using `serde_json` + `walkdir`. (§4 is written stack-agnostically — treat it as the port's contract.) No Node sidecar.

**Plugins to use:** `tauri-plugin-dialog` (export/import file pickers), `tauri-plugin-fs` (scoped file reads), `tauri-plugin-autostart` (launch-at-login), `tauri-plugin-notification` (budget alerts), `tauri-plugin-updater` (**macOS only** — disabled on Linux, where Flatpak owns updates), built-in `tray-icon` + `menu`, and `tauri-plugin-clipboard-manager` (resume-command copy, or do it in the webview). The `notify` crate (not a Tauri plugin) drives auto-refresh file-watching (§4.8).

---

## 3. Architecture

```
┌─────────────────────────────────────────────┐
│  Tauri core (Rust — src-tauri/)              │
│   • WebviewWindow (frameless, dark)          │
│   • system tray: icon + menu-on-click         │
│   • native menu + dialog plugin (export/import)│
│   • #[tauri::command] handlers (invoke bridge) │
│   • scoped file-read for session detail        │
│   • app-data dir per-OS (XDG / App Support)    │
│   • runs the collector on launch / refresh     │
└───────────────┬─────────────────────────────┘
                │ emits summary + session data
                ▼
┌─────────────────────────────────────────────┐
│  Collector (Rust module — port of            │
│  collect-usage.js; sidecar Node is fallback) │
│   • scans tool dirs → parses JSONL/JSON        │
│   • aggregates per (source,file,date)          │
│   • computes cost from pricing tables          │
│   • writes summary + sessions-cache + scan-index│
└───────────────┬─────────────────────────────┘
                │ via invoke() / window globals
                ▼
┌─────────────────────────────────────────────┐
│  Dashboard (webview — HTML/CSS/Chart.js)      │
│   • stat cards, charts, heatmap, session log   │
│   • filters, session-detail modal, export/import│
└─────────────────────────────────────────────┘
```

**How the dashboard gets its data:** the original injected a generated `data.js` that set `window.__SUMMARY__`, `window.__CLAUDE_SESSIONS__`, `window.__CODEX_SESSIONS__`, `window.__OPENCLAW_SESSIONS__`. In Tauri, prefer having the frontend call a `get_usage_data` command via `invoke()` on load (returns the same four payloads as JSON) — cleaner than writing a `data.js`. If you want the smallest diff from the original UI JS, you can instead still write `data.js` into app-data and load it with a `<script>`; the dashboard JS reads those globals either way, so shim them from the `invoke()` result.

Original files (for reference): `src/collect-usage.js` (940-line collector, the whole backend — **port to Rust**), `src/App.swift` (native shell — **replace with Tauri core**), `src/dashboard.html` + `src/css/**` + `src/js/**` (the web UI — **reuse as the Tauri frontend**), `build-app.sh` (macOS packaging — **replace with Tauri bundler**).

---

## 4. Data layer (the backend) — full spec

The collector is **pure Node** (`fs`, `path`, `os`; no npm deps). Port it nearly verbatim; the only real change is **per-OS source paths** and the **output dir**.

### 4.1 Data sources — paths to scan

`HOME = os.homedir()`. `findJsonl(dir, maxDepth=10)` recurses collecting `*.jsonl`, skipping `.git*` dirs and any filename containing `audit`.

| Source | Path(s) — mac | Linux equivalent | Parser |
|---|---|---|---|
| Claude Code CLI | `~/.claude/projects/` (recursive) | same (cross-platform) | Claude format |
| Claude Desktop (Agent) | `~/Library/Application Support/Claude/local-agent-mode-sessions/` | `~/.config/Claude/local-agent-mode-sessions/` | Claude format |
| OpenClaw / Clawdbot | `~/.openclaw/agents/main/sessions/`, `~/.clawdbot/agents/main/sessions/` | same | OpenClaw format |
| Cursor | `~/.cursor/projects`, `~/Library/Application Support/Cursor/User/workspaceStorage` | `~/.cursor/...`, `~/.config/Cursor/User/workspaceStorage` | Claude format |
| Windsurf | `~/.windsurf/...`, `~/Library/Application Support/Windsurf/User/workspaceStorage` | `~/.windsurf/...`, `~/.config/Windsurf/User/workspaceStorage` | Claude format |
| Cline (VS Code ext) | `~/.cline`, `~/Library/Application Support/Code/User/globalStorage/saoudrizwan.claude-dev`, `.../cline.cline` | `~/.cline`, `~/.config/Code/User/globalStorage/saoudrizwan.claude-dev`, `.../cline.cline` | Claude format |
| Roo Code (VS Code ext) | `~/.roo-code`, `~/Library/Application Support/Code/User/globalStorage/rooveterinaryinc.roo-cline` | `~/.roo-code`, `~/.config/Code/User/globalStorage/rooveterinaryinc.roo-cline` | Claude format |
| Aider | `~/.aider`, `~/.aider/logs` (`.jsonl`/`.json`) | same | Aider format |
| Continue.dev | `~/.continue/sessions` (`.json`) | same | Continue format |
| Codex CLI (OpenAI) | `~/.codex/sessions/**/rollout-*.jsonl` | same | Codex format |

**Action item:** replace the hardcoded `~/Library/Application Support/...` roots with a per-OS resolver (`process.platform === 'darwin'` → `~/Library/Application Support`; else XDG `~/.config`). Dotfile roots are already cross-platform.

### 4.2 Aggregation model

- **Unit = one record per `(source, file, date)`** — a "session-day". Each parser returns `dayData` keyed by **local** date `YYYY-MM-DD`.
- Per-day accumulator: `{ cost, input_tokens, output_tokens, cache_read, cache_write, reasoning_tokens, models:Set, times:[] }`.
- Emit one session record per date; **skip days where `cost < 0.0001`**.
- Record fields: `date, time` (earliest seen), `provider` (`claude`|`codex`), `source`, `file`, `cost` (4-dp), `input_tokens, output_tokens, cache_read, cache_write, model` (last in set), optional `reasoning_tokens, filePath, title, sessionId, cwd`.
- **Dedup / merge key: `provider|source|file|date`**.

### 4.3 Timestamps → local date

- Accept epoch-ms number **or** ISO string. Aider uses **Unix epoch seconds** (multiply by 1000 when value `< 2e9`).
- Fall back to file `mtimeMs` when no timestamp.
- Local-date conversion is done manually (no OS calls, cross-platform):
  `TZ_OFFSET = -new Date().getTimezoneOffset()/60`; `date = new Date(ts + TZ_OFFSET*3600000).toISOString().split('T')[0]`.

### 4.4 Per-format field extraction

- **Claude family** (Claude Code/Desktop/Cursor/Windsurf/Cline/Roo): read `entry.message.usage` (or `entry.usage`) → `input_tokens`, `output_tokens`, `cache_creation_input_tokens`→cache_write, `cache_read_input_tokens`→cache_read. Model from `message.model`; only added to the model set if it `startsWith('claude')`.
- **OpenClaw/Clawdbot**: `usage.input/output/cacheRead/cacheWrite`; **skip non-`claude`-prefixed models**; if `usage.cost.total` present, use it directly, else compute.
- **Aider**: `prompt_tokens`/`completion_tokens` (OpenAI-style) or `input_tokens`/`output_tokens`.
- **Continue.dev**: single JSON with `steps`/`history` array; `promptTokens`/`completionTokens`.
- **Codex**: read `event_msg`/`token_count` payloads; use `info.last_token_usage`; if only cumulative `info.total_token_usage` exists, **diff against previous cumulative** (clamp ≥0 for mid-session resets). Model comes from separate `turn_context` events. Handles `cached_input_tokens`, `reasoning_output_tokens`. Sub-classify source via `session_meta` into Codex CLI / Codex Exec / Codex Review.

### 4.5 Pricing tables (USD per **1,000,000 tokens**) — hardcoded

**In munim, pricing lives in one editable bundled config file** (TOML or JSON) read at startup — the **single source of truth** for the Rust cost math (the original's duplicate JS table in `model-utils.js` is not reproduced; the frontend only displays precomputed costs, or reads rates from a command if it ever needs them). Ship the tables below as that config's defaults; correcting a rate must not require a recompile. Matching stays **lowercased-substring, order-sensitive** — preserve row order in the config since the first match wins.

**Claude** (`getPricing`):
| match | input | output | cacheWrite | cacheRead |
|---|---|---|---|---|
| `opus-5` | 20 | 100 | 25 | 2.0 |
| `opus-4.5/.6/.7/.8/.9` | 5 | 25 | 6.25 | 0.50 |
| `opus-4.1` / `opus` | 15 | 75 | 18.75 | 1.50 |
| `sonnet` | 3 | 15 | 3.75 | 0.30 |
| `haiku-4.5` | 1 | 5 | 1.25 | 0.10 |
| `haiku` | 0.25 | 1.25 | 0.30 | 0.03 |
| default/unknown | 3 | 15 | 3.75 | 0.30 (Sonnet) |

**Codex/OpenAI** (`getCodexPricing`, cacheWrite always 0):
| match | input | output | cacheRead |
|---|---|---|---|
| `gpt-5.5` | 5.00 | 30.00 | 0.50 |
| `gpt-5.4-mini` | 0.75 | 4.50 | 0.075 |
| `gpt-5.4` | 2.50 | 15.00 | 0.25 |
| `gpt-5.3-codex` | 1.75 | 14.00 | 0.175 |
| `gpt-5.2` | 2.00 | 10.00 | 0.20 |
| default | 2.50 | 15.00 | 0.25 (gpt-5.4) |

**Cost formulas:**
- Claude: `(input*in + output*out + cache_write*cacheWrite + cache_read*cacheRead) / 1e6`.
- Codex: OpenAI `input_tokens` already includes cached, so `nonCached = max(0, input - cached)`; cost = `(nonCached*in + cached*cacheRead + output*out) / 1e6`. **Reasoning tokens are tracked but not billed separately** (already inside output).

### 4.6 Summary object (emitted for the dashboard)

`{ generated_at, today, current_month, totals{<source>…, grand_total}, provider_totals{claude,codex}, today_cost, month_cost, session_counts }` where `today = localDate(now)`, `current_month = today.slice(0,7)`, month cost = sum of sessions whose `date` starts with the current month.

### 4.7 Output & caching (in the app-data dir)

Output dir from env `CLAUDE_USAGE_DATA_DIR` (default `<script_dir>/data/` for standalone). The shell sets it to the per-OS app-data dir. **Remap** `~/Library/Application Support/AIUsageTracker` → Linux `~/.local/share/AIUsageTracker` (or `$XDG_DATA_HOME`). Files written:

1. **`data.js`** — the dashboard artifact; assigns `window.__SUMMARY__`, `window.__OPENCLAW_SESSIONS__`, `window.__CLAUDE_SESSIONS__`, `window.__CODEX_SESSIONS__`.
2. **`sessions-cache.json`** — full merged session-day array; on load, validate schema, strip legacy `history` fields, backfill `provider`; **preserves historical/imported entries whose source files no longer exist locally**.
3. **`scan-index.json`** — `{filePath: {mtime, size}}` fingerprint map; unchanged files (same mtime+size) are **skipped** and their cached records reused (steady-state scan ~20s → <1s).

Both caches use **atomic writes** (`.tmp` + `rename`). Also keep a `launcher.log`.

### 4.8 Refresh model — **auto-refresh is required**

The original has no polling — it refreshes only manually. **This rebuild must auto-refresh** in addition to keeping the manual paths (reload FAB / Cmd+Ctrl+R / menu / tray "Refresh Now").

Design:
- **File-watch (primary):** watch the resolved source dirs (§4.1) with the Rust **`notify`** crate (recursive). On relevant change (`*.jsonl`/`*.json` create/modify), trigger an **incremental** collect (the `scan-index.json` fingerprint means only changed files are re-parsed, so this is cheap).
- **Debounce:** active tools append to JSONL continuously, so coalesce bursts — wait for **~2–3 s of quiet** (or a max cadence of one collect per few seconds) before running. Never run overlapping collects; if a change arrives mid-collect, queue a single follow-up.
- **Interval fallback:** also run a periodic collect on a timer (default **60 s**) to cover platforms/paths where `notify` misses events (some networked/rare filesystems). The watcher and timer share the same debounced, non-overlapping collect entrypoint.
- **Propagation:** after each auto-collect, push refreshed data to the webview (emit a Tauri event the frontend listens for → re-render) **and** update the tray quick-stat labels (§6.1) and the header "Last sync" time.
- **UX:** auto-refresh is silent — no loading screen, no toast (that full-screen loader is only for launch and explicit manual refresh). Just update the numbers in place.
- **Config (minimal):** the 60 s interval and 2–3 s debounce are constants; no user-facing setting unless later requested. Auto-refresh is always on.

---

## 5. UI/UX spec (the dashboard) — reuse the web layer

Almost the entire UI is portable web (HTML/CSS/Chart.js) and already runs in "browser mode." Rebuild the native shell only. **Vendor Chart.js and the Google Fonts locally** (drop the CDN `<script>`/`<link>` — the only network deps).

### 5.1 Design tokens (`base.css :root`)

```
--bg-primary   #0a0e17    --bg-secondary #111827   --bg-card #151d2e
--bg-card-hover #1a2540   --bg-elevated  #1e293b
--border #1e293b          --border-light #253147
--text-primary #e2e8f0    --text-secondary #94a3b8  --text-muted #64748b
--accent-cyan #22d3ee  --accent-amber #fbbf24  --accent-emerald #34d399
--accent-rose #fb7185  --accent-violet #a78bfa  --accent-blue #60a5fa
--radius 12px  --radius-sm 8px  --radius-pill 20px
--glow-cyan 0 0 20px rgba(34,211,238,.15)
```
- **Fonts:** Outfit (300–900) for headings/values/UI; JetBrains Mono (300–700) for labels/numbers/badges/code. Fallback `-apple-system`/`SF Pro`.
- **Dark only.** No light theme; native window pinned dark (bg `#0A0E17`). Honor `prefers-reduced-motion` (disables history/skeleton animations). Ambient fixed radial blobs: cyan top-right `rgba(34,211,238,.06)`, amber bottom-left `rgba(251,191,36,.04)`.
- **Chart.js defaults:** text `#94A3B8`, gridlines `rgba(30,41,59,.4)`, JetBrains Mono 11px; frosted tooltips `rgba(15,23,42,.92)` with a cyan hairline border, 10px radius.

**Source colors:** OpenClaw/Clawdbot `#FBBF24`, Claude Code `#60A5FA`, Claude Desktop `#A78BFA`, Cursor `#22D3EE`, Windsurf `#34D399`, Cline `#FB7185`, Roo Code `#F472B6`, Aider `#2DD4BF`, Continue `#F59E0B`, Codex/Codex CLI `#10A37F`, Codex Exec `#22C55E`, Codex Review `#F59E0B`.
**Model colors:** Opus `#FB7185`, Sonnet `#60A5FA`, Haiku `#34D399`, Unknown `#A78BFA`, GPT/Codex greens `#10A37F`/`#22C55E`/`#84CC16`/`#0EA5E9`/`#14B8A6`.

### 5.2 Layout

Single scrolling page, max content width **1440px**, padding `32px 40px`, 16px grid gaps, 32px section margins. Responsive breakpoints: **1280** (stats→3 col), **1024** (stats→2 col, charts→1 col), **640** (single col, header stacks). Vertical order:

1. **Header** — 44×44 rounded logo (new **munim** mark, cyan→violet glow), wordmark "**munim**" (Outfit 800, gradient-clip treatment); **provider pills** `All / Claude / Codex` (active pill = cyan→violet gradient, dark text); right side: "**● Last sync: —**" (emerald `#34D399` pulsing dot, mono) + **Export** / **Import** + **⚙ Settings** buttons (Export hovers emerald, Import hovers violet, Settings opens the §5.2b panel). Bottom border `#1E293B`.

2. **Stats grid** — 5 cards `repeat(5,1fr)`, gradient bg `linear-gradient(135deg, rgba(21,29,46,.95), rgba(17,24,39,.95))`, 1px border, 12px radius, 24px pad, 1px top accent line per card, staggered fade-slide-up, lift + cyan border on hover:

   | Card | Value color | Sub-content |
   |---|---|---|
   | TODAY | cyan `#22D3EE` | yesterday delta + date (`2026-05-11`) |
   | THIS WEEK | blue `#60A5FA` | range `May 11 – May 17` |
   | THIS MONTH | amber `#FBBF24` | `May 2026` + projection line |
   | ALL TIME | emerald `#34D399` | `Since Mar 10, 2026` |
   | SESSIONS | violet `#A78BFA` | `Across all sources` |

   - **Yesterday delta**: `↓ $336.45 (78%) vs yesterday` — **↓ emerald** when spending less, **↑ rose** when more; neutral muted for same/no-data; % shown only if ≥1%.
   - **Monthly projection**: `→ ~$12663/mo | $408.48/day`; `dailyAvg = month_cost/dayOfMonth`, `projection = dailyAvg*daysInMonth`; color: `<$50` emerald, `$50–200` amber, `>$200` rose; `~$` prefix, whole dollars if ≥$10; first 3 days of month append "based on N days of M".

3. **Daily Spend by Source** — full-width **stacked bar** (Chart.js), last **15 days**, gradient bars (top `color+E6`→bottom `color+66`), borderRadius 3, barPercentage 0.7, legend top-right. Title has a 3px cyan→blue accent bar + right hint "click bar to filter". **Click a bar → filters the two donuts + shows a day-filter pill** ("Filtered: May 5" + "✕ clear"); selected bar stays `color+CC`, others dim to `color+22`; tooltip lists per-source `$x.xx` + `Total`. Canvas 260px.

4. **Two donuts** side by side (`1fr 1fr`, canvas 220px, `cutout 68%`, 2px dark gaps, rotate+scale 500ms):
   - **By Source** — cost share per tool; center total `$13670.75` + "TOTAL"; legend `● Claude Code 95%` (integer %).
   - **Spend by Model** — cost per model family; center total + "BY MODEL"; legend `● Opus $11315.52 (82.8%)` (label + $ + 1-dec %), families ordered by cost desc.

5. **Peak Hours heatmap** (28px pad, rose→amber hover accent) — title + segmented **Hours | Days** toggle (animated sliding pill):
   - **Hours view** (default): grid `90px + repeat(24,1fr)`, rows = dates (newest top), cols = hours 0–23 (labeled 0,3,6…21), cells 14px min, 4px radius, hover scale 1.3.
   - **Days view**: GitHub-contribution calendar — 7 day rows (Mon–Sun) × week cols, month labels on top, "today" cell cyan ring + click-pulse.
   - **5 intensity levels**: 0 `rgba(30,41,59,.25)`; 1 cyan `rgba(34,211,238,.15→.25)`; 2 cyan→blue `.4→.55`; 3 amber `.55→.7`; 4 rose `.7→.85` + glow. Legend "Less ▢▢▢▢▢ More".
   - **Tooltip** (frosted `rgba(15,23,42,.97)`): day title, hour subtext, "Sessions / Cost" rows. **Click a cell → scroll to + expand the matching session-day.**

6. **Most Expensive Session Today** callout (hidden until data) — rose/amber gradient bg, 2px rose left border, circle "!" icon, title (rose), chips (source, model, mono time, rose cost pill), token line.

7. **Session Log** — header: title + center **Timeline | Projects** toggle (sliding pill) + right: hint "click a row to expand" + **Expand All ▼ [Shift+E]** button + session count "4,319 sessions".
   - **Filter bar**: Source multi-select (checkbox dropdown, cyan check), Model multi-select, **From/To** date inputs (`color-scheme:dark`), **$ Min** number (debounced), filtered/total count, "Clear All" (hover rose). Active filters → removable **chips**, color-coded: source=blue, model=violet, date=emerald, cost=amber.
   - **Table** (scroll wrap max-height 70vh, sticky header + footer):
     - *Timeline* columns: **Date · Sessions · Models · Input · Output · Cache Read · Cache Write · Cost**. Per-day rows, expandable (chevron rotates 90° cyan), model chips inline, numbers K/M formatted (`≥1M→x.xM`, `≥1K→x.xK`). Between ISO weeks: a **week summary strip** (blue "Σ", `May 11 – May 17`, `SESSIONS 29 · IN 1.0K · OUT 818.8K`, blue cost pill).
     - *Projects* columns: **Project · Sources · Models · Input · Output · Cache Read · Cache Write · Cost**. Rows grouped by working dir (`cwd`): 📁 + bold name + muted "318 sessions", sorted by cost desc, expandable.
     - **Totals footer** (sticky): "TOTAL" + summed tokens + total cost badge; bg `#1A2540`, 2px cyan top border.
   - **Cost badges** (rounded-pill mono): `<$1` emerald (cost-low), `$1–20` amber (cost-medium), `≥$20` rose (cost-high).
   - Expanded rows lazily build per-source detail cards with animated cost-bar mini-fills.

8. **Session Detail modal** (520px, max 92vw, scale-in 0.95→1, overlay `rgba(0,0,0,.7)`, gradient bg + cyan hairline glow, **Esc** closes):
   - **Meta rows** (label col 80px, uppercase mono muted): DATE, SOURCE (badge), MODEL (badge), PROJECT (path), SESSION ID (mono).
   - **Token strip** (5-col grid): INPUT / OUTPUT / CACHE READ / CACHE WRITE / COST (Codex also shows reasoning tokens).
   - **CONVERSATION HISTORY**: chat timeline (user = indigo right-inset, AI = cyan left-inset), **lazy-loaded** with shimmer skeleton + staggered fade-in; reads raw JSONL through the secure file-read IPC bridge (with a `fetch('file://…')` fallback for browser mode).
   - **RESUME THIS SESSION**: cyan box with mono command rows + **Copy** buttons (flash emerald "copied"): `claude --resume <id>` (Claude) or `codex resume <id>` (Codex), plus a `cd <cwd> && claude --resume …` variant.

9. **Footer** — centered mono muted "ai usage dashboard · ☕ buy me a coffee" (coffee link hovers `#FFDD00`).

10. **Floating Reload FAB** — fixed bottom-right (28px), 50px circle, `#151D2E` bg, cyan→violet refresh icon, cyan inset ring; hover scale 1.1 + glow + tooltip "Refresh Data ⌘R"; spins + pulsing glow while reloading.

**Overlays:** heatmap tooltip, export/import **toast** (bottom-right, emerald/rose), import "merged" banner.

**Loading screen** (shown before dashboard; port from the Swift inline HTML): bg `#0A0E17`, animated aurora radial blobs (cyan/amber/violet), 5 floating particles, animated SVG logo (new munim mark) with 3 orbital rings, gradient wordmark **"munim"**, indeterminate progress bar (`linear-gradient(90deg,#22d3ee,#a78bfa,#fb7185)`), status text "Collecting usage data…", 7 equalizer bars, 4 shimmer skeleton cards. Window fades 0→1 over 0.35s when ready.

### 5.2b New surfaces (munim additions — build in the same visual style)

**Header changes:** the wordmark reads **"munim"** (keep the gradient-clip treatment on part of the word). Add a **settings gear button** in the header-meta row (next to Export/Import) that opens the settings panel.

**Budget bar (on the THIS MONTH stat card):** when a monthly budget is set, render a thin progress bar under the projection line — spend / budget, filled with the cost-tier gradient, plus a small `"$4,493 / $6,000 (75%)"` label. Bar turns amber at ≥80% and rose at ≥100% (reuse the cost-tier colors). Hidden if no budget is set.

**Settings panel** (modal/overlay in the dashboard window, same styling as the Session Detail modal — dark gradient bg, cyan hairline, scale-in, **Esc** to close). Sections:
- **Budget** — a single "Monthly budget ($)" number input (empty = off). Helper text: "Alerts at 80% and 100% of this month's spend." Save persists immediately.
- **Startup** — "Launch munim at login" toggle (checkable; mirrors the tray item; wired to `tauri-plugin-autostart`, **off by default**).
- **Behavior** — "Auto-refresh" is always on (show as read-only info, not a toggle) with the current cadence noted; "Close button hides to tray" shown as read-only info.
- **Data** — Export / Import buttons (same actions as the header) + the app-data dir path (read-only, with a "Reveal" affordance).
- **About** — app name, version, "MIT · github.com/…/munim" link, "Check for updates" (macOS only; hidden on Linux/Flatpak).

**Persistence:** settings (budget value, autostart pref, last-selected views/filters if you choose to persist them) live in a `settings.json` in the app-config dir, separate from `sessions-cache.json`. The original persisted no settings; this is new.

**Budget alerts (native):** after each collect, compare month-to-date spend against the budget. When it first crosses **80%** and first crosses **100%** *within a calendar month*, fire a native notification (`tauri-plugin-notification`), e.g. *"munim — 80% of your $6,000 monthly budget used ($4,800)."* Track which thresholds already fired this month (in `settings.json`) so each alert fires **once per month**; reset the flags when the month rolls over. Also tint the tray icon/menu header when over 100% (§6.1).

### 5.3 Number/format conventions

- Cost: USD, `$` prefix, 2 decimals (`$93.44`, `$13,670.75`); projections whole dollars ≥$10; y-axis whole dollars.
- Tokens: K/M suffix, 1 decimal (`1.4M`, `456.9K`).
- Percentages: source donut integer, model donut 1 decimal.
- Dates: ISO in subtext; "Mon, May 11" in tables; "May 11 – May 17" ranges; "Since Mar 10, 2026".
- Session counts: comma-separated integers.

---

## 6. Native shell — what to replace (the only real porting work)

Everything below lives in the macOS `App.swift`; rebuild it as the Tauri core (`src-tauri/`).

| macOS piece | Rebuild as (Tauri v2) |
|---|---|
| `NSWindow` frameless, transparent full-size-content titlebar, dark-aqua, movable-by-background | `tauri.conf.json` window `{ decorations:false, transparent:true, theme:"Dark", titleBarStyle:"Overlay" }` (macOS keeps the overlay traffic-lights); use CSS `-webkit-app-region: drag` (already present on FAB/buttons) or `data-tauri-drag-region` for drag |
| Native app menu (App/Edit/View/Window; **⌘R Refresh**, ⌘Q, ⌘W, ⌘M) | Tauri `Menu`/`MenuItem` builder with accelerators (`CmdOrCtrl+R` → emit a `refresh` event to the frontend) |
| Export/Import via `NSSavePanel`/`NSOpenPanel` | `tauri-plugin-dialog` `save()` / `open()` with a `.json` filter |
| JS↔native bridge (`WKScriptMessageHandler` messages: `reload`, `exportData`, `importData`, `saveImportedData`, `loadSessionDetail`) | `#[tauri::command]` handlers invoked via `invoke()` for the same 5 operations: `refresh`, `export_data`, `import_data`, `save_imported_data`, `load_session_detail` |
| `loadSessionDetail` reply bridge — reads **allow-listed** JSONL (path allowlist of known tool roots, 8 MB cap, non-dir check, symlink resolution) | `load_session_detail` command that re-implements the **same allowlist + size cap + symlink-resolve** in Rust before reading (security-critical). Also lock down `tauri-plugin-fs` scope + the Tauri capabilities/CSP to those roots only |
| Node discovery (`findNode` across Homebrew/nvm/fnm/volta/asdf + login-shell fallback; PATH widening for GUI launchd) | **Drop entirely** — the Rust collector runs in-process, no Node needed. (Only relevant if you take the sidecar fallback, in which case bundle node as a Tauri sidecar binary.) |
| Data dir `~/Library/Application Support/AIUsageTracker` (+ one-time migration from legacy `ClaudeUsageTracker`) | `app.path().app_data_dir()` (per-OS: macOS App Support, Linux `~/.local/share`); keep the legacy-migration step for existing mac users |
| `data.js` injected as a `WKUserScript` at documentStart | Serve data via the `get_usage_data` command on frontend load (see §3); no `data.js` needed |
| Loading screen (inline Swift HTML) | Port to an HTML file / route shown before the dashboard mounts |
| About panel / license gate / update checker (`#if PAID_BUILD`, sources gitignored) | **Omit** — proprietary, not in the open repo. (If you later want update checks, use `tauri-plugin-updater`.) |
| *(new)* System tray | `TrayIconBuilder` — see §6.1 |

### 6.1 System tray (new feature) — spec

Cross-platform (macOS + Linux) **icon + menu on click**, built with Tauri's `TrayIconBuilder`:

- **Icon:** the new **munim** mark, rasterized to PNG at tray sizes. On macOS provide a template (monochrome) variant so it adapts to light/dark menu bars; on Linux ship a colored PNG (appindicator). When month spend is **over budget**, use a rose-tinted icon variant.
- **Left-click:** show/focus the main window (or toggle it). **Right-click (or click on Linux):** open the menu.
- **Menu contents (the "details dropdown"):**
  - A non-interactive header showing quick stats from the latest summary — `Today  $12.34`, `This week  $56.78`, `This month  $234.56` (disabled `MenuItem`s as labels; update whenever a collect finishes). If a budget is set, add `Budget  $234.56 / $6,000 (4%)` (rose text when ≥100%).
  - `───`
  - **Open Dashboard** → show/focus window.
  - **Refresh Now** → re-run the collector, then refresh tray labels + window.
  - `Launch at login` → checkable item wired to `tauri-plugin-autostart` (**unchecked/off by default**; mirrors the settings-panel toggle — keep the two in sync).
  - **Settings…** → open the settings panel (§5.2b).
  - `───`
  - **Quit**.
- **Keep-running behavior (locked):** closing the window **hides to tray**; the app keeps running and auto-refreshing; **Quit** exits. This is the default (no user setting).
- **Linux caveat:** the tray needs a StatusNotifierItem/appindicator host (GNOME-with-extension, KDE, most others). If absent, the app must still be fully usable from the window — never gate core function behind the tray, and don't let "hide to tray" strand the user (if no tray host, closing should minimize or the app should surface a way back).

> Note: this is **net-new** — the original has no tray. Inline live-text next to the icon is deliberately **not** attempted (not portable to Linux).

---

## 7. Cross-platform reality checks

1. **Tray = icon + menu only (accepted).** No inline live-text — it isn't portable to Linux. The menu-on-click "details dropdown" in §6.1 is the agreed surface and works on macOS + Linux. On Linux, the tray depends on a StatusNotifierItem/appindicator host; if it's missing the window must remain fully functional.
2. **Launch-at-login is net-new** — add via `tauri-plugin-autostart` (uses `SMAppService`/Login Items on macOS, a `~/.config/autostart/*.desktop` on Linux). Wire it to the checkable tray item in §6.1.
3. **Vendor Chart.js + fonts** locally (bundle into the frontend assets) — the original loads them from CDNs, which breaks offline and adds a network dependency a packaged Tauri app shouldn't have. Tauri's default CSP will block the CDN loads anyway.
4. **Pricing = one editable config, Rust is the only calculator.** Ship pricing as a bundled TOML/JSON read at startup; the frontend never re-prices (it displays precomputed costs). Don't reproduce the original's duplicate JS table.
5. **Linux path coverage is uncertain** for some tools (Claude Desktop on Linux is unofficial). Scan defensively: try each candidate path and skip missing ones (the collector already skips non-existent dirs).
6. **Tauri Linux WebView is WebKitGTK** — test the Chart.js dashboard there specifically (most likely place for a rendering/JS quirk vs macOS WKWebView); requires `libwebkit2gtk-4.1` at runtime.
7. **Two separate update channels (by design):**
   - **macOS** → `tauri-plugin-updater` pulling `latest.json` + `.dmg` from **GitHub Release assets**. Requires a **minisign keypair** (distinct from Apple notarization): public key in `tauri.conf.json`, private key + password as CI secrets. Gate the plugin + the "Check for updates" UI to macOS only.
   - **Linux** → **self-hosted flatpak repo on GitHub Pages**. CI builds the Flatpak, exports it into an OSTree repo, and publishes the repo to GH Pages; users `flatpak remote-add` once and get auto-updates. **`tauri-plugin-updater` must be disabled on Linux** — a Flatpak can't self-replace its files. (README should document the one-time `remote-add`.)
8. **Flatpak sandbox reads home read-only.** `--filesystem=home:ro` covers every tool dir now and later; only munim's own app-data dir is writable (`--filesystem=xdg-data/munim:create` or via the app-data portal). The `notify` file-watch works within granted paths. Native notifications need the notification portal (standard). File pickers use the xdg-desktop-portal (Tauri's dialog plugin handles this).
9. **Signing is two independent things on macOS**: Apple **Developer ID + notarization** (so Gatekeeper is happy) *and* the **updater minisign** signature (so the updater trusts the download). Set up both in CI; don't conflate them.
10. **Budget alerts need dedupe + month rollover.** Fire the 80%/100% notification once each per calendar month; persist which fired in `settings.json`; clear on month change. Guard against spamming when auto-refresh runs every minute.

---

## 8. Suggested build order for the AI agent

1. **Collector first (Rust module in `src-tauri`).** Port the parsers + pricing/cost math + aggregation from `collect-usage.js`, with a per-OS path resolver (§4.1) and XDG/App-Support app-data dir. Use `serde_json` + `walkdir`. Expose a `get_usage_data` and a `refresh` command. Unit-test each parser and the cost math using the tables in §4.5 as fixtures; validate totals against the original Node output on the same input files.
2. **Frontend (webview).** Copy `dashboard.html` + `css/**` + `js/**` into the Tauri frontend; vendor Chart.js + fonts locally. Shim `window.__SUMMARY__` / `__*_SESSIONS__` from the `get_usage_data` `invoke()` result on load; confirm every section renders.
3. **Tauri core.** Frameless dark window, native menu (Ctrl/Cmd+R → refresh event), the 5 commands (`refresh`/`export_data`/`import_data`/`save_imported_data`/`load_session_detail`) with the **security allowlist + fs-scope** re-implemented in Rust, app-data dir wiring, loading screen/route.
4. **System tray (§6.1).** Icon + menu-on-click with live quick-stat labels, Open/Refresh/Launch-at-login/Quit, close-hides-to-tray. Wire `tauri-plugin-autostart`.
5. **Refresh + caching.** Wire manual refresh (window FAB, menu, tray) AND **auto-refresh** per §4.8 (`notify` file-watch + 60 s interval fallback, debounced, non-overlapping, silent, pushes to webview + tray). Confirm `sessions-cache.json` + `scan-index.json` incremental behavior works cross-platform (2nd launch fast).
6. **Settings panel + budget + alerts (§5.2b, §0.5 #10-11).** In-app settings modal (budget, launch-at-login, data, about); `settings.json` persistence; monthly-budget bar on the THIS MONTH card and tray; native 80%/100% notifications with once-per-month dedupe + month rollover.
7. **Pricing config.** Move all rates into an editable bundled TOML/JSON read at startup; Rust is the sole cost calculator (§4.5).
8. **Branding.** New munim logo/icon (icns + png tray sizes), wordmark swap in header + loading screen, bundle id `com.munim.app`.
9. **Packaging + signing.** `tauri build`: macOS `.dmg` universal, **Developer ID signed + notarized**; Linux **Flatpak** (`--filesystem=home:ro`, xdg portals for dialogs/notifications). (Windows deferred.)
10. **Update channels.** macOS: `tauri-plugin-updater` + minisign keypair, `latest.json`/bundles on GitHub Releases. Linux: CI builds Flatpak → OSTree repo → publish to GitHub Pages remote; updater disabled on Linux. GitHub Actions ties builds + signing + manifest + release together.

---

## 9. Acceptance criteria

- Runs on macOS 12+ **and** a mainstream Linux (e.g. Ubuntu 22.04+ with WebKitGTK), reading each tool's data from the correct per-OS path.
- Rust collector produces identical cost numbers to the original Node collector for the same input files (validate the pricing math).
- All UI sections from §5 render and behave identically under WebKitGTK **and** WKWebView (provider pills, bar click-to-filter, both donuts, both heatmap views, both session-log views, filters/chips, detail modal + resume copy, export/import round-trip, reload).
- **Tray** works on macOS + Linux: icon shows, click opens the details menu with current Today/Week/Month figures, Open/Refresh/Launch-at-login/Quit all function; closing the window hides to tray; the app is fully usable from the window if no tray host exists on Linux.
- No network calls at runtime (Chart.js + fonts vendored; CSP restrictive).
- **Auto-refresh works**: editing a session file (or a live tool writing to one) updates the dashboard numbers, tray quick-stats, and "Last sync" within a few seconds without any manual action — silently (no loader/toast), without overlapping collects, and the 60 s interval fallback still refreshes if file events are missed.
- Incremental scan cache works (2nd launch is fast).
- Session-detail file reads are constrained to the allowlist + size cap + fs scope (no arbitrary file read).
- **Settings + budget work**: setting a monthly budget shows the budget bar (card + tray) and, when month spend crosses 80% then 100%, fires exactly one native notification each per calendar month; launch-at-login toggles from both settings and tray and stays in sync; launch-at-login is **off** on a fresh install.
- **Pricing** is read from the editable config (edit a rate → cost changes on next collect, no recompile).
- **Branding**: app is named munim end-to-end (window, tray, wordmark, bundle id `com.munim.app`, app-data dir `munim`); fresh logo present at all icon sizes.
- **Updates**: macOS auto-updater detects + installs a newer GitHub release; Linux Flatpak updates from the GitHub Pages remote; no updater code path runs on Linux.
- **Packaging**: macOS `.dmg` is signed + notarized (launches with no Gatekeeper warning); Linux Flatpak installs and reads `~/.claude` etc. under `home:ro`.

---

*Original repo: `https://github.com/658jjh/claude-usage-tracker` · v3.0.0 · MIT © 2026 Thien Nguyen. munim is an independent, MIT-licensed cross-platform reimplementation; the paid `src-premium/` license/update code is not part of the open source and is intentionally omitted.*
