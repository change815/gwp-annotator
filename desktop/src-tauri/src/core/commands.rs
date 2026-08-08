use std::path::PathBuf;

use serde::Serialize;
use tauri::State;

use crate::core::error::CoreError;
use crate::core::scan::{ImageAssetData, LocalDisplayScan, PublicScan};
use crate::core::service::{
    CaptureFrameRequest, ProjectService, RefineRegionGeometryRequest, ReplaceRegionRequest,
    RepresentativeSelection, SaveRegionRequest, StartupRestore, TemplateDescriptor,
    UpdateRegionLabelsRequest,
};
use crate::core::store::{BackupResult, CapturedFrame, SavedRegion};

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

pub fn restore_project_session(state: State<'_, AppState>) -> Result<StartupRestore, CommandError> {
    state.service.restore_project_session().map_err(Into::into)
}

pub fn forget_last_position(state: State<'_, AppState>) -> Result<(), CommandError> {
    state.service.forget_last_position().map_err(Into::into)
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

pub fn install_fixed_toml_template(
    template_toml: String,
    state: State<'_, AppState>,
) -> Result<TemplateDescriptor, CommandError> {
    state
        .service
        .install_fixed_toml_template(&template_toml)
        .map_err(Into::into)
}

pub fn replace_fixed_toml_template(
    template_toml: String,
    state: State<'_, AppState>,
) -> Result<TemplateDescriptor, CommandError> {
    state
        .service
        .replace_fixed_toml_template(&template_toml)
        .map_err(Into::into)
}

pub fn install_fixed_label_txt(
    text: String,
    state: State<'_, AppState>,
) -> Result<TemplateDescriptor, CommandError> {
    state
        .service
        .install_fixed_label_txt(&text)
        .map_err(Into::into)
}

pub fn current_fixed_template(
    state: State<'_, AppState>,
) -> Result<Option<TemplateDescriptor>, CommandError> {
    state.service.current_fixed_template().map_err(Into::into)
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

pub fn update_region_labels(
    request: UpdateRegionLabelsRequest,
    state: State<'_, AppState>,
) -> Result<(), CommandError> {
    state
        .service
        .update_region_labels(request)
        .map_err(Into::into)
}

pub fn refine_region_geometry(
    request: RefineRegionGeometryRequest,
    state: State<'_, AppState>,
) -> Result<(), CommandError> {
    state
        .service
        .refine_region_geometry(request)
        .map_err(Into::into)
}

pub fn delete_region(
    case_id: String,
    asset_id: String,
    region_id: String,
    state: State<'_, AppState>,
) -> Result<(), CommandError> {
    state
        .service
        .delete_region(&case_id, &asset_id, &region_id)
        .map_err(Into::into)
}

pub fn replace_region(
    request: ReplaceRegionRequest,
    state: State<'_, AppState>,
) -> Result<(), CommandError> {
    state.service.replace_region(request).map_err(Into::into)
}

pub fn capture_video_frame(
    request: CaptureFrameRequest,
    state: State<'_, AppState>,
) -> Result<CapturedFrame, CommandError> {
    state
        .service
        .capture_video_frame(request)
        .map_err(Into::into)
}

pub fn complete_case_and_backup(
    case_id: String,
    state: State<'_, AppState>,
) -> Result<BackupResult, CommandError> {
    state
        .service
        .complete_case_and_backup(&case_id)
        .map_err(Into::into)
}

pub fn save_last_position(
    case_id: String,
    asset_id: String,
    state: State<'_, AppState>,
) -> Result<(), CommandError> {
    state
        .service
        .save_last_position(&case_id, &asset_id)
        .map_err(Into::into)
}

pub fn record_image_dimensions(
    case_id: String,
    asset_id: String,
    width: u32,
    height: u32,
    state: State<'_, AppState>,
) -> Result<(), CommandError> {
    state
        .service
        .record_image_dimensions(&case_id, &asset_id, width, height)
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
        CoreError::FixedTemplateMissing => "FIXED_TEMPLATE_MISSING",
        CoreError::FixedTemplateChecksumMismatch => "FIXED_TEMPLATE_CHECKSUM_MISMATCH",
        CoreError::FixedTemplateAlreadyInstalled => "FIXED_TEMPLATE_ALREADY_INSTALLED",
        CoreError::TemplateReplacementIdentityConflict => "TEMPLATE_REPLACEMENT_IDENTITY_CONFLICT",
        CoreError::TemplateNotActive => "TEMPLATE_NOT_ACTIVE",
        CoreError::InvalidLabels => "INVALID_LABELS",
        CoreError::InvalidPolygon => "INVALID_POLYGON",
        CoreError::TemplateSnapshotConflict => "TEMPLATE_SNAPSHOT_CONFLICT",
        CoreError::InvalidOpaqueId => "INVALID_OPAQUE_ID",
        CoreError::InvalidWorkPath => "INVALID_WORK_PATH",
        CoreError::WorkRootUnavailable => "WORK_ROOT_UNAVAILABLE",
        CoreError::UnsafeDerivedPath => "UNSAFE_DERIVED_PATH",
        CoreError::RepresentativeHasRegions => "REPRESENTATIVE_HAS_REGIONS",
        CoreError::RegionNotFound => "REGION_NOT_FOUND",
        CoreError::RegionNotActive => "REGION_NOT_ACTIVE",
        CoreError::RegionGeometryConflict => "REGION_GEOMETRY_CONFLICT",
        CoreError::NotVideoAsset => "NOT_VIDEO_ASSET",
        CoreError::InvalidCapture => "INVALID_CAPTURE",
        CoreError::InvalidAssetDimensions => "INVALID_ASSET_DIMENSIONS",
        CoreError::SavedRootUnavailable => "SAVED_ROOT_UNAVAILABLE",
        CoreError::CredentialStoreUnavailable => "CREDENTIAL_STORE_UNAVAILABLE",
        CoreError::InvalidSecret => "INVALID_SECRET",
        CoreError::Filesystem => "FILESYSTEM_ERROR",
        CoreError::Database => "DATABASE_ERROR",
        CoreError::DatabaseSchemaUnsupported => "DATABASE_SCHEMA_UNSUPPORTED",
        CoreError::StateUnavailable => "STATE_UNAVAILABLE",
        CoreError::FaultInjected => "FAULT_INJECTED",
    }
}
