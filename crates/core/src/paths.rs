use std::path::PathBuf;

use directories::ProjectDirs;

use crate::{Error, Result};

pub const CONFIG_HOME_ENV: &str = "CRABHOSTS_HOME";
pub const HOSTS_FILE_ENV: &str = "CRABHOSTS_HOSTS_FILE";

#[derive(Clone, Debug)]
pub struct AppPaths {
    pub config_dir: PathBuf,
    pub database_file: PathBuf,
    pub backups_dir: PathBuf,
    pub hosts_file: PathBuf,
}

impl AppPaths {
    pub fn from_env() -> Result<Self> {
        let config_dir = match std::env::var_os(CONFIG_HOME_ENV) {
            Some(path) => PathBuf::from(path),
            None => ProjectDirs::from("dev", "CrabHosts", "CrabHosts")
                .ok_or_else(|| Error::Invalid("cannot determine the user config directory".into()))?
                .config_dir()
                .to_path_buf(),
        };
        let hosts_file = std::env::var_os(HOSTS_FILE_ENV)
            .map(PathBuf::from)
            .unwrap_or_else(default_hosts_file);
        Ok(Self::new(config_dir, hosts_file))
    }

    pub fn new(config_dir: PathBuf, hosts_file: PathBuf) -> Self {
        Self {
            database_file: config_dir.join("profiles.json"),
            backups_dir: config_dir.join("backups"),
            config_dir,
            hosts_file,
        }
    }
}

#[cfg(windows)]
fn default_hosts_file() -> PathBuf {
    PathBuf::from(r"C:\Windows\System32\drivers\etc\hosts")
}

#[cfg(not(windows))]
fn default_hosts_file() -> PathBuf {
    PathBuf::from("/etc/hosts")
}
