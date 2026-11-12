# YAP — Yet Another Player

A desktop player for local music, built with Tauri, Rust, and Preact. Choose your music folders on first launch. YAP indexes file tags in a local SQLite catalog and plays audio through a native Rust engine. Install yt-dlp from Settings to search YouTube Music and download tracks; FFmpeg is optional for MP3 conversion.

## Development

```sh
bun install --frozen-lockfile
bun run tauri dev
```

Run `bun run build` to type-check and bundle the frontend and `cargo test --locked --manifest-path src-tauri/Cargo.toml` to test the native code. See [the refactor plan](REFACTOR_PLAN.md) for the architecture and design reference.

## Linux builds

On Ubuntu 22.04 x86_64, install the Linux build dependencies and build both packages:

```sh
sudo apt-get install libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev libasound2-dev patchelf libfuse2
bun install --frozen-lockfile
bun run build:linux
```

The `.deb` and `.AppImage` are written to `src-tauri/target/release/bundle/{deb,appimage}/`. On Debian or Ubuntu, install the `.deb` with `sudo apt install ./<package>.deb`. Make the AppImage executable before launching it.

The **Desktop builds** workflow also uploads both packages as Actions artifacts. For a manual build without a release, leave **Release tag** empty. Select **Build only Linux** to skip Windows.

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

Or run **Desktop builds** manually in Actions, select the tag ref and enter the same value under **Release tag**.

The workflow creates one draft GitHub Release after both platforms build. It includes the Windows NSIS installer, Linux x86_64 Debian package and AppImage, their updater signatures, and a shared `latest.json`. The **Desktop checks** workflow runs tests on a macOS runner as well; the release workflow currently packages Windows and Linux.

## WSL Dev Audio

`bun run tauri dev` automatically configures audio in WSL:

- on WSLg (Windows 11) it uses `PULSE_SERVER=unix:/mnt/wslg/PulseServer`
- PipeWire GStreamer plugins are disabled because they often break WebKit playback in WSL
- `PULSE_LATENCY_MSEC=150` increases the PulseAudio buffer
- otherwise it falls back to a null ALSA device to avoid PipeWire/ALSA startup errors

If you still need real audio in WSL, run a PulseAudio server on Windows and expose it on port `4713`.

Check **Settings → Diagnostics** for the active audio environment and WSL routing.

WSL always uses native playback. Volume normalization uses ReplayGain tags from MP3 files on all platforms.
