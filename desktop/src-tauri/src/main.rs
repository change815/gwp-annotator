#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::Arc;

use gwp_annotator_core::core::commands::{self as core_commands, CommandError};
use gwp_annotator_core::core::scan::{LocalDisplayScan, PublicScan};
use gwp_annotator_core::core::service::{
    RepresentativeSelection, SaveRegionRequest, TemplateDescriptor,
};
use gwp_annotator_core::core::store::SavedRegion;
use gwp_annotator_core::{AppState, ProjectService, SystemCredentialSecretProvider};
use tauri::{Manager, State};
use tauri_plugin_dialog::DialogExt;

/// Opens the native folder dialog inside the Rust process so the selected source path never
/// crosses IPC to the webview. Only a de-identified scan result is returned.
#[tauri::command]
fn choose_and_scan_root(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<Option<PublicScan>, CommandError> {
    let Some(selected) = app.dialog().file().blocking_pick_folder() else {
        return Ok(None);
    };
    let selected = selected.into_path().map_err(|_| CommandError {
        code: "FILESYSTEM_ERROR",
    })?;
    state
        .service
        .scan_selected_root(&selected)
        .map(Some)
        .map_err(Into::into)
}

#[tauri::command]
fn current_scan(state: State<'_, AppState>) -> Result<PublicScan, CommandError> {
    core_commands::current_scan(state)
}

#[tauri::command]
fn current_local_display_scan(
    state: State<'_, AppState>,
) -> Result<LocalDisplayScan, CommandError> {
    core_commands::current_local_display_scan(state)
}

#[tauri::command]
fn select_representative(
    case_id: String,
    asset_id: String,
    state: State<'_, AppState>,
) -> Result<RepresentativeSelection, CommandError> {
    core_commands::select_representative(case_id, asset_id, state)
}

#[tauri::command]
fn deselect_representative(
    case_id: String,
    asset_id: String,
    state: State<'_, AppState>,
) -> Result<RepresentativeSelection, CommandError> {
    core_commands::deselect_representative(case_id, asset_id, state)
}

#[tauri::command]
fn import_json_template(
    template_json: String,
    state: State<'_, AppState>,
) -> Result<TemplateDescriptor, CommandError> {
    core_commands::import_json_template(template_json, state)
}

#[tauri::command]
fn import_label_txt(
    text: String,
    template_id: String,
    version: u32,
    state: State<'_, AppState>,
) -> Result<TemplateDescriptor, CommandError> {
    core_commands::import_label_txt(text, template_id, version, state)
}

#[tauri::command]
fn save_confirmed_region(
    request: SaveRegionRequest,
    state: State<'_, AppState>,
) -> Result<(), CommandError> {
    core_commands::save_confirmed_region(request, state)
}

#[tauri::command]
fn read_image_asset(
    case_id: String,
    asset_id: String,
    state: State<'_, AppState>,
) -> Result<gwp_annotator_core::core::scan::ImageAssetData, CommandError> {
    core_commands::read_image_asset(case_id, asset_id, state)
}

#[tauri::command]
fn saved_regions_for_asset(
    case_id: String,
    asset_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<SavedRegion>, CommandError> {
    core_commands::saved_regions_for_asset(case_id, asset_id, state)
}

#[tauri::command]
fn representative_asset_ids(
    case_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<String>, CommandError> {
    core_commands::representative_asset_ids(case_id, state)
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let work_root = app
                .path()
                .app_local_data_dir()?
                .join("projects")
                .join("PROJECT-LOCAL");
            let service = ProjectService::new(
                work_root,
                "PROJECT-LOCAL",
                Arc::new(SystemCredentialSecretProvider::default()),
            )?;
            app.manage(AppState::new(service));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            choose_and_scan_root,
            current_scan,
            current_local_display_scan,
            select_representative,
            deselect_representative,
            import_json_template,
            import_label_txt,
            save_confirmed_region,
            read_image_asset,
            saved_regions_for_asset,
            representative_asset_ids
        ])
        .run(tauri::generate_context!())
        .expect("desktop runtime failed to start");
}
