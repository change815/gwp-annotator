use std::collections::{BTreeMap, BTreeSet};
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
const LEGACY_FIXED_TEMPLATE_FILE: &str = "templates/label-template-v1.toml";
const LEGACY_FIXED_TEMPLATE_MANIFEST_FILE: &str = "templates/label-template-v1.manifest.json";
const ACTIVE_TEMPLATE_MANIFEST_FILE: &str = "templates/active-template.manifest.json";
const ACTIVE_TEMPLATE_STORAGE_MARKER_FILE: &str = "templates/active-template-storage-v1.marker";
const TEMPLATE_ARCHIVE_DIRECTORY: &str = "templates/immutable";
const ACTIVE_TEMPLATE_STORAGE_MARKER: &[u8] = b"gwp-active-template-storage-v1\n";
const PROJECT_SESSION_FILE: &str = "session/project-session.json";
const BACKUP_FORMAT: &str = "gwp-current-backup";
const BACKUP_VERSION: u32 = 2;
const BACKUP_DIRECTORY: &str = "backups";
const BACKUP_STAGING_DIRECTORY: &str = "backups/.current-staging";
const BACKUP_PREVIOUS_DIRECTORY: &str = "backups/.current-previous";
const INSTALLATION_STORAGE_PARENT_DIRECTORY: &str = "GWP Annotator Data";
const INSTALLATION_WORK_ROOT_DIRECTORY: &str = "PROJECT-LOCAL";
const SCHEMA_VERSION: i64 = 4;
const INTERSECTION_EPSILON: f64 = 1e-12;
const PNG_SIGNATURE: [u8; 8] = [137, 80, 78, 71, 13, 10, 26, 10];
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);
static WORK_ROOT_PROBE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub(crate) struct Store {
    /// Canonical physical work root. All derived-file parents must resolve beneath this path.
    work_root: PathBuf,
    database_paths: DatabasePaths,
    connection: Mutex<Connection>,
    /// Completion snapshots use one fixed staging/current/previous set. Serialize publication so
    /// two UI actions cannot move the same directory concurrently.
    backup_publish: Mutex<()>,
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
    pub template: LabelTemplate,
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
    #[serde(default)]
    pub pending_template_content_sha256: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct FixedTemplate {
    pub content_sha256: String,
    pub template_sha256: String,
    pub previous_content_sha256: Option<String>,
    pub template: LabelTemplate,
}

#[derive(Clone, Debug)]
pub(crate) struct PreparedFixedTemplate {
    pub fixed: FixedTemplate,
    pub needs_activation: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
struct FixedTemplateManifest {
    format: String,
    content_sha256: String,
    template_sha256: String,
    #[serde(default)]
    previous_content_sha256: Option<String>,
    #[serde(default)]
    template_path: Option<String>,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackupFault {
    None,
    BeforeCurrentPromotion,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
struct BackupManifest {
    format: String,
    version: u32,
    completed_case_id: String,
    completed_at: i64,
    template_content_sha256: String,
    capture_asset_ids: Vec<String>,
}

impl Store {
    /// Checks only the application-owned final storage components. It intentionally does not
    /// inspect arbitrary ancestors: macOS legitimately exposes `/var` through a symlink, while
    /// Windows needs the two app-owned components checked before any canonicalization can follow
    /// a junction away from the chosen installation drive.
    pub(crate) fn validate_work_root_for_overlap(work_root: &Path) -> CoreResult<()> {
        validate_existing_work_root_components(work_root)
    }

    /// Resolves an existing portion without creating anything. This lets the service reject a
    /// source/work overlap before SQLite, session files, a write probe, or even the work root are
    /// created.
    pub(crate) fn resolve_work_root_for_overlap(work_root: &Path) -> CoreResult<PathBuf> {
        validate_existing_work_root_components(work_root)?;
        resolve_existing_work_root_path(work_root)
    }

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
        let work_root = prepare_work_root_after_overlap_check(&requested_work_root)?;
        let database_paths = DatabasePaths::in_work_root(&work_root);
        prepare_database_artifacts(&database_paths)?;
        let mut connection = open_database_no_follow(&database_paths.main)?;
        configure_and_migrate(&mut connection)?;
        // SQLite may remove an unused rollback journal while switching to WAL. Re-reserve any
        // missing sibling with create_new and validate every final component again afterwards.
        prepare_database_artifacts(&database_paths)?;
        let store = Self {
            work_root,
            database_paths,
            connection: Mutex::new(connection),
            backup_publish: Mutex::new(()),
        };
        // A crash between same-parent renames can leave backup publication state behind. Try to
        // recover that app-owned pointer on project open, but never let an invalid or inaccessible
        // backup prevent doctors from opening the live SQLite archive. Explicit backup creation
        // retries the same recovery and reports any remaining problem to the caller.
        let _ = store.recover_interrupted_backup_publish();
        Ok(store)
    }

    pub(crate) fn write_project_session(&self, state: &ProjectSessionState) -> CoreResult<()> {
        validate_project_session(state)?;
        let bytes = serde_json::to_vec(state).map_err(|_| CoreError::StateUnavailable)?;
        self.atomic_write_relative(Path::new(PROJECT_SESSION_FILE), &bytes)
            .map(|_| ())
    }

    pub(crate) fn prepare_fixed_template_install(
        &self,
        contents: &str,
        template: &LabelTemplate,
    ) -> CoreResult<PreparedFixedTemplate> {
        let candidate = fixed_template_from_input(contents, template, None)?;
        match self.fixed_template() {
            Ok(None) => {
                self.prepare_template_archive(contents.as_bytes(), &candidate)?;
                Ok(PreparedFixedTemplate {
                    fixed: candidate,
                    needs_activation: true,
                })
            }
            Ok(Some(existing))
                if existing.content_sha256 == candidate.content_sha256
                    && existing.template_sha256 == candidate.template_sha256 =>
            {
                Ok(PreparedFixedTemplate {
                    fixed: existing,
                    needs_activation: false,
                })
            }
            Ok(Some(_)) => Err(CoreError::FixedTemplateAlreadyInstalled),
            Err(CoreError::FixedTemplateMissing) => {
                self.prepare_missing_template_copy(contents.as_bytes(), candidate)
            }
            Err(error) => Err(error),
        }
    }

    pub(crate) fn prepare_fixed_template_replacement(
        &self,
        contents: &str,
        template: &LabelTemplate,
    ) -> CoreResult<PreparedFixedTemplate> {
        let current = self
            .fixed_template()?
            .ok_or(CoreError::FixedTemplateMissing)?;
        let candidate =
            fixed_template_from_input(contents, template, Some(current.content_sha256.clone()))?;
        if current.content_sha256 == candidate.content_sha256
            && current.template_sha256 == candidate.template_sha256
        {
            return Ok(PreparedFixedTemplate {
                fixed: current,
                needs_activation: false,
            });
        }

        self.ensure_replacement_identity_is_available(&candidate)?;
        self.prepare_template_archive(contents.as_bytes(), &candidate)?;
        Ok(PreparedFixedTemplate {
            fixed: candidate,
            needs_activation: true,
        })
    }

    pub(crate) fn activate_prepared_fixed_template(
        &self,
        fixed: &FixedTemplate,
    ) -> CoreResult<FixedTemplate> {
        self.activate_template_archive(fixed)
    }

    pub(crate) fn fixed_template(&self) -> CoreResult<Option<FixedTemplate>> {
        self.fixed_template_with_contents()
            .map(|fixed| fixed.map(|(fixed, _)| fixed))
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

    pub(crate) fn refine_region_geometry(
        &self,
        region_id: &str,
        asset_id: &str,
        expected_polygon_norm: &[PointNorm],
        polygon_norm: &[PointNorm],
    ) -> CoreResult<()> {
        if !is_opaque_id(region_id, "REGION-") || !is_opaque_id(asset_id, "ASSET-") {
            return Err(CoreError::InvalidOpaqueId);
        }
        validate_polygon(polygon_norm)?;
        let new_polygon_json =
            serde_json::to_string(polygon_norm).map_err(|_| CoreError::InvalidPolygon)?;

        self.verify_database_artifacts_safe()?;
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| CoreError::StateUnavailable)?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (stored_asset_id, current_polygon_json, status) = transaction
            .query_row(
                "SELECT asset_id, polygon_norm_json, status FROM regions WHERE region_id = ?1",
                params![region_id],
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
        if status != "active" {
            return Err(CoreError::RegionNotActive);
        }
        if stored_asset_id != asset_id {
            return Err(CoreError::UnknownSessionItem);
        }
        let is_representative: Option<i64> = transaction
            .query_row(
                "SELECT 1 FROM representative_assets WHERE asset_id = ?1",
                params![asset_id],
                |row| row.get(0),
            )
            .optional()?;
        if is_representative.is_none() {
            return Err(CoreError::AssetNotRepresentative);
        }
        let current_polygon: Vec<PointNorm> =
            serde_json::from_str(&current_polygon_json).map_err(|_| CoreError::Database)?;
        if current_polygon != expected_polygon_norm {
            return Err(CoreError::RegionGeometryConflict);
        }

        let labels = read_region_labels(&transaction, region_id)?;
        let labels_json = labels_json(&labels)?;
        let updated_at = now_millis();
        transaction.execute(
            "UPDATE regions SET polygon_norm_json = ?1, updated_at = ?2 WHERE region_id = ?3",
            params![new_polygon_json, updated_at, region_id],
        )?;
        insert_region_operation(
            &transaction,
            "geometry_refined",
            region_id,
            None,
            Some(&current_polygon_json),
            Some(&labels_json),
            Some(&new_polygon_json),
            Some(&labels_json),
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

    pub(crate) fn complete_case_backup_with_fault(
        &self,
        case_id: &str,
        fault: BackupFault,
    ) -> CoreResult<BackupResult> {
        if !is_opaque_id(case_id, "CASE-") {
            return Err(CoreError::InvalidOpaqueId);
        }
        let _publication = self
            .backup_publish
            .lock()
            .map_err(|_| CoreError::StateUnavailable)?;

        // A backup is a recoverable convenience copy. The live project database remains the
        // only startup and instant-save archive, so this function never reads a backup back into
        // the primary database.
        self.recover_interrupted_backup_publish()?;
        let (fixed_template, template_bytes) = self
            .fixed_template_with_contents()?
            .ok_or(CoreError::FixedTemplateMissing)?;
        let completed_at = now_millis();
        let staging = Path::new(BACKUP_STAGING_DIRECTORY);
        self.create_empty_backup_staging_directory()?;

        let publish_result = (|| -> CoreResult<BackupResult> {
            let database_relative = staging.join(DATABASE_FILE);
            let captures = self.create_online_backup_snapshot(&database_relative)?;

            self.atomic_write_relative(&staging.join("label-template-v1.toml"), &template_bytes)?;
            for capture in &captures {
                if !is_opaque_id(&capture.case_id, "CASE-")
                    || !is_opaque_id(&capture.asset_id, "ASSET-")
                {
                    return Err(CoreError::Database);
                }
                let capture_relative = PathBuf::from(format!(
                    "captures/{}/FRAME-{}.png",
                    capture.case_id, capture.asset_id
                ));
                let bytes = self.read_relative_required(&capture_relative)?;
                self.atomic_write_relative(&staging.join(&capture_relative), &bytes)?;
            }
            let manifest = BackupManifest {
                format: BACKUP_FORMAT.to_owned(),
                version: BACKUP_VERSION,
                completed_case_id: case_id.to_owned(),
                completed_at,
                template_content_sha256: fixed_template.content_sha256,
                capture_asset_ids: captures
                    .iter()
                    .map(|capture| capture.asset_id.clone())
                    .collect(),
            };
            let manifest_bytes =
                serde_json::to_vec(&manifest).map_err(|_| CoreError::StateUnavailable)?;
            // The manifest is written only after every file has been made durable in staging.
            self.atomic_write_relative(&staging.join("manifest.json"), &manifest_bytes)?;
            self.publish_current_backup(fault)?;
            Ok(BackupResult {
                case_id: case_id.to_owned(),
                completed_at,
                capture_count: captures.len(),
            })
        })();

        if publish_result.is_err() {
            // A failed staging write cannot affect `backups/current`. Cleanup is best effort: an
            // unsafe path is intentionally left in place rather than followed or removed.
            let _ = self.remove_backup_directory_if_exists(Path::new(BACKUP_STAGING_DIRECTORY));
        }
        publish_result
    }

    /// Builds the backup database with SQLite's online backup API while the application's single
    /// connection is locked. The returned capture list and the copied database therefore describe
    /// the same committed SQLite state; capture files are immutable after their row is committed.
    fn create_online_backup_snapshot(
        &self,
        database_relative: &Path,
    ) -> CoreResult<Vec<CapturedFrame>> {
        let staging_parent = self.safe_output_parent(database_relative)?;
        let snapshot_paths = DatabasePaths::in_work_root(&staging_parent);
        prepare_database_artifacts(&snapshot_paths)?;
        self.verify_database_artifacts_safe()?;
        let connection = self
            .connection
            .lock()
            .map_err(|_| CoreError::StateUnavailable)?;
        let captures = captured_frames_from_connection(&connection)?;
        let mut destination = open_database_no_follow(&snapshot_paths.main)?;
        let backup = Backup::new(&connection, &mut destination)?;
        backup.run_to_completion(64, Duration::from_millis(5), None)?;
        drop(backup);
        destination.close().map_err(|_| CoreError::Database)?;
        drop(connection);

        // SQLite has closed its writer before this explicit file flush. The directory flush at
        // publication time then makes both the database entry and its contents durable.
        OpenOptions::new()
            .read(true)
            .open(&snapshot_paths.main)?
            .sync_all()?;

        // A completed online backup is self-contained in the main database file. Reserve the
        // sibling paths while SQLite opens the destination, then remove those empty sidecars so
        // the published snapshot is unambiguous and cannot inherit a stale WAL.
        for sidecar in [DATABASE_JOURNAL_FILE, DATABASE_WAL_FILE, DATABASE_SHM_FILE] {
            self.remove_relative_if_exists(&database_relative.with_file_name(sidecar))?;
        }
        Ok(captures)
    }

    /// Creates one fixed, application-owned staging directory. It never enumerates or removes
    /// legacy `backup-*` directories created by earlier releases.
    fn create_empty_backup_staging_directory(&self) -> CoreResult<()> {
        let backup_parent = self.backup_parent()?;
        let staging = backup_parent.join(".current-staging");
        self.remove_backup_directory_if_exists(Path::new(BACKUP_STAGING_DIRECTORY))?;
        match fs::create_dir(&staging) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                return Err(CoreError::UnsafeDerivedPath)
            }
            Err(_) => return Err(CoreError::Filesystem),
        }
        self.validate_backup_directory(&staging, &backup_parent)?;
        self.sync_backup_parent(&backup_parent)
    }

    /// Restores a prior `current` directory if a process stopped after moving it aside but before
    /// promoting staging. A directory name alone is never treated as authority: the manifest,
    /// fixed template, SQLite snapshot, and capture set must form a complete self-contained
    /// snapshot before stale publication state can be removed.
    fn recover_interrupted_backup_publish(&self) -> CoreResult<()> {
        let backup_parent = self.backup_parent()?;
        let current = backup_parent.join("current");
        let previous = backup_parent.join(".current-previous");
        let staging = backup_parent.join(".current-staging");
        let current_exists = self.backup_directory_exists(&current, &backup_parent)?;
        let previous_exists = self.backup_directory_exists(&previous, &backup_parent)?;
        let staging_exists = self.backup_directory_exists(&staging, &backup_parent)?;

        let current_valid =
            current_exists.then(|| self.validate_backup_snapshot(&current, &backup_parent));
        let previous_valid =
            previous_exists.then(|| self.validate_backup_snapshot(&previous, &backup_parent));

        match (current_valid, previous_valid) {
            (Some(Ok(())), _) => {
                if previous_exists {
                    self.remove_backup_directory_if_exists(Path::new(BACKUP_PREVIOUS_DIRECTORY))?;
                }
                if staging_exists {
                    self.remove_backup_directory_if_exists(Path::new(BACKUP_STAGING_DIRECTORY))?;
                }
                Ok(())
            }
            // `current` is application-owned and invalid while the previous snapshot has
            // independently passed the full manifest/database/capture validation. Remove only
            // that invalid app-owned directory, then advance the validated previous snapshot
            // back to `current`. This never reads backup contents into the live SQLite store.
            (Some(Err(_)), Some(Ok(()))) => {
                self.remove_backup_directory_if_exists(Path::new("backups/current"))?;
                self.rename_backup_directory(&previous, &current, &backup_parent)?;
                if staging_exists {
                    self.remove_backup_directory_if_exists(Path::new(BACKUP_STAGING_DIRECTORY))?;
                }
                Ok(())
            }
            // Backup snapshots are never part of the live project-open path. If no validated
            // predecessor exists, preserve the invalid application-owned directories for a
            // later explicit repair, but keep the primary SQLite archive available to doctors.
            // A future backup attempt will validate `current` before publication and surface the
            // backup problem without ever reading it into the live store.
            (Some(Err(_)), _) | (None, Some(Err(_))) => Ok(()),
            (None, Some(Ok(()))) => {
                self.sync_backup_parent(&backup_parent)?;
                self.rename_backup_directory(&previous, &current, &backup_parent)?;
                self.sync_backup_parent(&backup_parent)?;
                if staging_exists {
                    self.remove_backup_directory_if_exists(Path::new(BACKUP_STAGING_DIRECTORY))?;
                }
                Ok(())
            }
            (None, None) => {
                if staging_exists {
                    self.remove_backup_directory_if_exists(Path::new(BACKUP_STAGING_DIRECTORY))?;
                }
                Ok(())
            }
        }
    }

    /// Promotes fully written staging only after its manifest exists. The old snapshot is moved
    /// aside first and restored if promotion fails, so a failed publish cannot silently destroy a
    /// previously valid `backups/current` snapshot.
    fn publish_current_backup(&self, fault: BackupFault) -> CoreResult<()> {
        let backup_parent = self.backup_parent()?;
        let current = backup_parent.join("current");
        let previous = backup_parent.join(".current-previous");
        let staging = backup_parent.join(".current-staging");
        if !self.backup_directory_exists(&staging, &backup_parent)? {
            return Err(CoreError::Filesystem);
        }
        // All files are individually synced while they are created. Flush the directory tree
        // after the manifest is written, before changing the public `current` pointer.
        self.validate_backup_snapshot(&staging, &backup_parent)?;
        self.sync_backup_tree(&staging, &backup_parent)?;
        if self.backup_directory_exists(&previous, &backup_parent)? {
            return Err(CoreError::StateUnavailable);
        }

        let moved_current = self.backup_directory_exists(&current, &backup_parent)?;
        if moved_current {
            self.validate_backup_snapshot(&current, &backup_parent)?;
            self.sync_backup_parent(&backup_parent)?;
            self.rename_backup_directory(&current, &previous, &backup_parent)?;
            // A failed directory flush is an explicit failed publication. Leave the previous
            // snapshot and staging directory in place rather than risking an unflushed rollback.
            self.sync_backup_parent(&backup_parent)?;
        }
        if fault == BackupFault::BeforeCurrentPromotion {
            if moved_current {
                self.sync_backup_parent(&backup_parent)?;
                self.rename_backup_directory(&previous, &current, &backup_parent)?;
                self.sync_backup_parent(&backup_parent)?;
            }
            return Err(CoreError::FaultInjected);
        }

        self.sync_backup_parent(&backup_parent)?;
        if let Err(error) = self.rename_backup_directory(&staging, &current, &backup_parent) {
            if moved_current {
                // Preserve the old completed snapshot when staging cannot be promoted. A failed
                // rollback is surfaced rather than pretending that either snapshot is current.
                self.sync_backup_parent(&backup_parent)?;
                self.rename_backup_directory(&previous, &current, &backup_parent)?;
                self.sync_backup_parent(&backup_parent)?;
            }
            return Err(error);
        }

        // Keep `.current-previous` until the promoted directory entry itself is durable. If
        // this fails, both snapshots remain available for the safe recovery decision above.
        self.sync_backup_parent(&backup_parent)?;

        if moved_current {
            self.remove_backup_directory_if_exists(Path::new(BACKUP_PREVIOUS_DIRECTORY))?;
        }
        Ok(())
    }

    fn backup_parent(&self) -> CoreResult<PathBuf> {
        let mut current = self.work_root.clone();
        self.ensure_safe_directory(&current)?;
        for component in Path::new(BACKUP_DIRECTORY).components() {
            let Component::Normal(name) = component else {
                return Err(CoreError::InvalidWorkPath);
            };
            current.push(name);
            self.ensure_safe_directory(&current)?;
        }
        self.resolved_parent_within_work_root(&current)
    }

    fn backup_directory_exists(&self, path: &Path, backup_parent: &Path) -> CoreResult<bool> {
        match fs::symlink_metadata(path) {
            Ok(_) => {
                self.validate_backup_directory(path, backup_parent)?;
                Ok(true)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(_) => Err(CoreError::Filesystem),
        }
    }

    fn validate_backup_directory(&self, path: &Path, backup_parent: &Path) -> CoreResult<()> {
        let metadata = fs::symlink_metadata(path)?;
        if is_link_or_reparse(&metadata) || !metadata.file_type().is_dir() {
            return Err(CoreError::UnsafeDerivedPath);
        }
        let resolved = fs::canonicalize(path)?;
        if !resolved.starts_with(backup_parent) {
            return Err(CoreError::UnsafeDerivedPath);
        }
        Ok(())
    }

    fn validate_backup_snapshot(&self, backup_root: &Path, backup_parent: &Path) -> CoreResult<()> {
        self.validate_backup_directory(backup_root, backup_parent)?;
        let manifest_bytes =
            self.read_backup_regular_file(&backup_root.join("manifest.json"), backup_root)?;
        let manifest: BackupManifest =
            serde_json::from_slice(&manifest_bytes).map_err(|_| CoreError::StateUnavailable)?;
        self.validate_backup_manifest(&manifest)?;

        let template_bytes = self
            .read_backup_regular_file(&backup_root.join("label-template-v1.toml"), backup_root)?;
        if sha256_hex(&template_bytes) != manifest.template_content_sha256
            || std::str::from_utf8(&template_bytes)
                .ok()
                .and_then(|input| LabelTemplate::parse_toml_for_import(input).ok())
                .is_none()
        {
            return Err(CoreError::StateUnavailable);
        }

        self.validate_backup_database_sidecars_safe(backup_root)?;
        let database_path = backup_root.join(DATABASE_FILE);
        self.validate_backup_regular_file(&database_path, backup_root)?;
        let connection = open_backup_database_read_only(&database_path)
            .map_err(|_| CoreError::StateUnavailable)?;
        self.validate_backup_database(&connection, backup_root, backup_parent, &manifest)?;
        drop(connection);
        self.validate_backup_database_sidecars_safe(backup_root)?;
        Ok(())
    }

    fn validate_backup_manifest(&self, manifest: &BackupManifest) -> CoreResult<()> {
        let capture_ids = manifest.capture_asset_ids.iter().collect::<BTreeSet<_>>();
        if manifest.format != BACKUP_FORMAT
            || manifest.version != BACKUP_VERSION
            || !is_opaque_id(&manifest.completed_case_id, "CASE-")
            || manifest.completed_at < 0
            || !is_sha256_hex(&manifest.template_content_sha256)
            || capture_ids.len() != manifest.capture_asset_ids.len()
            || manifest
                .capture_asset_ids
                .iter()
                .any(|asset_id| !is_opaque_id(asset_id, "ASSET-"))
        {
            return Err(CoreError::StateUnavailable);
        }
        Ok(())
    }

    fn validate_backup_database_sidecars_safe(&self, backup_root: &Path) -> CoreResult<()> {
        for sidecar in [DATABASE_JOURNAL_FILE, DATABASE_WAL_FILE, DATABASE_SHM_FILE] {
            let path = backup_root.join(sidecar);
            match fs::symlink_metadata(&path) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Ok(metadata) if is_link_or_reparse(&metadata) => {
                    return Err(CoreError::UnsafeDerivedPath)
                }
                // Old releases may leave an inert regular sidecar after an external inspection.
                // The immutable URI below ignores it and never creates or follows it.
                Ok(metadata) if metadata.file_type().is_file() => {}
                Ok(_) => return Err(CoreError::StateUnavailable),
                Err(_) => return Err(CoreError::Filesystem),
            }
        }
        Ok(())
    }

    fn validate_backup_database(
        &self,
        connection: &Connection,
        backup_root: &Path,
        backup_parent: &Path,
        manifest: &BackupManifest,
    ) -> CoreResult<()> {
        let completed_case_exists: Option<i64> = connection
            .query_row(
                "SELECT 1 FROM cases WHERE case_id = ?1",
                params![manifest.completed_case_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|_| CoreError::StateUnavailable)?;
        if completed_case_exists.is_none() {
            return Err(CoreError::StateUnavailable);
        }

        let captures =
            captured_frames_from_connection(connection).map_err(|_| CoreError::StateUnavailable)?;
        let manifest_capture_ids = manifest
            .capture_asset_ids
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>();
        let database_capture_ids = captures
            .iter()
            .map(|capture| capture.asset_id.clone())
            .collect::<BTreeSet<_>>();
        if database_capture_ids.len() != captures.len()
            || database_capture_ids != manifest_capture_ids
        {
            return Err(CoreError::StateUnavailable);
        }

        for capture in &captures {
            if !is_opaque_id(&capture.case_id, "CASE-")
                || !is_opaque_id(&capture.asset_id, "ASSET-")
                || !is_opaque_id(&capture.parent_video_asset_id, "ASSET-")
                || capture.width == 0
                || capture.height == 0
                || capture.capture_time_millis < 0
            {
                return Err(CoreError::StateUnavailable);
            }
            let captures_root = backup_root.join("captures");
            self.validate_backup_directory(&captures_root, backup_parent)?;
            let case_root = captures_root.join(&capture.case_id);
            self.validate_backup_directory(&case_root, backup_parent)?;
            let capture_path = case_root.join(format!("FRAME-{}.png", capture.asset_id));
            let bytes = self.read_backup_regular_file(&capture_path, &case_root)?;
            if bytes.len() as u64 != capture.size_bytes
                || validate_capture_png(&bytes, capture.width, capture.height).is_err()
            {
                return Err(CoreError::StateUnavailable);
            }
        }

        let mut integrity_statement = connection
            .prepare("PRAGMA integrity_check")
            .map_err(|_| CoreError::StateUnavailable)?;
        let integrity_rows = integrity_statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|_| CoreError::StateUnavailable)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| CoreError::StateUnavailable)?;
        if integrity_rows.len() != 1 || integrity_rows.first().map(String::as_str) != Some("ok") {
            return Err(CoreError::StateUnavailable);
        }
        let mut foreign_key_statement = connection
            .prepare("PRAGMA foreign_key_check")
            .map_err(|_| CoreError::StateUnavailable)?;
        let mut foreign_key_rows = foreign_key_statement
            .query([])
            .map_err(|_| CoreError::StateUnavailable)?;
        if foreign_key_rows
            .next()
            .map_err(|_| CoreError::StateUnavailable)?
            .is_some()
        {
            return Err(CoreError::StateUnavailable);
        }
        Ok(())
    }

    fn read_backup_regular_file(&self, path: &Path, parent: &Path) -> CoreResult<Vec<u8>> {
        self.validate_backup_regular_file(path, parent)?;
        fs::read(path).map_err(Into::into)
    }

    fn validate_backup_regular_file(&self, path: &Path, parent: &Path) -> CoreResult<()> {
        self.validate_backup_directory(parent, parent)?;
        let metadata = fs::symlink_metadata(path)?;
        if is_link_or_reparse(&metadata) {
            return Err(CoreError::UnsafeDerivedPath);
        }
        if !metadata.file_type().is_file() {
            return Err(CoreError::StateUnavailable);
        }
        Ok(())
    }

    #[cfg(not(windows))]
    fn sync_backup_tree(&self, path: &Path, backup_parent: &Path) -> CoreResult<()> {
        self.validate_backup_directory(path, backup_parent)?;
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            let child = entry.path();
            let metadata = fs::symlink_metadata(&child)?;
            if is_link_or_reparse(&metadata) {
                return Err(CoreError::UnsafeDerivedPath);
            }
            if metadata.file_type().is_dir() {
                self.sync_backup_tree(&child, backup_parent)?;
            } else if !metadata.file_type().is_file() {
                return Err(CoreError::UnsafeDerivedPath);
            }
        }
        sync_directory(path)
    }

    #[cfg(windows)]
    fn sync_backup_tree(&self, _: &Path, _: &Path) -> CoreResult<()> {
        // FlushFileBuffers does not provide a reliable writable directory handle on Windows.
        // Snapshot files are synced individually; MoveFileExW(...WRITE_THROUGH) durably
        // publishes the completed directory name below.
        Ok(())
    }

    #[cfg(not(windows))]
    fn sync_backup_parent(&self, backup_parent: &Path) -> CoreResult<()> {
        self.validate_backup_directory(backup_parent, backup_parent)?;
        sync_directory(backup_parent)
    }

    #[cfg(windows)]
    fn sync_backup_parent(&self, _: &Path) -> CoreResult<()> {
        // See sync_backup_tree: Windows publication uses MoveFileExW WRITE_THROUGH instead of
        // flushing directory handles, which can fail with access denied on normal installations.
        Ok(())
    }

    fn rename_backup_directory(
        &self,
        source: &Path,
        destination: &Path,
        backup_parent: &Path,
    ) -> CoreResult<()> {
        self.validate_backup_directory(source, backup_parent)?;
        match fs::symlink_metadata(destination) {
            Ok(_) => return Err(CoreError::UnsafeDerivedPath),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(CoreError::Filesystem),
        }
        #[cfg(unix)]
        {
            fs::rename(source, destination)?;
            self.sync_backup_parent(backup_parent)?;
        }
        #[cfg(windows)]
        move_backup_directory_windows(source, destination)?;
        #[cfg(not(any(unix, windows)))]
        fs::rename(source, destination)?;
        self.validate_backup_directory(destination, backup_parent)
    }

    fn remove_backup_directory_if_exists(&self, relative: &Path) -> CoreResult<()> {
        let backup_parent = self.backup_parent()?;
        let path = self.backup_directory_path(relative, &backup_parent)?;
        match fs::symlink_metadata(&path) {
            Ok(_) => {
                self.remove_owned_backup_tree(&path, &backup_parent)?;
                self.sync_backup_parent(&backup_parent)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(_) => Err(CoreError::Filesystem),
        }
    }

    fn backup_directory_path(&self, relative: &Path, backup_parent: &Path) -> CoreResult<PathBuf> {
        let expected_parent = Path::new(BACKUP_DIRECTORY);
        let file_name = relative
            .strip_prefix(expected_parent)
            .ok()
            .and_then(|suffix| {
                let mut components = suffix.components();
                let Component::Normal(name) = components.next()? else {
                    return None;
                };
                components.next().is_none().then_some(name)
            })
            .ok_or(CoreError::InvalidWorkPath)?;
        Ok(backup_parent.join(file_name))
    }

    fn remove_owned_backup_tree(&self, path: &Path, backup_parent: &Path) -> CoreResult<()> {
        self.validate_backup_directory(path, backup_parent)?;
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            let child = entry.path();
            let metadata = fs::symlink_metadata(&child)?;
            if is_link_or_reparse(&metadata) {
                return Err(CoreError::UnsafeDerivedPath);
            }
            if metadata.file_type().is_dir() {
                self.remove_owned_backup_tree(&child, backup_parent)?;
            } else if metadata.file_type().is_file() {
                fs::remove_file(child)?;
            } else {
                return Err(CoreError::UnsafeDerivedPath);
            }
        }
        fs::remove_dir(path)?;
        Ok(())
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
                        template: load_template_snapshot_from_connection(
                            &connection,
                            &template_id,
                            template_version,
                        )?
                        .ok_or(CoreError::Database)?,
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

    pub(crate) fn load_template_snapshot(
        &self,
        template_id: &str,
        version: u32,
    ) -> CoreResult<Option<LabelTemplate>> {
        self.verify_database_artifacts_safe()?;
        let connection = self
            .connection
            .lock()
            .map_err(|_| CoreError::StateUnavailable)?;
        load_template_snapshot_from_connection(&connection, template_id, version)
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

    fn fixed_template_with_contents(&self) -> CoreResult<Option<(FixedTemplate, Vec<u8>)>> {
        if let Some(manifest) = self.read_active_template_manifest()? {
            self.ensure_active_template_storage_marker()?;
            let relative = active_template_archive_path(&manifest)?;
            let contents = self
                .read_relative_optional(&relative)?
                .ok_or(CoreError::FixedTemplateMissing)?;
            return fixed_template_from_bytes(&manifest, &contents)
                .map(|fixed| Some((fixed, contents)));
        }

        if self.active_template_storage_marker_exists()? {
            return Err(CoreError::FixedTemplateMissing);
        }

        let manifest = self.read_fixed_template_manifest()?;
        let contents = self.read_relative_optional(Path::new(LEGACY_FIXED_TEMPLATE_FILE))?;
        match (manifest, contents) {
            (None, None) => Ok(None),
            (Some(manifest), Some(contents)) => {
                fixed_template_from_bytes(&manifest, &contents).map(|fixed| Some((fixed, contents)))
            }
            _ => Err(CoreError::FixedTemplateMissing),
        }
    }

    fn prepare_template_archive(&self, contents: &[u8], fixed: &FixedTemplate) -> CoreResult<()> {
        let archive_relative = template_archive_relative_path(&fixed.content_sha256);
        match self.read_relative_optional(&archive_relative)? {
            Some(existing) if existing != contents => {
                return Err(CoreError::FixedTemplateChecksumMismatch)
            }
            Some(_) => {}
            None => {
                self.atomic_write_relative(&archive_relative, contents)?;
            }
        }
        Ok(())
    }

    fn activate_template_archive(&self, fixed: &FixedTemplate) -> CoreResult<FixedTemplate> {
        let archive_relative = template_archive_relative_path(&fixed.content_sha256);
        let contents = self
            .read_relative_optional(&archive_relative)?
            .ok_or(CoreError::FixedTemplateMissing)?;
        let archive_fixed = fixed_template_from_input(
            std::str::from_utf8(&contents).map_err(|_| CoreError::FixedTemplateChecksumMismatch)?,
            &fixed.template,
            fixed.previous_content_sha256.clone(),
        )?;
        if archive_fixed.content_sha256 != fixed.content_sha256
            || archive_fixed.template_sha256 != fixed.template_sha256
        {
            return Err(CoreError::FixedTemplateChecksumMismatch);
        }

        let marker_created = self.ensure_active_template_storage_marker()?;
        let manifest = FixedTemplateManifest {
            format: "gwp-fixed-template".to_owned(),
            content_sha256: fixed.content_sha256.clone(),
            template_sha256: fixed.template_sha256.clone(),
            previous_content_sha256: fixed.previous_content_sha256.clone(),
            template_path: Some(
                archive_relative
                    .to_str()
                    .ok_or(CoreError::StateUnavailable)?
                    .to_owned(),
            ),
        };
        let manifest_bytes =
            serde_json::to_vec(&manifest).map_err(|_| CoreError::StateUnavailable)?;
        // The immutable content is durable before this single atomic pointer is advanced.
        if let Err(error) =
            self.atomic_write_relative(Path::new(ACTIVE_TEMPLATE_MANIFEST_FILE), &manifest_bytes)
        {
            if marker_created {
                let _ =
                    self.remove_relative_if_exists(Path::new(ACTIVE_TEMPLATE_STORAGE_MARKER_FILE));
            }
            return Err(error);
        }
        Ok(fixed.clone())
    }

    fn prepare_missing_template_copy(
        &self,
        contents: &[u8],
        mut candidate: FixedTemplate,
    ) -> CoreResult<PreparedFixedTemplate> {
        if let Some(manifest) = self.read_active_template_manifest()? {
            if manifest.content_sha256 != candidate.content_sha256
                || manifest.template_sha256 != candidate.template_sha256
            {
                return Err(CoreError::FixedTemplateChecksumMismatch);
            }
            let mut candidate = candidate;
            candidate.previous_content_sha256 = manifest.previous_content_sha256;
            self.prepare_template_archive(contents, &candidate)?;
            return Ok(PreparedFixedTemplate {
                fixed: candidate,
                needs_activation: true,
            });
        }
        if self.active_template_storage_marker_exists()? {
            let session = Self::read_project_session(&self.work_root)?
                .ok_or(CoreError::FixedTemplateMissing)?;
            let expected_content_sha256 = session
                .pending_template_content_sha256
                .as_ref()
                .or(session.template_content_sha256.as_ref())
                .ok_or(CoreError::FixedTemplateMissing)?;
            if expected_content_sha256 != &candidate.content_sha256 {
                return Err(CoreError::FixedTemplateChecksumMismatch);
            }
            candidate.previous_content_sha256 = session
                .template_content_sha256
                .filter(|sha256| sha256 != &candidate.content_sha256);
            self.prepare_template_archive(contents, &candidate)?;
            return Ok(PreparedFixedTemplate {
                fixed: candidate,
                needs_activation: true,
            });
        }
        if let Some(manifest) = self.read_fixed_template_manifest()? {
            if manifest.content_sha256 != candidate.content_sha256
                || manifest.template_sha256 != candidate.template_sha256
            {
                return Err(CoreError::FixedTemplateChecksumMismatch);
            }
            self.prepare_template_archive(contents, &candidate)?;
            return Ok(PreparedFixedTemplate {
                fixed: candidate,
                needs_activation: true,
            });
        }
        Err(CoreError::FixedTemplateMissing)
    }

    fn ensure_replacement_identity_is_available(
        &self,
        candidate: &FixedTemplate,
    ) -> CoreResult<()> {
        self.verify_database_artifacts_safe()?;
        let connection = self
            .connection
            .lock()
            .map_err(|_| CoreError::StateUnavailable)?;
        let existing_sha = connection
            .query_row(
                "SELECT sha256 FROM templates WHERE template_id = ?1 AND version = ?2",
                params![candidate.template.template_id, candidate.template.version],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        if existing_sha.is_some_and(|sha| sha != candidate.template_sha256) {
            return Err(CoreError::TemplateReplacementIdentityConflict);
        }
        Ok(())
    }

    fn read_active_template_manifest(&self) -> CoreResult<Option<FixedTemplateManifest>> {
        self.read_template_manifest(Path::new(ACTIVE_TEMPLATE_MANIFEST_FILE))
    }

    fn active_template_storage_marker_exists(&self) -> CoreResult<bool> {
        match self.read_relative_optional(Path::new(ACTIVE_TEMPLATE_STORAGE_MARKER_FILE))? {
            None => Ok(false),
            Some(bytes) if bytes == ACTIVE_TEMPLATE_STORAGE_MARKER => Ok(true),
            Some(_) => Err(CoreError::FixedTemplateMissing),
        }
    }

    /// Returns whether this call created the durable no-legacy-fallback marker.
    fn ensure_active_template_storage_marker(&self) -> CoreResult<bool> {
        if self.active_template_storage_marker_exists()? {
            return Ok(false);
        }
        self.atomic_write_relative(
            Path::new(ACTIVE_TEMPLATE_STORAGE_MARKER_FILE),
            ACTIVE_TEMPLATE_STORAGE_MARKER,
        )?;
        Ok(true)
    }

    fn read_fixed_template_manifest(&self) -> CoreResult<Option<FixedTemplateManifest>> {
        self.read_template_manifest(Path::new(LEGACY_FIXED_TEMPLATE_MANIFEST_FILE))
    }

    fn read_template_manifest(&self, relative: &Path) -> CoreResult<Option<FixedTemplateManifest>> {
        let Some(bytes) = self.read_relative_optional(relative)? else {
            return Ok(None);
        };
        let manifest: FixedTemplateManifest =
            serde_json::from_slice(&bytes).map_err(|_| CoreError::FixedTemplateMissing)?;
        if manifest.format != "gwp-fixed-template"
            || !is_sha256_hex(&manifest.content_sha256)
            || !is_sha256_hex(&manifest.template_sha256)
            || manifest
                .previous_content_sha256
                .as_deref()
                .is_some_and(|value| !is_sha256_hex(value))
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
}

fn installation_storage_parent(work_root: &Path) -> Option<&Path> {
    let storage_parent = work_root.parent()?;
    let work_root_name = work_root.file_name()?.to_str()?;
    let storage_parent_name = storage_parent.file_name()?.to_str()?;
    (work_root_name == INSTALLATION_WORK_ROOT_DIRECTORY
        && storage_parent_name == INSTALLATION_STORAGE_PARENT_DIRECTORY)
        .then_some(storage_parent)
}

/// Inspect only components that this application owns. In particular, do not walk arbitrary
/// ancestors here: on macOS `/var` is often a legitimate symlink, while on Windows the two
/// installation-adjacent storage components must be rejected before they can be followed as a
/// junction or another reparse point.
fn validate_existing_work_root_components(work_root: &Path) -> CoreResult<()> {
    if let Some(storage_parent) = installation_storage_parent(work_root) {
        validate_existing_work_root_directory(storage_parent)?;
    }
    validate_existing_work_root_directory(work_root)
}

fn validate_existing_work_root_directory(path: &Path) -> CoreResult<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => validate_work_root_directory_metadata(&metadata),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(CoreError::WorkRootUnavailable),
    }
}

fn validate_work_root_directory_metadata(metadata: &fs::Metadata) -> CoreResult<()> {
    if is_link_or_reparse(metadata) {
        return Err(CoreError::UnsafeDerivedPath);
    }
    if !metadata.file_type().is_dir() {
        return Err(CoreError::WorkRootUnavailable);
    }
    Ok(())
}

/// Called only from `Store::open`, which the service invokes after it has checked that the source
/// root and this work root do not overlap. It is deliberately the first point that may create,
/// probe, or otherwise mutate the work root.
fn prepare_work_root_after_overlap_check(requested_work_root: &Path) -> CoreResult<PathBuf> {
    validate_existing_work_root_components(requested_work_root)?;

    if let Some(storage_parent) = installation_storage_parent(requested_work_root) {
        // The executable's installation parent already exists. Create the application-owned
        // components one at a time so `create_dir_all` cannot follow a newly inserted junction.
        ensure_owned_work_root_directory(storage_parent)?;
        validate_existing_work_root_directory(storage_parent)?;
        ensure_owned_work_root_directory(requested_work_root)?;
    } else {
        // Non-Windows test and development roots retain the existing recursive behavior. Their
        // final component is still checked above and again below; this intentionally avoids
        // rejecting legitimate system-owned ancestors such as macOS `/var`.
        ensure_generic_work_root_directory(requested_work_root)?;
    }

    validate_existing_work_root_components(requested_work_root)?;
    let resolved_work_root = resolve_existing_work_root_path(requested_work_root)?;
    probe_work_root_writable(&resolved_work_root)?;
    Ok(resolved_work_root)
}

fn ensure_owned_work_root_directory(path: &Path) -> CoreResult<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => validate_work_root_directory_metadata(&metadata),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            match fs::create_dir(path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(_) => return Err(CoreError::WorkRootUnavailable),
            }
            let metadata =
                fs::symlink_metadata(path).map_err(|_| CoreError::WorkRootUnavailable)?;
            validate_work_root_directory_metadata(&metadata)
        }
        Err(_) => Err(CoreError::WorkRootUnavailable),
    }
}

fn ensure_generic_work_root_directory(path: &Path) -> CoreResult<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => validate_work_root_directory_metadata(&metadata),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir_all(path).map_err(|_| CoreError::WorkRootUnavailable)?;
            let metadata =
                fs::symlink_metadata(path).map_err(|_| CoreError::WorkRootUnavailable)?;
            validate_work_root_directory_metadata(&metadata)
        }
        Err(_) => Err(CoreError::WorkRootUnavailable),
    }
}

fn resolve_existing_work_root_path(work_root: &Path) -> CoreResult<PathBuf> {
    let requested_work_root = if work_root.is_absolute() {
        work_root.to_path_buf()
    } else {
        std::env::current_dir()?.join(work_root)
    };
    validate_existing_work_root_components(&requested_work_root)?;
    let storage_parent = installation_storage_parent(&requested_work_root).map(Path::to_path_buf);
    let mut candidate = requested_work_root.clone();
    let mut missing_components = Vec::new();

    loop {
        if candidate == requested_work_root
            || storage_parent
                .as_deref()
                .is_some_and(|storage_parent| candidate == storage_parent)
        {
            // Check immediately before canonicalization so a junction inserted after an earlier
            // check cannot redirect a lexical D: path to C: and be accepted.
            validate_existing_work_root_directory(&candidate)?;
        }
        match fs::canonicalize(&candidate) {
            Ok(mut resolved) => {
                ensure_same_windows_volume(&requested_work_root, &resolved)?;
                for component in missing_components.iter().rev() {
                    resolved.push(component);
                }
                return Ok(resolved);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let component = candidate
                    .file_name()
                    .ok_or(CoreError::WorkRootUnavailable)?
                    .to_os_string();
                missing_components.push(component);
                candidate = candidate
                    .parent()
                    .ok_or(CoreError::WorkRootUnavailable)?
                    .to_path_buf();
            }
            Err(_) => return Err(CoreError::WorkRootUnavailable),
        }
    }
}

fn ensure_same_windows_volume(requested: &Path, resolved: &Path) -> CoreResult<()> {
    #[cfg(windows)]
    {
        if let (Some(requested_volume), Some(resolved_volume)) = (
            windows_volume_letter(requested),
            windows_volume_letter(resolved),
        ) {
            if requested_volume != resolved_volume {
                return Err(CoreError::UnsafeDerivedPath);
            }
        }
    }
    #[cfg(not(windows))]
    {
        let _ = (requested, resolved);
    }
    Ok(())
}

#[cfg(windows)]
fn windows_volume_letter(path: &Path) -> Option<u8> {
    let display = path.as_os_str().to_string_lossy();
    let display = display.strip_prefix("\\\\?\\").unwrap_or(&display);
    let bytes = display.as_bytes();
    (bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic())
        .then(|| bytes[0].to_ascii_lowercase())
}

fn probe_work_root_writable(work_root: &Path) -> CoreResult<()> {
    let sequence = WORK_ROOT_PROBE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let probe = work_root.join(format!(
        ".gwp-work-root-probe-{}-{sequence}.tmp",
        std::process::id()
    ));
    let write_result = (|| -> std::io::Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&probe)?;
        file.write_all(b"gwp-work-root-probe")?;
        file.sync_all()?;
        Ok(())
    })();
    if write_result.is_err() {
        let _ = fs::remove_file(&probe);
        return Err(CoreError::WorkRootUnavailable);
    }
    fs::remove_file(&probe).map_err(|_| CoreError::WorkRootUnavailable)
}

fn read_optional_work_file(work_root: &Path, relative: &Path) -> CoreResult<Option<Vec<u8>>> {
    if !is_safe_relative_path(relative) {
        return Err(CoreError::InvalidWorkPath);
    }
    validate_existing_work_root_components(work_root)?;
    let root_metadata = match fs::symlink_metadata(work_root) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(CoreError::Filesystem),
    };
    if is_link_or_reparse(&root_metadata) || !root_metadata.file_type().is_dir() {
        return Err(CoreError::UnsafeDerivedPath);
    }
    validate_existing_work_root_components(work_root)?;
    let canonical_root = resolve_existing_work_root_path(work_root)?;
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
    validate_existing_work_root_components(work_root)?;
    let root_metadata = match fs::symlink_metadata(work_root) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(_) => return Err(CoreError::Filesystem),
    };
    if is_link_or_reparse(&root_metadata) || !root_metadata.file_type().is_dir() {
        return Err(CoreError::UnsafeDerivedPath);
    }
    validate_existing_work_root_components(work_root)?;
    let canonical_root = resolve_existing_work_root_path(work_root)?;
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
        || state
            .pending_template_content_sha256
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
        previous_content_sha256: manifest.previous_content_sha256.clone(),
        template,
    })
}

fn fixed_template_from_input(
    contents: &str,
    template: &LabelTemplate,
    previous_content_sha256: Option<String>,
) -> CoreResult<FixedTemplate> {
    let snapshot = template.snapshot()?;
    Ok(FixedTemplate {
        content_sha256: sha256_hex(contents.as_bytes()),
        template_sha256: snapshot.sha256,
        previous_content_sha256,
        template: template.clone(),
    })
}

fn template_archive_relative_path(content_sha256: &str) -> PathBuf {
    PathBuf::from(format!(
        "{TEMPLATE_ARCHIVE_DIRECTORY}/{content_sha256}.toml"
    ))
}

fn active_template_archive_path(manifest: &FixedTemplateManifest) -> CoreResult<PathBuf> {
    let relative = PathBuf::from(
        manifest
            .template_path
            .as_deref()
            .ok_or(CoreError::FixedTemplateMissing)?,
    );
    if relative != template_archive_relative_path(&manifest.content_sha256) {
        return Err(CoreError::FixedTemplateMissing);
    }
    Ok(relative)
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

fn load_template_snapshot_from_connection(
    connection: &Connection,
    template_id: &str,
    version: u32,
) -> CoreResult<Option<LabelTemplate>> {
    let stored = connection
        .query_row(
            "SELECT sha256, canonical_json FROM templates WHERE template_id = ?1 AND version = ?2",
            params![template_id, version],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()?;
    let Some((stored_sha256, canonical_json)) = stored else {
        return Ok(None);
    };
    let template = LabelTemplate::parse_json(&canonical_json).map_err(|_| CoreError::Database)?;
    let snapshot = template.snapshot().map_err(|_| CoreError::Database)?;
    if snapshot.template_id != template_id
        || snapshot.version != version
        || snapshot.sha256 != stored_sha256
        || snapshot.canonical_json != canonical_json
    {
        return Err(CoreError::Database);
    }
    Ok(Some(template))
}

fn captured_frames_from_connection(connection: &Connection) -> CoreResult<Vec<CapturedFrame>> {
    let mut statement = connection.prepare(
        "SELECT captures.captured_asset_id, captures.case_id, captures.parent_video_asset_id,
                captures.capture_time_millis, assets.width, assets.height, captures.size_bytes
         FROM capture_sources AS captures
         INNER JOIN assets ON assets.asset_id = captures.captured_asset_id
         ORDER BY captures.case_id ASC, captures.capture_time_millis ASC, captures.captured_asset_id ASC",
    )?;
    let rows = statement.query_map([], |row| {
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

/// Snapshot verification must never create a journal, WAL, or a new database. The primary
/// database uses read/write mode above; completed backups are intentionally opened read-only.
fn open_backup_database_read_only(database_path: &Path) -> CoreResult<Connection> {
    let database_uri = immutable_database_uri(database_path)?;
    Connection::open_with_flags(
        database_uri,
        OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_NOFOLLOW
            | OpenFlags::SQLITE_OPEN_URI,
    )
    .map_err(Into::into)
}

fn immutable_database_uri(database_path: &Path) -> CoreResult<String> {
    let raw_path = database_path
        .to_str()
        .ok_or(CoreError::StateUnavailable)?
        .replace('\\', "/");
    #[cfg(windows)]
    let path = {
        let mut path = raw_path;
        if !path.starts_with('/') {
            path.insert(0, '/');
        }
        path
    };
    #[cfg(not(windows))]
    let path = raw_path;
    let mut encoded = String::with_capacity(path.len());
    for byte in path.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~' | b'/' | b':') {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    Ok(format!("file:{encoded}?immutable=1"))
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> CoreResult<()> {
    fs::File::open(path)?.sync_all()?;
    Ok(())
}

#[cfg(not(any(unix, windows)))]
fn sync_directory(_: &Path) -> CoreResult<()> {
    // There is no portable directory-flush primitive outside the supported desktop targets.
    Err(CoreError::Filesystem)
}

#[cfg(windows)]
fn move_backup_directory_windows(source: &Path, destination: &Path) -> CoreResult<()> {
    use std::iter::once;
    use std::os::windows::ffi::OsStrExt;

    use windows_sys::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_WRITE_THROUGH};

    let source_wide = source
        .as_os_str()
        .encode_wide()
        .chain(once(0))
        .collect::<Vec<_>>();
    let destination_wide = destination
        .as_os_str()
        .encode_wide()
        .chain(once(0))
        .collect::<Vec<_>>();
    let moved = unsafe {
        MoveFileExW(
            source_wide.as_ptr(),
            destination_wide.as_ptr(),
            MOVEFILE_WRITE_THROUGH,
        )
    };
    if moved == 0 {
        return Err(CoreError::Filesystem);
    }
    Ok(())
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
    if version > SCHEMA_VERSION {
        return Err(CoreError::DatabaseSchemaUnsupported);
    }
    if version < 1 && !table_exists(connection, "cases")? {
        create_v4_schema(connection)?;
        return Ok(());
    } else if version < 2 {
        migrate_legacy_cases_to_v2(connection)?;
        version = 2;
    }
    if version < 3 {
        migrate_v2_to_v3(connection)?;
        version = 3;
    }
    if version < SCHEMA_VERSION {
        migrate_v3_to_v4(connection)?;
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

fn create_v4_schema(connection: &mut Connection) -> CoreResult<()> {
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
            operation_kind TEXT NOT NULL CHECK(operation_kind IN ('created', 'labels_updated', 'deleted', 'replaced', 'geometry_refined')),
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

fn migrate_v3_to_v4(connection: &mut Connection) -> CoreResult<()> {
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    transaction.execute_batch(
        "
        CREATE TABLE region_operations_v4 (
            operation_id INTEGER PRIMARY KEY,
            operation_kind TEXT NOT NULL CHECK(operation_kind IN ('created', 'labels_updated', 'deleted', 'replaced', 'geometry_refined')),
            subject_region_id TEXT NOT NULL REFERENCES regions(region_id),
            related_region_id TEXT REFERENCES regions(region_id),
            occurred_at INTEGER NOT NULL,
            before_polygon_norm_json TEXT,
            before_labels_json TEXT,
            after_polygon_norm_json TEXT,
            after_labels_json TEXT
        );
        INSERT INTO region_operations_v4 (
            operation_id,
            operation_kind,
            subject_region_id,
            related_region_id,
            occurred_at,
            before_polygon_norm_json,
            before_labels_json,
            after_polygon_norm_json,
            after_labels_json
        )
        SELECT
            operation_id,
            operation_kind,
            subject_region_id,
            related_region_id,
            occurred_at,
            before_polygon_norm_json,
            before_labels_json,
            after_polygon_norm_json,
            after_labels_json
        FROM region_operations;
        DROP TABLE region_operations;
        ALTER TABLE region_operations_v4 RENAME TO region_operations;
        ",
    )?;
    transaction.pragma_update(None, "user_version", SCHEMA_VERSION)?;
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
    use std::fs;

    use rusqlite::params;

    use crate::core::template::LabelTemplate;

    use super::{
        load_template_snapshot_from_connection, prepare_work_root_after_overlap_check,
        validate_polygon, CoreError, PointNorm, Store,
    };

    const LEGACY_DAMAGED_TOML: &str = r#"
format = "gwa-label-template"
version = 1
template_id = "legacy-template"

[[fields]]
id = "field_legacy"
label = "閫夐」"
control = "dropdown"
required = true

[[fields.options]]
id = "option_legacy"
label = "锟斤拷"
"#;

    const CLEAN_REPLACEMENT_TOML: &str = r#"
format = "gwa-label-template"
version = 2
template_id = "legacy-template"

[[fields]]
id = "field_replacement"
label = "字段甲"
control = "dropdown"
required = true

[[fields.options]]
id = "option_replacement"
label = "选项甲"
"#;

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

    #[test]
    fn work_root_preparation_creates_and_cleans_its_write_probe() {
        let temporary = tempfile::tempdir().expect("synthetic temporary directory");
        let work_root = temporary
            .path()
            .join("GWP Annotator Data")
            .join("PROJECT-LOCAL");

        let resolved =
            prepare_work_root_after_overlap_check(&work_root).expect("prepare synthetic work root");

        assert!(resolved.is_dir());
        assert!(
            fs::read_dir(&resolved)
                .expect("read prepared work root")
                .next()
                .is_none(),
            "write probe must be removed before database creation"
        );
    }

    #[test]
    fn work_root_preparation_rejects_a_non_directory_target() {
        let temporary = tempfile::tempdir().expect("synthetic temporary directory");
        let work_root = temporary
            .path()
            .join("GWP Annotator Data")
            .join("PROJECT-LOCAL");
        fs::create_dir_all(work_root.parent().expect("storage parent"))
            .expect("create synthetic storage parent");
        fs::write(&work_root, b"not a directory").expect("write synthetic target");

        assert!(matches!(
            prepare_work_root_after_overlap_check(&work_root),
            Err(CoreError::WorkRootUnavailable)
        ));
    }

    #[test]
    fn historical_damaged_templates_can_be_verified_recovered_and_replaced() {
        let temporary = tempfile::tempdir().expect("synthetic temporary directory");
        let store =
            Store::open(temporary.path().join("PROJECT-LOCAL")).expect("open synthetic store");
        let legacy = LabelTemplate::parse_toml(LEGACY_DAMAGED_TOML)
            .expect("historical template remains structurally valid");
        assert!(LabelTemplate::parse_toml_for_import(LEGACY_DAMAGED_TOML).is_err());

        let prepared = store
            .prepare_fixed_template_install(LEGACY_DAMAGED_TOML, &legacy)
            .expect("prepare legacy fixed template");
        store
            .activate_prepared_fixed_template(&prepared.fixed)
            .expect("activate legacy fixed template");
        let recovered = store
            .fixed_template()
            .expect("read verified legacy fixed template")
            .expect("legacy fixed template");
        assert_eq!(recovered.template.fields[0].label, "閫夐」");

        let snapshot = legacy.snapshot().expect("historical SQLite snapshot");
        {
            let connection = store.connection.lock().expect("store connection");
            connection
                .execute(
                    "INSERT INTO templates (template_id, version, sha256, canonical_json) VALUES (?1, ?2, ?3, ?4)",
                    params![
                        snapshot.template_id,
                        snapshot.version,
                        snapshot.sha256,
                        snapshot.canonical_json,
                    ],
                )
                .expect("insert historical SQLite snapshot");
            let loaded = load_template_snapshot_from_connection(&connection, "legacy-template", 1)
                .expect("load historical SQLite snapshot")
                .expect("historical SQLite template");
            assert_eq!(loaded.fields[0].options[0].label, "锟斤拷");
        }

        let replacement = LabelTemplate::parse_toml_for_import(CLEAN_REPLACEMENT_TOML)
            .expect("valid UTF-8 replacement");
        let prepared = store
            .prepare_fixed_template_replacement(CLEAN_REPLACEMENT_TOML, &replacement)
            .expect("prepare clean replacement");
        store
            .activate_prepared_fixed_template(&prepared.fixed)
            .expect("activate clean replacement");
        let active = store
            .fixed_template()
            .expect("read active replacement")
            .expect("active replacement");
        assert_eq!(active.template.fields[0].label, "字段甲");
    }

    #[cfg(unix)]
    #[test]
    fn work_root_preparation_rejects_a_non_writable_directory() {
        use std::os::unix::fs::PermissionsExt;

        if unsafe { libc::geteuid() } == 0 {
            return;
        }

        let temporary = tempfile::tempdir().expect("synthetic temporary directory");
        let work_root = temporary
            .path()
            .join("GWP Annotator Data")
            .join("PROJECT-LOCAL");
        fs::create_dir_all(&work_root).expect("create synthetic work root");
        let original_permissions = fs::metadata(&work_root)
            .expect("inspect synthetic work root")
            .permissions();
        let mut blocked_permissions = original_permissions.clone();
        blocked_permissions.set_mode(0o500);
        fs::set_permissions(&work_root, blocked_permissions)
            .expect("make synthetic root read-only");

        let result = prepare_work_root_after_overlap_check(&work_root);

        fs::set_permissions(&work_root, original_permissions)
            .expect("restore synthetic work root permissions");
        assert!(matches!(result, Err(CoreError::WorkRootUnavailable)));
    }
}
