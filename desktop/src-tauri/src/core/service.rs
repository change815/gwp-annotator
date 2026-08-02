use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use serde::{Deserialize, Serialize};

use crate::core::error::{CoreError, CoreResult};
use crate::core::ids::SharedSecretProvider;
use crate::core::scan::{scan_data_root, LocalDisplayScan, PublicScan, ScanSession};
use crate::core::store::{PointNorm, RecoveryState, RegionDraft, SaveFault, Store};
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
            templates: Mutex::new(HashMap::new()),
        })
    }

    pub fn scan_selected_root(&self, selected_root: &Path) -> CoreResult<PublicScan> {
        self.ensure_roots_separate(selected_root)?;
        let secret = self.secret_provider.secret_for(&self.project_id)?;
        let outcome = scan_data_root(selected_root, &secret)?;
        // Recheck after the read-only scan and immediately before the first work-directory
        // write so a resolved symlink/junction change cannot silently make the roots overlap.
        self.ensure_roots_separate(selected_root)?;
        self.open_store_after_root_check()?;
        let response = outcome.public.clone();
        let mut active_session = self
            .active_session
            .lock()
            .map_err(|_| CoreError::StateUnavailable)?;
        *active_session = Some(ActiveSession {
            public: outcome.public,
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
        let (case, asset) = {
            let active = self.active()?;
            let active = active.as_ref().ok_or(CoreError::NoActiveSession)?;
            let case = active
                .mapping
                .persisted_case(case_id)
                .ok_or(CoreError::UnknownSessionItem)?;
            let asset = active.mapping.persisted_image_asset(case_id, asset_id)?;
            (case, asset)
        };
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
        let (case, asset) = {
            let active = self.active()?;
            let active = active.as_ref().ok_or(CoreError::NoActiveSession)?;
            let case = active
                .mapping
                .persisted_case(case_id)
                .ok_or(CoreError::UnknownSessionItem)?;
            let asset = active.mapping.persisted_image_asset(case_id, asset_id)?;
            (case, asset)
        };
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
        self.register_template(LabelTemplate::parse_json(input)?)
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
        let draft = self.region_draft_for_request(request)?;
        let store = self.active_store()?;
        store
            .as_ref()
            .ok_or(CoreError::NoActiveSession)?
            .save_confirmed_region(&draft)
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

    fn region_draft_for_request(&self, request: SaveRegionRequest) -> CoreResult<RegionDraft> {
        let key = TemplateKey {
            template_id: request.template_id.clone(),
            version: request.template_version,
        };
        let template = self
            .templates
            .lock()
            .map_err(|_| CoreError::StateUnavailable)?
            .get(&key)
            .cloned()
            .ok_or(CoreError::TemplateNotLoaded)?;
        Ok(RegionDraft {
            region_id: request.region_id,
            asset_id: request.asset_id,
            polygon_norm: request.polygon_norm,
            template,
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
        let resolved_work = resolve_path_for_overlap(&self.work_root)?;
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
        };
        self.templates
            .lock()
            .map_err(|_| CoreError::StateUnavailable)?
            .insert(
                TemplateKey {
                    template_id: template.template_id.clone(),
                    version: template.version,
                },
                template,
            );
        Ok(descriptor)
    }
}

fn roots_overlap(left: &Path, right: &Path) -> bool {
    left == right || left.starts_with(right) || right.starts_with(left)
}

/// Resolve the existing portion of a path without creating it, then append the nonexistent tail.
/// `canonicalize` follows both Unix symlinks and Windows junctions for the existing portion.
fn resolve_path_for_overlap(path: &Path) -> CoreResult<PathBuf> {
    let mut candidate = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut missing_components = Vec::new();

    loop {
        match fs::canonicalize(&candidate) {
            Ok(mut resolved) => {
                for component in missing_components.iter().rev() {
                    resolved.push(component);
                }
                return Ok(resolved);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let component = candidate
                    .file_name()
                    .ok_or(CoreError::Filesystem)?
                    .to_os_string();
                missing_components.push(component);
                candidate = candidate
                    .parent()
                    .ok_or(CoreError::Filesystem)?
                    .to_path_buf();
            }
            Err(_) => return Err(CoreError::Filesystem),
        }
    }
}
