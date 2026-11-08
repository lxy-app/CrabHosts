use std::fs;
use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{Error, Result, atomic_write, sha256_hex};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct BackupMetadata {
    pub id: String,
    pub created_at: String,
    pub source_path: PathBuf,
    pub sha256: String,
    pub size: u64,
}

#[derive(Clone, Debug)]
pub struct BackupStore {
    directory: PathBuf,
}

impl BackupStore {
    pub fn new(directory: PathBuf) -> Self {
        Self { directory }
    }

    pub fn create(&self, content: &str, source_path: &Path) -> Result<BackupMetadata> {
        let id = format!(
            "{}-{}",
            Utc::now().format("%Y%m%dT%H%M%SZ"),
            &Uuid::new_v4().simple().to_string()[..8]
        );
        let metadata = BackupMetadata {
            id: id.clone(),
            created_at: Utc::now().to_rfc3339(),
            source_path: source_path.to_path_buf(),
            sha256: sha256_hex(content.as_bytes()),
            size: content.len() as u64,
        };
        atomic_write(
            &self.directory.join(format!("{id}.hosts")),
            content.as_bytes(),
        )?;
        atomic_write(
            &self.directory.join(format!("{id}.json")),
            serde_json::to_vec_pretty(&metadata)?.as_slice(),
        )?;
        Ok(metadata)
    }

    pub fn list(&self) -> Result<Vec<BackupMetadata>> {
        let mut backups = Vec::new();
        match fs::read_dir(&self.directory) {
            Ok(items) => {
                for item in items {
                    let path = item?.path();
                    if path.extension().and_then(|value| value.to_str()) != Some("json") {
                        continue;
                    }
                    let metadata: BackupMetadata =
                        serde_json::from_str(&fs::read_to_string(path)?)?;
                    backups.push(metadata);
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        backups.sort_by(|left, right| right.created_at.cmp(&left.created_at));
        Ok(backups)
    }

    pub fn load(&self, id: &str) -> Result<(BackupMetadata, String)> {
        let metadata = self
            .list()?
            .into_iter()
            .find(|backup| backup.id == id)
            .ok_or_else(|| Error::NotFound(format!("backup {id:?}")))?;
        let content = fs::read_to_string(self.directory.join(format!("{id}.hosts")))?;
        if sha256_hex(content.as_bytes()) != metadata.sha256 {
            return Err(Error::Invalid(format!(
                "backup {id:?} failed its SHA-256 integrity check"
            )));
        }
        Ok((metadata, content))
    }
}
