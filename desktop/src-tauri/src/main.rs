#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::Arc;

use gwp_annotator_core::core::commands::{self as core_commands, CommandError};
use gwp_annotator_core::core::scan::{LocalDisplayScan, PublicScan};
use gwp_annotator_core::core::service::{
    RepresentativeSelection, SaveRegionRequest, TemplateDescriptor,
};
use gwp_annotator_core::{AppState, ProjectService, SystemCredentialSecretProvider};
use tauri::{Manager, State};

#[tauri::command]
fn scan_selected_root(
    selected_root: String,
    state: State<'_, AppState>,
) -> Result<PublicScan, CommandError> {
    core_commands::scan_selected_root(selected_root, state)
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

fn main() {
    tauri::Builder::default()
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
            scan_selected_root,
            current_scan,
            current_local_display_scan,
            select_representative,
            deselect_representative,
            import_json_template,
            import_label_txt,
            save_confirmed_region
        ])
        .run(tauri::generate_context!())
        .expect("desktop runtime failed to start");
}
