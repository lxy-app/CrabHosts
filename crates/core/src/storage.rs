use std::fs;
use std::io::Write;
use std::path::Path;

use sha2::{Digest, Sha256};
use tempfile::NamedTempFile;

use crate::{AppPaths, BackupStore, Document, Error, Result};

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
