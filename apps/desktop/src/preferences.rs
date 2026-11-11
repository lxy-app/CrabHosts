use std::fs;

use crabhosts_core::{AppPaths, Error, Result, atomic_write};
use serde::{Deserialize, Serialize};

const SCHEMA_VERSION: u32 = 1;
const PREFERENCES_FILE: &str = "preferences.json";

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CloseAction {
    #[default]
    Tray,
    Quit,
}

impl CloseAction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Tray => "tray",
            Self::Quit => "quit",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Density {
    #[default]
    Comfortable,
    Compact,
}

impl Density {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Comfortable => "comfortable",
            Self::Compact => "compact",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ColorTheme {
    #[default]
    Dark,
    Light,
}

/// Device-local desktop preferences. These values never enter the synced Profile document.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct DesktopPreferences {
    pub schema_version: u32,
    pub launch_at_login: bool,
    pub show_tray: bool,
    pub close_action: CloseAction,
    pub theme: ColorTheme,
    pub density: Density,
    pub show_profile_counts: bool,
    pub reduce_motion: bool,
    pub backup_retention: usize,
}

impl Default for DesktopPreferences {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            launch_at_login: false,
            show_tray: true,
            close_action: CloseAction::Tray,
            theme: ColorTheme::Dark,
            density: Density::Comfortable,
            show_profile_counts: true,
            reduce_motion: false,
            backup_retention: 20,
        }
    }
}

impl DesktopPreferences {
    pub fn path(paths: &AppPaths) -> std::path::PathBuf {
        paths.config_dir.join(PREFERENCES_FILE)
    }

    pub fn load(paths: &AppPaths) -> Result<Self> {
        match fs::read_to_string(Self::path(paths)) {
            Ok(content) => {
                let mut preferences: Self = serde_json::from_str(&content)?;
                preferences.normalize();
                preferences.validate()?;
                Ok(preferences)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error.into()),
        }
    }

    pub fn save(&self, paths: &AppPaths) -> Result<()> {
        self.validate()?;
        let mut content = serde_json::to_vec_pretty(self)?;
        content.push(b'\n');
        atomic_write(&Self::path(paths), &content)
    }

    /// Applies a validated change transactionally; invalid input leaves `self` unchanged.
    pub fn apply(&mut self, key: &str, value: &str) -> Result<()> {
        let mut candidate = self.clone();
        candidate.apply_inner(key, value)?;
        candidate.normalize();
        candidate.validate()?;
        *self = candidate;
        Ok(())
    }

    fn apply_inner(&mut self, key: &str, value: &str) -> Result<()> {
        match key {
            "launch-at-login" => self.launch_at_login = parse_bool(value)?,
            "show-tray" => self.show_tray = parse_bool(value)?,
            "close-action" => {
                self.close_action = match value {
                    "tray" => CloseAction::Tray,
                    "quit" => CloseAction::Quit,
                    _ => return Err(Error::Invalid(format!("invalid close action {value:?}"))),
                };
                if self.close_action == CloseAction::Tray {
                    self.show_tray = true;
                }
            }
            "theme" => {
                self.theme = match value {
                    "light" => ColorTheme::Light,
                    "dark" => ColorTheme::Dark,
                    _ => return Err(Error::Invalid(format!("invalid theme {value:?}"))),
                };
            }
            "density" => {
                self.density = match value {
                    "comfortable" => Density::Comfortable,
                    "compact" => Density::Compact,
                    _ => return Err(Error::Invalid(format!("invalid density {value:?}"))),
                };
            }
            "show-profile-counts" => self.show_profile_counts = parse_bool(value)?,
            "reduce-motion" => self.reduce_motion = parse_bool(value)?,
            "backup-retention" => {
                self.backup_retention = value
                    .parse()
                    .map_err(|_| Error::Invalid(format!("invalid backup retention {value:?}")))?;
            }
            _ => {
                return Err(Error::Invalid(format!(
                    "unknown desktop preference {key:?}"
                )));
            }
        }
        Ok(())
    }

    fn normalize(&mut self) {
        if !self.show_tray {
            self.close_action = CloseAction::Quit;
        }
    }

    fn validate(&self) -> Result<()> {
        if self.schema_version != SCHEMA_VERSION {
            return Err(Error::Invalid(format!(
                "unsupported desktop preferences schema {}",
                self.schema_version
            )));
        }
        if !matches!(self.backup_retention, 10 | 20 | 50) {
            return Err(Error::Invalid(format!(
                "backup retention must be 10, 20, or 50, got {}",
                self.backup_retention
            )));
        }
        if !self.show_tray && self.close_action != CloseAction::Quit {
            return Err(Error::Invalid(
                "closing to the tray requires a visible Tray icon".into(),
            ));
        }
        Ok(())
    }
}

fn parse_bool(value: &str) -> Result<bool> {
    match value {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(Error::Invalid(format!("invalid boolean {value:?}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(directory: &std::path::Path) -> AppPaths {
        AppPaths::new(directory.join("config"), directory.join("hosts"))
    }

    #[test]
    fn missing_file_uses_safe_device_local_defaults() {
        let directory = tempfile::tempdir().unwrap();
        let preferences = DesktopPreferences::load(&paths(directory.path())).unwrap();
        assert_eq!(preferences, DesktopPreferences::default());
        assert!(!preferences.launch_at_login);
        assert!(preferences.show_tray);
    }

    #[test]
    fn preferences_round_trip_atomically() {
        let directory = tempfile::tempdir().unwrap();
        let paths = paths(directory.path());
        let mut preferences = DesktopPreferences::default();
        preferences.apply("density", "compact").unwrap();
        preferences.apply("backup-retention", "50").unwrap();
        preferences.apply("reduce-motion", "true").unwrap();
        preferences.save(&paths).unwrap();

        let loaded = DesktopPreferences::load(&paths).unwrap();
        assert_eq!(loaded, preferences);
        assert_eq!(loaded.density, Density::Compact);
        assert_eq!(loaded.backup_retention, 50);
        assert!(loaded.reduce_motion);
    }

    #[test]
    fn tray_and_close_action_stay_consistent() {
        let mut preferences = DesktopPreferences::default();
        preferences.apply("show-tray", "false").unwrap();
        assert_eq!(preferences.close_action, CloseAction::Quit);
        preferences.apply("close-action", "tray").unwrap();
        assert!(preferences.show_tray);
        assert_eq!(preferences.close_action, CloseAction::Tray);
    }

    #[test]
    fn invalid_or_corrupt_preferences_are_rejected_without_overwriting() {
        let directory = tempfile::tempdir().unwrap();
        let paths = paths(directory.path());
        let path = DesktopPreferences::path(&paths);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "{broken").unwrap();
        assert!(DesktopPreferences::load(&paths).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "{broken");

        let mut preferences = DesktopPreferences::default();
        assert!(preferences.apply("backup-retention", "7").is_err());
        assert!(preferences.apply("show-tray", "yes").is_err());
        assert!(preferences.apply("unknown", "true").is_err());
        assert_eq!(preferences, DesktopPreferences::default());
    }

    #[test]
    fn unknown_schema_and_fields_fail_closed() {
        let directory = tempfile::tempdir().unwrap();
        let paths = paths(directory.path());
        let path = DesktopPreferences::path(&paths);
        fs::create_dir_all(path.parent().unwrap()).unwrap();

        fs::write(&path, r#"{"schema_version":99}"#).unwrap();
        assert!(DesktopPreferences::load(&paths).is_err());
        fs::write(&path, r#"{"schema_version":1,"unexpected":true}"#).unwrap();
        assert!(DesktopPreferences::load(&paths).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn saving_refuses_a_symlinked_preferences_file() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().unwrap();
        let paths = paths(directory.path());
        let path = DesktopPreferences::path(&paths);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let target = directory.path().join("target");
        fs::write(&target, "do not overwrite").unwrap();
        symlink(&target, &path).unwrap();

        assert!(DesktopPreferences::default().save(&paths).is_err());
        assert_eq!(fs::read_to_string(target).unwrap(), "do not overwrite");
    }
}
