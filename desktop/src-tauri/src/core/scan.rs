use std::cmp::Ordering;
use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine;
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

#[cfg(unix)]
use std::ffi::CString;
#[cfg(windows)]
use std::fs::OpenOptions;
#[cfg(windows)]
use std::mem;
#[cfg(unix)]
use std::os::unix::ffi::OsStrExt;
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
#[cfg(unix)]
use std::os::unix::io::{AsRawFd, FromRawFd};
#[cfg(windows)]
use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
#[cfg(windows)]
use std::os::windows::io::AsRawHandle;
#[cfg(windows)]
use windows_sys::Win32::Foundation::{HANDLE, INVALID_HANDLE_VALUE};
#[cfg(windows)]
use windows_sys::Win32::Storage::FileSystem::{
    GetFileInformationByHandle, GetFinalPathNameByHandleW, BY_HANDLE_FILE_INFORMATION,
    FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_NAME_NORMALIZED,
};

use crate::core::error::{CoreError, CoreResult};
use crate::core::ids::{asset_id, campus_code, case_id, source_fingerprint, ProjectSecret};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MediaKind {
    Image,
    Video,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct PublicAsset {
    pub asset_id: String,
    pub media_kind: MediaKind,
    pub size_bytes: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct PublicCase {
    pub case_id: String,
    pub date: Option<String>,
    pub assets: Vec<PublicAsset>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct PublicCampus {
    pub campus_code: String,
    pub case_count: usize,
    pub cases: Vec<PublicCase>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct PublicScan {
    pub campuses: Vec<PublicCampus>,
}

/// Local-only UI data. These display fields are derived afresh from the active in-memory scan and
/// must never be written to the project database, recovery state, cache, or export artifacts.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct LocalDisplayScan {
    pub campuses: Vec<LocalDisplayCampus>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct LocalDisplayCampus {
    pub campus_code: String,
    pub display_name: String,
    pub cases: Vec<LocalDisplayCase>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct LocalDisplayCase {
    pub case_id: String,
    pub date: Option<String>,
    pub display_label: String,
}

/// Pixel bytes are returned only for the opaque asset requested during the active local session.
/// Source paths and source file names remain in `ScanSession` and are never serialized.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ImageAssetData {
    pub asset_id: String,
    pub mime_type: String,
    pub data_base64: String,
}

pub struct ScanOutcome {
    pub public: PublicScan,
    pub session: ScanSession,
}

/// This map is intentionally process-only. It is not serializable and is never handed to SQLite.
pub struct ScanSession {
    source_root: SourceReadRoot,
    cases: HashMap<String, MemoryCase>,
    campuses: HashMap<String, MemoryCampus>,
}

/// Anchors source reads to the directory that was open at scan time. It is never serialized.
struct SourceReadRoot {
    handle: File,
    #[cfg(windows)]
    selected_path: PathBuf,
    #[cfg(windows)]
    physical_path: Vec<u16>,
}

struct MemoryCampus {
    original_display_name: String,
}

struct MemoryCase {
    campus_code: String,
    original_display_name: String,
    assets: HashMap<String, MemoryAsset>,
}

struct MemoryAsset {
    /// This relative path stays in the active process only. Reads are resolved from
    /// `ScanSession::source_root`, never from a fresh absolute source path lookup.
    source_relative_path: PathBuf,
    #[allow(dead_code)] // Process-only mapping for local display, never serialized.
    original_display_name: String,
    media_kind: MediaKind,
    size_bytes: u64,
    source_fingerprint: String,
    #[cfg(unix)]
    source_identity: Option<UnixSourceIdentity>,
    #[cfg(windows)]
    source_identity: Option<WindowsSourceIdentity>,
}

#[cfg(unix)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct UnixSourceIdentity {
    device: u64,
    inode: u64,
}

#[cfg(windows)]
#[derive(Clone, Debug, PartialEq, Eq)]
struct WindowsSourceIdentity {
    volume_serial_number: u32,
    file_index: u64,
    physical_path: Vec<u16>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PersistedCase {
    pub case_id: String,
    pub campus_code: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PersistedAsset {
    pub asset_id: String,
    pub case_id: String,
    pub kind: StoredAssetKind,
    pub width: u32,
    pub height: u32,
    pub source_fingerprint: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum StoredAssetKind {
    OriginalImage,
    #[allow(dead_code)] // Reserved for the later frame-extraction command.
    CapturedFrame,
}

impl StoredAssetKind {
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            Self::OriginalImage => "original_image",
            Self::CapturedFrame => "captured_frame",
        }
    }
}

impl ScanSession {
    pub(crate) fn local_display_scan(&self, public: &PublicScan) -> CoreResult<LocalDisplayScan> {
        let mut campuses = Vec::with_capacity(public.campuses.len());
        for campus in &public.campuses {
            let memory_campus = self
                .campuses
                .get(&campus.campus_code)
                .ok_or(CoreError::StateUnavailable)?;
            let mut cases = Vec::with_capacity(campus.cases.len());
            for case in &campus.cases {
                let memory_case = self
                    .cases
                    .get(&case.case_id)
                    .ok_or(CoreError::StateUnavailable)?;
                cases.push(LocalDisplayCase {
                    case_id: case.case_id.clone(),
                    date: case.date.clone(),
                    display_label: memory_case.original_display_name.clone(),
                });
            }
            campuses.push(LocalDisplayCampus {
                campus_code: campus.campus_code.clone(),
                display_name: memory_campus.original_display_name.clone(),
                cases,
            });
        }
        Ok(LocalDisplayScan { campuses })
    }

    pub(crate) fn persisted_case(&self, case_id: &str) -> Option<PersistedCase> {
        let case = self.cases.get(case_id)?;
        Some(PersistedCase {
            case_id: case_id.to_owned(),
            campus_code: case.campus_code.clone(),
        })
    }

    pub(crate) fn persisted_image_asset(
        &self,
        case_id: &str,
        asset_id: &str,
    ) -> CoreResult<PersistedAsset> {
        let case = self
            .cases
            .get(case_id)
            .ok_or(CoreError::UnknownSessionItem)?;
        let asset = case
            .assets
            .get(asset_id)
            .ok_or(CoreError::UnknownSessionItem)?;

        if asset.media_kind != MediaKind::Image {
            return Err(CoreError::NotImageAsset);
        }

        Ok(PersistedAsset {
            asset_id: asset_id.to_owned(),
            case_id: case_id.to_owned(),
            kind: StoredAssetKind::OriginalImage,
            width: 0,
            height: 0,
            source_fingerprint: asset.source_fingerprint.clone(),
        })
    }

    pub(crate) fn image_asset_data(
        &self,
        case_id: &str,
        asset_id: &str,
    ) -> CoreResult<ImageAssetData> {
        let case = self
            .cases
            .get(case_id)
            .ok_or(CoreError::UnknownSessionItem)?;
        let asset = case
            .assets
            .get(asset_id)
            .ok_or(CoreError::UnknownSessionItem)?;
        if asset.media_kind != MediaKind::Image {
            return Err(CoreError::NotImageAsset);
        }

        // The source image is opened from the scan-time root handle. Unix opens every path
        // component with O_NOFOLLOW; Windows validates the final opened handle against the
        // scan-time file identity and physical path before any byte is read.
        let mut file = open_source_image_no_follow(&self.source_root, asset)?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        Ok(ImageAssetData {
            asset_id: asset_id.to_owned(),
            mime_type: "image/jpeg".to_owned(),
            data_base64: BASE64_STANDARD.encode(bytes),
        })
    }

    #[cfg(test)]
    fn source_mapping(&self, case_id: &str, asset_id: &str) -> Option<(&Path, &str, &str)> {
        let case = self.cases.get(case_id)?;
        let asset = case.assets.get(asset_id)?;
        Some((
            asset.source_relative_path.as_path(),
            case.original_display_name.as_str(),
            asset.original_display_name.as_str(),
        ))
    }
}

fn open_source_read_root(path: &Path) -> CoreResult<SourceReadRoot> {
    #[cfg(unix)]
    {
        let file = unix_open_path_no_follow(path, true)?;
        if !file.metadata()?.is_dir() {
            return Err(CoreError::UnsafeSourceAsset);
        }
        Ok(SourceReadRoot { handle: file })
    }

    #[cfg(windows)]
    {
        let file = windows_open_path_no_follow(path, true)?;
        let metadata = file.metadata()?;
        if unsafe_source_metadata(&metadata) || !metadata.is_dir() {
            return Err(CoreError::UnsafeSourceAsset);
        }
        Ok(SourceReadRoot {
            physical_path: windows_final_path(&file)?,
            selected_path: path.to_path_buf(),
            handle: file,
        })
    }

    #[cfg(not(any(unix, windows)))]
    {
        let _ = path;
        Err(CoreError::UnsafeSourceAsset)
    }
}

fn open_source_image_no_follow(root: &SourceReadRoot, asset: &MemoryAsset) -> CoreResult<File> {
    #[cfg(unix)]
    {
        let identity = asset.source_identity.ok_or(CoreError::StateUnavailable)?;
        let file = unix_open_relative_image_no_follow(&root.handle, &asset.source_relative_path)?;
        let metadata = file.metadata()?;
        if !metadata.file_type().is_file() || unix_source_identity(&metadata) != identity {
            return Err(CoreError::UnsafeSourceAsset);
        }
        Ok(file)
    }

    #[cfg(windows)]
    {
        let identity = asset
            .source_identity
            .as_ref()
            .ok_or(CoreError::StateUnavailable)?;
        // Keeping the root handle open pins the scan-time root. Re-reading its final path makes
        // a stale/reused selected path fail closed before the asset handle can be trusted.
        if windows_final_path(&root.handle)? != root.physical_path {
            return Err(CoreError::UnsafeSourceAsset);
        }
        let file = windows_open_path_no_follow(
            &root.selected_path.join(&asset.source_relative_path),
            false,
        )?;
        let metadata = file.metadata()?;
        if unsafe_source_metadata(&metadata)
            || !metadata.file_type().is_file()
            || windows_source_identity(&file)? != *identity
        {
            return Err(CoreError::UnsafeSourceAsset);
        }
        Ok(file)
    }

    #[cfg(not(any(unix, windows)))]
    {
        let _ = (root, asset);
        Err(CoreError::UnsafeSourceAsset)
    }
}

#[cfg(unix)]
fn unix_open_path_no_follow(path: &Path, directory: bool) -> CoreResult<File> {
    let path =
        CString::new(path.as_os_str().as_bytes()).map_err(|_| CoreError::UnsafeSourceAsset)?;
    let flags = libc::O_RDONLY
        | libc::O_CLOEXEC
        | libc::O_NOFOLLOW
        | if directory { libc::O_DIRECTORY } else { 0 };
    let descriptor = unsafe { libc::open(path.as_ptr(), flags) };
    if descriptor < 0 {
        return Err(CoreError::UnsafeSourceAsset);
    }
    Ok(unsafe { File::from_raw_fd(descriptor) })
}

#[cfg(unix)]
fn unix_open_relative_image_no_follow(root: &File, relative_path: &Path) -> CoreResult<File> {
    let components = relative_path
        .components()
        .map(|component| match component {
            std::path::Component::Normal(part) => Ok(part),
            _ => Err(CoreError::UnsafeSourceAsset),
        })
        .collect::<CoreResult<Vec<_>>>()?;
    let (file_name, parent_components) = components
        .split_last()
        .ok_or(CoreError::UnsafeSourceAsset)?;

    let mut parent_handle: Option<File> = None;
    for component in parent_components {
        let descriptor = unix_openat_no_follow(
            parent_handle
                .as_ref()
                .map_or_else(|| root.as_raw_fd(), AsRawFd::as_raw_fd),
            component,
            true,
        )?;
        parent_handle = Some(descriptor);
    }
    unix_openat_no_follow(
        parent_handle
            .as_ref()
            .map_or_else(|| root.as_raw_fd(), AsRawFd::as_raw_fd),
        file_name,
        false,
    )
}

#[cfg(unix)]
fn unix_openat_no_follow(
    directory_fd: std::os::fd::RawFd,
    component: &std::ffi::OsStr,
    directory: bool,
) -> CoreResult<File> {
    let component = CString::new(component.as_bytes()).map_err(|_| CoreError::UnsafeSourceAsset)?;
    let flags = libc::O_RDONLY
        | libc::O_CLOEXEC
        | libc::O_NOFOLLOW
        | if directory { libc::O_DIRECTORY } else { 0 };
    let descriptor = unsafe { libc::openat(directory_fd, component.as_ptr(), flags) };
    if descriptor < 0 {
        return Err(CoreError::UnsafeSourceAsset);
    }
    Ok(unsafe { File::from_raw_fd(descriptor) })
}

#[cfg(unix)]
fn unix_source_identity(metadata: &fs::Metadata) -> UnixSourceIdentity {
    UnixSourceIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
    }
}

#[cfg(windows)]
fn windows_open_path_no_follow(path: &Path, directory: bool) -> CoreResult<File> {
    let mut options = OpenOptions::new();
    options.read(true).custom_flags(
        FILE_FLAG_OPEN_REPARSE_POINT
            | if directory {
                FILE_FLAG_BACKUP_SEMANTICS
            } else {
                0
            },
    );
    let file = options.open(path)?;
    let metadata = file.metadata()?;
    if unsafe_source_metadata(&metadata) {
        return Err(CoreError::UnsafeSourceAsset);
    }
    Ok(file)
}

#[cfg(windows)]
fn windows_final_path(file: &File) -> CoreResult<Vec<u16>> {
    let handle = windows_file_handle(file)?;
    let mut buffer = vec![0_u16; 512];
    loop {
        let length = unsafe {
            GetFinalPathNameByHandleW(
                handle,
                buffer.as_mut_ptr(),
                buffer.len() as u32,
                FILE_NAME_NORMALIZED,
            )
        };
        if length == 0 {
            return Err(CoreError::UnsafeSourceAsset);
        }
        if (length as usize) < buffer.len() {
            buffer.truncate(length as usize);
            return Ok(buffer);
        }
        buffer.resize(length as usize + 1, 0);
    }
}

#[cfg(windows)]
fn windows_source_identity(file: &File) -> CoreResult<WindowsSourceIdentity> {
    let handle = windows_file_handle(file)?;
    let mut information: BY_HANDLE_FILE_INFORMATION = unsafe { mem::zeroed() };
    if unsafe { GetFileInformationByHandle(handle, &mut information) } == 0 {
        return Err(CoreError::UnsafeSourceAsset);
    }
    let file_index = ((information.nFileIndexHigh as u64) << 32) | information.nFileIndexLow as u64;
    if information.dwVolumeSerialNumber == 0 || file_index == 0 {
        return Err(CoreError::UnsafeSourceAsset);
    }
    Ok(WindowsSourceIdentity {
        volume_serial_number: information.dwVolumeSerialNumber,
        file_index,
        physical_path: windows_final_path(file)?,
    })
}

#[cfg(windows)]
fn windows_file_handle(file: &File) -> CoreResult<HANDLE> {
    let handle = file.as_raw_handle() as HANDLE;
    if handle.is_null() || handle == INVALID_HANDLE_VALUE {
        return Err(CoreError::UnsafeSourceAsset);
    }
    Ok(handle)
}

#[cfg(unix)]
fn scan_image_identity(_path: &Path, metadata: &fs::Metadata) -> CoreResult<UnixSourceIdentity> {
    Ok(unix_source_identity(metadata))
}

#[cfg(windows)]
fn scan_image_identity(path: &Path, _metadata: &fs::Metadata) -> CoreResult<WindowsSourceIdentity> {
    let file = windows_open_path_no_follow(path, false)?;
    let metadata = file.metadata()?;
    if !metadata.file_type().is_file() {
        return Err(CoreError::UnsafeSourceAsset);
    }
    windows_source_identity(&file)
}

#[cfg(windows)]
fn unsafe_source_metadata(metadata: &fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }

    metadata.file_attributes() & 0x400 != 0
}

pub fn scan_data_root(root: &Path, secret: &ProjectSecret) -> CoreResult<ScanOutcome> {
    // A selected root may be a pre-existing safe alias. Resolve it once while establishing the
    // session, then hold the opened real directory for all later source reads. No read-time
    // safety decision relies on this path resolution.
    let resolved_root = fs::canonicalize(root)?;
    // Hold this directory handle for the life of the active session. A later source-directory
    // replacement therefore cannot redirect Unix reads away from the scanned root.
    let source_root = open_source_read_root(&resolved_root)?;
    let mut campus_paths = direct_directories(&resolved_root)?;
    if campus_paths.len() != 2 {
        return Err(CoreError::InvalidCampusLayout);
    }
    campus_paths.sort_by(|left, right| natural_path_cmp(left, right));

    let mut session_cases = HashMap::new();
    let mut session_campuses = HashMap::new();
    let mut campuses = Vec::with_capacity(2);

    for campus_path in campus_paths {
        let raw_campus_name = file_name_text(&campus_path)?;
        let campus_code = campus_code(secret, &raw_campus_name);
        if session_campuses
            .insert(
                campus_code.clone(),
                MemoryCampus {
                    original_display_name: raw_campus_name.clone(),
                },
            )
            .is_some()
        {
            return Err(CoreError::StateUnavailable);
        }
        let mut scanned_cases = Vec::new();

        for case_path in direct_directories(&campus_path)? {
            let raw_case_name = file_name_text(&case_path)?;
            let case_id = case_id(secret, &campus_code, &raw_case_name);
            let metadata = fs::metadata(&case_path)?;
            let parsed_date = parse_date_prefix(&raw_case_name);
            let case_modified_millis = modified_millis(&metadata);
            let mut memory_assets = HashMap::new();
            let mut public_assets = Vec::new();
            let mut sortable_assets = Vec::new();

            for entry in fs::read_dir(&case_path)? {
                let entry = entry?;
                let file_type = entry.file_type()?;
                if !file_type.is_file() {
                    continue;
                }

                let path = entry.path();
                let Some(media_kind) = media_kind_for_path(&path) else {
                    continue;
                };
                let raw_media_name = file_name_text(&path)?;
                let metadata = entry.metadata()?;
                let size_bytes = metadata.len();
                let modified_millis = modified_millis(&metadata);
                #[cfg(unix)]
                let source_identity = if media_kind == MediaKind::Image {
                    Some(scan_image_identity(&path, &metadata)?)
                } else {
                    None
                };
                #[cfg(windows)]
                let source_identity = if media_kind == MediaKind::Image {
                    Some(scan_image_identity(&path, &metadata)?)
                } else {
                    None
                };
                let asset_id = asset_id(
                    secret,
                    &case_id,
                    &raw_media_name,
                    size_bytes,
                    modified_millis,
                );
                let fingerprint = source_fingerprint(
                    secret,
                    &case_id,
                    &raw_media_name,
                    size_bytes,
                    modified_millis,
                );

                if memory_assets.contains_key(&asset_id) {
                    return Err(CoreError::StateUnavailable);
                }

                memory_assets.insert(
                    asset_id.clone(),
                    MemoryAsset {
                        source_relative_path: PathBuf::from(&raw_campus_name)
                            .join(&raw_case_name)
                            .join(&raw_media_name),
                        original_display_name: raw_media_name.clone(),
                        media_kind: media_kind.clone(),
                        size_bytes,
                        source_fingerprint: fingerprint,
                        #[cfg(unix)]
                        source_identity,
                        #[cfg(windows)]
                        source_identity,
                    },
                );
                sortable_assets.push((raw_media_name, asset_id));
            }

            sortable_assets.sort_by(|left, right| natural_cmp(&left.0, &right.0));
            for (_, asset_id) in sortable_assets {
                let asset = memory_assets
                    .get(&asset_id)
                    .ok_or(CoreError::StateUnavailable)?;
                public_assets.push(PublicAsset {
                    asset_id,
                    media_kind: asset.media_kind.clone(),
                    size_bytes: asset.size_bytes,
                });
            }

            if session_cases.contains_key(&case_id) {
                return Err(CoreError::StateUnavailable);
            }

            session_cases.insert(
                case_id.clone(),
                MemoryCase {
                    campus_code: campus_code.clone(),
                    original_display_name: raw_case_name,
                    assets: memory_assets,
                },
            );
            scanned_cases.push(SortableCase {
                public: PublicCase {
                    case_id,
                    date: parsed_date.map(|date| date.format("%Y-%m-%d").to_string()),
                    assets: public_assets,
                },
                parsed_date,
                modified_millis: case_modified_millis,
            });
        }

        scanned_cases.sort_by(case_sort_cmp);
        let cases = scanned_cases
            .into_iter()
            .map(|sortable| sortable.public)
            .collect::<Vec<_>>();
        campuses.push(PublicCampus {
            campus_code,
            case_count: cases.len(),
            cases,
        });
    }

    campuses.sort_by(|left, right| left.campus_code.cmp(&right.campus_code));
    Ok(ScanOutcome {
        public: PublicScan { campuses },
        session: ScanSession {
            source_root,
            cases: session_cases,
            campuses: session_campuses,
        },
    })
}

#[derive(Clone)]
struct SortableCase {
    public: PublicCase,
    parsed_date: Option<NaiveDate>,
    modified_millis: u128,
}

fn case_sort_cmp(left: &SortableCase, right: &SortableCase) -> Ordering {
    match (left.parsed_date, right.parsed_date) {
        (Some(left_date), Some(right_date)) => right_date
            .cmp(&left_date)
            .then_with(|| left.public.case_id.cmp(&right.public.case_id)),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => right
            .modified_millis
            .cmp(&left.modified_millis)
            .then_with(|| left.public.case_id.cmp(&right.public.case_id)),
    }
}

fn direct_directories(root: &Path) -> CoreResult<Vec<PathBuf>> {
    let mut directories = Vec::new();
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            directories.push(entry.path());
        }
    }
    Ok(directories)
}

fn media_kind_for_path(path: &Path) -> Option<MediaKind> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    match extension.as_str() {
        "jpg" | "jpeg" => Some(MediaKind::Image),
        "mp4" => Some(MediaKind::Video),
        _ => None,
    }
}

fn file_name_text(path: &Path) -> CoreResult<String> {
    path.file_name()
        .and_then(OsStr::to_str)
        .map(str::to_owned)
        .ok_or(CoreError::Filesystem)
}

fn parse_date_prefix(value: &str) -> Option<NaiveDate> {
    let prefix = value.split_once('_')?.0;
    ["%Y%m%d", "%Y-%m-%d", "%Y.%m.%d"]
        .iter()
        .find_map(|format| NaiveDate::parse_from_str(prefix, format).ok())
}

fn modified_millis(metadata: &fs::Metadata) -> u128 {
    metadata
        .modified()
        .unwrap_or(SystemTime::UNIX_EPOCH)
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn natural_path_cmp(left: &Path, right: &Path) -> Ordering {
    let left_name = left.file_name().and_then(OsStr::to_str).unwrap_or_default();
    let right_name = right
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or_default();
    natural_cmp(left_name, right_name)
}

pub(crate) fn natural_cmp(left: &str, right: &str) -> Ordering {
    let left_bytes = left.as_bytes();
    let right_bytes = right.as_bytes();
    let mut left_index = 0;
    let mut right_index = 0;

    while left_index < left_bytes.len() && right_index < right_bytes.len() {
        let left_byte = left_bytes[left_index];
        let right_byte = right_bytes[right_index];
        if left_byte.is_ascii_digit() && right_byte.is_ascii_digit() {
            let left_end = digit_end(left_bytes, left_index);
            let right_end = digit_end(right_bytes, right_index);
            let numeric = numeric_segment_cmp(
                &left_bytes[left_index..left_end],
                &right_bytes[right_index..right_end],
            );
            if numeric != Ordering::Equal {
                return numeric;
            }
            left_index = left_end;
            right_index = right_end;
            continue;
        }

        let normalized_left = left_byte.to_ascii_lowercase();
        let normalized_right = right_byte.to_ascii_lowercase();
        let compared = normalized_left.cmp(&normalized_right);
        if compared != Ordering::Equal {
            return compared;
        }
        left_index += 1;
        right_index += 1;
    }

    left_bytes
        .len()
        .cmp(&right_bytes.len())
        .then_with(|| left.cmp(right))
}

fn digit_end(bytes: &[u8], start: usize) -> usize {
    let mut end = start;
    while end < bytes.len() && bytes[end].is_ascii_digit() {
        end += 1;
    }
    end
}

fn numeric_segment_cmp(left: &[u8], right: &[u8]) -> Ordering {
    let left_trimmed = trim_zeroes(left);
    let right_trimmed = trim_zeroes(right);
    left_trimmed
        .len()
        .cmp(&right_trimmed.len())
        .then_with(|| left_trimmed.cmp(right_trimmed))
        .then_with(|| left.len().cmp(&right.len()))
}

fn trim_zeroes(value: &[u8]) -> &[u8] {
    let first_non_zero = value.iter().position(|byte| *byte != b'0');
    match first_non_zero {
        Some(index) => &value[index..],
        None => &value[value.len().saturating_sub(1)..],
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::{natural_cmp, scan_data_root};
    use crate::core::ids::ProjectSecret;

    fn secret() -> ProjectSecret {
        ProjectSecret::from_bytes([9; 32])
    }

    fn empty_two_campus_root() -> (TempDir, std::path::PathBuf) {
        let temporary = tempfile::tempdir().expect("temporary root");
        let root = temporary.path().join("ROOT-CASE-FIXTURE");
        fs::create_dir_all(root.join("CAMPUS-ALPHA")).expect("campus alpha");
        fs::create_dir_all(root.join("CAMPUS-BETA")).expect("campus beta");
        (temporary, root)
    }

    #[test]
    fn file_scan_two_campuses() {
        let (_temporary, root) = empty_two_campus_root();
        for campus in ["CAMPUS-ALPHA", "CAMPUS-BETA"] {
            for number in 0..101 {
                let case = format!("20240101_CASE-{number:03}");
                fs::create_dir_all(root.join(campus).join(case)).expect("fixture case");
            }
        }

        let outcome = scan_data_root(&root, &secret()).expect("scan fixture");
        assert_eq!(outcome.public.campuses.len(), 2);
        assert!(outcome
            .public
            .campuses
            .iter()
            .all(|campus| campus.case_count == 101));
    }

    #[test]
    fn scan_direct_media_only() {
        let (_temporary, root) = empty_two_campus_root();
        let case = root.join("CAMPUS-ALPHA").join("20240102_CASE-ALPHA");
        fs::create_dir_all(case.join("NESTED")).expect("nested fixture");
        for file in [
            "ASSET-02.jPeG",
            "ASSET-10.MP4",
            "ASSET-1.JPG",
            "ASSET-03.PNG",
        ] {
            fs::write(case.join(file), b"fixture").expect("fixture media");
        }
        fs::write(case.join("NESTED").join("ASSET-99.JPG"), b"fixture").expect("nested media");

        let outcome = scan_data_root(&root, &secret()).expect("scan fixture");
        let alpha = outcome
            .public
            .campuses
            .iter()
            .find(|campus| campus.case_count == 1)
            .expect("campus with case");
        let scanned_case = alpha.cases.first().expect("one case");
        assert_eq!(scanned_case.assets.len(), 3);
        assert_eq!(
            scanned_case
                .assets
                .iter()
                .map(|asset| asset.media_kind.clone())
                .collect::<Vec<_>>(),
            vec![
                super::MediaKind::Image,
                super::MediaKind::Image,
                super::MediaKind::Video
            ]
        );
    }

    #[test]
    fn case_sorting_is_stable() {
        let (_temporary, root) = empty_two_campus_root();
        let campus = root.join("CAMPUS-ALPHA");
        for case in [
            "2024.01.01_CASE-ALPHA",
            "INVALID_CASE-BETA",
            "20240103_CASE-ALPHA",
            "20240103_CASE-BETA",
            "20240103_CASE-GAMMA",
            "2024-01-02_CASE-DELTA",
        ] {
            fs::create_dir_all(campus.join(case)).expect("fixture case");
        }

        let outcome = scan_data_root(&root, &secret()).expect("scan fixture");
        let cases = outcome
            .public
            .campuses
            .iter()
            .find(|campus| campus.case_count == 6)
            .expect("sorted campus")
            .cases
            .as_slice();
        assert_eq!(cases[0].date.as_deref(), Some("2024-01-03"));
        assert_eq!(cases[1].date.as_deref(), Some("2024-01-03"));
        assert_eq!(cases[2].date.as_deref(), Some("2024-01-03"));
        assert!(cases[..3]
            .windows(2)
            .all(|pair| pair[0].case_id <= pair[1].case_id));
        assert_eq!(cases[3].date.as_deref(), Some("2024-01-02"));
        assert_eq!(cases[4].date.as_deref(), Some("2024-01-01"));
        assert_eq!(cases[5].date, None);
        assert!(natural_cmp("ASSET-2.JPG", "ASSET-10.JPG").is_lt());
    }

    #[test]
    fn session_mapping_is_memory_only() {
        let (_temporary, root) = empty_two_campus_root();
        let case = root.join("CAMPUS-BETA").join("20240102_CASE-ALPHA");
        fs::create_dir_all(&case).expect("fixture case");
        fs::write(case.join("ASSET-01.JPG"), b"fixture").expect("fixture media");

        let outcome = scan_data_root(&root, &secret()).expect("scan fixture");
        let public_case = outcome
            .public
            .campuses
            .iter()
            .find_map(|campus| campus.cases.first())
            .expect("public case");
        let public_asset = public_case.assets.first().expect("public asset");
        let mapping = outcome
            .session
            .source_mapping(&public_case.case_id, &public_asset.asset_id)
            .expect("memory mapping");

        assert!(mapping.0.ends_with("20240102_CASE-ALPHA/ASSET-01.JPG"));
        assert_eq!(mapping.1, "20240102_CASE-ALPHA");
        assert_eq!(mapping.2, "ASSET-01.JPG");
        let public_json = serde_json::to_string(&outcome.public).expect("public serialization");
        assert!(!public_json.contains("20240102_CASE-ALPHA"));
        assert!(!public_json.contains("ASSET-01.JPG"));
    }

    #[test]
    fn local_display_contract_is_only_derived_from_memory() {
        let (_temporary, root) = empty_two_campus_root();
        let case = root.join("CAMPUS-ALPHA").join("20240102_CASE-ALPHA");
        fs::create_dir_all(&case).expect("fixture case");
        fs::write(case.join("ASSET-01.JPG"), b"fixture").expect("fixture media");

        let outcome = scan_data_root(&root, &secret()).expect("scan fixture");
        let local = outcome
            .session
            .local_display_scan(&outcome.public)
            .expect("local display scan");
        let local_json = serde_json::to_string(&local).expect("local display serialization");
        let public_json = serde_json::to_string(&outcome.public).expect("public serialization");

        assert!(local_json.contains("CAMPUS-ALPHA"));
        assert!(local_json.contains("20240102_CASE-ALPHA"));
        assert!(!public_json.contains("CAMPUS-ALPHA"));
        assert!(!public_json.contains("20240102_CASE-ALPHA"));
    }
}
