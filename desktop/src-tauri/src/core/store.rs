use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{
    params, Connection, OpenFlags, OptionalExtension, Transaction, TransactionBehavior,
};
use serde::{Deserialize, Serialize};

use crate::core::error::{CoreError, CoreResult};
use crate::core::ids::is_opaque_id;
use crate::core::scan::{PersistedAsset, PersistedCase};
use crate::core::template::{LabelTemplate, LabelValue};

const DATABASE_FILE: &str = "annotations.sqlite3";
const DATABASE_JOURNAL_FILE: &str = "annotations.sqlite3-journal";
const DATABASE_WAL_FILE: &str = "annotations.sqlite3-wal";
const DATABASE_SHM_FILE: &str = "annotations.sqlite3-shm";
const INTERSECTION_EPSILON: f64 = 1e-12;
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

impl Store {
    pub(crate) fn open(work_root: impl Into<PathBuf>) -> CoreResult<Self> {
        let requested_work_root = work_root.into();
        fs::create_dir_all(&requested_work_root)?;
        let work_root = fs::canonicalize(&requested_work_root)?;
        let work_root_metadata = fs::symlink_metadata(&work_root)?;
        if is_link_or_reparse(&work_root_metadata.file_type())
            || !work_root_metadata.file_type().is_dir()
        {
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
            let count: i64 = transaction.query_row(
                "SELECT COUNT(*) FROM representative_assets WHERE case_id = ?1",
                params![case.case_id],
                |row| row.get(0),
            )?;
            if count >= 2 {
                return Err(CoreError::RepresentativeLimit);
            }
            transaction.execute(
                "INSERT INTO representative_assets (case_id, asset_id, selected_at) VALUES (?1, ?2, ?3)",
                params![case.case_id, asset.asset_id, now_millis()],
            )?;
        }

        let selected_count = update_representative_count(&transaction, &case.case_id)?;
        transaction.commit()?;
        Ok(selected_count)
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
                "SELECT 1 FROM regions WHERE asset_id = ?1 LIMIT 1",
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

        transaction.execute(
            "INSERT INTO regions (region_id, asset_id, polygon_norm_json, template_id, template_version, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                draft.region_id,
                draft.asset_id,
                polygon_json,
                draft.template.template_id,
                draft.template.version,
                now_millis(),
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
        transaction.commit()?;
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
            "regions" => "SELECT COUNT(*) FROM regions",
            "region_label_values" => "SELECT COUNT(*) FROM region_label_values",
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
        if is_link_or_reparse(&file_type) || !file_type.is_dir() {
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
                if is_link_or_reparse(&file_type) || !file_type.is_file() {
                    return Err(CoreError::UnsafeDerivedPath);
                }
                Ok(())
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(_) => Err(CoreError::Filesystem),
        }
    }
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
    if is_link_or_reparse(&file_type) || !file_type.is_file() {
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
            if is_link_or_reparse(&file_type) || !file_type.is_file() {
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

    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let version: i64 = transaction.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if version < 1 {
        transaction.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS cases (
                case_id TEXT PRIMARY KEY CHECK(case_id GLOB 'CASE-*'),
                campus_code TEXT NOT NULL CHECK(campus_code GLOB 'CAMPUS-*'),
                representative_count INTEGER NOT NULL DEFAULT 0 CHECK(representative_count BETWEEN 0 AND 2),
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
            CREATE TRIGGER IF NOT EXISTS representative_assets_limit_before_insert
            BEFORE INSERT ON representative_assets
            WHEN (SELECT COUNT(*) FROM representative_assets WHERE case_id = NEW.case_id) >= 2
            BEGIN
                SELECT RAISE(ABORT, 'representative_limit');
            END;
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
                FOREIGN KEY(template_id, template_version) REFERENCES templates(template_id, version)
            );
            CREATE TABLE IF NOT EXISTS region_label_values (
                region_id TEXT NOT NULL REFERENCES regions(region_id),
                field_id TEXT NOT NULL,
                value_json TEXT NOT NULL,
                PRIMARY KEY(region_id, field_id)
            );
            ",
        )?;
        transaction.pragma_update(None, "user_version", 1)?;
    }
    transaction.commit()?;
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
         ON CONFLICT(asset_id) DO UPDATE SET case_id = excluded.case_id, kind = excluded.kind, width = excluded.width, height = excluded.height, source_fingerprint = excluded.source_fingerprint",
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

fn is_link_or_reparse(file_type: &fs::FileType) -> bool {
    if file_type.is_symlink() {
        return true;
    }

    #[cfg(windows)]
    {
        use std::os::windows::fs::FileTypeExt;

        file_type.is_reparse_point()
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
