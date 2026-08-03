use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use data_encoding::{BASE32_NOPAD, HEXLOWER};
use rand::rngs::OsRng;
use rand::RngCore;
use rusqlite::{
    backup::Backup, params, Connection, OpenFlags, OptionalExtension, Transaction,
    TransactionBehavior,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::core::error::{CoreError, CoreResult};
use crate::core::ids::is_opaque_id;
use crate::core::scan::{PersistedAsset, PersistedCase};
use crate::core::template::{LabelTemplate, LabelValue};

const DATABASE_FILE: &str = "annotations.sqlite3";
const DATABASE_JOURNAL_FILE: &str = "annotations.sqlite3-journal";
const DATABASE_WAL_FILE: &str = "annotations.sqlite3-wal";
const DATABASE_SHM_FILE: &str = "annotations.sqlite3-shm";
const FIXED_TEMPLATE_FILE: &str = "templates/label-template-v1.toml";
const FIXED_TEMPLATE_MANIFEST_FILE: &str = "templates/label-template-v1.manifest.json";
const PROJECT_SESSION_FILE: &str = "session/project-session.json";
const BACKUP_FORMAT: &str = "gwp-case-backup";
const SCHEMA_VERSION: i64 = 3;
const INTERSECTION_EPSILON: f64 = 1e-12;
const PNG_SIGNATURE: [u8; 8] = [137, 80, 78, 71, 13, 10, 26, 10];
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub(crate) struct Store {
    /// Canonical physical work root. All derived-file parents must resolve beneath this path.
    work_root: PathBuf,
    database_paths: DatabasePaths,
    connection: Mutex<Connection>,
}

/// SQLite uses three sibling paths for this database. Keeping the complete set together makes
/// it harder to add a write path without applying the derived-data safety checks first.
#[derive(Clone, Debug)]
struct DatabasePaths {
    main: PathBuf,
    journal: PathBuf,
    wal: PathBuf,
    shm: PathBuf,
}

impl DatabasePaths {
    fn in_work_root(work_root: &Path) -> Self {
        Self {
            main: work_root.join(DATABASE_FILE),
            journal: work_root.join(DATABASE_JOURNAL_FILE),
            wal: work_root.join(DATABASE_WAL_FILE),
            shm: work_root.join(DATABASE_SHM_FILE),
        }
    }

    fn all(&self) -> [&Path; 4] {
        [&self.main, &self.journal, &self.wal, &self.shm]
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct PointNorm {
    pub x: f64,
    pub y: f64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct RegionDraft {
    pub region_id: String,
    pub asset_id: String,
    pub polygon_norm: Vec<PointNorm>,
    pub template: LabelTemplate,
    pub labels: BTreeMap<String, LabelValue>,
}

/// Saved drawing geometry contains only opaque IDs and normalized points, never source paths or
/// display labels. It is safe to expose to the active local UI session.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct SavedRegion {
    pub region_id: String,
    pub asset_id: String,
    pub polygon_norm: Vec<PointNorm>,
    pub template_id: String,
    pub template_version: u32,
    pub labels: BTreeMap<String, LabelValue>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaveFault {
    None,
    AfterRegionInsert,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct RecoveryState {
    pub current_case_id: String,
    pub current_asset_id: String,
    pub zoom: f64,
    pub pan_x: f64,
    pub pan_y: f64,
}

/// This file is deliberately outside SQLite and any future training export. The root path is
/// needed only to restore the local browsing session and is never copied into annotation data.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ProjectSessionState {
    pub source_root: String,
    pub current_case_id: Option<String>,
    pub current_asset_id: Option<String>,
    pub template_content_sha256: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct FixedTemplate {
    pub content_sha256: String,
    pub template_sha256: String,
    pub template: LabelTemplate,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
struct FixedTemplateManifest {
    format: String,
    content_sha256: String,
    template_sha256: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct CapturedFrame {
    pub asset_id: String,
    pub case_id: String,
    pub parent_video_asset_id: String,
    pub capture_time_millis: i64,
    pub width: u32,
    pub height: u32,
    pub size_bytes: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct BackupResult {
    pub case_id: String,
    pub completed_at: i64,
    pub capture_count: usize,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
struct BackupManifest {
    format: String,
    version: u32,
    case_id: String,
    completed_at: i64,
    template_content_sha256: String,
    capture_asset_ids: Vec<String>,
}

impl Store {
    /// Reads the local-only session file without creating a database. This is used before the
    /// application can reopen a selected source root on startup.
    pub(crate) fn read_project_session(
        work_root: &Path,
    ) -> CoreResult<Option<ProjectSessionState>> {
        let Some(bytes) = read_optional_work_file(work_root, Path::new(PROJECT_SESSION_FILE))?
        else {
            return Ok(None);
        };
        let state: ProjectSessionState =
            serde_json::from_slice(&bytes).map_err(|_| CoreError::StateUnavailable)?;
        validate_project_session(&state)?;
        Ok(Some(state))
    }

    pub(crate) fn forget_project_session_at(work_root: &Path) -> CoreResult<()> {
        remove_optional_work_file(work_root, Path::new(PROJECT_SESSION_FILE))
    }

    pub(crate) fn open(work_root: impl Into<PathBuf>) -> CoreResult<Self> {
        let requested_work_root = work_root.into();
        fs::create_dir_all(&requested_work_root)?;
        let work_root = fs::canonicalize(&requested_work_root)?;
        let work_root_metadata = fs::symlink_metadata(&work_root)?;
        if is_link_or_reparse(&work_root_metadata) || !work_root_metadata.file_type().is_dir() {
            return Err(CoreError::UnsafeDerivedPath);
        }
        let database_paths = DatabasePaths::in_work_root(&work_root);
        prepare_database_artifacts(&database_paths)?;
        let mut connection = open_database_no_follow(&database_paths.main)?;
        configure_and_migrate(&mut connection)?;
        // SQLite may remove an unused rollback journal while switching to WAL. Re-reserve any
        // missing sibling with create_new and validate every final component again afterwards.
        prepare_database_artifacts(&database_paths)?;
        Ok(Self {
            work_root,
            database_paths,
            connection: Mutex::new(connection),
        })
    }

    pub(crate) fn write_project_session(&self, state: &ProjectSessionState) -> CoreResult<()> {
        validate_project_session(state)?;
        let bytes = serde_json::to_vec(state).map_err(|_| CoreError::StateUnavailable)?;
        self.atomic_write_relative(Path::new(PROJECT_SESSION_FILE), &bytes)
            .map(|_| ())
    }

    pub(crate) fn install_fixed_template(
        &self,
        contents: &str,
        template: &LabelTemplate,
    ) -> CoreResult<FixedTemplate> {
        let snapshot = template.snapshot()?;
        let fixed = FixedTemplate {
            content_sha256: sha256_hex(contents.as_bytes()),
            template_sha256: snapshot.sha256,
            template: template.clone(),
        };
        let manifest = FixedTemplateManifest {
            format: "gwp-fixed-template".to_owned(),
            content_sha256: fixed.content_sha256.clone(),
            template_sha256: fixed.template_sha256.clone(),
        };

        let existing_manifest = self.read_fixed_template_manifest()?;
        let existing_contents = self.read_relative_optional(Path::new(FIXED_TEMPLATE_FILE))?;
        match (existing_manifest, existing_contents) {
            (None, None) => {
                self.atomic_write_relative(Path::new(FIXED_TEMPLATE_FILE), contents.as_bytes())?;
                let manifest_bytes =
                    serde_json::to_vec(&manifest).map_err(|_| CoreError::StateUnavailable)?;
                self.atomic_write_relative(
                    Path::new(FIXED_TEMPLATE_MANIFEST_FILE),
                    &manifest_bytes,
                )?;
                Ok(fixed)
            }
            (Some(existing), Some(existing_bytes)) => {
                let existing_template = fixed_template_from_bytes(&existing, &existing_bytes)?;
                if existing_template.content_sha256 == fixed.content_sha256
                    && existing_template.template_sha256 == fixed.template_sha256
                {
                    Ok(existing_template)
                } else {
                    Err(CoreError::FixedTemplateAlreadyInstalled)
                }
            }
            (Some(existing), None) => {
                // A lost fixed copy may be restored only from identical source content. This
                // prevents the recovery path from becoming an unreviewed template switch.
                if existing.content_sha256 != fixed.content_sha256
                    || existing.template_sha256 != fixed.template_sha256
                {
                    return Err(CoreError::FixedTemplateChecksumMismatch);
                }
                self.atomic_write_relative(Path::new(FIXED_TEMPLATE_FILE), contents.as_bytes())?;
                Ok(fixed)
            }
            (None, Some(_)) => Err(CoreError::FixedTemplateMissing),
        }
    }

    pub(crate) fn fixed_template(&self) -> CoreResult<Option<FixedTemplate>> {
        let manifest = self.read_fixed_template_manifest()?;
        let contents = self.read_relative_optional(Path::new(FIXED_TEMPLATE_FILE))?;
        match (manifest, contents) {
            (None, None) => Ok(None),
            (Some(manifest), Some(contents)) => {
                fixed_template_from_bytes(&manifest, &contents).map(Some)
            }
            _ => Err(CoreError::FixedTemplateMissing),
        }
    }

    pub(crate) fn select_representative(
        &self,
        case: &PersistedCase,
        asset: &PersistedAsset,
    ) -> CoreResult<usize> {
        if case.case_id != asset.case_id {
            return Err(CoreError::UnknownSessionItem);
        }
        self.verify_database_artifacts_safe()?;
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| CoreError::StateUnavailable)?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        upsert_case_and_asset(&transaction, case, asset)?;

        let existing: Option<i64> = transaction
            .query_row(
                "SELECT 1 FROM representative_assets WHERE case_id = ?1 AND asset_id = ?2",
                params![case.case_id, asset.asset_id],
                |row| row.get(0),
            )
            .optional()?;
        if existing.is_none() {
            transaction.execute(
                "INSERT INTO representative_assets (case_id, asset_id, selected_at) VALUES (?1, ?2, ?3)",
                params![case.case_id, asset.asset_id, now_millis()],
            )?;
        }

        let selected_count = update_representative_count(&transaction, &case.case_id)?;
        transaction.commit()?;
        Ok(selected_count)
    }

    /// Persists dimensions decoded by the local WebView for an active image asset. Source paths
    /// remain in the in-memory scan only; SQLite receives only opaque IDs and pixel dimensions.
    pub(crate) fn record_asset_dimensions(
        &self,
        case: &PersistedCase,
        asset: &PersistedAsset,
    ) -> CoreResult<()> {
        if case.case_id != asset.case_id || asset.width == 0 || asset.height == 0 {
            return Err(CoreError::InvalidAssetDimensions);
        }
        self.verify_database_artifacts_safe()?;
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| CoreError::StateUnavailable)?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        upsert_case_and_asset(&transaction, case, asset)?;
        transaction.commit()?;
        Ok(())
    }

    pub(crate) fn deselect_representative(
        &self,
        case: &PersistedCase,
        asset: &PersistedAsset,
    ) -> CoreResult<usize> {
        if case.case_id != asset.case_id {
            return Err(CoreError::UnknownSessionItem);
        }
        self.verify_database_artifacts_safe()?;
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| CoreError::StateUnavailable)?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let has_saved_regions: Option<i64> = transaction
            .query_row(
                "SELECT 1 FROM regions WHERE asset_id = ?1 AND status = 'active' LIMIT 1",
                params![asset.asset_id],
                |row| row.get(0),
            )
            .optional()?;
        if has_saved_regions.is_some() {
            return Err(CoreError::RepresentativeHasRegions);
        }
        let deleted = transaction.execute(
            "DELETE FROM representative_assets WHERE case_id = ?1 AND asset_id = ?2",
            params![case.case_id, asset.asset_id],
        )?;
        if deleted != 1 {
            return Err(CoreError::RepresentativeNotSelected);
        }
        let selected_count = update_representative_count(&transaction, &case.case_id)?;
        transaction.commit()?;
        Ok(selected_count)
    }

    pub(crate) fn save_confirmed_region(&self, draft: &RegionDraft) -> CoreResult<()> {
        self.save_confirmed_region_with_fault(draft, SaveFault::None)
    }

    pub(crate) fn save_confirmed_region_with_fault(
        &self,
        draft: &RegionDraft,
        fault: SaveFault,
    ) -> CoreResult<()> {
        validate_region_draft(draft)?;
        let template_snapshot = draft.template.snapshot()?;
        let polygon_json =
            serde_json::to_string(&draft.polygon_norm).map_err(|_| CoreError::InvalidPolygon)?;

        self.verify_database_artifacts_safe()?;
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| CoreError::StateUnavailable)?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let is_representative: Option<i64> = transaction
            .query_row(
                "SELECT 1 FROM representative_assets WHERE asset_id = ?1",
                params![draft.asset_id],
                |row| row.get(0),
            )
            .optional()?;
        if is_representative.is_none() {
            return Err(CoreError::AssetNotRepresentative);
        }

        match transaction
            .query_row(
                "SELECT sha256 FROM templates WHERE template_id = ?1 AND version = ?2",
                params![template_snapshot.template_id, template_snapshot.version],
                |row| row.get::<_, String>(0),
            )
            .optional()?
        {
            Some(existing_sha) if existing_sha != template_snapshot.sha256 => {
                return Err(CoreError::TemplateSnapshotConflict);
            }
            Some(_) => {}
            None => {
                transaction.execute(
                    "INSERT INTO templates (template_id, version, sha256, canonical_json) VALUES (?1, ?2, ?3, ?4)",
                    params![
                        template_snapshot.template_id,
                        template_snapshot.version,
                        template_snapshot.sha256,
                        template_snapshot.canonical_json,
                    ],
                )?;
            }
        }

        let created_at = now_millis();
        transaction.execute(
            "INSERT INTO regions (region_id, asset_id, polygon_norm_json, template_id, template_version, created_at, status, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'active', ?6)",
            params![
                draft.region_id,
                draft.asset_id,
                polygon_json,
                draft.template.template_id,
                draft.template.version,
                created_at,
            ],
        )?;

        if fault == SaveFault::AfterRegionInsert {
            return Err(CoreError::FaultInjected);
        }

        for (field_id, value) in &draft.labels {
            let value_json = serde_json::to_string(value).map_err(|_| CoreError::InvalidLabels)?;
            transaction.execute(
                "INSERT INTO region_label_values (region_id, field_id, value_json) VALUES (?1, ?2, ?3)",
                params![draft.region_id, field_id, value_json],
            )?;
        }
        insert_region_operation(
            &transaction,
            "created",
            &draft.region_id,
            None,
            None,
            None,
            Some(
                &serde_json::to_string(&draft.polygon_norm)
                    .map_err(|_| CoreError::InvalidPolygon)?,
            ),
            Some(&labels_json(&draft.labels)?),
            created_at,
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub(crate) fn update_region_labels(
        &self,
        region_id: &str,
        template: &LabelTemplate,
        labels: &BTreeMap<String, LabelValue>,
    ) -> CoreResult<()> {
        if !is_opaque_id(region_id, "REGION-") {
            return Err(CoreError::InvalidOpaqueId);
        }
        template.validate_label_values(labels)?;
        let snapshot = template.snapshot()?;
        self.verify_database_artifacts_safe()?;
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| CoreError::StateUnavailable)?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (polygon_json, template_id, template_version, status) = transaction
            .query_row(
                "SELECT polygon_norm_json, template_id, template_version, status FROM regions WHERE region_id = ?1",
                params![region_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, u32>(2)?, row.get::<_, String>(3)?)),
            )
            .optional()?
            .ok_or(CoreError::RegionNotFound)?;
        if status != "active" {
            return Err(CoreError::RegionNotActive);
        }
        if template_id != snapshot.template_id || template_version != snapshot.version {
            return Err(CoreError::TemplateSnapshotConflict);
        }
        ensure_template_snapshot(&transaction, &snapshot)?;
        let before_labels = read_region_labels(&transaction, region_id)?;
        transaction.execute(
            "DELETE FROM region_label_values WHERE region_id = ?1",
            params![region_id],
        )?;
        insert_region_labels(&transaction, region_id, labels)?;
        let updated_at = now_millis();
        transaction.execute(
            "UPDATE regions SET updated_at = ?1 WHERE region_id = ?2",
            params![updated_at, region_id],
        )?;
        insert_region_operation(
            &transaction,
            "labels_updated",
            region_id,
            None,
            Some(&polygon_json),
            Some(&labels_json(&before_labels)?),
            Some(&polygon_json),
            Some(&labels_json(labels)?),
            updated_at,
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub(crate) fn delete_region(&self, region_id: &str) -> CoreResult<()> {
        if !is_opaque_id(region_id, "REGION-") {
            return Err(CoreError::InvalidOpaqueId);
        }
        self.verify_database_artifacts_safe()?;
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| CoreError::StateUnavailable)?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (polygon_json, status) = transaction
            .query_row(
                "SELECT polygon_norm_json, status FROM regions WHERE region_id = ?1",
                params![region_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?
            .ok_or(CoreError::RegionNotFound)?;
        if status != "active" {
            return Err(CoreError::RegionNotActive);
        }
        let before_labels = read_region_labels(&transaction, region_id)?;
        let updated_at = now_millis();
        transaction.execute(
            "UPDATE regions SET status = 'deleted', updated_at = ?1 WHERE region_id = ?2",
            params![updated_at, region_id],
        )?;
        insert_region_operation(
            &transaction,
            "deleted",
            region_id,
            None,
            Some(&polygon_json),
            Some(&labels_json(&before_labels)?),
            None,
            None,
            updated_at,
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub(crate) fn replace_region(
        &self,
        old_region_id: &str,
        draft: &RegionDraft,
    ) -> CoreResult<()> {
        if !is_opaque_id(old_region_id, "REGION-") {
            return Err(CoreError::InvalidOpaqueId);
        }
        validate_region_draft(draft)?;
        let snapshot = draft.template.snapshot()?;
        let new_polygon_json =
            serde_json::to_string(&draft.polygon_norm).map_err(|_| CoreError::InvalidPolygon)?;

        self.verify_database_artifacts_safe()?;
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| CoreError::StateUnavailable)?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (old_asset_id, old_polygon_json, old_status) = transaction
            .query_row(
                "SELECT asset_id, polygon_norm_json, status FROM regions WHERE region_id = ?1",
                params![old_region_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .optional()?
            .ok_or(CoreError::RegionNotFound)?;
        if old_status != "active" {
            return Err(CoreError::RegionNotActive);
        }
        if old_asset_id != draft.asset_id {
            return Err(CoreError::UnknownSessionItem);
        }
        let is_representative: Option<i64> = transaction
            .query_row(
                "SELECT 1 FROM representative_assets WHERE asset_id = ?1",
                params![draft.asset_id],
                |row| row.get(0),
            )
            .optional()?;
        if is_representative.is_none() {
            return Err(CoreError::AssetNotRepresentative);
        }
        ensure_template_snapshot(&transaction, &snapshot)?;
        let old_labels = read_region_labels(&transaction, old_region_id)?;
        let updated_at = now_millis();
        transaction.execute(
            "INSERT INTO regions (region_id, asset_id, polygon_norm_json, template_id, template_version, created_at, status, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'active', ?6)",
            params![
                draft.region_id,
                draft.asset_id,
                new_polygon_json,
                draft.template.template_id,
                draft.template.version,
                updated_at,
            ],
        )?;
        insert_region_labels(&transaction, &draft.region_id, &draft.labels)?;
        transaction.execute(
            "UPDATE regions SET status = 'replaced', replacement_region_id = ?1, updated_at = ?2 WHERE region_id = ?3",
            params![draft.region_id, updated_at, old_region_id],
        )?;
        insert_region_operation(
            &transaction,
            "created",
            &draft.region_id,
            None,
            None,
            None,
            Some(
                &serde_json::to_string(&draft.polygon_norm)
                    .map_err(|_| CoreError::InvalidPolygon)?,
            ),
            Some(&labels_json(&draft.labels)?),
            updated_at,
        )?;
        insert_region_operation(
            &transaction,
            "replaced",
            old_region_id,
            Some(&draft.region_id),
            Some(&old_polygon_json),
            Some(&labels_json(&old_labels)?),
            Some(&new_polygon_json),
            Some(&labels_json(&draft.labels)?),
            updated_at,
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub(crate) fn capture_video_frame(
        &self,
        case: &PersistedCase,
        parent_video_asset_id: &str,
        capture_time_millis: i64,
        width: u32,
        height: u32,
        bytes: &[u8],
    ) -> CoreResult<CapturedFrame> {
        if !is_opaque_id(&case.case_id, "CASE-")
            || !is_opaque_id(parent_video_asset_id, "ASSET-")
            || capture_time_millis < 0
            || width == 0
            || height == 0
            || bytes.is_empty()
        {
            return Err(CoreError::InvalidCapture);
        }
        validate_capture_png(bytes, width, height)?;
        let asset_id = random_opaque_id("ASSET-");
        let relative_path =
            PathBuf::from(format!("captures/{}/FRAME-{}.png", case.case_id, asset_id));
        self.atomic_write_relative(&relative_path, bytes)?;
        let frame = CapturedFrame {
            asset_id: asset_id.clone(),
            case_id: case.case_id.clone(),
            parent_video_asset_id: parent_video_asset_id.to_owned(),
            capture_time_millis,
            width,
            height,
            size_bytes: bytes
                .len()
                .try_into()
                .map_err(|_| CoreError::InvalidCapture)?,
        };
        let persisted_asset = PersistedAsset {
            asset_id: asset_id.clone(),
            case_id: case.case_id.clone(),
            kind: crate::core::scan::StoredAssetKind::CapturedFrame,
            width,
            height,
            source_fingerprint: format!(
                "CAPTURE-{}-{}",
                parent_video_asset_id, capture_time_millis
            ),
        };
        let persist_result = (|| -> CoreResult<()> {
            self.verify_database_artifacts_safe()?;
            let mut connection = self
                .connection
                .lock()
                .map_err(|_| CoreError::StateUnavailable)?;
            let transaction =
                connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
            upsert_case_and_asset(&transaction, case, &persisted_asset)?;
            transaction.execute(
                "INSERT INTO capture_sources (captured_asset_id, case_id, parent_video_asset_id, capture_time_millis, size_bytes) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    frame.asset_id,
                    frame.case_id,
                    frame.parent_video_asset_id,
                    frame.capture_time_millis,
                    frame.size_bytes,
                ],
            )?;
            transaction.commit()?;
            Ok(())
        })();
        if let Err(error) = persist_result {
            let _ = self.remove_relative_if_exists(&relative_path);
            return Err(error);
        }
        Ok(frame)
    }

    pub(crate) fn captured_frames_for_case(&self, case_id: &str) -> CoreResult<Vec<CapturedFrame>> {
        if !is_opaque_id(case_id, "CASE-") {
            return Err(CoreError::InvalidOpaqueId);
        }
        self.verify_database_artifacts_safe()?;
        let connection = self
            .connection
            .lock()
            .map_err(|_| CoreError::StateUnavailable)?;
        let mut statement = connection.prepare(
            "SELECT captures.captured_asset_id, captures.case_id, captures.parent_video_asset_id,
                    captures.capture_time_millis, assets.width, assets.height, captures.size_bytes
             FROM capture_sources AS captures
             INNER JOIN assets ON assets.asset_id = captures.captured_asset_id
             WHERE captures.case_id = ?1
             ORDER BY captures.capture_time_millis ASC, captures.captured_asset_id ASC",
        )?;
        let rows = statement.query_map(params![case_id], |row| {
            Ok(CapturedFrame {
                asset_id: row.get(0)?,
                case_id: row.get(1)?,
                parent_video_asset_id: row.get(2)?,
                capture_time_millis: row.get(3)?,
                width: row.get(4)?,
                height: row.get(5)?,
                size_bytes: row.get(6)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub(crate) fn read_captured_frame(&self, case_id: &str, asset_id: &str) -> CoreResult<Vec<u8>> {
        if !is_opaque_id(case_id, "CASE-") || !is_opaque_id(asset_id, "ASSET-") {
            return Err(CoreError::InvalidOpaqueId);
        }
        self.verify_database_artifacts_safe()?;
        let connection = self
            .connection
            .lock()
            .map_err(|_| CoreError::StateUnavailable)?;
        let exists: Option<i64> = connection
            .query_row(
                "SELECT 1 FROM capture_sources WHERE case_id = ?1 AND captured_asset_id = ?2",
                params![case_id, asset_id],
                |row| row.get(0),
            )
            .optional()?;
        drop(connection);
        if exists.is_none() {
            return Err(CoreError::UnknownSessionItem);
        }
        self.read_relative_required(Path::new(&format!(
            "captures/{case_id}/FRAME-{asset_id}.png"
        )))
    }

    pub(crate) fn complete_case_backup(&self, case_id: &str) -> CoreResult<BackupResult> {
        if !is_opaque_id(case_id, "CASE-") {
            return Err(CoreError::InvalidOpaqueId);
        }
        let fixed_template = self
            .fixed_template()?
            .ok_or(CoreError::FixedTemplateMissing)?;
        let template_bytes = self
            .read_relative_optional(Path::new(FIXED_TEMPLATE_FILE))?
            .ok_or(CoreError::FixedTemplateMissing)?;
        let captures = self.captured_frames_for_case(case_id)?;
        let completed_at = now_millis();
        let backup_name = format!(
            "backup-{}-{}",
            completed_at,
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        );
        let base = PathBuf::from(format!("backups/{case_id}/{backup_name}"));
        let database_relative = base.join("annotations.sqlite3");
        let database_path = self.reserve_relative_file(&database_relative)?;

        let backup_result = (|| -> CoreResult<()> {
            self.verify_database_artifacts_safe()?;
            let connection = self
                .connection
                .lock()
                .map_err(|_| CoreError::StateUnavailable)?;
            let mut destination = Connection::open_with_flags(
                &database_path,
                OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NOFOLLOW,
            )?;
            let backup = Backup::new(&connection, &mut destination)?;
            backup.run_to_completion(64, Duration::from_millis(5), None)?;
            drop(backup);
            destination.close().map_err(|_| CoreError::Database)?;
            Ok(())
        })();
        if let Err(error) = backup_result {
            let _ = self.remove_relative_if_exists(&database_relative);
            return Err(error);
        }

        self.atomic_write_relative(&base.join("label-template-v1.toml"), &template_bytes)?;
        for capture in &captures {
            let bytes = self.read_captured_frame(case_id, &capture.asset_id)?;
            self.atomic_write_relative(
                &base.join(format!("captures/FRAME-{}.png", capture.asset_id)),
                &bytes,
            )?;
        }
        let manifest = BackupManifest {
            format: BACKUP_FORMAT.to_owned(),
            version: 1,
            case_id: case_id.to_owned(),
            completed_at,
            template_content_sha256: fixed_template.content_sha256,
            capture_asset_ids: captures
                .iter()
                .map(|capture| capture.asset_id.clone())
                .collect(),
        };
        let manifest_bytes =
            serde_json::to_vec(&manifest).map_err(|_| CoreError::StateUnavailable)?;
        // The manifest is deliberately written last. Incomplete backups lack it and are ignored.
        self.atomic_write_relative(&base.join("manifest.json"), &manifest_bytes)?;
        Ok(BackupResult {
            case_id: case_id.to_owned(),
            completed_at,
            capture_count: captures.len(),
        })
    }

    pub(crate) fn write_recovery_state(&self, state: &RecoveryState) -> CoreResult<PathBuf> {
        if !is_opaque_id(&state.current_case_id, "CASE-")
            || !is_opaque_id(&state.current_asset_id, "ASSET-")
            || !state.zoom.is_finite()
            || !state.pan_x.is_finite()
            || !state.pan_y.is_finite()
        {
            return Err(CoreError::InvalidOpaqueId);
        }
        let bytes = serde_json::to_vec(state).map_err(|_| CoreError::StateUnavailable)?;
        self.atomic_write_relative(Path::new("recovery/session-state.json"), &bytes)
    }

    pub(crate) fn write_thumbnail(&self, asset_id: &str, bytes: &[u8]) -> CoreResult<PathBuf> {
        if !is_opaque_id(asset_id, "ASSET-") {
            return Err(CoreError::InvalidOpaqueId);
        }
        self.atomic_write_relative(Path::new(&format!("thumbnails/{asset_id}.webp")), bytes)
    }

    pub(crate) fn write_capture(
        &self,
        case_id: &str,
        asset_id: &str,
        bytes: &[u8],
    ) -> CoreResult<PathBuf> {
        if !is_opaque_id(case_id, "CASE-") || !is_opaque_id(asset_id, "ASSET-") {
            return Err(CoreError::InvalidOpaqueId);
        }
        self.atomic_write_relative(
            Path::new(&format!("captures/{case_id}/FRAME-{asset_id}.jpg")),
            bytes,
        )
    }

    pub(crate) fn representative_count(&self, case_id: &str) -> CoreResult<usize> {
        self.verify_database_artifacts_safe()?;
        let connection = self
            .connection
            .lock()
            .map_err(|_| CoreError::StateUnavailable)?;
        let count: i64 = connection.query_row(
            "SELECT COUNT(*) FROM representative_assets WHERE case_id = ?1",
            params![case_id],
            |row| row.get(0),
        )?;
        Ok(count as usize)
    }

    pub(crate) fn representative_asset_ids(&self, case_id: &str) -> CoreResult<Vec<String>> {
        self.verify_database_artifacts_safe()?;
        let connection = self
            .connection
            .lock()
            .map_err(|_| CoreError::StateUnavailable)?;
        let mut statement = connection.prepare(
            "SELECT asset_id FROM representative_assets WHERE case_id = ?1 ORDER BY selected_at ASC, asset_id ASC",
        )?;
        let rows = statement.query_map(params![case_id], |row| row.get(0))?;
        rows.collect::<Result<Vec<String>, _>>().map_err(Into::into)
    }

    pub(crate) fn recorded_representative_count(&self, case_id: &str) -> CoreResult<usize> {
        self.verify_database_artifacts_safe()?;
        let connection = self
            .connection
            .lock()
            .map_err(|_| CoreError::StateUnavailable)?;
        let count: i64 = connection.query_row(
            "SELECT representative_count FROM cases WHERE case_id = ?1",
            params![case_id],
            |row| row.get(0),
        )?;
        Ok(count as usize)
    }

    pub(crate) fn region_count(&self) -> CoreResult<usize> {
        self.count_rows("regions")
    }

    pub(crate) fn region_label_count(&self) -> CoreResult<usize> {
        self.count_rows("region_label_values")
    }

    pub(crate) fn saved_regions_for_asset(&self, asset_id: &str) -> CoreResult<Vec<SavedRegion>> {
        self.verify_database_artifacts_safe()?;
        let connection = self
            .connection
            .lock()
            .map_err(|_| CoreError::StateUnavailable)?;
        let mut statement = connection.prepare(
            "SELECT region_id, asset_id, polygon_norm_json, template_id, template_version
             FROM regions
             WHERE asset_id = ?1 AND status = 'active'
             ORDER BY created_at ASC, region_id ASC",
        )?;
        let rows = statement.query_map(params![asset_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, u32>(4)?,
            ))
        })?;
        let records = rows.collect::<Result<Vec<_>, _>>()?;
        drop(statement);
        records
            .into_iter()
            .map(
                |(region_id, asset_id, polygon_json, template_id, template_version)| {
                    let polygon_norm =
                        serde_json::from_str(&polygon_json).map_err(|_| CoreError::Database)?;
                    Ok(SavedRegion {
                        labels: read_region_labels(&connection, &region_id)?,
                        region_id,
                        asset_id,
                        polygon_norm,
                        template_id,
                        template_version,
                    })
                },
            )
            .collect()
    }

    pub(crate) fn template_snapshot_json(
        &self,
        template_id: &str,
        version: u32,
    ) -> CoreResult<Option<String>> {
        self.verify_database_artifacts_safe()?;
        let connection = self
            .connection
            .lock()
            .map_err(|_| CoreError::StateUnavailable)?;
        connection
            .query_row(
                "SELECT canonical_json FROM templates WHERE template_id = ?1 AND version = ?2",
                params![template_id, version],
                |row| row.get(0),
            )
            .optional()
            .map_err(Into::into)
    }

    fn count_rows(&self, table: &str) -> CoreResult<usize> {
        let query = match table {
            "regions" => "SELECT COUNT(*) FROM regions WHERE status = 'active'",
            "region_label_values" => "SELECT COUNT(*) FROM region_label_values WHERE region_id IN (SELECT region_id FROM regions WHERE status = 'active')",
            _ => return Err(CoreError::StateUnavailable),
        };
        self.verify_database_artifacts_safe()?;
        let connection = self
            .connection
            .lock()
            .map_err(|_| CoreError::StateUnavailable)?;
        let count: i64 = connection.query_row(query, [], |row| row.get(0))?;
        Ok(count as usize)
    }

    fn verify_database_artifacts_safe(&self) -> CoreResult<()> {
        prepare_database_artifacts(&self.database_paths)
    }

    fn atomic_write_relative(&self, relative: &Path, bytes: &[u8]) -> CoreResult<PathBuf> {
        if !is_safe_relative_path(relative) {
            return Err(CoreError::InvalidWorkPath);
        }
        let file_name = relative
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or(CoreError::InvalidWorkPath)?;
        let parent = self.safe_output_parent(relative)?;
        // This is deliberately the final parent resolution before creating the temporary file.
        // It catches a pre-existing nested symlink/junction without first touching its target.
        let parent = self.resolved_parent_within_work_root(&parent)?;
        let destination = parent.join(file_name);
        self.ensure_safe_output_target(&destination)?;
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let temporary = parent.join(format!(".{file_name}.{}.{}.tmp", now_millis(), sequence));

        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        if let Err(error) = replace_temp_file(&temporary, &destination) {
            let _ = fs::remove_file(&temporary);
            return Err(error);
        }
        Ok(destination)
    }

    fn safe_output_parent(&self, relative: &Path) -> CoreResult<PathBuf> {
        let parent_relative = relative.parent().ok_or(CoreError::InvalidWorkPath)?;
        let mut current = self.work_root.clone();
        self.ensure_safe_directory(&current)?;

        for component in parent_relative.components() {
            let Component::Normal(name) = component else {
                return Err(CoreError::InvalidWorkPath);
            };
            current.push(name);
            self.ensure_safe_directory(&current)?;
        }

        Ok(current)
    }

    fn ensure_safe_directory(&self, path: &Path) -> CoreResult<()> {
        match fs::symlink_metadata(path) {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                match fs::create_dir(path) {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(_) => return Err(CoreError::Filesystem),
                }
            }
            Err(_) => return Err(CoreError::Filesystem),
        }

        let metadata = fs::symlink_metadata(path)?;
        let file_type = metadata.file_type();
        if is_link_or_reparse(&metadata) || !file_type.is_dir() {
            return Err(CoreError::UnsafeDerivedPath);
        }
        self.resolved_parent_within_work_root(path).map(|_| ())
    }

    fn resolved_parent_within_work_root(&self, path: &Path) -> CoreResult<PathBuf> {
        let resolved = fs::canonicalize(path)?;
        if !resolved.starts_with(&self.work_root) {
            return Err(CoreError::UnsafeDerivedPath);
        }
        Ok(resolved)
    }

    fn ensure_safe_output_target(&self, destination: &Path) -> CoreResult<()> {
        match fs::symlink_metadata(destination) {
            Ok(metadata) => {
                let file_type = metadata.file_type();
                if is_link_or_reparse(&metadata) || !file_type.is_file() {
                    return Err(CoreError::UnsafeDerivedPath);
                }
                Ok(())
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(_) => Err(CoreError::Filesystem),
        }
    }

    fn read_fixed_template_manifest(&self) -> CoreResult<Option<FixedTemplateManifest>> {
        let Some(bytes) = self.read_relative_optional(Path::new(FIXED_TEMPLATE_MANIFEST_FILE))?
        else {
            return Ok(None);
        };
        let manifest: FixedTemplateManifest =
            serde_json::from_slice(&bytes).map_err(|_| CoreError::FixedTemplateMissing)?;
        if manifest.format != "gwp-fixed-template"
            || !is_sha256_hex(&manifest.content_sha256)
            || !is_sha256_hex(&manifest.template_sha256)
        {
            return Err(CoreError::FixedTemplateMissing);
        }
        Ok(Some(manifest))
    }

    fn read_relative_optional(&self, relative: &Path) -> CoreResult<Option<Vec<u8>>> {
        read_optional_work_file(&self.work_root, relative)
    }

    fn read_relative_required(&self, relative: &Path) -> CoreResult<Vec<u8>> {
        self.read_relative_optional(relative)?
            .ok_or(CoreError::Filesystem)
    }

    fn remove_relative_if_exists(&self, relative: &Path) -> CoreResult<()> {
        if !is_safe_relative_path(relative) {
            return Err(CoreError::InvalidWorkPath);
        }
        let file_name = relative
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or(CoreError::InvalidWorkPath)?;
        let parent = self.safe_output_parent(relative)?;
        let parent = self.resolved_parent_within_work_root(&parent)?;
        let destination = parent.join(file_name);
        match fs::symlink_metadata(&destination) {
            Ok(metadata) => {
                let file_type = metadata.file_type();
                if is_link_or_reparse(&metadata) || !file_type.is_file() {
                    return Err(CoreError::UnsafeDerivedPath);
                }
                fs::remove_file(destination)?;
                Ok(())
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(_) => Err(CoreError::Filesystem),
        }
    }

    fn reserve_relative_file(&self, relative: &Path) -> CoreResult<PathBuf> {
        if !is_safe_relative_path(relative) {
            return Err(CoreError::InvalidWorkPath);
        }
        let file_name = relative
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or(CoreError::InvalidWorkPath)?;
        let parent = self.safe_output_parent(relative)?;
        let parent = self.resolved_parent_within_work_root(&parent)?;
        let destination = parent.join(file_name);
        self.ensure_safe_output_target(&destination)?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&destination)?;
        file.sync_all()?;
        Ok(destination)
    }
}

fn read_optional_work_file(work_root: &Path, relative: &Path) -> CoreResult<Option<Vec<u8>>> {
    if !is_safe_relative_path(relative) {
        return Err(CoreError::InvalidWorkPath);
    }
    let root_metadata = match fs::symlink_metadata(work_root) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(CoreError::Filesystem),
    };
    if is_link_or_reparse(&root_metadata) || !root_metadata.file_type().is_dir() {
        return Err(CoreError::UnsafeDerivedPath);
    }
    let canonical_root = fs::canonicalize(work_root)?;
    let mut current = canonical_root.clone();
    let components = relative.components().collect::<Vec<_>>();
    let (file_component, parent_components) =
        components.split_last().ok_or(CoreError::InvalidWorkPath)?;
    for component in parent_components {
        let Component::Normal(name) = component else {
            return Err(CoreError::InvalidWorkPath);
        };
        current.push(name);
        match fs::symlink_metadata(&current) {
            Ok(metadata) => {
                if is_link_or_reparse(&metadata) || !metadata.file_type().is_dir() {
                    return Err(CoreError::UnsafeDerivedPath);
                }
                let resolved = fs::canonicalize(&current)?;
                if !resolved.starts_with(&canonical_root) {
                    return Err(CoreError::UnsafeDerivedPath);
                }
                current = resolved;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(CoreError::Filesystem),
        }
    }
    let Component::Normal(file_name) = file_component else {
        return Err(CoreError::InvalidWorkPath);
    };
    let path = current.join(file_name);
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(CoreError::Filesystem),
    };
    if is_link_or_reparse(&metadata) || !metadata.file_type().is_file() {
        return Err(CoreError::UnsafeDerivedPath);
    }
    fs::read(path).map(Some).map_err(Into::into)
}

fn remove_optional_work_file(work_root: &Path, relative: &Path) -> CoreResult<()> {
    if !is_safe_relative_path(relative) {
        return Err(CoreError::InvalidWorkPath);
    }
    let root_metadata = match fs::symlink_metadata(work_root) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(_) => return Err(CoreError::Filesystem),
    };
    if is_link_or_reparse(&root_metadata) || !root_metadata.file_type().is_dir() {
        return Err(CoreError::UnsafeDerivedPath);
    }
    let canonical_root = fs::canonicalize(work_root)?;
    let mut current = canonical_root.clone();
    let components = relative.components().collect::<Vec<_>>();
    let (file_component, parent_components) =
        components.split_last().ok_or(CoreError::InvalidWorkPath)?;
    for component in parent_components {
        let Component::Normal(name) = component else {
            return Err(CoreError::InvalidWorkPath);
        };
        current.push(name);
        match fs::symlink_metadata(&current) {
            Ok(metadata) => {
                if is_link_or_reparse(&metadata) || !metadata.file_type().is_dir() {
                    return Err(CoreError::UnsafeDerivedPath);
                }
                let resolved = fs::canonicalize(&current)?;
                if !resolved.starts_with(&canonical_root) {
                    return Err(CoreError::UnsafeDerivedPath);
                }
                current = resolved;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(_) => return Err(CoreError::Filesystem),
        }
    }
    let Component::Normal(file_name) = file_component else {
        return Err(CoreError::InvalidWorkPath);
    };
    let path = current.join(file_name);
    match fs::symlink_metadata(&path) {
        Ok(metadata) => {
            if is_link_or_reparse(&metadata) || !metadata.file_type().is_file() {
                return Err(CoreError::UnsafeDerivedPath);
            }
            fs::remove_file(path)?;
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(CoreError::Filesystem),
    }
}

fn validate_project_session(state: &ProjectSessionState) -> CoreResult<()> {
    if state.source_root.trim().is_empty()
        || !Path::new(&state.source_root).is_absolute()
        || state.current_case_id.is_some() != state.current_asset_id.is_some()
        || state
            .current_case_id
            .as_deref()
            .is_some_and(|value| !is_opaque_id(value, "CASE-"))
        || state
            .current_asset_id
            .as_deref()
            .is_some_and(|value| !is_opaque_id(value, "ASSET-"))
        || state
            .template_content_sha256
            .as_deref()
            .is_some_and(|value| !is_sha256_hex(value))
    {
        return Err(CoreError::StateUnavailable);
    }
    Ok(())
}

fn fixed_template_from_bytes(
    manifest: &FixedTemplateManifest,
    contents: &[u8],
) -> CoreResult<FixedTemplate> {
    let text =
        std::str::from_utf8(contents).map_err(|_| CoreError::FixedTemplateChecksumMismatch)?;
    let template =
        LabelTemplate::parse_toml(text).map_err(|_| CoreError::FixedTemplateChecksumMismatch)?;
    let snapshot = template.snapshot()?;
    let content_sha256 = sha256_hex(contents);
    if manifest.content_sha256 != content_sha256 || manifest.template_sha256 != snapshot.sha256 {
        return Err(CoreError::FixedTemplateChecksumMismatch);
    }
    Ok(FixedTemplate {
        content_sha256,
        template_sha256: snapshot.sha256,
        template,
    })
}

fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn sha256_hex(bytes: &[u8]) -> String {
    HEXLOWER.encode(Sha256::digest(bytes).as_slice())
}

fn random_opaque_id(prefix: &str) -> String {
    let mut bytes = [0_u8; 16];
    OsRng.fill_bytes(&mut bytes);
    format!("{prefix}{}", BASE32_NOPAD.encode(&bytes))
}

fn ensure_template_snapshot(
    transaction: &Transaction<'_>,
    snapshot: &crate::core::template::TemplateSnapshot,
) -> CoreResult<()> {
    match transaction
        .query_row(
            "SELECT sha256 FROM templates WHERE template_id = ?1 AND version = ?2",
            params![snapshot.template_id, snapshot.version],
            |row| row.get::<_, String>(0),
        )
        .optional()?
    {
        Some(existing_sha) if existing_sha != snapshot.sha256 => {
            Err(CoreError::TemplateSnapshotConflict)
        }
        Some(_) => Ok(()),
        None => {
            transaction.execute(
                "INSERT INTO templates (template_id, version, sha256, canonical_json) VALUES (?1, ?2, ?3, ?4)",
                params![
                    snapshot.template_id,
                    snapshot.version,
                    snapshot.sha256,
                    snapshot.canonical_json,
                ],
            )?;
            Ok(())
        }
    }
}

fn read_region_labels(
    connection: &Connection,
    region_id: &str,
) -> CoreResult<BTreeMap<String, LabelValue>> {
    let mut statement = connection.prepare(
        "SELECT field_id, value_json FROM region_label_values WHERE region_id = ?1 ORDER BY field_id ASC",
    )?;
    let rows = statement.query_map(params![region_id], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    let mut labels = BTreeMap::new();
    for row in rows {
        let (field_id, value_json) = row?;
        let value = serde_json::from_str(&value_json).map_err(|_| CoreError::Database)?;
        if labels.insert(field_id, value).is_some() {
            return Err(CoreError::Database);
        }
    }
    Ok(labels)
}

fn labels_json(labels: &BTreeMap<String, LabelValue>) -> CoreResult<String> {
    serde_json::to_string(labels).map_err(|_| CoreError::InvalidLabels)
}

fn insert_region_labels(
    transaction: &Transaction<'_>,
    region_id: &str,
    labels: &BTreeMap<String, LabelValue>,
) -> CoreResult<()> {
    for (field_id, value) in labels {
        let value_json = serde_json::to_string(value).map_err(|_| CoreError::InvalidLabels)?;
        transaction.execute(
            "INSERT INTO region_label_values (region_id, field_id, value_json) VALUES (?1, ?2, ?3)",
            params![region_id, field_id, value_json],
        )?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn insert_region_operation(
    transaction: &Transaction<'_>,
    operation_kind: &str,
    subject_region_id: &str,
    related_region_id: Option<&str>,
    before_polygon_norm_json: Option<&str>,
    before_labels_json: Option<&str>,
    after_polygon_norm_json: Option<&str>,
    after_labels_json: Option<&str>,
    occurred_at: i64,
) -> CoreResult<()> {
    transaction.execute(
        "INSERT INTO region_operations (operation_kind, subject_region_id, related_region_id, occurred_at, before_polygon_norm_json, before_labels_json, after_polygon_norm_json, after_labels_json) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            operation_kind,
            subject_region_id,
            related_region_id,
            occurred_at,
            before_polygon_norm_json,
            before_labels_json,
            after_polygon_norm_json,
            after_labels_json,
        ],
    )?;
    Ok(())
}

/// The temporary file is always created in `destination.parent()`, so every replacement stays on
/// the same volume. Windows requires ReplaceFileW for an atomic overwrite of an existing target;
/// std::fs::rename does not provide that behavior there.
fn replace_temp_file(temporary: &Path, destination: &Path) -> CoreResult<()> {
    #[cfg(windows)]
    {
        replace_temp_file_windows(temporary, destination)
    }
    #[cfg(not(windows))]
    {
        fs::rename(temporary, destination)?;
        Ok(())
    }
}

/// Reserve and validate every SQLite file before asking SQLite to open the database. `create_new`
/// refuses an already-existing final component, so it cannot create through a pre-existing
/// symlink/reparse point. The main database is also opened with SQLite's no-follow flag below.
/// This is reused after WAL setup and before every core database operation because SQLite can
/// remove an unused rollback journal when it commits the transition from DELETE to WAL mode.
fn prepare_database_artifacts(paths: &DatabasePaths) -> CoreResult<()> {
    for path in paths.all() {
        ensure_safe_database_file(path)?;
    }
    Ok(())
}

fn ensure_safe_database_file(path: &Path) -> CoreResult<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => validate_database_file_metadata(&metadata),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            match OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .open(path)
            {
                Ok(file) => {
                    drop(file);
                    let metadata = fs::symlink_metadata(path)?;
                    validate_database_file_metadata(&metadata)
                }
                // A concurrent creator won the race. Re-inspect the exact final component and
                // reject it if it is anything other than a regular file.
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    let metadata = fs::symlink_metadata(path)?;
                    validate_database_file_metadata(&metadata)
                }
                Err(_) => Err(CoreError::Filesystem),
            }
        }
        Err(_) => Err(CoreError::Filesystem),
    }
}

fn validate_database_file_metadata(metadata: &fs::Metadata) -> CoreResult<()> {
    let file_type = metadata.file_type();
    if is_link_or_reparse(metadata) || !file_type.is_file() {
        return Err(CoreError::UnsafeDerivedPath);
    }
    Ok(())
}

fn open_database_no_follow(database_path: &Path) -> CoreResult<Connection> {
    Connection::open_with_flags(
        database_path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )
    .map_err(Into::into)
}

#[cfg(windows)]
fn replace_temp_file_windows(temporary: &Path, destination: &Path) -> CoreResult<()> {
    use std::iter::once;
    use std::os::windows::ffi::OsStrExt;

    use windows_sys::Win32::Storage::FileSystem::{ReplaceFileW, REPLACEFILE_WRITE_THROUGH};

    let destination_exists = match fs::symlink_metadata(destination) {
        Ok(metadata) => {
            let file_type = metadata.file_type();
            if is_link_or_reparse(&metadata) || !file_type.is_file() {
                return Err(CoreError::UnsafeDerivedPath);
            }
            true
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(_) => return Err(CoreError::Filesystem),
    };
    if !destination_exists {
        fs::rename(temporary, destination)?;
        return Ok(());
    }

    let destination_wide = destination
        .as_os_str()
        .encode_wide()
        .chain(once(0))
        .collect::<Vec<_>>();
    let temporary_wide = temporary
        .as_os_str()
        .encode_wide()
        .chain(once(0))
        .collect::<Vec<_>>();
    let replaced = unsafe {
        ReplaceFileW(
            destination_wide.as_ptr(),
            temporary_wide.as_ptr(),
            std::ptr::null(),
            REPLACEFILE_WRITE_THROUGH,
            std::ptr::null(),
            std::ptr::null(),
        )
    };
    if replaced == 0 {
        return Err(CoreError::Filesystem);
    }
    Ok(())
}

fn configure_and_migrate(connection: &mut Connection) -> CoreResult<()> {
    connection.pragma_update(None, "journal_mode", "WAL")?;
    connection.pragma_update(None, "synchronous", "FULL")?;
    connection.pragma_update(None, "foreign_keys", "ON")?;

    let mut version: i64 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if version < 1 && !table_exists(connection, "cases")? {
        create_v3_schema(connection)?;
        return Ok(());
    } else if version < 2 {
        migrate_legacy_cases_to_v2(connection)?;
        version = 2;
    }
    if version < SCHEMA_VERSION {
        migrate_v2_to_v3(connection)?;
    } else {
        // A partially upgraded directory may retain this named v1 trigger even after its
        // user_version changed. It is harmless to remove and keeps reopening idempotent.
        connection
            .execute_batch("DROP TRIGGER IF EXISTS representative_assets_limit_before_insert;")?;
    }
    Ok(())
}

fn table_exists(connection: &Connection, table: &str) -> CoreResult<bool> {
    Ok(connection
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1",
            params![table],
            |row| row.get::<_, i64>(0),
        )
        .optional()?
        .is_some())
}

fn create_v3_schema(connection: &mut Connection) -> CoreResult<()> {
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    transaction.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS cases (
            case_id TEXT PRIMARY KEY CHECK(case_id GLOB 'CASE-*'),
            campus_code TEXT NOT NULL CHECK(campus_code GLOB 'CAMPUS-*'),
            representative_count INTEGER NOT NULL DEFAULT 0 CHECK(representative_count >= 0),
            updated_at INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS assets (
            asset_id TEXT PRIMARY KEY CHECK(asset_id GLOB 'ASSET-*'),
            case_id TEXT NOT NULL REFERENCES cases(case_id),
            kind TEXT NOT NULL CHECK(kind IN ('original_image', 'captured_frame')),
            width INTEGER NOT NULL CHECK(width >= 0),
            height INTEGER NOT NULL CHECK(height >= 0),
            source_fingerprint TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS representative_assets (
            case_id TEXT NOT NULL REFERENCES cases(case_id),
            asset_id TEXT NOT NULL REFERENCES assets(asset_id),
            selected_at INTEGER NOT NULL,
            PRIMARY KEY(case_id, asset_id)
        );
        CREATE TABLE IF NOT EXISTS templates (
            template_id TEXT NOT NULL,
            version INTEGER NOT NULL,
            sha256 TEXT NOT NULL,
            canonical_json TEXT NOT NULL,
            PRIMARY KEY(template_id, version)
        );
        CREATE TABLE IF NOT EXISTS regions (
            region_id TEXT PRIMARY KEY CHECK(region_id GLOB 'REGION-*'),
            asset_id TEXT NOT NULL REFERENCES assets(asset_id),
            polygon_norm_json TEXT NOT NULL,
            template_id TEXT NOT NULL,
            template_version INTEGER NOT NULL,
            created_at INTEGER NOT NULL,
            status TEXT NOT NULL DEFAULT 'active' CHECK(status IN ('active', 'deleted', 'replaced')),
            updated_at INTEGER NOT NULL,
            replacement_region_id TEXT REFERENCES regions(region_id),
            FOREIGN KEY(template_id, template_version) REFERENCES templates(template_id, version)
        );
        CREATE TABLE IF NOT EXISTS region_label_values (
            region_id TEXT NOT NULL REFERENCES regions(region_id),
            field_id TEXT NOT NULL,
            value_json TEXT NOT NULL,
            PRIMARY KEY(region_id, field_id)
        );
        CREATE TABLE IF NOT EXISTS region_operations (
            operation_id INTEGER PRIMARY KEY,
            operation_kind TEXT NOT NULL CHECK(operation_kind IN ('created', 'labels_updated', 'deleted', 'replaced')),
            subject_region_id TEXT NOT NULL REFERENCES regions(region_id),
            related_region_id TEXT REFERENCES regions(region_id),
            occurred_at INTEGER NOT NULL,
            before_polygon_norm_json TEXT,
            before_labels_json TEXT,
            after_polygon_norm_json TEXT,
            after_labels_json TEXT
        );
        CREATE TABLE IF NOT EXISTS capture_sources (
            captured_asset_id TEXT PRIMARY KEY REFERENCES assets(asset_id),
            case_id TEXT NOT NULL REFERENCES cases(case_id),
            parent_video_asset_id TEXT NOT NULL CHECK(parent_video_asset_id GLOB 'ASSET-*'),
            capture_time_millis INTEGER NOT NULL CHECK(capture_time_millis >= 0),
            size_bytes INTEGER NOT NULL CHECK(size_bytes >= 0)
        );
        DROP TRIGGER IF EXISTS representative_assets_limit_before_insert;
        ",
    )?;
    transaction.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    transaction.commit()?;
    Ok(())
}

fn migrate_v2_to_v3(connection: &mut Connection) -> CoreResult<()> {
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    transaction.execute_batch(
        "
        DROP TRIGGER IF EXISTS representative_assets_limit_before_insert;
        ALTER TABLE regions ADD COLUMN status TEXT NOT NULL DEFAULT 'active' CHECK(status IN ('active', 'deleted', 'replaced'));
        ALTER TABLE regions ADD COLUMN updated_at INTEGER NOT NULL DEFAULT 0;
        ALTER TABLE regions ADD COLUMN replacement_region_id TEXT REFERENCES regions(region_id);
        UPDATE regions SET updated_at = created_at WHERE updated_at = 0;
        CREATE TABLE region_operations (
            operation_id INTEGER PRIMARY KEY,
            operation_kind TEXT NOT NULL CHECK(operation_kind IN ('created', 'labels_updated', 'deleted', 'replaced')),
            subject_region_id TEXT NOT NULL REFERENCES regions(region_id),
            related_region_id TEXT REFERENCES regions(region_id),
            occurred_at INTEGER NOT NULL,
            before_polygon_norm_json TEXT,
            before_labels_json TEXT,
            after_polygon_norm_json TEXT,
            after_labels_json TEXT
        );
        CREATE TABLE capture_sources (
            captured_asset_id TEXT PRIMARY KEY REFERENCES assets(asset_id),
            case_id TEXT NOT NULL REFERENCES cases(case_id),
            parent_video_asset_id TEXT NOT NULL CHECK(parent_video_asset_id GLOB 'ASSET-*'),
            capture_time_millis INTEGER NOT NULL CHECK(capture_time_millis >= 0),
            size_bytes INTEGER NOT NULL CHECK(size_bytes >= 0)
        );
        PRAGMA user_version = 3;
        ",
    )?;
    transaction.commit()?;
    verify_foreign_keys(connection)
}

fn migrate_legacy_cases_to_v2(connection: &mut Connection) -> CoreResult<()> {
    // SQLite cannot drop a CHECK constraint in place. Foreign keys must be disabled before
    // replacing the referenced table, then restored even when the migration transaction fails.
    connection.pragma_update(None, "foreign_keys", "OFF")?;
    let migration_result = (|| -> CoreResult<()> {
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute_batch(
            "
            DROP TRIGGER IF EXISTS representative_assets_limit_before_insert;
            CREATE TABLE cases_v2 (
                case_id TEXT PRIMARY KEY CHECK(case_id GLOB 'CASE-*'),
                campus_code TEXT NOT NULL CHECK(campus_code GLOB 'CAMPUS-*'),
                representative_count INTEGER NOT NULL DEFAULT 0 CHECK(representative_count >= 0),
                updated_at INTEGER NOT NULL
            );
            INSERT INTO cases_v2 (case_id, campus_code, representative_count, updated_at)
            SELECT
                cases.case_id,
                cases.campus_code,
                (SELECT COUNT(*) FROM representative_assets WHERE representative_assets.case_id = cases.case_id),
                cases.updated_at
            FROM cases;
            DROP TABLE cases;
            ALTER TABLE cases_v2 RENAME TO cases;
            PRAGMA user_version = 2;
            ",
        )?;
        transaction.commit()?;
        Ok(())
    })();
    let restore_result = connection.pragma_update(None, "foreign_keys", "ON");
    migration_result?;
    restore_result?;
    verify_foreign_keys(connection)
}

fn verify_foreign_keys(connection: &Connection) -> CoreResult<()> {
    let mut statement = connection.prepare("PRAGMA foreign_key_check")?;
    let mut rows = statement.query([])?;
    if rows.next()?.is_some() {
        return Err(CoreError::Database);
    }
    Ok(())
}

fn upsert_case_and_asset(
    transaction: &Transaction<'_>,
    case: &PersistedCase,
    asset: &PersistedAsset,
) -> CoreResult<()> {
    transaction.execute(
        "INSERT INTO cases (case_id, campus_code, representative_count, updated_at) VALUES (?1, ?2, 0, ?3)
         ON CONFLICT(case_id) DO UPDATE SET campus_code = excluded.campus_code, updated_at = excluded.updated_at",
        params![case.case_id, case.campus_code, now_millis()],
    )?;
    transaction.execute(
        "INSERT INTO assets (asset_id, case_id, kind, width, height, source_fingerprint) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(asset_id) DO UPDATE SET
           case_id = excluded.case_id,
           kind = excluded.kind,
           width = CASE WHEN excluded.width > 0 THEN excluded.width ELSE assets.width END,
           height = CASE WHEN excluded.height > 0 THEN excluded.height ELSE assets.height END,
           source_fingerprint = excluded.source_fingerprint",
        params![
            asset.asset_id,
            asset.case_id,
            asset.kind.as_str(),
            asset.width,
            asset.height,
            asset.source_fingerprint,
        ],
    )?;
    Ok(())
}

fn update_representative_count(transaction: &Transaction<'_>, case_id: &str) -> CoreResult<usize> {
    let count: i64 = transaction.query_row(
        "SELECT COUNT(*) FROM representative_assets WHERE case_id = ?1",
        params![case_id],
        |row| row.get(0),
    )?;
    transaction.execute(
        "UPDATE cases SET representative_count = ?1, updated_at = ?2 WHERE case_id = ?3",
        params![count, now_millis(), case_id],
    )?;
    Ok(count as usize)
}

fn validate_region_draft(draft: &RegionDraft) -> CoreResult<()> {
    if !is_opaque_id(&draft.region_id, "REGION-") || !is_opaque_id(&draft.asset_id, "ASSET-") {
        return Err(CoreError::InvalidOpaqueId);
    }
    draft.template.validate_label_values(&draft.labels)?;
    validate_polygon(&draft.polygon_norm)
}

fn validate_capture_png(bytes: &[u8], width: u32, height: u32) -> CoreResult<()> {
    // The WebView creates captures via canvas.toDataURL("image/png"). Verify the immutable PNG
    // signature and IHDR dimensions before turning the bytes into a persistent image asset.
    if bytes.len() < 33
        || bytes[..8] != PNG_SIGNATURE
        || bytes[8..12] != [0, 0, 0, 13]
        || bytes[12..16] != *b"IHDR"
    {
        return Err(CoreError::InvalidCapture);
    }
    let png_width = u32::from_be_bytes(bytes[16..20].try_into().expect("four-byte PNG width"));
    let png_height = u32::from_be_bytes(bytes[20..24].try_into().expect("four-byte PNG height"));
    if png_width == 0 || png_height == 0 || png_width != width || png_height != height {
        return Err(CoreError::InvalidCapture);
    }
    Ok(())
}

fn validate_polygon(points: &[PointNorm]) -> CoreResult<()> {
    if points.len() < 3
        || points.iter().any(|point| {
            !point.x.is_finite()
                || !point.y.is_finite()
                || !(0.0..=1.0).contains(&point.x)
                || !(0.0..=1.0).contains(&point.y)
        })
    {
        return Err(CoreError::InvalidPolygon);
    }

    let mut twice_area = 0.0;
    for (current, next) in points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .take(points.len())
    {
        twice_area += current.x * next.y - next.x * current.y;
    }
    if twice_area.abs() < 0.000_001 || polygon_self_intersects(points) {
        return Err(CoreError::InvalidPolygon);
    }
    Ok(())
}

fn polygon_self_intersects(points: &[PointNorm]) -> bool {
    for first in 0..points.len() {
        let first_next = (first + 1) % points.len();
        for second in (first + 1)..points.len() {
            let second_next = (second + 1) % points.len();
            if edges_are_adjacent(first, second, points.len()) {
                continue;
            }
            if segments_intersect(
                &points[first],
                &points[first_next],
                &points[second],
                &points[second_next],
            ) {
                return true;
            }
        }
    }
    false
}

fn edges_are_adjacent(first: usize, second: usize, point_count: usize) -> bool {
    first == second || (first + 1) % point_count == second || (second + 1) % point_count == first
}

fn segments_intersect(a: &PointNorm, b: &PointNorm, c: &PointNorm, d: &PointNorm) -> bool {
    let ab_c = orientation(a, b, c);
    let ab_d = orientation(a, b, d);
    let cd_a = orientation(c, d, a);
    let cd_b = orientation(c, d, b);

    if ab_c == 0 && point_on_segment(a, b, c) {
        return true;
    }
    if ab_d == 0 && point_on_segment(a, b, d) {
        return true;
    }
    if cd_a == 0 && point_on_segment(c, d, a) {
        return true;
    }
    if cd_b == 0 && point_on_segment(c, d, b) {
        return true;
    }
    ab_c != ab_d && cd_a != cd_b
}

fn orientation(a: &PointNorm, b: &PointNorm, point: &PointNorm) -> i8 {
    let value = cross(a, b, point);
    if value.abs() <= INTERSECTION_EPSILON {
        0
    } else if value.is_sign_positive() {
        1
    } else {
        -1
    }
}

fn point_on_segment(start: &PointNorm, end: &PointNorm, point: &PointNorm) -> bool {
    point.x >= start.x.min(end.x) - INTERSECTION_EPSILON
        && point.x <= start.x.max(end.x) + INTERSECTION_EPSILON
        && point.y >= start.y.min(end.y) - INTERSECTION_EPSILON
        && point.y <= start.y.max(end.y) + INTERSECTION_EPSILON
}

fn cross(a: &PointNorm, b: &PointNorm, point: &PointNorm) -> f64 {
    (b.x - a.x) * (point.y - a.y) - (b.y - a.y) * (point.x - a.x)
}

fn is_safe_relative_path(path: &Path) -> bool {
    !path.as_os_str().is_empty()
        && !path.is_absolute()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
}

fn is_link_or_reparse(metadata: &fs::Metadata) -> bool {
    let file_type = metadata.file_type();
    if file_type.is_symlink() {
        return true;
    }

    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;

        metadata.file_attributes() & 0x400 != 0
    }

    #[cfg(not(windows))]
    {
        false
    }
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    use super::{validate_polygon, PointNorm};

    fn points(values: &[(f64, f64)]) -> Vec<PointNorm> {
        values
            .iter()
            .map(|(x, y)| PointNorm { x: *x, y: *y })
            .collect()
    }

    #[test]
    fn polygon_rejects_nonadjacent_collinear_overlap() {
        let polygon = points(&[
            (0.1, 0.1),
            (0.9, 0.1),
            (0.9, 0.7),
            (0.8, 0.1),
            (0.3, 0.1),
            (0.2, 0.7),
        ]);
        assert!(validate_polygon(&polygon).is_err());
    }

    #[test]
    fn polygon_rejects_nonadjacent_endpoint_contact() {
        let polygon = points(&[(0.1, 0.1), (0.9, 0.1), (0.9, 0.8), (0.5, 0.1), (0.1, 0.8)]);
        assert!(validate_polygon(&polygon).is_err());
    }
}
