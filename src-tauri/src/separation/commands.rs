//! Tauri commands of the stem engine (see AGENTS.md "스템 라이브러리 계약").
//! Progress arrives as `stems://separation` events.

use tauri::State;

use crate::stems::SeparationEvent;

use super::service::SeparationService;

/// Queues song `songId` for separation; progress follows as events.
#[tauri::command]
pub async fn stems_separate(
    separation: State<'_, SeparationService>,
    song_id: String,
) -> Result<(), String> {
    separation
        .separate(&song_id)
        .map_err(|error| error.to_string())
}

/// Cancels the separation of song `songId` (queued or running).
#[tauri::command]
pub async fn stems_cancel_separation(
    separation: State<'_, SeparationService>,
    song_id: String,
) -> Result<(), String> {
    separation
        .cancel(&song_id)
        .map_err(|error| error.to_string())
}

/// Separations not finished yet (running first), as their latest event.
#[tauri::command]
pub async fn stems_separations(
    separation: State<'_, SeparationService>,
) -> Result<Vec<SeparationEvent>, String> {
    Ok(separation.status())
}
