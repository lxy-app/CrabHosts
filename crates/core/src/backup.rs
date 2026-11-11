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

/// CrabHosts generated IDs are `<YYYYMMDD>T<HHMMSS>Z-<8 lowercase hex>`.
/// Binding this exact shape to the metadata filename keeps pruning away from unrelated files.
fn is_managed_backup_id(id: &str) -> bool {
    let bytes = id.as_bytes();
    bytes.len() == 25
        && bytes[8] == b'T'
        && bytes[15] == b'Z'
        && bytes[16] == b'-'
        && bytes[..8].iter().all(u8::is_ascii_digit)
        && bytes[9..15].iter().all(u8::is_ascii_digit)
        && bytes[17..].iter().all(u8::is_ascii_hexdigit)
        && !bytes[17..].iter().any(u8::is_ascii_uppercase)
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
                        serde_json::from_str(&fs::read_to_string(&path)?)?;
                    let expected_name = format!("{}.json", metadata.id);
                    if !is_managed_backup_id(&metadata.id)
                        || path.file_name().and_then(|name| name.to_str())
                            != Some(expected_name.as_str())
                    {
                        // Parseable but unrelated or mismatched metadata is never a prune candidate.
                        continue;
                    }
                    backups.push(metadata);
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        backups.sort_by(|left, right| right.created_at.cmp(&left.created_at));
        Ok(backups)
    }

    /// Removes the oldest managed backups beyond `keep`, returning the number removed.
    /// Only files with validated CrabHosts backup IDs in this store are touched.
    pub fn prune(&self, keep: usize) -> Result<usize> {
        if keep == 0 {
            return Err(Error::Invalid(
                "backup retention must be greater than zero".into(),
            ));
        }
        let expired = self.list()?.into_iter().skip(keep).collect::<Vec<_>>();
        for backup in &expired {
            if !is_managed_backup_id(&backup.id) {
                return Err(Error::Invalid(format!(
                    "unsafe backup ID {:?}; refusing to prune",
                    backup.id
                )));
            }
        }
        for backup in &expired {
            // Remove metadata first: an interrupted prune may leave an unlisted data file, but
            // never leaves a listed backup whose contents have already disappeared.
            fs::remove_file(self.directory.join(format!("{}.json", backup.id)))?;
            fs::remove_file(self.directory.join(format!("{}.hosts", backup.id)))?;
        }
        Ok(expired.len())
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

#[cfg(test)]
mod tests {
    use super::*;

    fn write_backup(store: &BackupStore, id: &str, created_at: &str, content: &str) {
        let metadata = BackupMetadata {
            id: id.into(),
            created_at: created_at.into(),
            source_path: PathBuf::from("/tmp/fixture-hosts"),
            sha256: sha256_hex(content.as_bytes()),
            size: content.len() as u64,
        };
        atomic_write(
            &store.directory.join(format!("{id}.hosts")),
            content.as_bytes(),
        )
        .unwrap();
        atomic_write(
            &store.directory.join(format!("{id}.json")),
            &serde_json::to_vec_pretty(&metadata).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn prune_removes_only_oldest_managed_backups() {
        let directory = tempfile::tempdir().unwrap();
        let store = BackupStore::new(directory.path().join("backups"));
        write_backup(
            &store,
            "20260101T000000Z-00000001",
            "2026-01-01T00:00:00Z",
            "old\n",
        );
        write_backup(
            &store,
            "20260103T000000Z-00000002",
            "2026-01-03T00:00:00Z",
            "new a\n",
        );
        write_backup(
            &store,
            "20260102T000000Z-00000003",
            "2026-01-02T00:00:00Z",
            "new b\n",
        );

        assert_eq!(store.prune(2).unwrap(), 1);
        let remaining = store.list().unwrap();
        assert_eq!(remaining.len(), 2);
        assert!(
            remaining
                .iter()
                .all(|backup| backup.id != "20260101T000000Z-00000001")
        );
        assert!(
            !store
                .directory
                .join("20260101T000000Z-00000001.hosts")
                .exists()
        );
        assert!(
            !store
                .directory
                .join("20260101T000000Z-00000001.json")
                .exists()
        );
        assert_eq!(store.prune(2).unwrap(), 0);
    }

    #[test]
    fn prune_rejects_zero_and_unsafe_ids_before_deleting() {
        let directory = tempfile::tempdir().unwrap();
        let store = BackupStore::new(directory.path().join("backups"));
        write_backup(
            &store,
            "20260102T000000Z-00000004",
            "2026-01-02T00:00:00Z",
            "safe\n",
        );
        assert!(store.prune(0).is_err());
        assert!(store.load("20260102T000000Z-00000004").is_ok());

        let outside_hosts = directory.path().join("outside.hosts");
        let outside_json = directory.path().join("outside.json");
        fs::write(&outside_hosts, "outside sentinel\n").unwrap();
        fs::write(&outside_json, "outside sentinel\n").unwrap();
        let unsafe_metadata = BackupMetadata {
            id: "../outside".into(),
            created_at: "2026-01-01T00:00:00Z".into(),
            source_path: PathBuf::from("/tmp/fixture-hosts"),
            sha256: sha256_hex(b"unsafe\n"),
            size: 7,
        };
        atomic_write(
            &store.directory.join("unsafe.json"),
            &serde_json::to_vec_pretty(&unsafe_metadata).unwrap(),
        )
        .unwrap();
        assert_eq!(store.prune(1).unwrap(), 0);
        assert!(store.load("20260102T000000Z-00000004").is_ok());
        assert_eq!(
            fs::read_to_string(outside_hosts).unwrap(),
            "outside sentinel\n"
        );
        assert_eq!(
            fs::read_to_string(outside_json).unwrap(),
            "outside sentinel\n"
        );
    }

    #[test]
    fn mismatched_metadata_cannot_nominate_a_valid_backup_for_deletion() {
        let directory = tempfile::tempdir().unwrap();
        let store = BackupStore::new(directory.path().join("backups"));
        let valid_id = "20260103T000000Z-00000002";
        write_backup(&store, valid_id, "2026-01-03T00:00:00Z", "keep me\n");

        let mismatched = BackupMetadata {
            id: valid_id.into(),
            created_at: "2026-01-01T00:00:00Z".into(),
            source_path: PathBuf::from("/tmp/fixture-hosts"),
            sha256: sha256_hex(b"do not delete\n"),
            size: 14,
        };
        atomic_write(
            &store.directory.join("20260101T000000Z-00000001.json"),
            &serde_json::to_vec_pretty(&mismatched).unwrap(),
        )
        .unwrap();

        assert_eq!(store.prune(1).unwrap(), 0);
        assert_eq!(store.load(valid_id).unwrap().1, "keep me\n");
        assert!(store.directory.join(format!("{valid_id}.hosts")).exists());
        assert!(store.directory.join(format!("{valid_id}.json")).exists());
    }
}
