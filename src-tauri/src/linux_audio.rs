#[cfg(target_os = "linux")]
use std::env;
#[cfg(target_os = "linux")]
use std::path::Path;

use serde::Serialize;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioEnvironment {
    pub platform: String,
    pub is_wsl: bool,
    pub pulse_server: Option<String>,
    pub pulse_latency_ms: Option<String>,
    pub web_audio_recommended: bool,
    pub web_audio_disabled_by_env: bool,
    pub notes: Vec<String>,
}

pub fn configure() -> AudioEnvironment {
    #[cfg(target_os = "linux")]
    {
        configure_linux()
    }

    #[cfg(not(target_os = "linux"))]
    {
        AudioEnvironment {
            platform: std::env::consts::OS.to_string(),
            is_wsl: false,
            pulse_server: None,
            pulse_latency_ms: None,
            web_audio_recommended: false,
            web_audio_disabled_by_env: false,
            notes: vec![
                "Volume normalization uses ReplayGain tags and native audio output.".into(),
            ],
        }
    }
}

#[cfg(target_os = "linux")]
fn configure_linux() -> AudioEnvironment {
    let is_wsl = is_wsl();
    let mut notes = Vec::new();

    disable_pipewire(&mut notes);
    configure_pulse(is_wsl, &mut notes);
    configure_gstreamer(&mut notes);
    configure_alsa(&mut notes);

    let pulse_server = env::var("PULSE_SERVER").ok();
    let pulse_latency_ms = env::var("PULSE_LATENCY_MSEC").ok();
    let web_audio_recommended = false;

    if is_wsl && pulse_server.is_some() {
        notes.push("Native PulseAudio playback is used in WSL.".into());
    } else if is_wsl && pulse_server.is_none() {
        notes.push("PulseAudio is unavailable in WSL.".into());
    }

    notes.push("Volume normalization uses ReplayGain tags and native audio output.".into());

    AudioEnvironment {
        platform: "linux".to_string(),
        is_wsl,
        pulse_server,
        pulse_latency_ms,
        web_audio_recommended,
        web_audio_disabled_by_env: false,
        notes,
    }
}

#[cfg(target_os = "linux")]
fn disable_pipewire(notes: &mut Vec<String>) {
    env::set_var("ALSOFT_DRIVERS", "pulse");
    env::set_var("SDL_AUDIODRIVER", "pulse");

    if env::var("PIPEWIRE_LATENCY").is_err() {
        env::set_var("PIPEWIRE_LATENCY", "1024/48000");
    }

    env::set_var(
        "GST_PLUGIN_FEATURE_RANK",
        "pipewiresink:NONE,pipewiresrc:NONE,pipewiredeviceprovider:NONE,pipewire:NONE,pulsesink:PRIMARY(+1000),pulse:PRIMARY(+500)",
    );

    notes.push("Disabled PipeWire GStreamer plugins; PulseAudio is preferred.".into());
}

#[cfg(target_os = "linux")]
fn configure_pulse(is_wsl: bool, notes: &mut Vec<String>) {
    if is_wsl && Path::new("/mnt/wslg/PulseServer").exists() {
        env::set_var("PULSE_SERVER", "unix:/mnt/wslg/PulseServer");
        env::set_var("PULSE_ALLOW_AUTOSPAWN", "0");
        notes.push("Using WSLg PulseAudio socket.".into());
    } else if env::var("PULSE_SERVER").is_err() {
        if let Ok(runtime_dir) = env::var("XDG_RUNTIME_DIR") {
            let pulse_socket = format!("{runtime_dir}/pulse/native");
            if Path::new(&pulse_socket).exists() {
                env::set_var("PULSE_SERVER", format!("unix:{pulse_socket}"));
                notes.push("Using user PulseAudio socket.".into());
            }
        }
    }

    if env::var("PULSE_LATENCY_MSEC").is_err() {
        let latency = if is_wsl { "150" } else { "80" };
        env::set_var("PULSE_LATENCY_MSEC", latency);
        notes.push(format!(
            "Set PULSE_LATENCY_MSEC={latency} for larger playback buffers."
        ));
    }

    env::set_var(
        "PULSE_PROP",
        "media.role=music application.name=yet-another-player",
    );
}

#[cfg(target_os = "linux")]
fn configure_gstreamer(notes: &mut Vec<String>) {
    env::set_var("GST_REGISTRY_FORK", "no");

    if env::var("GST_PLUGIN_SCANNER").is_err() {
        notes.push("Using default GStreamer plugin scanner.".into());
    }
}

#[cfg(target_os = "linux")]
fn configure_alsa(notes: &mut Vec<String>) {
    if env::var("PULSE_SERVER").is_ok() {
        env::set_var("ALSA_PCM_CARD", "pulse");
        env::set_var("ALSA_PCM_DEVICE", "0");
        notes.push("Routed ALSA clients through PulseAudio.".into());
    }
}

#[cfg(target_os = "linux")]
fn is_wsl() -> bool {
    env::var("WSL_DISTRO_NAME").is_ok()
        || std::fs::read_to_string("/proc/version")
            .map(|version| {
                let version = version.to_lowercase();
                version.contains("microsoft") || version.contains("wsl")
            })
            .unwrap_or(false)
}
