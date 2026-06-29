# Tauri + Preact + Typescript

This template should help get you started developing with Tauri, Preact and Typescript in Vite.

## Recommended IDE Setup

- [VS Code](https://code.visualstudio.com/) + [Tauri](https://marketplace.visualstudio.com/items?itemName=tauri-apps.tauri-vscode) + [rust-analyzer](https://marketplace.visualstudio.com/items?itemName=rust-lang.rust-analyzer)

## Windows Build From WSL

Install Bun and Rust in WSL, then prepare the Windows cross-compile toolchain:

```sh
./scripts/setup-wsl-windows-build.sh
```

Build the Windows NSIS installer from WSL:

```sh
bun install --frozen-lockfile
bun run build:windows
```

The installer is written to:

`src-tauri/target/x86_64-pc-windows-msvc/release/bundle/nsis/yet-another-player_*_x64-setup.exe`

Use `bun run build:windows:native` on Windows when building locally without cross-compilation.

### Signed release build (updater artifacts)

Updater signatures are generated only when `TAURI_SIGNING_PRIVATE_KEY` is set:

```sh
export TAURI_SIGNING_PRIVATE_KEY="$(cat .tauri/signing.key)"
bun run build:windows:release
```

This also produces `.sig` files next to the installer for GitHub Releases auto-update.

By default the app scans the system audio directory, for example `%USERPROFILE%\Music` on Windows.

## Auto-update (GitHub Releases)

The app uses [Tauri updater](https://v2.tauri.app/plugin/updater/) and checks for updates on startup in production builds.

1. Update the updater endpoint in `src-tauri/tauri.conf.json` if your GitHub repo URL differs from `bad/yet-another-player`.
2. Add repository secrets for release signing:
   - `TAURI_SIGNING_PRIVATE_KEY` — full contents of `.tauri/signing.key` (generate with `bun run tauri signer generate -w .tauri/signing.key --ci`)
   - `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` — leave empty if the key has no password
3. Create a release by pushing a tag:

```sh
git tag v0.1.0
git push origin v0.1.0
```

The `Release` workflow builds the NSIS installer, uploads it to GitHub Releases, and publishes `latest.json` for the in-app updater.

## WSL Dev Audio

`bun run tauri dev` automatically configures audio in WSL:

- on WSLg (Windows 11) it uses `PULSE_SERVER=unix:/mnt/wslg/PulseServer`
- otherwise it falls back to a null ALSA device to avoid PipeWire/ALSA startup errors

If you still need real audio in WSL, run a PulseAudio server on Windows and expose it on port `4713`.
