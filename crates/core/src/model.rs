use std::collections::HashSet;
use std::net::IpAddr;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{Error, Result};

pub const SCHEMA_VERSION: u32 = 1;

fn new_id() -> String {
    Uuid::new_v4().to_string()
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Document {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub revision: u64,
    #[serde(default)]
    pub profiles: Vec<Profile>,
}

const fn default_schema_version() -> u32 {
    SCHEMA_VERSION
}

impl Default for Document {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            revision: 0,
            profiles: Vec::new(),
        }
    }
}

impl Document {
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != SCHEMA_VERSION {
            return Err(Error::Invalid(format!(
                "unsupported schema version {}; expected {SCHEMA_VERSION}",
                self.schema_version
            )));
        }
        let mut names = HashSet::new();
        let mut profile_ids = HashSet::new();
        let mut entry_ids = HashSet::new();
        for profile in &self.profiles {
            profile.validate()?;
            if !names.insert(profile.name.to_lowercase()) {
                return Err(Error::Invalid(format!(
                    "duplicate profile name {:?}",
                    profile.name
                )));
            }
            if !profile_ids.insert(&profile.id) {
                return Err(Error::Invalid(format!(
                    "duplicate profile id {}",
                    profile.id
                )));
            }
            for entry in &profile.entries {
                if !entry_ids.insert(&entry.id) {
                    return Err(Error::Invalid(format!("duplicate entry id {}", entry.id)));
                }
            }
        }
        Ok(())
    }

    pub fn profile(&self, name: &str) -> Result<&Profile> {
        self.profiles
            .iter()
            .find(|profile| profile.name == name)
            .ok_or_else(|| Error::NotFound(format!("profile {name:?}")))
    }

    pub fn profile_mut(&mut self, name: &str) -> Result<&mut Profile> {
        self.profiles
            .iter_mut()
            .find(|profile| profile.name == name)
            .ok_or_else(|| Error::NotFound(format!("profile {name:?}")))
    }

    pub fn add_profile(&mut self, profile: Profile) -> Result<()> {
        if self
            .profiles
            .iter()
            .any(|item| item.name.eq_ignore_ascii_case(&profile.name))
        {
            return Err(Error::AlreadyExists(format!("profile {:?}", profile.name)));
        }
        profile.validate()?;
        self.profiles.push(profile);
        Ok(())
    }

    pub fn remove_profile(&mut self, name: &str) -> Result<Profile> {
        let index = self
            .profiles
            .iter()
            .position(|profile| profile.name == name)
            .ok_or_else(|| Error::NotFound(format!("profile {name:?}")))?;
        Ok(self.profiles.remove(index))
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Profile {
    #[serde(default = "new_id")]
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub entries: Vec<HostEntry>,
}

impl Profile {
    pub fn new(name: impl Into<String>) -> Result<Self> {
        let profile = Self {
            id: new_id(),
            name: name.into().trim().to_owned(),
            description: String::new(),
            entries: Vec::new(),
        };
        profile.validate()?;
        Ok(profile)
    }

    pub fn validate(&self) -> Result<()> {
        if self.id.is_empty() {
            return Err(Error::Invalid("profile id cannot be empty".into()));
        }
        if self.name.is_empty() || self.name.chars().count() > 64 {
            return Err(Error::Invalid(
                "profile name must contain 1 to 64 characters".into(),
            ));
        }
        if self.name.chars().any(char::is_control) {
            return Err(Error::Invalid(
                "profile name cannot contain control characters".into(),
            ));
        }
        if self.description.contains(['\r', '\n']) {
            return Err(Error::Invalid(
                "profile description must be one line".into(),
            ));
        }
        for entry in &self.entries {
            entry.validate()?;
        }
        Ok(())
    }

    pub fn enabled_entries(&self) -> impl Iterator<Item = &HostEntry> {
        self.entries.iter().filter(|entry| entry.enabled)
    }

    pub fn render_enabled(&self) -> String {
        let mut output = self
            .enabled_entries()
            .map(HostEntry::render)
            .collect::<Vec<_>>()
            .join("\n");
        if !output.is_empty() {
            output.push('\n');
        }
        output
    }

    pub fn set_hostname_enabled(&mut self, hostname: &str, enabled: bool) -> Result<usize> {
        let hostname = normalize_hostname(hostname)?;
        let mut found = false;
        let mut changed = 0;
        let mut updated = Vec::with_capacity(self.entries.len());
        for mut entry in self.entries.drain(..) {
            let contains = entry
                .hostnames
                .iter()
                .any(|item| item.eq_ignore_ascii_case(&hostname));
            if !contains {
                updated.push(entry);
                continue;
            }
            found = true;
            if entry.enabled != enabled {
                if entry.hostnames.len() > 1 {
                    entry
                        .hostnames
                        .retain(|item| !item.eq_ignore_ascii_case(&hostname));
                    let mut split = entry.clone();
                    split.id = new_id();
                    split.hostnames = vec![hostname.clone()];
                    split.enabled = enabled;
                    updated.push(entry);
                    updated.push(split);
                } else {
                    entry.enabled = enabled;
                    updated.push(entry);
                }
                changed += 1;
            } else {
                updated.push(entry);
            }
        }
        self.entries = updated;
        if !found {
            return Err(Error::NotFound(format!("hostname {hostname:?}")));
        }
        Ok(changed)
    }

    pub fn remove_hostname(&mut self, hostname: &str) -> Result<usize> {
        let hostname = normalize_hostname(hostname)?;
        let mut removed = 0;
        for entry in &mut self.entries {
            let before = entry.hostnames.len();
            entry
                .hostnames
                .retain(|item| !item.eq_ignore_ascii_case(&hostname));
            removed += before - entry.hostnames.len();
        }
        self.entries.retain(|entry| !entry.hostnames.is_empty());
        if removed == 0 {
            return Err(Error::NotFound(format!("hostname {hostname:?}")));
        }
        Ok(removed)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct HostEntry {
    #[serde(default = "new_id")]
    pub id: String,
    pub ip: String,
    pub hostnames: Vec<String>,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
}

const fn default_enabled() -> bool {
    true
}

impl HostEntry {
    pub fn new(
        ip: impl AsRef<str>,
        hostnames: Vec<String>,
        enabled: bool,
        comment: Option<String>,
    ) -> Result<Self> {
        let ip = ip.as_ref().parse::<IpAddr>().map_err(|_| {
            Error::Invalid(format!(
                "{:?} is not a valid IPv4 or IPv6 address",
                ip.as_ref()
            ))
        })?;
        let mut normalized = Vec::new();
        for hostname in hostnames {
            let hostname = normalize_hostname(&hostname)?;
            if !normalized
                .iter()
                .any(|item: &String| item.eq_ignore_ascii_case(&hostname))
            {
                normalized.push(hostname);
            }
        }
        let entry = Self {
            id: new_id(),
            ip: ip.to_string(),
            hostnames: normalized,
            enabled,
            comment,
        };
        entry.validate()?;
        Ok(entry)
    }

    pub fn validate(&self) -> Result<()> {
        if self.id.is_empty() {
            return Err(Error::Invalid("entry id cannot be empty".into()));
        }
        self.ip
            .parse::<IpAddr>()
            .map_err(|_| Error::Invalid(format!("invalid IP address {:?}", self.ip)))?;
        if self.hostnames.is_empty() {
            return Err(Error::Invalid(format!(
                "entry {} must have at least one hostname",
                self.id
            )));
        }
        for hostname in &self.hostnames {
            validate_hostname(hostname)?;
        }
        if let Some(comment) = &self.comment {
            if comment.contains(['\r', '\n']) {
                return Err(Error::Invalid("entry comment must be one line".into()));
            }
            if comment.chars().count() > 1024 {
                return Err(Error::Invalid("entry comment is too long".into()));
            }
        }
        Ok(())
    }

    pub fn render(&self) -> String {
        let mut line = format!("{}\t{}", self.ip, self.hostnames.join(" "));
        if let Some(comment) = &self.comment
            && !comment.is_empty()
        {
            line.push_str("\t# ");
            line.push_str(comment);
        }
        line
    }
}

pub fn validate_hostname(hostname: &str) -> Result<()> {
    normalize_hostname(hostname).map(|_| ())
}

fn normalize_hostname(hostname: &str) -> Result<String> {
    let hostname = hostname.trim().trim_end_matches('.').to_lowercase();
    if hostname.is_empty() || hostname.len() > 253 {
        return Err(Error::Invalid(format!("invalid hostname {hostname:?}")));
    }
    if !hostname.is_ascii()
        || hostname
            .chars()
            .any(|ch| !(ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_')))
        || hostname.starts_with('.')
        || hostname.contains("..")
    {
        return Err(Error::Invalid(format!("invalid hostname {hostname:?}")));
    }
    if hostname.split('.').any(|label| {
        label.is_empty() || label.len() > 63 || label.starts_with('-') || label.ends_with('-')
    }) {
        return Err(Error::Invalid(format!("invalid hostname {hostname:?}")));
    }
    Ok(hostname)
}
