pub mod app_audio;
pub mod audio;
pub mod drums;
pub mod settings;

use tauri::{Manager, State};

use settings::{Settings, SettingsStore};

#[tauri::command]
fn settings_load(store: State<'_, SettingsStore>) -> Settings {
    store.get()
}

#[tauri::command]
fn settings_save(
    store: State<'_, SettingsStore>,
    mut settings: Settings,
) -> Result<Settings, String> {
    // The output device belongs to the audio output manager
    // (`audio_set_output_device`); a save from the screen never changes it.
    settings.output_device_id = store.get().output_device_id;
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
            let audio = app_audio::AudioService::start(app.handle(), &app.state::<SettingsStore>());
            let drums = drums::DrumService::start(audio.engine(), &data_dir)?;
            app.manage(audio);
            app.manage(drums);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            settings_load,
            settings_save,
            app_audio::audio_status,
            app_audio::audio_list_devices,
            app_audio::audio_set_output_device,
            app_audio::audio_subscribe,
            app_audio::audio_heard_at,
            app_audio::audio_play_test_sound,
            drums::service::drums_state,
            drums::service::drums_set_playing,
            drums::service::drums_set_pattern,
            drums::service::drums_subscribe,
            drums::service::drums_save_pattern,
            drums::service::drums_delete_pattern,
        ])
        .build(tauri::generate_context!())
        .expect("error while building Woodshed")
        .run(|app, event| {
            if let tauri::RunEvent::Exit = event {
                app.state::<app_audio::AudioService>().shutdown();
            }
        });
}
