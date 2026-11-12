use std::{future::Future, sync::Mutex};

use serde::Deserialize;
use tokio::sync::watch;

use super::{ffmpeg, yt_dlp_bundle};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Preparing,
    Cancelled,
    Publishing,
}

pub(super) struct Installer(Mutex<Option<(String, watch::Sender<Phase>)>>);

pub(super) struct Installation<'a> {
    installer: &'a Installer,
    phase: watch::Sender<Phase>,
}

impl Installer {
    pub(super) const fn new() -> Self {
        Self(Mutex::new(None))
    }

    pub(super) fn start(&self, id: String) -> Result<Installation<'_>, String> {
        let mut active = self.0.lock().map_err(|error| error.to_string())?;
        if active.is_some() {
            return Err("Другая операция с инструментом уже выполняется.".into());
        }
        if id.is_empty() || id.len() > 64 {
            return Err("Неверный идентификатор установки.".into());
        }
        let phase = watch::Sender::new(Phase::Preparing);
        *active = Some((id, phase.clone()));
        Ok(Installation {
            installer: self,
            phase,
        })
    }

    fn cancel(&self, id: &str) -> bool {
        let Ok(active) = self.0.lock() else {
            return false;
        };
        active
            .as_ref()
            .is_some_and(|(active_id, phase)| active_id == id && set_phase(phase, Phase::Cancelled))
    }
}

fn set_phase(sender: &watch::Sender<Phase>, next: Phase) -> bool {
    sender.send_if_modified(|phase| {
        if *phase != Phase::Preparing {
            return false;
        }
        *phase = next;
        true
    })
}

impl Installation<'_> {
    pub(super) async fn wait<T>(
        &self,
        work: impl Future<Output = Result<T, String>>,
    ) -> Result<T, String> {
        let mut phase = self.phase.subscribe();
        tokio::select! {
            biased;
            _ = phase.wait_for(|phase| *phase == Phase::Cancelled) => Err("installCancelled".into()),
            result = work => result,
        }
    }

    pub(super) fn commit(&self) -> Result<(), String> {
        set_phase(&self.phase, Phase::Publishing)
            .then_some(())
            .ok_or_else(|| "installCancelled".into())
    }
}

impl Drop for Installation<'_> {
    fn drop(&mut self) {
        if let Ok(mut active) = self.installer.0.lock() {
            *active = None;
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Tool {
    Ffmpeg,
    #[serde(rename = "yt-dlp")]
    YtDlp,
}

#[tauri::command]
pub(crate) fn cancel_tool_install(tool: Tool, id: String) -> bool {
    match tool {
        Tool::Ffmpeg => ffmpeg::INSTALLER.cancel(&id),
        Tool::YtDlp => yt_dlp_bundle::INSTALLER.cancel(&id),
    }
}

#[tauri::command]
pub(crate) async fn remove_external_tool(app: tauri::AppHandle, tool: Tool) -> Result<(), String> {
    match tool {
        Tool::Ffmpeg => ffmpeg::remove(&app).await,
        Tool::YtDlp => yt_dlp_bundle::remove(&app).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn cancel_is_scoped_to_the_installation_and_stops_at_publish() {
        let installer = Installer::new();
        let first = installer.start("first".into()).unwrap();
        assert!(!installer.cancel("old"));
        assert!(installer.cancel("first"));
        assert!(first
            .wait(std::future::pending::<Result<(), String>>())
            .await
            .is_err());
        assert!(first.commit().is_err());
        drop(first);

        let next = installer.start("next".into()).unwrap();
        assert!(next.commit().is_ok());
        assert!(!installer.cancel("next"));
    }
}
