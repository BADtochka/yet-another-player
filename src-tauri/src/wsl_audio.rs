use std::env;
use std::path::Path;

pub fn configure() {
    if !is_wsl() {
        return;
    }

    if Path::new("/mnt/wslg/PulseServer").exists() {
        env::set_var("PULSE_SERVER", "unix:/mnt/wslg/PulseServer");
    } else if let Ok(runtime_dir) = env::var("XDG_RUNTIME_DIR") {
        let pulse_socket = format!("{runtime_dir}/pulse/native");
        if Path::new(&pulse_socket).exists() {
            env::set_var("PULSE_SERVER", format!("unix:{pulse_socket}"));
        }
    }

    // OpenAL Soft and GStreamer may try PipeWire first; in WSL it is usually broken.
    env::set_var("ALSOFT_DRIVERS", "pulse");
    env::set_var(
        "GST_PLUGIN_FEATURE_RANK",
        "pipewiresink:NONE,pipewiresrc:NONE,pipewiredeviceprovider:NONE,pipewire:NONE",
    );
}

fn is_wsl() -> bool {
    env::var("WSL_DISTRO_NAME").is_ok()
        || std::fs::read_to_string("/proc/version")
            .map(|version| {
                let version = version.to_lowercase();
                version.contains("microsoft") || version.contains("wsl")
            })
            .unwrap_or(false)
}
