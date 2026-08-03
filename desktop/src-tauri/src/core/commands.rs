use std::path::PathBuf;

use serde::Serialize;
use tauri::State;

use crate::core::error::CoreError;
use crate::core::scan::{ImageAssetData, LocalDisplayScan, PublicScan};
use crate::core::service::{
    ProjectService, RepresentativeSelection, SaveRegionRequest, TemplateDescriptor,
};
use crate::core::store::SavedRegion;

pub struct AppState {
    pub service: ProjectService,
}

impl AppState {
    pub fn new(service: ProjectService) -> Self {
        Self { service }
    }
}

#[derive(Debug, Serialize)]
pub struct CommandError {
    pub code: &'static str,
}

impl From<CoreError> for CommandError {
    fn from(error: CoreError) -> Self {
        Self {
            code: error_code(&error),
        }
    }
}

pub fn scan_selected_root(
    selected_root: String,
    state: State<'_, AppState>,
) -> Result<PublicScan, CommandError> {
    // The path is transient input from the user-selection flow. It is never persisted or logged.
    state
        .service
        .scan_selected_root(&PathBuf::from(selected_root))
        .map_err(Into::into)
}

pub fn current_scan(state: State<'_, AppState>) -> Result<PublicScan, CommandError> {
    state.service.current_scan().map_err(Into::into)
}

/// Local UI-only command. The returned display fields are transient and are not persisted.
pub fn current_local_display_scan(
    state: State<'_, AppState>,
) -> Result<LocalDisplayScan, CommandError> {
    state
        .service
        .current_local_display_scan()
        .map_err(Into::into)
}

pub fn select_representative(
    case_id: String,
    asset_id: String,
    state: State<'_, AppState>,
) -> Result<RepresentativeSelection, CommandError> {
    state
        .service
        .select_representative(&case_id, &asset_id)
        .map_err(Into::into)
}

pub fn deselect_representative(
    case_id: String,
    asset_id: String,
    state: State<'_, AppState>,
) -> Result<RepresentativeSelection, CommandError> {
    state
        .service
        .deselect_representative(&case_id, &asset_id)
        .map_err(Into::into)
}

pub fn import_json_template(
    template_json: String,
    state: State<'_, AppState>,
) -> Result<TemplateDescriptor, CommandError> {
    state
        .service
        .import_json_template(&template_json)
        .map_err(Into::into)
}

pub fn import_label_txt(
    text: String,
    template_id: String,
    version: u32,
    state: State<'_, AppState>,
) -> Result<TemplateDescriptor, CommandError> {
    state
        .service
        .import_label_txt(&text, &template_id, version)
        .map_err(Into::into)
}

pub fn save_confirmed_region(
    request: SaveRegionRequest,
    state: State<'_, AppState>,
) -> Result<(), CommandError> {
    state
        .service
        .save_confirmed_region(request)
        .map_err(Into::into)
}

pub fn read_image_asset(
    case_id: String,
    asset_id: String,
    state: State<'_, AppState>,
) -> Result<ImageAssetData, CommandError> {
    state
        .service
        .read_image_asset(&case_id, &asset_id)
        .map_err(Into::into)
}

pub fn saved_regions_for_asset(
    case_id: String,
    asset_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<SavedRegion>, CommandError> {
    state
        .service
        .saved_regions_for_asset(&case_id, &asset_id)
        .map_err(Into::into)
}

pub fn representative_asset_ids(
    case_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<String>, CommandError> {
    state
        .service
        .representative_asset_ids(&case_id)
        .map_err(Into::into)
}

fn error_code(error: &CoreError) -> &'static str {
    match error {
        CoreError::InvalidCampusLayout => "INVALID_CAMPUS_LAYOUT",
        CoreError::SourceWorkOverlap => "SOURCE_WORK_OVERLAP",
        CoreError::NoActiveSession => "NO_ACTIVE_SESSION",
        CoreError::UnknownSessionItem => "UNKNOWN_SESSION_ITEM",
        CoreError::NotImageAsset => "NOT_IMAGE_ASSET",
        CoreError::UnsafeSourceAsset => "UNSAFE_SOURCE_ASSET",
        CoreError::RepresentativeNotSelected => "REPRESENTATIVE_NOT_SELECTED",
        CoreError::AssetNotRepresentative => "ASSET_NOT_REPRESENTATIVE",
        CoreError::TemplateNotLoaded => "TEMPLATE_NOT_LOADED",
        CoreError::InvalidTemplate => "INVALID_TEMPLATE",
        CoreError::InvalidLabels => "INVALID_LABELS",
        CoreError::InvalidPolygon => "INVALID_POLYGON",
        CoreError::TemplateSnapshotConflict => "TEMPLATE_SNAPSHOT_CONFLICT",
        CoreError::InvalidOpaqueId => "INVALID_OPAQUE_ID",
        CoreError::InvalidWorkPath => "INVALID_WORK_PATH",
        CoreError::UnsafeDerivedPath => "UNSAFE_DERIVED_PATH",
        CoreError::RepresentativeHasRegions => "REPRESENTATIVE_HAS_REGIONS",
        CoreError::CredentialStoreUnavailable => "CREDENTIAL_STORE_UNAVAILABLE",
        CoreError::InvalidSecret => "INVALID_SECRET",
        CoreError::Filesystem => "FILESYSTEM_ERROR",
        CoreError::Database => "DATABASE_ERROR",
        CoreError::StateUnavailable => "STATE_UNAVAILABLE",
        CoreError::FaultInjected => "FAULT_INJECTED",
    }
}
