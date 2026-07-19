# munim

A local-first, cross-platform (macOS + Linux) tracker for your AI-coding **token usage and dollar cost**. munim auto-discovers session logs from 10+ tools (Claude Code, Claude Desktop, Cursor, Windsurf, Cline, Roo, Aider, Continue, OpenClaw, OpenAI Codex), computes per-model cost, and shows it in a dark interactive dashboard — with a system tray, auto-refresh, and a monthly budget with alerts. No cloud, no accounts, no telemetry.

> _munim_ (मुनीम) — a bookkeeper. It keeps your books.

Modeled on [`658jjh/claude-usage-tracker`](https://github.com/658jjh/claude-usage-tracker) (MIT). munim is an independent MIT-licensed cross-platform rebuild in **Tauri v2** (Rust core + system WebView, vanilla-JS + Chart.js frontend).

**The full build specification lives in [`BUILD_SPEC.md`](./BUILD_SPEC.md).** Start there — §0.5 is the authoritative decision list. This scaffold implements the project shape; the modules are stubs with `TODO(spec §…)` markers.

## Status

🚧 Scaffold only. Command handlers, the Rust collector, the tray, auto-refresh, budget/alerts, and the ported dashboard are stubbed — see `BUILD_SPEC.md` §8 for the build order.

## Layout

```
munim/
├── BUILD_SPEC.md              ← the spec (read this first)
├── README.md
├── LICENSE                    MIT
├── package.json               frontend scripts + `tauri` CLI
├── pricing.toml               editable per-model pricing (BUILD_SPEC §4.5)
├── src/                       frontend (vanilla JS + Chart.js) — port the original dashboard here
│   └── index.html
└── src-tauri/
    ├── Cargo.toml
    ├── build.rs
    ├── tauri.conf.json
    ├── capabilities/default.json
    ├── flatpak/com.munim.app.yml   Linux Flatpak manifest (BUILD_SPEC §7)
    └── src/
        ├── main.rs
        ├── lib.rs             Tauri builder, plugins, tray + watcher setup
        ├── commands.rs        the invoke() bridge (get_usage_data / refresh / export / import / …)
        ├── collector.rs       port of collect-usage.js (BUILD_SPEC §4)
        ├── pricing.rs         loads pricing.toml, cost math
        └── settings.rs        budget + autostart + alert-dedupe (settings.json)
```

## Prerequisites

- **Rust** (stable) + the Tauri v2 prerequisites for your OS: <https://v2.tauri.app/start/prerequisites/>
- **Node** (only for the `@tauri-apps/cli` dev tooling — not shipped in the app)
- Linux build needs `libwebkit2gtk-4.1-dev` and friends (see the Tauri prereqs page)

## Develop

```bash
npm install          # installs @tauri-apps/cli
npm run tauri dev     # runs the app (serves ./src, builds src-tauri)
```

## Build

```bash
npm run tauri build   # macOS .dmg / .app
```

Linux is packaged as a **Flatpak** (not a raw Tauri bundle) — see `src-tauri/flatpak/com.munim.app.yml` and the CI workflow.

## Distribution & updates

- **macOS** — Developer ID signed + notarized `.dmg`; in-app auto-update via `tauri-plugin-updater` reading `latest.json` from GitHub Releases (needs a minisign keypair; see BUILD_SPEC §7).
- **Linux** — Flatpak, auto-updating from a **self-hosted flatpak repo on GitHub Pages**. Users add the remote once:
  ```bash
  flatpak remote-add --if-not-exists munim https://surdy.github.io/munim/index.flatpakrepo
  flatpak install munim com.munim.app
  ```

CI (`.github/workflows/release.yml`) builds both, signs, generates the updater manifest, and publishes.

## License

MIT — see [`LICENSE`](./LICENSE).
