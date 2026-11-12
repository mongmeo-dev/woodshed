pub mod app_audio;
pub mod audio;
pub mod settings;
pub mod stems;

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
        .plugin(tauri_plugin_dialog::init())
        .plugin(
            tauri_plugin_log::Builder::new()
                .level(log::LevelFilter::Info)
                .build(),
        )
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            app.manage(SettingsStore::load(&data_dir));
            let audio = app_audio::AudioService::start(app.handle(), &app.state::<SettingsStore>());
            app.manage(audio);
            app.manage(stems::StemService::new(
                &data_dir,
                app.path().app_cache_dir()?,
            ));
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
            stems::stems_list,
            stems::stems_pick_file,
            stems::stems_import_file,
            stems::stems_import_link,
            stems::stems_cancel_import,
            stems::stems_delete,
            stems::stems_open,
            stems::stems_close,
            stems::stems_play,
            stems::stems_pause,
            stems::stems_seek,
            stems::stems_set_speed,
            stems::stems_set_loop,
            stems::stems_set_track,
            stems::stems_set_master,
            stems::stems_transport,
            stems::stems_resync,
        ])
        .build(tauri::generate_context!())
        .expect("error while building Woodshed")
        .run(|app, event| {
            if let tauri::RunEvent::Exit = event {
                app.state::<stems::StemService>().shutdown();
                app.state::<app_audio::AudioService>().shutdown();
            }
        });
}
