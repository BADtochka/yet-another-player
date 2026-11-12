#[tauri::command]
pub(crate) fn uses_arch_package() -> bool {
    cfg!(target_os = "linux")
        && std::env::current_exe().is_ok_and(|path| {
            path == std::path::Path::new("/usr/lib/yet-another-player/yet-another-player")
        })
}

async fn arch_update(
    app: &tauri::AppHandle,
) -> Result<Option<tauri_plugin_updater::Update>, String> {
    use tauri_plugin_updater::UpdaterExt;

    if !uses_arch_package() {
        return Err("Обновление через pacman доступно в пакете для Arch Linux.".into());
    }
    let updater = app
        .updater_builder()
        .target("linux-x86_64-pacman")
        .build()
        .map_err(|error| error.to_string())?;
    updater.check().await.map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) async fn check_arch_update(app: tauri::AppHandle) -> Result<Option<String>, String> {
    Ok(arch_update(&app).await?.map(|update| update.version))
}

#[tauri::command]
pub(crate) async fn download_arch_update(
    app: tauri::AppHandle,
    version: String,
) -> Result<String, String> {
    use std::io::Write;
    use tauri::Manager;

    let update = arch_update(&app)
        .await?
        .filter(|update| update.version == version)
        .ok_or("Версия обновления изменилась. Проверьте обновления ещё раз.")?;
    let bytes = update
        .download(|_, _| {}, || {})
        .await
        .map_err(|error| error.to_string())?;
    let directory = app
        .path()
        .app_cache_dir()
        .map_err(|error| error.to_string())?
        .join("updates");
    let version = update.version.replace(['-', '+'], "_");
    let path = tauri::async_runtime::spawn_blocking(move || -> std::io::Result<_> {
        std::fs::create_dir_all(&directory)?;
        let path = directory.join(format!("yet-another-player-{version}-1-x86_64.pkg.tar.zst"));
        let mut temporary = tempfile::NamedTempFile::new_in(&directory)?;
        temporary.write_all(&bytes)?;
        temporary.persist(&path)?;
        Ok(path)
    })
    .await
    .map_err(|error| error.to_string())?
    .map_err(|error| error.to_string())?;
    path.into_os_string()
        .into_string()
        .map_err(|_| "Путь к пакету содержит неподдерживаемые символы.".into())
}
