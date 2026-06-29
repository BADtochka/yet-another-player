#!/usr/bin/env bash
set -euo pipefail

is_wsl() {
  [ -n "${WSL_DISTRO_NAME:-}" ] || grep -qiE '(microsoft|wsl)' /proc/version 2>/dev/null
}

setup_wsl_audio() {
  if ! is_wsl; then
    return
  fi

  if [ -S /mnt/wslg/PulseServer ]; then
    export PULSE_SERVER=unix:/mnt/wslg/PulseServer
  elif [ -S "${XDG_RUNTIME_DIR:-/run/user/$(id -u)}/pulse/native" ]; then
    export PULSE_SERVER="unix:${XDG_RUNTIME_DIR:-/run/user/$(id -u)}/pulse/native"
  elif command -v nc >/dev/null 2>&1 && nc -z 127.0.0.1 4713 2>/dev/null; then
    export PULSE_SERVER=tcp:127.0.0.1:4713
  else
    script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
    export ALSA_CONFIG_PATH="${script_dir}/alsa-null${ALSA_CONFIG_PATH:+:${ALSA_CONFIG_PATH}}"
  fi

  # PipeWire is often half-configured in WSL and breaks WebKit/OpenAL intermittently.
  export ALSOFT_DRIVERS=pulse
  export GST_PLUGIN_FEATURE_RANK=pipewiresink:NONE,pipewiresrc:NONE,pipewiredeviceprovider:NONE,pipewire:NONE
}

setup_wsl_audio
exec "$@"
