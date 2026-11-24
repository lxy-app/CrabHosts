use std::fs::{self, OpenOptions};
use std::io::Write;

use fs2::FileExt;
use std::path::Path;

use sha2::{Digest, Sha256};
use tempfile::NamedTempFile;

use crate::{ActivationState, AppPaths, BackupStore, Document, Error, Result};

#[derive(Clone, Debug)]
pub struct Storage {
    paths: AppPaths,
}

impl Storage {
    pub fn new(paths: AppPaths) -> Self {
        Self { paths }
    }

    pub fn from_env() -> Result<Self> {
        Ok(Self::new(AppPaths::from_env()?))
    }

    pub fn paths(&self) -> &AppPaths {
        &self.paths
    }

    pub fn backups(&self) -> BackupStore {
        BackupStore::new(self.paths.backups_dir.clone())
    }

    pub fn load(&self) -> Result<Document> {
        match fs::read_to_string(&self.paths.database_file) {
            Ok(content) => {
                let document: Document = serde_json::from_str(&content)?;
                document.validate()?;
                Ok(document)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Document {
                schema_version: 1,
                revision: 0,
                profiles: Vec::new(),
            }),
            Err(error) => Err(error.into()),
        }
    }

    pub fn save(&self, document: &mut Document) -> Result<()> {
        document.validate()?;
        document.revision = document.revision.saturating_add(1);
        let content = serde_json::to_vec_pretty(document)?;
        atomic_write(&self.paths.database_file, &content)?;
        Ok(())
    }

    /// Loads device-local activation, recovering it from an existing managed block on first run.
    /// Unknown Profile IDs left by a deleted Profile are ignored rather than resurrected.
    pub fn load_activation(&self, document: &Document) -> Result<ActivationState> {
        self.load_activation_inner(document, true)
    }

    fn load_activation_inner(
        &self,
        document: &Document,
        filter_existing: bool,
    ) -> Result<ActivationState> {
        let mut state = match fs::read_to_string(&self.paths.activation_file) {
            Ok(content) => {
                let state: ActivationState = serde_json::from_str(&content)?;
                state.validate()?;
                state
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                match fs::read_to_string(&self.paths.hosts_file) {
                    Ok(hosts) => ActivationState::recover_from_hosts(&hosts, document),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        ActivationState::default()
                    }
                    Err(error) => return Err(error.into()),
                }
            }
            Err(error) => return Err(error.into()),
        };
        if filter_existing {
            state.retain_existing(document);
        }
        Ok(state)
    }

    /// Serializes a complete activation read-modify-write across desktop and CLI processes.
    /// The OS releases the lock if a writer exits or crashes.
    pub fn update_activation<T>(
        &self,
        document: &Document,
        update: impl FnOnce(&mut ActivationState) -> Result<T>,
    ) -> Result<(ActivationState, T)> {
        let _lock = self.lock_activation()?;
        // Preserve IDs unknown to this possibly stale Document so a concurrent Profile creation
        // is not dropped. Public reads filter deleted IDs before planning system content.
        let current = self.load_activation_inner(document, false)?;
        let mut candidate = current.clone();
        let output = update(&mut candidate)?;
        if candidate != current {
            self.save_activation_unlocked(&mut candidate)?;
        }
        let mut visible = candidate;
        visible.retain_existing(document);
        Ok((visible, output))
    }

    /// Saves an already prepared state under the activation lock. Prefer `update_activation`
    /// whenever the new state depends on values previously loaded from disk.
    pub fn save_activation(&self, state: &mut ActivationState) -> Result<()> {
        let _lock = self.lock_activation()?;
        self.save_activation_unlocked(state)
    }

    fn save_activation_unlocked(&self, state: &mut ActivationState) -> Result<()> {
        state.validate()?;
        state.revision = state.revision.saturating_add(1);
        let mut content = serde_json::to_vec_pretty(state)?;
        content.push(b'\n');
        atomic_write(&self.paths.activation_file, &content)
    }

    fn lock_activation(&self) -> Result<fs::File> {
        fs::create_dir_all(&self.paths.config_dir)?;
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.paths.config_dir.join("activation.lock"))?;
        lock.lock_exclusive()?;
        Ok(lock)
    }

    pub fn read_hosts(&self) -> Result<String> {
        Ok(fs::read_to_string(&self.paths.hosts_file)?)
    }

    pub fn write_hosts(&self, content: &str) -> Result<()> {
        atomic_write(&self.paths.hosts_file, content.as_bytes())
    }
}

/// Writes a file in the same directory, preserving existing permissions where possible.
pub fn atomic_write(path: &Path, content: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| Error::Invalid(format!("path {} has no parent", path.display())))?;
    fs::create_dir_all(parent)?;
    if fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        return Err(Error::Invalid(format!(
            "refusing to replace symbolic link {}",
            path.display()
        )));
    }
    let existing_permissions = fs::metadata(path).ok().map(|meta| meta.permissions());
    let mut temporary = NamedTempFile::new_in(parent)?;
    temporary.write_all(content)?;
    temporary.flush()?;
    temporary.as_file().sync_all()?;

    if let Some(permissions) = existing_permissions {
        temporary.as_file().set_permissions(permissions)?;
    } else {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            temporary
                .as_file()
                .set_permissions(fs::Permissions::from_mode(0o600))?;
        }
    }
    temporary
        .persist(path)
        .map_err(|error| Error::Io(error.error))?;
    Ok(())
}

pub fn sha256_hex(content: &[u8]) -> String {
    format!("{:x}", Sha256::digest(content))
}
