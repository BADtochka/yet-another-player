# YAP — Yet Another Player

A desktop player for local music, built with Tauri, Rust, and Preact. Choose your music folders on first launch. YAP indexes file tags in a local SQLite catalog and plays audio through a native Rust engine. Install yt-dlp from Settings to search YouTube Music and download tracks; FFmpeg is optional for MP3 conversion.

## Development

```sh
bun install --frozen-lockfile
bun run tauri dev
```

Run `bun run build` to type-check and bundle the frontend and `cargo test --locked --manifest-path src-tauri/Cargo.toml` to test the native code. See [the refactor plan](REFACTOR_PLAN.md) for the architecture and design reference.

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

## Auto-update (GitHub Releases)

The app uses [Tauri updater](https://v2.tauri.app/plugin/updater/) and checks for updates on startup in production builds.

1. Update the updater endpoint in `src-tauri/tauri.conf.json` if your GitHub repo URL differs from `BADtochka/yet-another-player`.
2. Add repository secrets for release signing:
   - `TAURI_SIGNING_PRIVATE_KEY` — full contents of `.tauri/signing.key` (generate with `bun run tauri signer generate -w .tauri/signing.key --ci`)
   - `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` — leave empty if the key has no password
3. Create a release by pushing a tag:

```sh
git tag v0.1.0
git push origin v0.1.0
```

Or run **Windows build** manually in Actions, set **release tag** to `v0.1.0`, and pick the tag ref to build from.

The `Windows build` workflow compiles the NSIS installer on tags, uploads `latest.json` and signatures, and creates a draft GitHub Release.

## WSL Dev Audio

`bun run tauri dev` automatically configures audio in WSL:

- on WSLg (Windows 11) it uses `PULSE_SERVER=unix:/mnt/wslg/PulseServer`
- PipeWire GStreamer plugins are disabled because they often break WebKit playback in WSL
- `PULSE_LATENCY_MSEC=150` increases the PulseAudio buffer
- otherwise it falls back to a null ALSA device to avoid PipeWire/ALSA startup errors

If you still need real audio in WSL, run a PulseAudio server on Windows and expose it on port `4713`.

Check **Settings → Diagnostics** for the active audio environment and WSL routing.

WSL always uses native playback. Volume normalization uses ReplayGain tags from MP3 files on all platforms.
