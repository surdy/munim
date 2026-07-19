# Releasing munim

munim ships on two channels, each with its own auto-update mechanism (BUILD_SPEC §7):

- **macOS** — a Developer-ID-signed + notarized `.dmg`, with in-app auto-update via
  `tauri-plugin-updater` reading `latest.json` from the GitHub Release.
- **Linux** — a Flatpak, auto-updating from a self-hosted OSTree repo on GitHub Pages
  (the `tauri-updater` is disabled on Linux; Flatpak owns updates there).

Both are produced by `.github/workflows/release.yml`, which runs on any `v*` tag.

## One-time setup (issue #14 — only a human can do this)

Nothing in the release pipeline works until these exist:

1. **Apple Developer account** ($99/yr). Create a *Developer ID Application* certificate and
   export it as a base64 `.p12`.
2. **Notarization** credentials (an app-specific password or an App Store Connect API key).
3. **Updater keypair**: `npm run tauri signer generate -w munim.key` → put the **public**
   key into `src-tauri/tauri.conf.json` at `plugins.updater.pubkey` (currently the
   placeholder `TODO_MINISIGN_PUBLIC_KEY`); keep the private key + password safe.
4. **Repo secrets** (Settings → Secrets and variables → Actions):
   - `APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`, `APPLE_SIGNING_IDENTITY`,
     `APPLE_ID`, `APPLE_PASSWORD`, `APPLE_TEAM_ID`
   - `TAURI_SIGNING_PRIVATE_KEY`, `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`
5. **GitHub Pages**: Settings → Pages → Source = *GitHub Actions* (the Linux job deploys the
   Flatpak OSTree repo there).

## Cutting a release

1. Bump the version in `src-tauri/tauri.conf.json` (and `Cargo.toml` / `package.json` to match).
2. Add a `<release>` entry to `src-tauri/flatpak/com.munim.app.metainfo.xml`.
3. Tag and push:
   ```bash
   git tag v0.1.0 && git push origin v0.1.0
   ```
4. The **macOS** job builds/signs/notarizes the universal `.dmg`, signs the updater artifact,
   and creates a **draft** GitHub Release with `.dmg` + `.app.tar.gz` + `.sig` + `latest.json`.
   Review and publish it.
5. The **Linux** job builds the Flatpak, exports the OSTree repo, and deploys it to Pages.

## Installing (end users)

- **macOS**: download the `.dmg` from Releases, drag to Applications. Updates apply in-app.
- **Linux**:
  ```bash
  flatpak remote-add --if-not-exists munim https://surdy.github.io/munim/index.flatpakrepo
  flatpak install munim com.munim.app
  ```

## Status / caveats

- The **macOS bundle build is verified locally** (an unsigned `.dmg` + a signed updater
  artifact are produced with a throwaway key). Signing + notarization are only exercised
  in CI once the secrets above exist.
- The **Linux Flatpak path is UNVERIFIED** on macOS (flatpak-builder is Linux-only); it is
  written to the standard pattern and runs on `ubuntu-latest` in CI. Expect to iterate on
  the manifest/CI the first time it runs on real hardware.
- The updater `pubkey` in `tauri.conf.json` is still the placeholder — replace it (step 3).
