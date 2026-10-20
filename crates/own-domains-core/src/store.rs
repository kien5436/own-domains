use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

use crate::{CoreError, CoreResult, Settings, validate_settings};

#[derive(Clone, Debug)]
pub struct SettingsStore {
    directory: PathBuf,
    path: PathBuf,
    temporary_path: PathBuf,
}

impl SettingsStore {
    pub fn new(directory: impl Into<PathBuf>) -> Self {
        let directory = directory.into();
        Self {
            path: directory.join("settings.json"),
            temporary_path: directory.join("settings.json.tmp"),
            directory,
        }
    }

    pub fn load(&self) -> CoreResult<Settings> {
        self.create_directory()?;
        let contents = match fs::read_to_string(&self.path) {
            Ok(contents) => contents,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Settings::default()),
            Err(source) => return Err(io_error("read", &self.path, source)),
        };
        let parsed = serde_json::from_str::<Settings>(&contents).map_err(|error| CoreError::CorruptSettings {
            path: self.path.clone(),
            message: error.to_string(),
        })?;

        validate_settings(parsed).map_err(|error| CoreError::CorruptSettings {
            path: self.path.clone(),
            message: error.to_string(),
        })
    }

    pub fn save(&self, settings: &Settings, expected_revision: Option<u64>) -> CoreResult<()> {
        self.save_with_before_rename(settings, expected_revision, || Ok(()))
    }

    pub(crate) fn lock_key(&self) -> PathBuf {
        if self.path.is_absolute() {
            self.path.clone()
        } else {
            std::env::current_dir().unwrap_or_default().join(&self.path)
        }
    }

    #[cfg(test)]
    fn save_with_before_rename_for_test(
        &self,
        settings: &Settings,
        expected_revision: Option<u64>,
        before_rename: impl FnOnce() -> std::io::Result<()>,
    ) -> CoreResult<()> {
        self.save_with_before_rename(settings, expected_revision, before_rename)
    }

    fn save_with_before_rename(
        &self,
        settings: &Settings,
        expected_revision: Option<u64>,
        before_rename: impl FnOnce() -> std::io::Result<()>,
    ) -> CoreResult<()> {
        self.create_directory()?;
        let current = self.load()?;
        if let Some(expected) = expected_revision
            && current.revision != expected
        {
            return Err(CoreError::RevisionConflict { expected, actual: current.revision });
        }
        validate_settings(settings.clone())?;
        let mut contents = serde_json::to_vec_pretty(settings).map_err(|error| CoreError::CorruptSettings {
            path: self.path.clone(),
            message: error.to_string(),
        })?;
        contents.push(b'\n');

        let mut options = OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        options.mode(0o600);
        let mut temporary = options
            .open(&self.temporary_path)
            .map_err(|source| io_error("open temporary settings", &self.temporary_path, source))?;
        temporary
            .write_all(&contents)
            .map_err(|source| io_error("write temporary settings", &self.temporary_path, source))?;
        temporary
            .sync_all()
            .map_err(|source| io_error("sync temporary settings", &self.temporary_path, source))?;
        drop(temporary);
        before_rename().map_err(|source| io_error("prepare settings replacement", &self.temporary_path, source))?;
        fs::rename(&self.temporary_path, &self.path)
            .map_err(|source| io_error("replace settings", &self.path, source))?;
        #[cfg(unix)]
        File::open(&self.directory)
            .and_then(|directory| directory.sync_all())
            .map_err(|source| io_error("sync settings directory", &self.directory, source))?;
        Ok(())
    }

    fn create_directory(&self) -> CoreResult<()> {
        fs::create_dir_all(&self.directory)
            .map_err(|source| io_error("create settings directory", &self.directory, source))?;
        #[cfg(unix)]
        fs::set_permissions(&self.directory, fs::Permissions::from_mode(0o700))
            .map_err(|source| io_error("secure settings directory", &self.directory, source))?;
        Ok(())
    }
}

fn io_error(operation: &'static str, path: &Path, source: std::io::Error) -> CoreError {
    CoreError::Io { operation, path: path.to_path_buf(), source }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failure_after_sync_before_rename_preserves_previous_file_and_temporary_file() {
        let directory = std::env::temp_dir().join(format!("own-domains-rename-test-{}", uuid::Uuid::new_v4()));
        let store = SettingsStore::new(&directory);
        store.save(&Settings { revision: 1, ..Settings::default() }, Some(0)).unwrap();

        let result = store.save_with_before_rename_for_test(
            &Settings { revision: 2, ..Settings::default() },
            Some(1),
            || Err(std::io::Error::other("injected before rename")),
        );

        assert!(matches!(result, Err(CoreError::Io { operation: "prepare settings replacement", .. })));
        assert_eq!(store.load().unwrap().revision, 1);
        assert!(directory.join("settings.json.tmp").is_file());
        fs::remove_dir_all(directory).unwrap();
    }
}
