use std::fs;
use std::path::{Path, PathBuf};

use crabhosts_core::{Error, Result, atomic_write};

const PROFILE_BEGIN: &str = "# >>> CrabHosts CLI >>>";
const PROFILE_END: &str = "# <<< CrabHosts CLI <<<";
const PROFILE_BLOCK: &str = r#"# >>> CrabHosts CLI >>>
# Keep ~/.local/bin available to the `crab` command in new zsh sessions.
case ":$PATH:" in
    *":$HOME/.local/bin:"*) ;;
    *) export PATH="$HOME/.local/bin${PATH:+:$PATH}" ;;
esac
# <<< CrabHosts CLI <<<
"#;

#[derive(Debug)]
pub struct InstallReport {
    pub link: PathBuf,
    pub profile: PathBuf,
}

pub fn bundled_cli_available() -> bool {
    bundled_cli_path().is_ok()
}

pub fn install_bundled_cli() -> Result<InstallReport> {
    let bundled = bundled_cli_path()?;
    let home = std::env::var_os("HOME")
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| Error::Invalid("HOME is unavailable; cannot install the CLI".into()))?;
    install_at(&home, &bundled)
}

fn bundled_cli_path() -> Result<PathBuf> {
    let executable = std::env::current_exe()?;
    let macos_dir = executable.parent().ok_or_else(|| {
        Error::Invalid("the desktop executable has no containing directory".into())
    })?;
    if macos_dir.file_name().is_none_or(|name| name != "MacOS") {
        return Err(Error::Invalid(
            "CLI installation is available only from the packaged CrabHosts.app".into(),
        ));
    }
    let contents = macos_dir.parent().ok_or_else(|| {
        Error::Invalid("the desktop executable is not inside a Contents directory".into())
    })?;
    let bundled = contents.join("SharedSupport/crab");
    if !fs::metadata(&bundled).is_ok_and(|metadata| metadata.is_file()) {
        return Err(Error::NotFound(format!(
            "bundled CLI was not found at {}",
            bundled.display()
        )));
    }
    Ok(bundled)
}

#[cfg(unix)]
fn install_at(home: &Path, bundled: &Path) -> Result<InstallReport> {
    use std::os::unix::fs::symlink;

    let bundled = fs::canonicalize(bundled)?;
    if !fs::metadata(&bundled).is_ok_and(|metadata| metadata.is_file()) {
        return Err(Error::Invalid(format!(
            "bundled CLI is not a regular file: {}",
            bundled.display()
        )));
    }

    let profile = home.join(".zprofile");
    let previous_profile = match fs::read_to_string(&profile) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error.into()),
    };
    // Validate the existing profile before creating a link, so malformed managed markers cannot
    // leave a half-installed command behind.
    let next_profile = profile_with_path(&previous_profile)?;

    let bin = home.join(".local/bin");
    let link = bin.join("crab");
    let link_exists = match fs::symlink_metadata(&link) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            let points_to_bundled = fs::canonicalize(&link).is_ok_and(|target| target == bundled);
            if !points_to_bundled {
                return Err(Error::AlreadyExists(format!(
                    "refusing to replace the existing crab command at {}",
                    link.display()
                )));
            }
            true
        }
        Ok(_) => {
            return Err(Error::AlreadyExists(format!(
                "refusing to replace the existing non-symlink at {}",
                link.display()
            )));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => return Err(error.into()),
    };

    // Complete predictable directory and profile work before creating the link. If link creation
    // then fails, the only partial result is a harmless, idempotent PATH configuration; no
    // rollback needs to delete a concurrently replaced link.
    if !link_exists {
        fs::create_dir_all(&bin)?;
    }
    if next_profile != previous_profile {
        atomic_write(&profile, next_profile.as_bytes())?;
    }
    if !link_exists && let Err(error) = symlink(&bundled, &link) {
        return Err(Error::Invalid(format!(
            "PATH configuration was saved to {}, but creating {} failed: {error}; retry after resolving the link path",
            profile.display(),
            link.display()
        )));
    }

    Ok(InstallReport { link, profile })
}

#[cfg(not(unix))]
fn install_at(_home: &Path, _bundled: &Path) -> Result<InstallReport> {
    Err(Error::Invalid(
        "the user-local CLI installer currently supports macOS and other Unix systems".into(),
    ))
}

fn profile_with_path(content: &str) -> Result<String> {
    let mut begin = None;
    let mut end = None;
    let mut offset = 0;
    for line in content.split_inclusive('\n') {
        let marker = line.strip_suffix('\n').unwrap_or(line);
        let marker = marker.strip_suffix('\r').unwrap_or(marker);
        if marker == PROFILE_BEGIN && begin.replace(offset).is_some() {
            return Err(Error::Invalid(
                "~/.zprofile contains duplicate CrabHosts CLI begin markers; fix that block before installing".into(),
            ));
        } else if marker == PROFILE_END && end.replace((offset, offset + line.len())).is_some() {
            return Err(Error::Invalid(
                "~/.zprofile contains duplicate CrabHosts CLI end markers; fix that block before installing".into(),
            ));
        }
        offset += line.len();
    }

    match (begin, end) {
        (None, None) => {
            let mut updated = content.to_owned();
            if !updated.is_empty() && !updated.ends_with('\n') {
                updated.push('\n');
            }
            if !updated.is_empty() {
                updated.push('\n');
            }
            updated.push_str(PROFILE_BLOCK);
            Ok(updated)
        }
        (Some(begin), Some((end_start, end))) if begin < end_start => {
            let mut updated = String::with_capacity(content.len() + PROFILE_BLOCK.len());
            updated.push_str(&content[..begin]);
            updated.push_str(PROFILE_BLOCK);
            updated.push_str(&content[end..]);
            Ok(updated)
        }
        (Some(_), Some(_)) => Err(Error::Invalid(
            "~/.zprofile contains reversed CrabHosts CLI markers; fix that block before installing".into(),
        )),
        _ => Err(Error::Invalid(
            "~/.zprofile contains an incomplete CrabHosts CLI block; fix that block before installing".into(),
        )),
    }
}

#[cfg(all(test, unix))]
mod tests {
    use std::os::unix::fs::symlink;

    use tempfile::tempdir;

    use super::*;

    fn fixture() -> (tempfile::TempDir, PathBuf) {
        let directory = tempdir().unwrap();
        let home = directory.path().join("home");
        let bundled = directory
            .path()
            .join("CrabHosts.app/Contents/SharedSupport/crab");
        fs::create_dir_all(bundled.parent().unwrap()).unwrap();
        fs::write(&bundled, "signed test executable").unwrap();
        fs::create_dir_all(&home).unwrap();
        (directory, home)
    }

    #[test]
    fn install_is_user_local_idempotent_and_preserves_zprofile() {
        let (directory, home) = fixture();
        let bundled = directory
            .path()
            .join("CrabHosts.app/Contents/SharedSupport/crab");
        fs::write(home.join(".zprofile"), "export EDITOR=vim\n").unwrap();

        let first = install_at(&home, &bundled).unwrap();
        assert_eq!(
            fs::read_link(&first.link).unwrap(),
            fs::canonicalize(&bundled).unwrap()
        );
        let expected = "export EDITOR=vim\n\n".to_owned() + PROFILE_BLOCK;
        assert_eq!(fs::read_to_string(&first.profile).unwrap(), expected);

        let second = install_at(&home, &bundled).unwrap();
        assert_eq!(
            fs::read_link(&second.link).unwrap(),
            fs::canonicalize(&bundled).unwrap()
        );
        assert_eq!(fs::read_to_string(&second.profile).unwrap(), expected);
    }

    #[test]
    fn install_replaces_only_its_own_complete_profile_block() {
        let (directory, home) = fixture();
        let bundled = directory
            .path()
            .join("CrabHosts.app/Contents/SharedSupport/crab");
        let profile = home.join(".zprofile");
        let stale =
            format!("before\n{PROFILE_BEGIN}\nexport PATH=/stale:$PATH\n{PROFILE_END}\nafter\n");
        fs::write(&profile, stale).unwrap();

        install_at(&home, &bundled).unwrap();
        let updated = fs::read_to_string(profile).unwrap();
        assert!(updated.starts_with("before\n"));
        assert!(updated.ends_with("after\n"));
        assert!(updated.contains(PROFILE_BLOCK));
        assert!(!updated.contains("/stale"));
    }

    #[test]
    fn install_refuses_to_overwrite_an_existing_command_or_bad_markers() {
        let (directory, home) = fixture();
        let bundled = directory
            .path()
            .join("CrabHosts.app/Contents/SharedSupport/crab");
        let bin = home.join(".local/bin");
        fs::create_dir_all(&bin).unwrap();
        fs::write(bin.join("crab"), "unrelated command").unwrap();
        let error = install_at(&home, &bundled).unwrap_err();
        assert!(matches!(error, Error::AlreadyExists(_)));
        assert_eq!(
            fs::read_to_string(bin.join("crab")).unwrap(),
            "unrelated command"
        );
        assert!(!home.join(".zprofile").exists());

        fs::remove_file(bin.join("crab")).unwrap();
        let other = directory.path().join("other-command");
        fs::write(&other, "other").unwrap();
        symlink(&other, bin.join("crab")).unwrap();
        let error = install_at(&home, &bundled).unwrap_err();
        assert!(matches!(error, Error::AlreadyExists(_)));
        assert_eq!(fs::read_link(bin.join("crab")).unwrap(), other);
        assert!(!home.join(".zprofile").exists());

        fs::remove_file(bin.join("crab")).unwrap();
        fs::write(home.join(".zprofile"), PROFILE_BEGIN).unwrap();
        let error = install_at(&home, &bundled).unwrap_err();
        assert!(matches!(error, Error::Invalid(_)));
        assert!(!bin.join("crab").exists());
        assert_eq!(
            fs::read_to_string(home.join(".zprofile")).unwrap(),
            PROFILE_BEGIN
        );
    }

    #[test]
    fn profile_markers_are_whole_lines_and_crlf_is_preserved_outside_the_block() {
        let content = format!("before\r\n{PROFILE_BEGIN}\r\nstale\r\n{PROFILE_END}\r\nafter\r\n");
        let updated = profile_with_path(&content).unwrap();
        assert_eq!(
            updated,
            "before\r\n".to_owned() + PROFILE_BLOCK + "after\r\n"
        );

        for malformed in [
            PROFILE_BEGIN.to_owned(),
            PROFILE_END.to_owned(),
            format!("{PROFILE_BEGIN}\n{PROFILE_BEGIN}\n{PROFILE_END}\n"),
            format!("{PROFILE_BEGIN}\n{PROFILE_END}\n{PROFILE_END}\n"),
            format!("{PROFILE_END}\n{PROFILE_BEGIN}\n"),
        ] {
            assert!(profile_with_path(&malformed).is_err(), "{malformed:?}");
        }
    }

    #[test]
    fn link_failure_reports_the_saved_path_and_can_be_retried() {
        use std::os::unix::fs::PermissionsExt;

        let (directory, home) = fixture();
        let bundled = directory
            .path()
            .join("CrabHosts.app/Contents/SharedSupport/crab");
        let bin = home.join(".local/bin");
        fs::create_dir_all(&bin).unwrap();
        fs::set_permissions(&bin, fs::Permissions::from_mode(0o555)).unwrap();

        let error = install_at(&home, &bundled).unwrap_err();
        assert!(error.to_string().contains("PATH configuration was saved"));
        assert!(
            fs::read_to_string(home.join(".zprofile"))
                .unwrap()
                .contains(PROFILE_BLOCK)
        );
        assert!(!bin.join("crab").exists());

        fs::set_permissions(&bin, fs::Permissions::from_mode(0o755)).unwrap();
        install_at(&home, &bundled).unwrap();
        assert!(
            fs::symlink_metadata(bin.join("crab"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
    }

    #[test]
    fn profile_failure_does_not_create_a_link_or_overwrite_a_symlink() {
        let (directory, home) = fixture();
        let bundled = directory
            .path()
            .join("CrabHosts.app/Contents/SharedSupport/crab");
        let outside = directory.path().join("outside-zprofile");
        fs::write(&outside, "do not overwrite\n").unwrap();
        symlink(&outside, home.join(".zprofile")).unwrap();

        let error = install_at(&home, &bundled).unwrap_err();
        assert!(matches!(error, Error::Invalid(_)));
        assert!(!home.join(".local/bin/crab").exists());
        assert_eq!(fs::read_to_string(outside).unwrap(), "do not overwrite\n");
    }
}
