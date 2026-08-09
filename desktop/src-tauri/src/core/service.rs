use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine;
use serde::{Deserialize, Serialize};

use crate::core::error::{CoreError, CoreResult};
use crate::core::ids::SharedSecretProvider;
use crate::core::scan::{
    scan_data_root, ImageAssetData, LocalDisplayScan, MediaKind, PersistedAsset, PublicAsset,
    PublicScan, ScanSession, StoredAssetKind,
};
use crate::core::store::{
    BackupFault, BackupResult, CapturedFrame, PointNorm, PreparedFixedTemplate,
    ProjectSessionState, RecoveryState, RegionDraft, SaveFault, SavedRegion, Store,
};
use crate::core::template::{parse_label_txt, LabelTemplate, LabelValue};

pub struct ProjectService {
    project_id: String,
    secret_provider: SharedSecretProvider,
    /// The work directory stays unopened until a selected source root has passed the overlap
    /// check. This prevents construction of a service with a mistakenly nested work directory
    /// from creating a database in the selected source tree before the scan is rejected.
    work_root: PathBuf,
    store: Mutex<Option<Store>>,
    active_session: Mutex<Option<ActiveSession>>,
    template_activation: Mutex<()>,
    templates: Mutex<HashMap<TemplateKey, LabelTemplate>>,
}

struct ActiveSession {
    public: PublicScan,
    mapping: ScanSession,
    source_root: PathBuf,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct TemplateDescriptor {
    pub template_id: String,
    pub version: u32,
    pub sha256: String,
    pub template: LabelTemplate,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct RepresentativeSelection {
    pub case_id: String,
    pub asset_id: String,
    pub representative_count: usize,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct SaveRegionRequest {
    pub region_id: String,
    pub asset_id: String,
    pub polygon_norm: Vec<PointNorm>,
    pub template_id: String,
    pub template_version: u32,
    pub labels: std::collections::BTreeMap<String, LabelValue>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct UpdateRegionLabelsRequest {
    pub case_id: String,
    pub asset_id: String,
    pub region_id: String,
    pub template_id: String,
    pub template_version: u32,
    pub labels: std::collections::BTreeMap<String, LabelValue>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct RefineRegionGeometryRequest {
    pub case_id: String,
    pub asset_id: String,
    pub region_id: String,
    pub expected_polygon_norm: Vec<PointNorm>,
    pub polygon_norm: Vec<PointNorm>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct ReplaceRegionRequest {
    pub case_id: String,
    pub old_region_id: String,
    pub replacement: SaveRegionRequest,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct CaptureFrameRequest {
    pub case_id: String,
    pub video_asset_id: String,
    pub capture_time_millis: i64,
    pub width: u32,
    pub height: u32,
    pub png_base64: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RestoreStatus {
    NoSession,
    Restored,
    SavedRootUnavailable,
    FixedTemplateMissing,
    FixedTemplateChecksumMismatch,
    LastMediaUnavailable,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct StartupRestore {
    pub status: RestoreStatus,
    pub scan: Option<PublicScan>,
    pub template: Option<TemplateDescriptor>,
    pub current_case_id: Option<String>,
    pub current_asset_id: Option<String>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct TemplateKey {
    template_id: String,
    version: u32,
}

impl ProjectService {
    pub fn new(
        work_root: impl Into<std::path::PathBuf>,
        project_id: impl Into<String>,
        secret_provider: SharedSecretProvider,
    ) -> CoreResult<Self> {
        Ok(Self {
            project_id: project_id.into(),
            secret_provider,
            work_root: work_root.into(),
            store: Mutex::new(None),
            active_session: Mutex::new(None),
            template_activation: Mutex::new(()),
            templates: Mutex::new(HashMap::new()),
        })
    }

    pub fn scan_selected_root(&self, selected_root: &Path) -> CoreResult<PublicScan> {
        let previous_session = Store::read_project_session(&self.work_root)?;
        let scan = self.activate_root(selected_root)?;
        let selected_resolved = fs::canonicalize(selected_root)?;
        let restored_position = previous_session.and_then(|saved| {
            if fs::canonicalize(&saved.source_root).ok().as_ref() == Some(&selected_resolved) {
                saved
                    .current_case_id
                    .zip(saved.current_asset_id)
                    .and_then(|(case_id, asset_id)| {
                        scan_contains_position(&scan, &case_id, &asset_id)
                            .then_some((case_id, asset_id))
                    })
            } else {
                None
            }
        });
        self.write_session_for_active(
            restored_position
                .as_ref()
                .map(|(case_id, _)| case_id.as_str()),
            restored_position
                .as_ref()
                .map(|(_, asset_id)| asset_id.as_str()),
            None,
        )?;
        Ok(scan)
    }

    pub fn restore_project_session(&self) -> CoreResult<StartupRestore> {
        let Some(saved) = Store::read_project_session(&self.work_root)? else {
            return Ok(StartupRestore {
                status: RestoreStatus::NoSession,
                scan: None,
                template: None,
                current_case_id: None,
                current_asset_id: None,
            });
        };
        let source_root = PathBuf::from(&saved.source_root);
        let scan = match self.activate_root(&source_root) {
            Ok(scan) => scan,
            Err(
                CoreError::Filesystem
                | CoreError::UnsafeSourceAsset
                | CoreError::InvalidCampusLayout,
            ) => {
                return Ok(StartupRestore {
                    status: RestoreStatus::SavedRootUnavailable,
                    scan: None,
                    template: None,
                    current_case_id: None,
                    current_asset_id: None,
                });
            }
            Err(error) => return Err(error),
        };
        let (template, content_sha256, previous_content_sha256) = match self.load_fixed_template() {
            Ok(Some(template)) => template,
            Ok(None) | Err(CoreError::FixedTemplateMissing) => {
                return Ok(StartupRestore {
                    status: RestoreStatus::FixedTemplateMissing,
                    scan: Some(scan),
                    template: None,
                    current_case_id: None,
                    current_asset_id: None,
                });
            }
            Err(CoreError::FixedTemplateChecksumMismatch) => {
                return Ok(StartupRestore {
                    status: RestoreStatus::FixedTemplateChecksumMismatch,
                    scan: Some(scan),
                    template: None,
                    current_case_id: None,
                    current_asset_id: None,
                });
            }
            Err(error) => return Err(error),
        };
        let session_matches_active =
            saved.template_content_sha256.as_deref() == Some(content_sha256.as_str());
        let pending_matches_active =
            saved.pending_template_content_sha256.as_deref() == Some(content_sha256.as_str());
        let prior_active_matches = saved
            .template_content_sha256
            .as_deref()
            .is_some_and(|checksum| previous_content_sha256.as_deref() == Some(checksum));
        if saved.template_content_sha256.is_some()
            && !session_matches_active
            && !pending_matches_active
            && !prior_active_matches
        {
            return Ok(StartupRestore {
                status: RestoreStatus::FixedTemplateChecksumMismatch,
                scan: Some(scan),
                template: None,
                current_case_id: None,
                current_asset_id: None,
            });
        }
        let restored_asset = saved
            .current_case_id
            .as_ref()
            .zip(saved.current_asset_id.as_ref())
            .and_then(|(case_id, asset_id)| {
                scan.campuses
                    .iter()
                    .flat_map(|campus| campus.cases.iter())
                    .find(|case| &case.case_id == case_id)
                    .and_then(|case| {
                        case.assets
                            .iter()
                            .any(|asset| &asset.asset_id == asset_id)
                            .then_some((case_id.clone(), asset_id.clone()))
                    })
            });
        let (status, current_case_id, current_asset_id) = match restored_asset {
            Some((case_id, asset_id)) => (RestoreStatus::Restored, Some(case_id), Some(asset_id)),
            None if saved.current_case_id.is_some() => {
                (RestoreStatus::LastMediaUnavailable, None, None)
            }
            None => (RestoreStatus::Restored, None, None),
        };
        if !session_matches_active || saved.pending_template_content_sha256.is_some() {
            self.write_session_for_active(
                current_case_id.as_deref(),
                current_asset_id.as_deref(),
                Some(content_sha256),
            )?;
        }
        Ok(StartupRestore {
            status,
            scan: Some(scan),
            template: Some(template),
            current_case_id,
            current_asset_id,
        })
    }

    pub fn install_fixed_toml_template(&self, input: &str) -> CoreResult<TemplateDescriptor> {
        self.ensure_active_roots_separate()?;
        let template = LabelTemplate::parse_toml_for_import(input)?;
        self.install_fixed_template(input, template)
    }

    pub fn replace_fixed_toml_template(&self, input: &str) -> CoreResult<TemplateDescriptor> {
        self.ensure_active_roots_separate()?;
        let template = LabelTemplate::parse_toml_for_import(input)?;
        let _activation = self
            .template_activation
            .lock()
            .map_err(|_| CoreError::StateUnavailable)?;
        let prepared = {
            let store = self.active_store()?;
            store
                .as_ref()
                .ok_or(CoreError::NoActiveSession)?
                .prepare_fixed_template_replacement(input, &template)?
        };
        self.activate_prepared_fixed_template(prepared)
    }

    /// TXT remains available only for an initial compatibility import. It is converted and fixed
    /// as TOML so later launches never need the original TXT source file.
    pub fn install_fixed_label_txt(&self, input: &str) -> CoreResult<TemplateDescriptor> {
        self.ensure_active_roots_separate()?;
        let template = parse_label_txt(input, "compatibility-template-v1", 1)?;
        let toml = toml::to_string_pretty(&template).map_err(|_| CoreError::InvalidTemplate)?;
        self.install_fixed_template(&toml, template)
    }

    pub fn current_fixed_template(&self) -> CoreResult<Option<TemplateDescriptor>> {
        self.ensure_active_roots_separate()?;
        self.load_fixed_template()
            .map(|template| template.map(|(descriptor, _, _)| descriptor))
    }

    pub fn save_last_position(&self, case_id: &str, asset_id: &str) -> CoreResult<()> {
        self.ensure_active_roots_separate()?;
        {
            let active = self.active()?;
            let active = active.as_ref().ok_or(CoreError::NoActiveSession)?;
            let case = active
                .public
                .campuses
                .iter()
                .flat_map(|campus| campus.cases.iter())
                .find(|case| case.case_id == case_id)
                .ok_or(CoreError::UnknownSessionItem)?;
            if !case.assets.iter().any(|asset| asset.asset_id == asset_id) {
                return Err(CoreError::UnknownSessionItem);
            }
        }
        self.write_session_for_active(Some(case_id), Some(asset_id), None)
    }

    pub fn forget_last_position(&self) -> CoreResult<()> {
        Store::forget_project_session_at(&self.work_root)
    }

    fn activate_root(&self, selected_root: &Path) -> CoreResult<PublicScan> {
        self.ensure_roots_separate(selected_root)?;
        let secret = self.secret_provider.secret_for(&self.project_id)?;
        let outcome = scan_data_root(selected_root, &secret)?;
        // Recheck after the read-only scan and immediately before the first work-directory
        // write so a resolved symlink/junction change cannot silently make the roots overlap.
        self.ensure_roots_separate(selected_root)?;
        self.open_store_after_root_check()?;
        let response = self.merge_captured_frames(outcome.public.clone())?;
        let mut active_session = self
            .active_session
            .lock()
            .map_err(|_| CoreError::StateUnavailable)?;
        *active_session = Some(ActiveSession {
            public: response.clone(),
            mapping: outcome.session,
            source_root: selected_root.to_path_buf(),
        });
        Ok(response)
    }

    pub fn current_scan(&self) -> CoreResult<PublicScan> {
        let active = self.active()?;
        Ok(active
            .as_ref()
            .ok_or(CoreError::NoActiveSession)?
            .public
            .clone())
    }

    /// This response is intentionally limited to the active local UI session. Its display fields
    /// are never passed to the persistence layer.
    pub fn current_local_display_scan(&self) -> CoreResult<LocalDisplayScan> {
        let active = self.active()?;
        let active = active.as_ref().ok_or(CoreError::NoActiveSession)?;
        active.mapping.local_display_scan(&active.public)
    }

    pub fn select_representative(
        &self,
        case_id: &str,
        asset_id: &str,
    ) -> CoreResult<RepresentativeSelection> {
        self.ensure_active_roots_separate()?;
        let (case, asset) = self.persisted_image_asset(case_id, asset_id)?;
        let store = self.active_store()?;
        let representative_count = store
            .as_ref()
            .ok_or(CoreError::NoActiveSession)?
            .select_representative(&case, &asset)?;
        Ok(RepresentativeSelection {
            case_id: case_id.to_owned(),
            asset_id: asset_id.to_owned(),
            representative_count,
        })
    }

    pub fn deselect_representative(
        &self,
        case_id: &str,
        asset_id: &str,
    ) -> CoreResult<RepresentativeSelection> {
        self.ensure_active_roots_separate()?;
        let (case, asset) = self.persisted_image_asset(case_id, asset_id)?;
        let store = self.active_store()?;
        let representative_count = store
            .as_ref()
            .ok_or(CoreError::NoActiveSession)?
            .deselect_representative(&case, &asset)?;
        Ok(RepresentativeSelection {
            case_id: case_id.to_owned(),
            asset_id: asset_id.to_owned(),
            representative_count,
        })
    }

    pub fn import_json_template(&self, input: &str) -> CoreResult<TemplateDescriptor> {
        self.register_template(LabelTemplate::parse_json_for_import(input)?)
    }

    pub fn import_label_txt(
        &self,
        input: &str,
        template_id: &str,
        version: u32,
    ) -> CoreResult<TemplateDescriptor> {
        self.register_template(parse_label_txt(input, template_id, version)?)
    }

    pub fn save_confirmed_region(&self, request: SaveRegionRequest) -> CoreResult<()> {
        self.ensure_active_roots_separate()?;
        let _activation = self
            .template_activation
            .lock()
            .map_err(|_| CoreError::StateUnavailable)?;
        let draft = self.region_draft_for_request(request)?;
        let store = self.active_store()?;
        store
            .as_ref()
            .ok_or(CoreError::NoActiveSession)?
            .save_confirmed_region(&draft)
    }

    pub fn update_region_labels(&self, request: UpdateRegionLabelsRequest) -> CoreResult<()> {
        self.ensure_active_roots_separate()?;
        self.ensure_active_region_for_asset(
            &request.case_id,
            &request.asset_id,
            &request.region_id,
        )?;
        let template = self.template_for(&request.template_id, request.template_version)?;
        let store = self.active_store()?;
        store
            .as_ref()
            .ok_or(CoreError::NoActiveSession)?
            .update_region_labels(&request.region_id, &template, &request.labels)
    }

    pub fn refine_region_geometry(&self, request: RefineRegionGeometryRequest) -> CoreResult<()> {
        self.ensure_active_roots_separate()?;
        self.persisted_image_asset(&request.case_id, &request.asset_id)?;
        let store = self.active_store()?;
        store
            .as_ref()
            .ok_or(CoreError::NoActiveSession)?
            .refine_region_geometry(
                &request.region_id,
                &request.asset_id,
                &request.expected_polygon_norm,
                &request.polygon_norm,
            )
    }

    pub fn delete_region(&self, case_id: &str, asset_id: &str, region_id: &str) -> CoreResult<()> {
        self.ensure_active_roots_separate()?;
        self.ensure_active_region_for_asset(case_id, asset_id, region_id)?;
        let store = self.active_store()?;
        store
            .as_ref()
            .ok_or(CoreError::NoActiveSession)?
            .delete_region(region_id)
    }

    pub fn replace_region(&self, request: ReplaceRegionRequest) -> CoreResult<()> {
        self.ensure_active_roots_separate()?;
        let _activation = self
            .template_activation
            .lock()
            .map_err(|_| CoreError::StateUnavailable)?;
        self.ensure_active_region_for_asset(
            &request.case_id,
            &request.replacement.asset_id,
            &request.old_region_id,
        )?;
        let draft = self.region_draft_for_request(request.replacement)?;
        let store = self.active_store()?;
        store
            .as_ref()
            .ok_or(CoreError::NoActiveSession)?
            .replace_region(&request.old_region_id, &draft)
    }

    pub fn capture_video_frame(&self, request: CaptureFrameRequest) -> CoreResult<CapturedFrame> {
        self.ensure_active_roots_separate()?;
        let case = {
            let active = self.active()?;
            let active = active.as_ref().ok_or(CoreError::NoActiveSession)?;
            active
                .mapping
                .persisted_case(&request.case_id)
                .ok_or(CoreError::UnknownSessionItem)?
        };
        {
            let active = self.active()?;
            active
                .as_ref()
                .ok_or(CoreError::NoActiveSession)?
                .mapping
                .ensure_video_asset(&request.case_id, &request.video_asset_id)?;
        }
        let bytes = BASE64_STANDARD
            .decode(&request.png_base64)
            .map_err(|_| CoreError::InvalidCapture)?;
        let store = self.active_store()?;
        let frame = store
            .as_ref()
            .ok_or(CoreError::NoActiveSession)?
            .capture_video_frame(
                &case,
                &request.video_asset_id,
                request.capture_time_millis,
                request.width,
                request.height,
                &bytes,
            )?;
        self.append_captured_frame_to_active_scan(&frame)?;
        Ok(frame)
    }

    pub fn complete_case_and_backup(&self, case_id: &str) -> CoreResult<BackupResult> {
        self.complete_case_and_backup_with_fault(case_id, BackupFault::None)
    }

    /// This test seam deliberately has no command binding. It lets the storage integration tests
    /// prove that a failed directory promotion restores the previously completed snapshot.
    pub fn complete_case_and_backup_with_fault(
        &self,
        case_id: &str,
        fault: BackupFault,
    ) -> CoreResult<BackupResult> {
        let _template_activation = self
            .template_activation
            .lock()
            .map_err(|_| CoreError::StateUnavailable)?;
        self.ensure_active_roots_separate()?;
        {
            let active = self.active()?;
            if active
                .as_ref()
                .ok_or(CoreError::NoActiveSession)?
                .mapping
                .persisted_case(case_id)
                .is_none()
            {
                return Err(CoreError::UnknownSessionItem);
            }
        }
        let store = self.active_store()?;
        store
            .as_ref()
            .ok_or(CoreError::NoActiveSession)?
            .complete_case_backup_with_fault(case_id, fault)
    }

    pub fn video_asset_bytes(&self, case_id: &str, asset_id: &str) -> CoreResult<Vec<u8>> {
        self.ensure_active_roots_separate()?;
        let active = self.active()?;
        active
            .as_ref()
            .ok_or(CoreError::NoActiveSession)?
            .mapping
            .video_asset_bytes(case_id, asset_id)
    }

    pub fn video_asset_size(&self, case_id: &str, asset_id: &str) -> CoreResult<usize> {
        self.ensure_active_roots_separate()?;
        let active = self.active()?;
        active
            .as_ref()
            .ok_or(CoreError::NoActiveSession)?
            .mapping
            .video_asset_size(case_id, asset_id)
    }

    pub fn video_asset_range_bytes(
        &self,
        case_id: &str,
        asset_id: &str,
        start: usize,
        end: usize,
    ) -> CoreResult<Vec<u8>> {
        self.ensure_active_roots_separate()?;
        let active = self.active()?;
        active
            .as_ref()
            .ok_or(CoreError::NoActiveSession)?
            .mapping
            .video_asset_range_bytes(case_id, asset_id, start, end)
    }

    pub fn record_image_dimensions(
        &self,
        case_id: &str,
        asset_id: &str,
        width: u32,
        height: u32,
    ) -> CoreResult<()> {
        self.ensure_active_roots_separate()?;
        if width == 0 || height == 0 {
            return Err(CoreError::InvalidAssetDimensions);
        }
        let (case, mut asset) = self.persisted_image_asset(case_id, asset_id)?;
        asset.width = width;
        asset.height = height;
        let store = self.active_store()?;
        store
            .as_ref()
            .ok_or(CoreError::NoActiveSession)?
            .record_asset_dimensions(&case, &asset)
    }

    pub fn read_image_asset(&self, case_id: &str, asset_id: &str) -> CoreResult<ImageAssetData> {
        self.ensure_active_roots_separate()?;
        let source_result = {
            let active = self.active()?;
            active
                .as_ref()
                .ok_or(CoreError::NoActiveSession)?
                .mapping
                .image_asset_data(case_id, asset_id)
        };
        match source_result {
            Ok(data) => Ok(data),
            Err(CoreError::UnknownSessionItem) => {
                let store = self.active_store()?;
                let bytes = store
                    .as_ref()
                    .ok_or(CoreError::NoActiveSession)?
                    .read_captured_frame(case_id, asset_id)?;
                Ok(ImageAssetData {
                    asset_id: asset_id.to_owned(),
                    mime_type: "image/png".to_owned(),
                    data_base64: BASE64_STANDARD.encode(bytes),
                })
            }
            Err(error) => Err(error),
        }
    }

    pub fn saved_regions_for_asset(
        &self,
        case_id: &str,
        asset_id: &str,
    ) -> CoreResult<Vec<SavedRegion>> {
        self.ensure_active_roots_separate()?;
        self.persisted_image_asset(case_id, asset_id)?;
        let store = self.active_store()?;
        store
            .as_ref()
            .ok_or(CoreError::NoActiveSession)?
            .saved_regions_for_asset(asset_id)
    }

    pub fn save_confirmed_region_with_fault(
        &self,
        request: SaveRegionRequest,
        fault: SaveFault,
    ) -> CoreResult<()> {
        self.ensure_active_roots_separate()?;
        let draft = self.region_draft_for_request(request)?;
        let store = self.active_store()?;
        store
            .as_ref()
            .ok_or(CoreError::NoActiveSession)?
            .save_confirmed_region_with_fault(&draft, fault)
    }

    pub fn write_recovery_state(&self, state: &RecoveryState) -> CoreResult<PathBuf> {
        self.ensure_active_roots_separate()?;
        let store = self.active_store()?;
        store
            .as_ref()
            .ok_or(CoreError::NoActiveSession)?
            .write_recovery_state(state)
    }

    pub fn write_thumbnail(&self, asset_id: &str, bytes: &[u8]) -> CoreResult<PathBuf> {
        self.ensure_active_roots_separate()?;
        let store = self.active_store()?;
        store
            .as_ref()
            .ok_or(CoreError::NoActiveSession)?
            .write_thumbnail(asset_id, bytes)
    }

    pub fn write_capture(
        &self,
        case_id: &str,
        asset_id: &str,
        bytes: &[u8],
    ) -> CoreResult<PathBuf> {
        self.ensure_active_roots_separate()?;
        let store = self.active_store()?;
        store
            .as_ref()
            .ok_or(CoreError::NoActiveSession)?
            .write_capture(case_id, asset_id, bytes)
    }

    pub fn representative_count(&self, case_id: &str) -> CoreResult<usize> {
        let store = self.active_store()?;
        store
            .as_ref()
            .ok_or(CoreError::NoActiveSession)?
            .representative_count(case_id)
    }

    pub fn representative_asset_ids(&self, case_id: &str) -> CoreResult<Vec<String>> {
        self.ensure_active_roots_separate()?;
        {
            let active = self.active()?;
            active
                .as_ref()
                .ok_or(CoreError::NoActiveSession)?
                .mapping
                .persisted_case(case_id)
                .ok_or(CoreError::UnknownSessionItem)?;
        }
        let store = self.active_store()?;
        store
            .as_ref()
            .ok_or(CoreError::NoActiveSession)?
            .representative_asset_ids(case_id)
    }

    pub fn recorded_representative_count(&self, case_id: &str) -> CoreResult<usize> {
        let store = self.active_store()?;
        store
            .as_ref()
            .ok_or(CoreError::NoActiveSession)?
            .recorded_representative_count(case_id)
    }

    pub fn region_count(&self) -> CoreResult<usize> {
        let store = self.active_store()?;
        store
            .as_ref()
            .ok_or(CoreError::NoActiveSession)?
            .region_count()
    }

    pub fn region_label_count(&self) -> CoreResult<usize> {
        let store = self.active_store()?;
        store
            .as_ref()
            .ok_or(CoreError::NoActiveSession)?
            .region_label_count()
    }

    pub fn template_snapshot_json(
        &self,
        template_id: &str,
        version: u32,
    ) -> CoreResult<Option<String>> {
        let store = self.active_store()?;
        store
            .as_ref()
            .ok_or(CoreError::NoActiveSession)?
            .template_snapshot_json(template_id, version)
    }

    pub fn work_root(&self) -> &Path {
        &self.work_root
    }

    fn install_fixed_template(
        &self,
        contents: &str,
        template: LabelTemplate,
    ) -> CoreResult<TemplateDescriptor> {
        let _activation = self
            .template_activation
            .lock()
            .map_err(|_| CoreError::StateUnavailable)?;
        let prepared = {
            let store = self.active_store()?;
            store
                .as_ref()
                .ok_or(CoreError::NoActiveSession)?
                .prepare_fixed_template_install(contents, &template)?
        };
        self.activate_prepared_fixed_template(prepared)
    }

    fn activate_prepared_fixed_template(
        &self,
        prepared: PreparedFixedTemplate,
    ) -> CoreResult<TemplateDescriptor> {
        let snapshot = prepared.fixed.template.snapshot()?;
        let descriptor = TemplateDescriptor {
            template_id: snapshot.template_id.clone(),
            version: snapshot.version,
            sha256: snapshot.sha256,
            template: prepared.fixed.template.clone(),
        };
        if !prepared.needs_activation {
            self.register_template(prepared.fixed.template)?;
            self.write_template_session(Some(prepared.fixed.content_sha256), None)?;
            return Ok(descriptor);
        }

        self.write_template_session(
            prepared.fixed.previous_content_sha256.clone(),
            Some(prepared.fixed.content_sha256.clone()),
        )?;

        let activation = self.active_store().and_then(|store| {
            store
                .as_ref()
                .ok_or(CoreError::NoActiveSession)?
                .activate_prepared_fixed_template(&prepared.fixed)
        });
        if let Err(error) = activation {
            // The active pointer did not advance. A pending session is harmless and is repaired
            // on the next restore; best-effort cleanup avoids retaining it during this process.
            let _ = self.write_template_session(prepared.fixed.previous_content_sha256, None);
            return Err(error);
        }

        // The active manifest is authoritative. A failed final session write leaves the pending
        // value, which restore_project_session recognizes and repairs, so this command never
        // reports an already-active switch as a failure.
        let _ = self.write_template_session(Some(prepared.fixed.content_sha256), None);
        Ok(descriptor)
    }

    fn load_fixed_template(
        &self,
    ) -> CoreResult<Option<(TemplateDescriptor, String, Option<String>)>> {
        let fixed = {
            let store = self.active_store()?;
            store
                .as_ref()
                .ok_or(CoreError::NoActiveSession)?
                .fixed_template()?
        };
        fixed
            .map(|fixed| {
                let content_sha256 = fixed.content_sha256;
                let previous_content_sha256 = fixed.previous_content_sha256;
                let snapshot = fixed.template.snapshot()?;
                Ok((
                    TemplateDescriptor {
                        template_id: snapshot.template_id,
                        version: snapshot.version,
                        sha256: snapshot.sha256,
                        template: fixed.template,
                    },
                    content_sha256,
                    previous_content_sha256,
                ))
            })
            .transpose()
    }

    fn write_session_for_active(
        &self,
        current_case_id: Option<&str>,
        current_asset_id: Option<&str>,
        template_content_sha256: Option<String>,
    ) -> CoreResult<()> {
        let source_root = {
            let active = self.active()?;
            active
                .as_ref()
                .ok_or(CoreError::NoActiveSession)?
                .source_root
                .clone()
        };
        let previous = Store::read_project_session(&self.work_root)?;
        let state = ProjectSessionState {
            source_root: source_root
                .to_str()
                .ok_or(CoreError::StateUnavailable)?
                .to_owned(),
            current_case_id: current_case_id.map(str::to_owned),
            current_asset_id: current_asset_id.map(str::to_owned),
            template_content_sha256: template_content_sha256
                .or_else(|| previous.and_then(|state| state.template_content_sha256)),
            pending_template_content_sha256: None,
        };
        let store = self.active_store()?;
        store
            .as_ref()
            .ok_or(CoreError::NoActiveSession)?
            .write_project_session(&state)
    }

    fn write_template_session(
        &self,
        template_content_sha256: Option<String>,
        pending_template_content_sha256: Option<String>,
    ) -> CoreResult<()> {
        let source_root = {
            let active = self.active()?;
            active
                .as_ref()
                .ok_or(CoreError::NoActiveSession)?
                .source_root
                .clone()
        };
        let source_root = source_root
            .to_str()
            .ok_or(CoreError::StateUnavailable)?
            .to_owned();
        let previous = Store::read_project_session(&self.work_root)?;
        let (current_case_id, current_asset_id) = previous
            .filter(|state| state.source_root == source_root)
            .map(|state| (state.current_case_id, state.current_asset_id))
            .unwrap_or((None, None));
        let state = ProjectSessionState {
            source_root,
            current_case_id,
            current_asset_id,
            template_content_sha256,
            pending_template_content_sha256,
        };
        let store = self.active_store()?;
        store
            .as_ref()
            .ok_or(CoreError::NoActiveSession)?
            .write_project_session(&state)
    }

    fn merge_captured_frames(&self, mut scan: PublicScan) -> CoreResult<PublicScan> {
        let store = self.active_store()?;
        let store = store.as_ref().ok_or(CoreError::NoActiveSession)?;
        for campus in &mut scan.campuses {
            for case in &mut campus.cases {
                for capture in store.captured_frames_for_case(&case.case_id)? {
                    if case
                        .assets
                        .iter()
                        .any(|asset| asset.asset_id == capture.asset_id)
                    {
                        return Err(CoreError::StateUnavailable);
                    }
                    case.assets.push(PublicAsset {
                        asset_id: capture.asset_id,
                        media_kind: MediaKind::Image,
                        size_bytes: capture.size_bytes,
                    });
                }
            }
        }
        Ok(scan)
    }

    fn append_captured_frame_to_active_scan(&self, frame: &CapturedFrame) -> CoreResult<()> {
        let mut active = self.active()?;
        let active = active.as_mut().ok_or(CoreError::NoActiveSession)?;
        let case = active
            .public
            .campuses
            .iter_mut()
            .flat_map(|campus| campus.cases.iter_mut())
            .find(|case| case.case_id == frame.case_id)
            .ok_or(CoreError::UnknownSessionItem)?;
        if case
            .assets
            .iter()
            .any(|asset| asset.asset_id == frame.asset_id)
        {
            return Err(CoreError::StateUnavailable);
        }
        case.assets.push(PublicAsset {
            asset_id: frame.asset_id.clone(),
            media_kind: MediaKind::Image,
            size_bytes: frame.size_bytes,
        });
        Ok(())
    }

    fn persisted_image_asset(
        &self,
        case_id: &str,
        asset_id: &str,
    ) -> CoreResult<(crate::core::scan::PersistedCase, PersistedAsset)> {
        let source = {
            let active = self.active()?;
            let active = active.as_ref().ok_or(CoreError::NoActiveSession)?;
            let case = active
                .mapping
                .persisted_case(case_id)
                .ok_or(CoreError::UnknownSessionItem)?;
            match active.mapping.persisted_image_asset(case_id, asset_id) {
                Ok(asset) => return Ok((case, asset)),
                Err(CoreError::UnknownSessionItem) => case,
                Err(error) => return Err(error),
            }
        };
        let store = self.active_store()?;
        let frame = store
            .as_ref()
            .ok_or(CoreError::NoActiveSession)?
            .captured_frames_for_case(case_id)?
            .into_iter()
            .find(|frame| frame.asset_id == asset_id)
            .ok_or(CoreError::UnknownSessionItem)?;
        Ok((
            source,
            PersistedAsset {
                asset_id: frame.asset_id,
                case_id: frame.case_id,
                kind: StoredAssetKind::CapturedFrame,
                width: frame.width,
                height: frame.height,
                source_fingerprint: format!(
                    "CAPTURE-{}-{}",
                    frame.parent_video_asset_id, frame.capture_time_millis
                ),
            },
        ))
    }

    fn ensure_active_region_for_asset(
        &self,
        case_id: &str,
        asset_id: &str,
        region_id: &str,
    ) -> CoreResult<()> {
        self.persisted_image_asset(case_id, asset_id)?;
        let store = self.active_store()?;
        let exists = store
            .as_ref()
            .ok_or(CoreError::NoActiveSession)?
            .saved_regions_for_asset(asset_id)?
            .into_iter()
            .any(|region| region.region_id == region_id);
        if exists {
            Ok(())
        } else {
            Err(CoreError::RegionNotFound)
        }
    }

    fn template_for(&self, template_id: &str, template_version: u32) -> CoreResult<LabelTemplate> {
        let snapshot = {
            let store = self.active_store()?;
            store
                .as_ref()
                .ok_or(CoreError::NoActiveSession)?
                .load_template_snapshot(template_id, template_version)?
        };
        if let Some(template) = snapshot {
            return Ok(template);
        }

        let key = TemplateKey {
            template_id: template_id.to_owned(),
            version: template_version,
        };
        if let Some(template) = self
            .templates
            .lock()
            .map_err(|_| CoreError::StateUnavailable)?
            .get(&key)
            .cloned()
        {
            return Ok(template);
        }

        Err(CoreError::TemplateNotLoaded)
    }

    fn region_draft_for_request(&self, request: SaveRegionRequest) -> CoreResult<RegionDraft> {
        let active = self
            .current_fixed_template()?
            .ok_or(CoreError::TemplateNotActive)?;
        if active.template_id != request.template_id || active.version != request.template_version {
            return Err(CoreError::TemplateNotActive);
        }
        if active.template.snapshot()?.sha256 != active.sha256 {
            return Err(CoreError::TemplateNotActive);
        }
        // Historical damaged templates remain readable so the user can replace them, but they
        // must never create a new active region or a replacement region.
        active.template.validate_import_text()?;
        Ok(RegionDraft {
            region_id: request.region_id,
            asset_id: request.asset_id,
            polygon_norm: request.polygon_norm,
            template: active.template,
            labels: request.labels,
        })
    }

    fn active(&self) -> CoreResult<MutexGuard<'_, Option<ActiveSession>>> {
        let active = self
            .active_session
            .lock()
            .map_err(|_| CoreError::StateUnavailable)?;
        if active.is_none() {
            return Err(CoreError::NoActiveSession);
        }
        Ok(active)
    }

    fn ensure_active_roots_separate(&self) -> CoreResult<()> {
        let source_root = {
            let active = self.active()?;
            active
                .as_ref()
                .ok_or(CoreError::NoActiveSession)?
                .source_root
                .clone()
        };
        self.ensure_roots_separate(&source_root)
    }

    fn ensure_roots_separate(&self, source_root: &Path) -> CoreResult<()> {
        let resolved_source = fs::canonicalize(source_root)?;
        // This preflight is intentionally read-only. It rejects an existing app-owned work-root
        // component that is a symlink/junction before overlap resolution can follow it.
        Store::validate_work_root_for_overlap(&self.work_root)?;
        let resolved_work = Store::resolve_work_root_for_overlap(&self.work_root)?;
        if roots_overlap(&resolved_source, &resolved_work) {
            return Err(CoreError::SourceWorkOverlap);
        }
        Ok(())
    }

    fn open_store_after_root_check(&self) -> CoreResult<()> {
        let mut store = self.store.lock().map_err(|_| CoreError::StateUnavailable)?;
        if store.is_none() {
            *store = Some(Store::open(&self.work_root)?);
        }
        Ok(())
    }

    fn active_store(&self) -> CoreResult<MutexGuard<'_, Option<Store>>> {
        self.store.lock().map_err(|_| CoreError::StateUnavailable)
    }

    fn register_template(&self, template: LabelTemplate) -> CoreResult<TemplateDescriptor> {
        let snapshot = template.snapshot()?;
        let descriptor = TemplateDescriptor {
            template_id: snapshot.template_id.clone(),
            version: snapshot.version,
            sha256: snapshot.sha256,
            template: template.clone(),
        };
        let mut templates = self
            .templates
            .lock()
            .map_err(|_| CoreError::StateUnavailable)?;
        let key = TemplateKey {
            template_id: template.template_id.clone(),
            version: template.version,
        };
        if let Some(existing) = templates.get(&key) {
            if existing.snapshot()?.sha256 != descriptor.sha256 {
                return Err(CoreError::TemplateSnapshotConflict);
            }
            return Ok(descriptor);
        }
        templates.insert(key, template);
        Ok(descriptor)
    }
}

fn scan_contains_position(scan: &PublicScan, case_id: &str, asset_id: &str) -> bool {
    scan.campuses
        .iter()
        .flat_map(|campus| campus.cases.iter())
        .find(|case| case.case_id == case_id)
        .is_some_and(|case| case.assets.iter().any(|asset| asset.asset_id == asset_id))
}

fn roots_overlap(left: &Path, right: &Path) -> bool {
    left == right || left.starts_with(right) || right.starts_with(left)
}
