pub mod audio;
pub mod settings;

use tauri::{Manager, State};

use settings::{Settings, SettingsStore};

#[tauri::command]
fn settings_load(store: State<'_, SettingsStore>) -> Settings {
    store.get()
}

#[tauri::command]
fn settings_save(store: State<'_, SettingsStore>, settings: Settings) -> Result<Settings, String> {
    store.save(settings).map_err(|error| error.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(
            tauri_plugin_log::Builder::new()
                .level(log::LevelFilter::Info)
                .build(),
        )
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            app.manage(SettingsStore::load(&data_dir));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            settings_load,
            settings_save,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Woodshed");
}
