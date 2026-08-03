#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::Arc;

use gwp_annotator_core::core::commands::{self as core_commands, CommandError};
use gwp_annotator_core::core::scan::{LocalDisplayScan, PublicScan};
use gwp_annotator_core::core::service::{
    CaptureFrameRequest, ReplaceRegionRequest, RepresentativeSelection, SaveRegionRequest,
    StartupRestore, TemplateDescriptor, UpdateRegionLabelsRequest,
};
use gwp_annotator_core::core::store::{BackupResult, CapturedFrame, SavedRegion};
use gwp_annotator_core::{AppState, ProjectService, SystemCredentialSecretProvider};
use tauri::{http, Manager, State, UriSchemeContext};
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
fn restore_project_session(state: State<'_, AppState>) -> Result<StartupRestore, CommandError> {
    core_commands::restore_project_session(state)
}

#[tauri::command]
fn forget_last_position(state: State<'_, AppState>) -> Result<(), CommandError> {
    core_commands::forget_last_position(state)
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
fn install_fixed_toml_template(
    template_toml: String,
    state: State<'_, AppState>,
) -> Result<TemplateDescriptor, CommandError> {
    core_commands::install_fixed_toml_template(template_toml, state)
}

#[tauri::command]
fn install_fixed_label_txt(
    text: String,
    state: State<'_, AppState>,
) -> Result<TemplateDescriptor, CommandError> {
    core_commands::install_fixed_label_txt(text, state)
}

#[tauri::command]
fn current_fixed_template(
    state: State<'_, AppState>,
) -> Result<Option<TemplateDescriptor>, CommandError> {
    core_commands::current_fixed_template(state)
}

#[tauri::command]
fn save_confirmed_region(
    request: SaveRegionRequest,
    state: State<'_, AppState>,
) -> Result<(), CommandError> {
    core_commands::save_confirmed_region(request, state)
}

#[tauri::command]
fn update_region_labels(
    request: UpdateRegionLabelsRequest,
    state: State<'_, AppState>,
) -> Result<(), CommandError> {
    core_commands::update_region_labels(request, state)
}

#[tauri::command]
fn delete_region(
    case_id: String,
    asset_id: String,
    region_id: String,
    state: State<'_, AppState>,
) -> Result<(), CommandError> {
    core_commands::delete_region(case_id, asset_id, region_id, state)
}

#[tauri::command]
fn replace_region(
    request: ReplaceRegionRequest,
    state: State<'_, AppState>,
) -> Result<(), CommandError> {
    core_commands::replace_region(request, state)
}

#[tauri::command]
fn capture_video_frame(
    request: CaptureFrameRequest,
    state: State<'_, AppState>,
) -> Result<CapturedFrame, CommandError> {
    core_commands::capture_video_frame(request, state)
}

#[tauri::command]
fn complete_case_and_backup(
    case_id: String,
    state: State<'_, AppState>,
) -> Result<BackupResult, CommandError> {
    core_commands::complete_case_and_backup(case_id, state)
}

#[tauri::command]
fn save_last_position(
    case_id: String,
    asset_id: String,
    state: State<'_, AppState>,
) -> Result<(), CommandError> {
    core_commands::save_last_position(case_id, asset_id, state)
}

#[tauri::command]
fn record_image_dimensions(
    case_id: String,
    asset_id: String,
    width: u32,
    height: u32,
    state: State<'_, AppState>,
) -> Result<(), CommandError> {
    core_commands::record_image_dimensions(case_id, asset_id, width, height, state)
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

fn media_protocol_response(
    context: UriSchemeContext<'_, tauri::Wry>,
    request: http::Request<Vec<u8>>,
) -> http::Response<Vec<u8>> {
    let mut segments = request
        .uri()
        .path()
        .split('/')
        .filter(|segment| !segment.is_empty());
    let (Some("video"), Some(case_id), Some(asset_id), None) = (
        segments.next(),
        segments.next(),
        segments.next(),
        segments.next(),
    ) else {
        return media_error_response(http::StatusCode::BAD_REQUEST);
    };
    let app_state = context.app_handle().state::<AppState>();
    let length = match app_state.service.video_asset_size(case_id, asset_id) {
        Ok(length) => length,
        Err(_) => return media_error_response(http::StatusCode::NOT_FOUND),
    };
    let range = request
        .headers()
        .get(http::header::RANGE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| parse_byte_range(value, length));
    match range {
        Some((start, end)) => {
            let body = match app_state
                .service
                .video_asset_range_bytes(case_id, asset_id, start, end)
            {
                Ok(bytes) => bytes,
                Err(_) => return media_error_response(http::StatusCode::NOT_FOUND),
            };
            http::Response::builder()
                .status(http::StatusCode::PARTIAL_CONTENT)
                .header(http::header::CONTENT_TYPE, "video/mp4")
                .header(http::header::ACCEPT_RANGES, "bytes")
                .header(http::header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
                .header(
                    http::header::CONTENT_RANGE,
                    format!("bytes {start}-{end}/{length}"),
                )
                .header(http::header::CONTENT_LENGTH, body.len())
                .body(body)
                .expect("valid local media response")
        }
        None if request.headers().contains_key(http::header::RANGE) => {
            media_error_response(http::StatusCode::RANGE_NOT_SATISFIABLE)
        }
        None => match app_state.service.video_asset_bytes(case_id, asset_id) {
            Ok(bytes) => http::Response::builder()
                .status(http::StatusCode::OK)
                .header(http::header::CONTENT_TYPE, "video/mp4")
                .header(http::header::ACCEPT_RANGES, "bytes")
                .header(http::header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
                .header(http::header::CONTENT_LENGTH, length)
                .body(bytes)
                .expect("valid local media response"),
            Err(_) => media_error_response(http::StatusCode::NOT_FOUND),
        },
    }
}

fn media_error_response(status: http::StatusCode) -> http::Response<Vec<u8>> {
    http::Response::builder()
        .status(status)
        .header(http::header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .body(b"Local media unavailable".to_vec())
        .expect("valid local media error")
}

fn parse_byte_range(value: &str, length: usize) -> Option<(usize, usize)> {
    let value = value.strip_prefix("bytes=")?;
    if value.contains(',') || length == 0 {
        return None;
    }
    let (start, end) = value.split_once('-')?;
    if start.is_empty() {
        let suffix = end.parse::<usize>().ok()?;
        if suffix == 0 {
            return None;
        }
        return Some((length.saturating_sub(suffix), length - 1));
    }
    let start = start.parse::<usize>().ok()?;
    if start >= length {
        return None;
    }
    let end = if end.is_empty() {
        length - 1
    } else {
        end.parse::<usize>().ok()?.min(length - 1)
    };
    (end >= start).then_some((start, end))
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .register_uri_scheme_protocol("gwp-media", media_protocol_response)
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
            restore_project_session,
            forget_last_position,
            current_local_display_scan,
            select_representative,
            deselect_representative,
            import_json_template,
            import_label_txt,
            install_fixed_toml_template,
            install_fixed_label_txt,
            current_fixed_template,
            save_confirmed_region,
            update_region_labels,
            delete_region,
            replace_region,
            capture_video_frame,
            complete_case_and_backup,
            save_last_position,
            record_image_dimensions,
            read_image_asset,
            saved_regions_for_asset,
            representative_asset_ids
        ])
        .run(tauri::generate_context!())
        .expect("desktop runtime failed to start");
}

#[cfg(test)]
mod tests {
    use super::parse_byte_range;

    #[test]
    fn parses_supported_single_video_byte_ranges() {
        assert_eq!(parse_byte_range("bytes=0-9", 100), Some((0, 9)));
        assert_eq!(parse_byte_range("bytes=90-", 100), Some((90, 99)));
        assert_eq!(parse_byte_range("bytes=-10", 100), Some((90, 99)));
        assert_eq!(parse_byte_range("bytes=95-120", 100), Some((95, 99)));
        assert_eq!(parse_byte_range("bytes=100-", 100), None);
        assert_eq!(parse_byte_range("bytes=9-8", 100), None);
        assert_eq!(parse_byte_range("bytes=0-1,4-5", 100), None);
        assert_eq!(parse_byte_range("bytes=0-0", 0), None);
    }
}
