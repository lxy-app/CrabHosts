use std::fs::{File, Metadata, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;

use tempfile::NamedTempFile;

use crate::{Error, Result};

/// Atomically replaces an existing system hosts file after exact content validation.
///
/// The target and its parent must already exist. The final path is opened without following
/// symbolic links, and only regular files are accepted. No directory is created implicitly.
pub fn write_system_hosts(path: &Path, expected_current: &str, proposed: &str) -> Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .ok_or_else(|| Error::Invalid(format!("path {} has no parent", path.display())))?;

    let (actual_current, original_metadata) = read_regular_file(path)?;
    if actual_current != expected_current.as_bytes() {
        return Err(Error::Conflict);
    }

    let mut temporary = NamedTempFile::new_in(parent)?;
    temporary.write_all(proposed.as_bytes())?;
    temporary.flush()?;
    preserve_metadata(temporary.as_file(), &original_metadata)?;
    temporary.as_file().sync_all()?;

    // Recheck both file identity and content after preparing the replacement. This does not
    // claim to eliminate races with non-cooperating writers, but avoids replacing a file that
    // observably changed while the temporary file was being written.
    let (current_before_rename, metadata_before_rename) = read_regular_file(path)?;
    if !same_file(&original_metadata, &metadata_before_rename)
        || current_before_rename != expected_current.as_bytes()
    {
        return Err(Error::Conflict);
    }

    temporary
        .persist(path)
        .map_err(|error| Error::Io(error.error))?;
    sync_parent(parent)?;

    let (written, _) = read_regular_file(path)?;
    if written != proposed.as_bytes() {
        return Err(Error::Conflict);
    }
    Ok(())
}

fn read_regular_file(path: &Path) -> Result<(Vec<u8>, Metadata)> {
    let mut options = OpenOptions::new();
    options.read(true);

    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }

    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // Open a reparse point itself so the regular-file check below rejects it.
        const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }

    #[cfg(not(any(unix, windows)))]
    {
        let metadata = std::fs::symlink_metadata(path)?;
        if !metadata.file_type().is_file() {
            return Err(not_regular_file(path));
        }
    }

    let mut file = options.open(path)?;
    let metadata = file.metadata()?;
    if !metadata.file_type().is_file() {
        return Err(not_regular_file(path));
    }

    let mut content = Vec::new();
    file.read_to_end(&mut content)?;
    Ok((content, metadata))
}

fn not_regular_file(path: &Path) -> Error {
    Error::Invalid(format!(
        "system hosts path {} is not a regular file",
        path.display()
    ))
}

fn preserve_metadata(file: &File, original: &Metadata) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;
        use std::os::unix::fs::MetadataExt;

        let current = file.metadata()?;
        if current.uid() != original.uid() || current.gid() != original.gid() {
            // SAFETY: `file` owns a valid open descriptor for the duration of this call.
            // `fchown` does not retain the descriptor or any Rust pointers.
            let result = unsafe { libc::fchown(file.as_raw_fd(), original.uid(), original.gid()) };
            if result == -1 {
                return Err(std::io::Error::last_os_error().into());
            }
        }
    }

    // Set permissions after ownership because chown may clear permission bits on some systems.
    file.set_permissions(original.permissions())?;
    Ok(())
}

#[cfg(unix)]
fn same_file(left: &Metadata, right: &Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    left.dev() == right.dev() && left.ino() == right.ino()
}

#[cfg(not(unix))]
fn same_file(_left: &Metadata, _right: &Metadata) -> bool {
    // Stable Rust exposes no portable file ID. Content is still rechecked on every platform.
    true
}

#[cfg(unix)]
fn sync_parent(parent: &Path) -> Result<()> {
    match File::open(parent)?.sync_all() {
        Ok(()) => Ok(()),
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::InvalidInput | std::io::ErrorKind::Unsupported
            ) =>
        {
            Ok(())
        }
        Err(error) => Err(error.into()),
    }
}

#[cfg(not(unix))]
fn sync_parent(_parent: &Path) -> Result<()> {
    // Directory fsync is not a portable Windows operation.
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::*;

    #[test]
    fn writes_and_verifies_proposed_content() {
        let directory = tempdir().unwrap();
        let hosts = directory.path().join("hosts");
        fs::write(&hosts, "127.0.0.1 localhost\n").unwrap();

        write_system_hosts(
            &hosts,
            "127.0.0.1 localhost\n",
            "127.0.0.1 localhost\n10.0.0.2 api.local\n",
        )
        .unwrap();

        assert_eq!(
            fs::read_to_string(&hosts).unwrap(),
            "127.0.0.1 localhost\n10.0.0.2 api.local\n"
        );
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[test]
    fn rejects_stale_content_without_writing() {
        let directory = tempdir().unwrap();
        let hosts = directory.path().join("hosts");
        fs::write(&hosts, "changed elsewhere\n").unwrap();

        let error = write_system_hosts(&hosts, "expected\n", "proposed\n").unwrap_err();

        assert!(matches!(error, Error::Conflict));
        assert_eq!(fs::read_to_string(&hosts).unwrap(), "changed elsewhere\n");
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symbolic_link_without_touching_its_target() {
        use std::os::unix::fs::symlink;

        let directory = tempdir().unwrap();
        let target = directory.path().join("target");
        let hosts = directory.path().join("hosts");
        fs::write(&target, "original\n").unwrap();
        symlink(&target, &hosts).unwrap();

        assert!(write_system_hosts(&hosts, "original\n", "proposed\n").is_err());

        assert_eq!(fs::read_to_string(&target).unwrap(), "original\n");
        assert!(
            fs::symlink_metadata(&hosts)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 2);
    }

    #[test]
    fn rejects_missing_target_without_creating_it() {
        let directory = tempdir().unwrap();
        let hosts = directory.path().join("hosts");

        let error = write_system_hosts(&hosts, "", "proposed\n").unwrap_err();

        assert!(matches!(
            error,
            Error::Io(ref inner) if inner.kind() == std::io::ErrorKind::NotFound
        ));
        assert!(!hosts.exists());
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 0);
    }

    #[test]
    fn rejects_missing_parent_without_creating_it() {
        let directory = tempdir().unwrap();
        let parent = directory.path().join("missing");
        let hosts = parent.join("hosts");

        assert!(write_system_hosts(&hosts, "", "proposed\n").is_err());

        assert!(!parent.exists());
    }

    #[cfg(unix)]
    #[test]
    fn preserves_unix_mode_and_ownership() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};

        let directory = tempdir().unwrap();
        let hosts = directory.path().join("hosts");
        fs::write(&hosts, "original\n").unwrap();
        fs::set_permissions(&hosts, fs::Permissions::from_mode(0o640)).unwrap();
        let original = fs::metadata(&hosts).unwrap();

        write_system_hosts(&hosts, "original\n", "proposed\n").unwrap();

        let written = fs::metadata(&hosts).unwrap();
        assert_eq!(written.permissions().mode() & 0o777, 0o640);
        assert_eq!(written.uid(), original.uid());
        assert_eq!(written.gid(), original.gid());
    }
}
