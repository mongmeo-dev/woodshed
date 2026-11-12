pub mod app_audio;
pub mod audio;
pub mod drums;
pub mod separation;
pub mod settings;
pub mod stems;
pub mod tuner;

use std::sync::Arc;

use tauri::{Emitter, Manager, State};

use settings::{Settings, SettingsStore};

// Async so they never run on the webview's IPC (main) thread; a save writes
// a file.

#[tauri::command]
async fn settings_load(store: State<'_, SettingsStore>) -> Result<Settings, String> {
    Ok(store.get())
}

#[tauri::command]
async fn settings_save(
    store: State<'_, SettingsStore>,
    settings: Settings,
) -> Result<Settings, String> {
    store
        .update(|current| {
            // The output device belongs to the audio output manager
            // (`audio_set_output_device`) and tuner choices to the tuner
            // commands (`tuner_select_input`, ...): a save from the screen
            // never changes them.
            let output_device_id = current.output_device_id.take();
            let tuner = std::mem::take(&mut current.tuner);
            *current = settings;
            current.output_device_id = output_device_id;
            current.tuner = tuner;
        })
        .map_err(|error| error.to_string())
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
            let drums = drums::DrumService::start(audio.engine(), &data_dir)?;
            app.manage(audio);
            app.manage(drums);
            app.manage(stems::StemService::new(
                &data_dir,
                app.path().app_cache_dir()?,
            ));
            let events = app.handle().clone();
            app.manage(separation::SeparationService::start(
                stems::Library::new(&data_dir),
                separation::app_model_loader(
                    app.path().resource_dir().ok(),
                    app.path().app_cache_dir().ok(),
                ),
                Arc::new(move |event| {
                    if let Err(error) = events.emit(stems::SEPARATION_EVENT, event) {
                        log::warn!("cannot send a separation event: {error}");
                    }
                }),
                separation::ServiceConfig::APP,
            ));
            let tuner = tuner::TunerService::new(app.handle(), &app.state::<SettingsStore>());
            app.manage(tuner);
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
            tuner::service::tuner_start,
            tuner::service::tuner_stop,
            tuner::service::tuner_select_input,
            tuner::service::tuner_set_reference,
            tuner::service::tuner_open_microphone_settings,
            drums::service::drums_state,
            drums::service::drums_set_playing,
            drums::service::drums_set_pattern,
            drums::service::drums_subscribe,
            drums::service::drums_save_pattern,
            drums::service::drums_delete_pattern,
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
            stems::stems_restore,
            separation::stems_separate,
            separation::stems_cancel_separation,
            separation::stems_separations,
        ])
        .build(tauri::generate_context!())
        .expect("error while building Woodshed")
        .run(|app, event| {
            if let tauri::RunEvent::Exit = event {
                app.state::<separation::SeparationService>().shutdown();
                app.state::<stems::StemService>().shutdown();
                app.state::<tuner::TunerService>().shutdown();
                app.state::<app_audio::AudioService>().shutdown();
            }
        });
}
