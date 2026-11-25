use std::{collections::HashSet, fs, io, path::Path, sync::Mutex};

use tauri::{AppHandle, Manager, State};

use super::{install::Tool, lyrica_server::LocalLyrica, ommr::LocalServer};

#[derive(Default)]
pub(crate) struct DisabledTools(Mutex<HashSet<Tool>>);

impl DisabledTools {
    pub(crate) fn load(directory: &Path) -> Self {
        let tools = match fs::read(directory.join("disabled-tools.json")) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(io::Error::other),
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Self::default(),
            Err(error) => Err(error),
        };
        Self(Mutex::new(tools.unwrap_or_else(|error| {
            eprintln!("External tool settings: {error}");
            HashSet::new()
        })))
    }
}

pub(crate) fn tool_enabled(app: &AppHandle, tool: Tool) -> bool {
    app.state::<DisabledTools>()
        .0
        .lock()
        .is_ok_and(|tools| !tools.contains(&tool))
}

#[tauri::command]
pub(crate) fn disabled_external_tools(
    state: State<'_, DisabledTools>,
) -> Result<HashSet<Tool>, String> {
    state
        .0
        .lock()
        .map(|tools| tools.clone())
        .map_err(|_| "Настройки инструментов недоступны.".into())
}

#[tauri::command]
pub(crate) async fn set_external_tool_enabled(
    app: AppHandle,
    tool: Tool,
    enabled: bool,
) -> Result<(), String> {
    let directory = app.path().app_data_dir().map_err(|err| err.to_string())?;
    let state = app.state::<DisabledTools>();
    {
        let mut tools = state
            .0
            .lock()
            .map_err(|_| "Настройки инструментов недоступны.")?;
        let mut next = tools.clone();
        if enabled {
            next.remove(&tool);
        } else {
            next.insert(tool);
        }
        fs::create_dir_all(&directory).map_err(|err| err.to_string())?;
        let mut file =
            tempfile::NamedTempFile::new_in(&directory).map_err(|err| err.to_string())?;
        serde_json::to_writer(file.as_file_mut(), &next).map_err(|err| err.to_string())?;
        file.persist(directory.join("disabled-tools.json"))
            .map_err(|err| err.to_string())?;
        *tools = next;
    }
    if !enabled {
        match tool {
            Tool::Ommr => app.state::<LocalServer>().stop_if_disabled(&app).await?,
            Tool::Lyrica => app.state::<LocalLyrica>().stop_if_disabled(&app).await?,
            _ => {}
        }
    }
    Ok(())
}
