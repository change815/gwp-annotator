use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use super::error::{CoreError, CoreResult};
use super::store::{
    prepare_work_root_after_overlap_check, read_optional_work_file, replace_temp_file,
};

const SETTINGS_FILE: &str = "storage-location.json";

pub fn load_storage_location(config_dir: &Path) -> CoreResult<Option<PathBuf>> {
    let Some(bytes) = read_optional_work_file(config_dir, Path::new(SETTINGS_FILE))? else {
        return Ok(None);
    };
    let location: PathBuf =
        serde_json::from_slice(&bytes).map_err(|_| CoreError::StateUnavailable)?;
    if !location.is_absolute() {
        return Err(CoreError::InvalidWorkPath);
    }
    Ok(Some(location))
}

pub(crate) fn save_storage_location(config_dir: &Path, location: &Path) -> CoreResult<()> {
    let root = prepare_work_root_after_overlap_check(config_dir)?;
    read_optional_work_file(&root, Path::new(SETTINGS_FILE))?;
    let bytes = serde_json::to_vec(location).map_err(|_| CoreError::InvalidWorkPath)?;
    let temporary = root.join(format!(".storage-{:016x}.tmp", rand::random::<u64>()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    let result = (|| {
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        replace_temp_file(&temporary, &root.join(SETTINGS_FILE))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}
