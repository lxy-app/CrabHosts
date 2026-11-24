use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::{Document, Error, Result};

pub const ACTIVATION_SCHEMA_VERSION: u32 = 1;

/// Device-local Profile activation state. This never enters the synced Profile document.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ActivationState {
    pub schema_version: u32,
    pub revision: u64,
    pub active_profile_ids: Vec<String>,
}

impl Default for ActivationState {
    fn default() -> Self {
        Self {
            schema_version: ACTIVATION_SCHEMA_VERSION,
            revision: 0,
            active_profile_ids: Vec::new(),
        }
    }
}

impl ActivationState {
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != ACTIVATION_SCHEMA_VERSION {
            return Err(Error::Invalid(format!(
                "unsupported activation schema {}; expected {ACTIVATION_SCHEMA_VERSION}",
                self.schema_version
            )));
        }
        let mut ids = HashSet::new();
        for id in &self.active_profile_ids {
            if id.is_empty() || id.len() > 128 || id.chars().any(char::is_control) {
                return Err(Error::Invalid(
                    "active profile id must contain 1 to 128 characters without control characters"
                        .into(),
                ));
            }
            if !ids.insert(id) {
                return Err(Error::Invalid(format!("duplicate active profile id {id}")));
            }
        }
        Ok(())
    }

    pub fn is_active(&self, profile_id: &str) -> bool {
        self.active_profile_ids
            .iter()
            .any(|candidate| candidate == profile_id)
    }

    /// Changes one Profile's activation without touching system hosts.
    pub fn set_active(&mut self, profile_id: &str, active: bool) -> Result<bool> {
        if profile_id.is_empty()
            || profile_id.len() > 128
            || profile_id.chars().any(char::is_control)
        {
            return Err(Error::Invalid(
                "active profile id must contain 1 to 128 characters without control characters"
                    .into(),
            ));
        }
        let position = self
            .active_profile_ids
            .iter()
            .position(|candidate| candidate == profile_id);
        let changed = match (active, position) {
            (true, None) => {
                self.active_profile_ids.push(profile_id.to_owned());
                true
            }
            (false, Some(index)) => {
                self.active_profile_ids.remove(index);
                true
            }
            _ => false,
        };
        self.validate()?;
        Ok(changed)
    }

    /// Deleted Profiles cannot participate in a future composition.
    pub fn retain_existing(&mut self, document: &Document) -> bool {
        let before = self.active_profile_ids.len();
        self.active_profile_ids
            .retain(|id| document.profiles.iter().any(|profile| &profile.id == id));
        self.active_profile_ids.len() != before
    }

    /// Recovers activation from a managed block when the local state file is absent.
    /// New blocks carry stable IDs; legacy blocks carry one Profile name.
    pub(crate) fn recover_from_hosts(hosts: &str, document: &Document) -> Self {
        let mut state = Self::default();
        let mut inside_managed = false;
        let mut legacy_name = None;

        for line in hosts.lines() {
            if line.starts_with("# >>> CrabHosts managed") {
                inside_managed = true;
                if let Some(name) = line
                    .strip_prefix("# >>> CrabHosts managed (profile: ")
                    .and_then(|value| value.strip_suffix(')'))
                {
                    legacy_name = Some(name);
                }
                continue;
            }
            if line == "# <<< CrabHosts managed" {
                inside_managed = false;
                continue;
            }
            if inside_managed && let Some(id) = line.strip_prefix("# profile-id: ") {
                let id = id.trim();
                if document.profiles.iter().any(|profile| profile.id == id) && !state.is_active(id)
                {
                    state.active_profile_ids.push(id.to_owned());
                }
            }
        }

        if state.active_profile_ids.is_empty()
            && let Some(name) = legacy_name
            && let Some(profile) = document
                .profiles
                .iter()
                .find(|profile| profile.name == name)
        {
            state.active_profile_ids.push(profile.id.clone());
        }
        state
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Profile;

    #[test]
    fn activation_is_ordered_and_idempotent() {
        let mut state = ActivationState::default();
        assert!(state.set_active("dev-id", true).unwrap());
        assert!(state.set_active("work-id", true).unwrap());
        assert!(!state.set_active("dev-id", true).unwrap());
        assert_eq!(state.active_profile_ids, ["dev-id", "work-id"]);
        assert!(state.set_active("dev-id", false).unwrap());
        assert_eq!(state.active_profile_ids, ["work-id"]);
        assert!(!state.set_active("dev-id", false).unwrap());
    }

    #[test]
    fn recovers_new_and_legacy_activation() {
        let mut dev = Profile::new("dev").unwrap();
        dev.id = "dev-id".into();
        let mut work = Profile::new("work").unwrap();
        work.id = "work-id".into();
        let document = Document {
            profiles: vec![dev, work],
            ..Document::default()
        };
        let current = "# >>> CrabHosts managed\n# profile-id: dev-id\n# profile-id: work-id\n# <<< CrabHosts managed\n";
        assert_eq!(
            ActivationState::recover_from_hosts(current, &document).active_profile_ids,
            ["dev-id", "work-id"]
        );
        let legacy =
            "# >>> CrabHosts managed (profile: dev)\n10.0.0.2 api.local\n# <<< CrabHosts managed\n";
        assert_eq!(
            ActivationState::recover_from_hosts(legacy, &document).active_profile_ids,
            ["dev-id"]
        );
    }
}
