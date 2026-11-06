# Building and publishing installers

Bayonet is released like every app on the release kit. The process, the commands and the machine
setup are in [scripts/release-kit/README.md](scripts/release-kit/README.md). In short, run these
from the repository root:

1. On either machine:
   ```bash
   make bump
   ```
   Release notes for the version can go in `release-notes/<version>.md`; the download page and the
   update notification show them.
   ```bash
   git commit -am "Version 0.1.38" && git push
   ```
2. On the Windows PC:
   ```bash
   git pull && make release
   ```
   That builds Windows, then Linux in WSL Ubuntu-22.04, then publishes both.
3. On the Mac:
   ```bash
   git pull && make release
   ```
   That builds Apple Silicon and Intel, then publishes them.

Nothing is built on GitHub Actions; CI only runs the checks.

| Platform            | Installers on the site                                                                         |
| ------------------- | ---------------------------------------------------------------------------------------------- |
| Windows 10/11 (x64) | `Bayonet-Windows-Setup.exe` (NSIS), `Bayonet-Windows.msi`                                      |
| macOS 11+           | `Bayonet-macOS-AppleSilicon.dmg`, `Bayonet-macOS-Intel.dmg`, plus `.app.tar.gz` update bundles |
| Linux x64           | `Bayonet-Linux-x86_64.AppImage`, `Bayonet-Linux-amd64.deb`, `Bayonet-Linux-x86_64.rpm`         |

## Bayonet's settings

These are in [release.config.mjs](release.config.mjs).

- **Download site:** https://skuirrels.github.io/bayonet-releases/ (GitHub Pages of the public repo
  `skuirrels/bayonet-releases`, serving its `gh-pages` branch; the first publish creates it).
- **Update feeds:** `updates/<os>-<arch>.json`, plus `latest.json`.
- **Version:** `src-tauri/tauri.conf.json`, `package.json` and the Cargo workspace (`make bump`
  sets all three and `Cargo.lock`). The app reports the Cargo version, and remote development
  downloads the `openidea-server` of that version.
- **Plugin API:** when `wit/` moves to a new plugin API version, also run
  `pnpm release:prepare <version>` before committing; it updates the released API snapshot.

## Updates

Installed copies check the feeds once when they start, and again from **Help ▸ Check for Updates**.
An available update shows as a notification with its release notes; **Install and Restart**
downloads it with progress (and Cancel) in the status bar, installs it and restarts. A feed that has a newer version but no build for this platform yet
counts as up to date, since each platform is published on its own. Debug builds never update.

Updates are signed with a minisign key. The public half is `plugins.updater.pubkey` in
`src-tauri/tauri.conf.json`. Installed copies refuse updates that aren't signed with the matching
private key.

The private key is `~/.tauri/bayonet-updater.key`, and its password is in
`~/.tauri/bayonet-updater.key.password`. Both machines need them; the WSL build reads the Windows
copy. **Back them up.** If the key is lost, installed copies can't be updated, and users must
reinstall a build signed with a new key.

## Code signing (optional, recommended before public distribution)

Without signing, the installers work, but the OS warns about them:

- **Windows:** SmartScreen shows "Windows protected your PC". Users choose _More info → Run anyway_.
  To fix it, sign with a code-signing certificate. See
  [Windows code signing](https://v2.tauri.app/distribute/sign/windows/), and set
  `bundle.windows.certificateThumbprint` or a `signCommand` (e.g. Azure Trusted Signing).
- **macOS:** builds are ad-hoc signed (`bundle.macOS.signingIdentity: "-"`), which Apple Silicon
  requires. Gatekeeper still blocks apps downloaded from the internet, so users right-click the
  app → _Open_. To fix it, you need an Apple Developer account. With the certificate in the Mac's
  keychain, set `APPLE_SIGNING_IDENTITY` before `make release`; also set `APPLE_ID`,
  `APPLE_PASSWORD` and `APPLE_TEAM_ID` to notarize. See
  [macOS signing](https://v2.tauri.app/distribute/sign/macos/).
- **Linux:** no signing needed. The AppImage needs FUSE 2 on the target machine (`libfuse2` on
  Ubuntu 22.04+).
