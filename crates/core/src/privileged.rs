use serde::{Deserialize, Serialize};

use crate::{
    ApplyReport, Error, Result, Storage, SystemPlan, sha256_hex,
    system::{MANAGED_END, MANAGED_START, make_plan, replace_managed_block},
    write_system_hosts,
};

pub const HELPER_PROTOCOL_VERSION: u16 = 2;
pub const MAX_HOSTS_BYTES: usize = 4 * 1024 * 1024;

/// The complete, path-independent payload accepted by the privileged helper.
///
/// The helper reads the current hosts content itself and reconstructs the plan before writing.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HostsWriteRequest {
    pub protocol_version: u16,
    pub plan_id: String,
    pub operation: String,
    pub source: String,
    pub profile_ids: Vec<String>,
    pub profile_names: Vec<String>,
    pub current_sha256: String,
    pub proposed_sha256: String,
    pub proposed: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum HelperRequest {
    Status { protocol_version: u16 },
    Write(HostsWriteRequest),
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct HelperStatus {
    pub protocol_version: u16,
    pub helper_version: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HelperResponse {
    Status(HelperStatus),
    Write(ApplyReport),
    Error { code: String, message: String },
}

impl HelperResponse {
    pub fn error(error: &Error) -> Self {
        Self::Error {
            code: error.code().to_owned(),
            message: error.to_string(),
        }
    }
}

impl SystemPlan {
    pub fn write_request(&self) -> Result<HostsWriteRequest> {
        if !self.changed {
            return Err(Error::Invalid(
                "a system plan without changes cannot be sent to the helper".into(),
            ));
        }
        Ok(HostsWriteRequest {
            protocol_version: HELPER_PROTOCOL_VERSION,
            plan_id: self.plan_id.clone(),
            operation: self.operation.clone(),
            source: self.source.clone(),
            profile_ids: self.profile_ids.clone(),
            profile_names: self.profile_names.clone(),
            current_sha256: self.current_sha256.clone(),
            proposed_sha256: self.proposed_sha256.clone(),
            proposed: self.proposed.clone(),
        })
    }
}

/// Validates and executes a privileged write against storage with an explicitly fixed hosts path.
/// Backup creation deliberately remains in the unprivileged client process.
pub fn execute_privileged_write(
    storage: &Storage,
    request: &HostsWriteRequest,
) -> Result<ApplyReport> {
    validate_request(request)?;
    let current = storage.read_hosts()?;
    if current.len() > MAX_HOSTS_BYTES {
        return Err(Error::Invalid(format!(
            "system hosts exceeds the {MAX_HOSTS_BYTES}-byte helper limit"
        )));
    }
    if sha256_hex(current.as_bytes()) != request.current_sha256 {
        return Err(Error::Conflict);
    }

    if request.operation == "apply_profiles" {
        let managed = extract_managed_block(
            &request.proposed,
            &request.profile_ids,
            &request.profile_names,
        )?;
        let expected = replace_managed_block(&current, &managed)?;
        if expected != request.proposed {
            return Err(Error::UnsafeBlock(
                "Profile apply would modify content outside the CrabHosts managed block".into(),
            ));
        }
    }

    let plan = make_plan(
        &request.operation,
        &request.source,
        current,
        request.proposed.clone(),
        &request.source,
        &request.profile_ids,
        &request.profile_names,
    );
    if plan.plan_id != request.plan_id
        || plan.current_sha256 != request.current_sha256
        || plan.proposed_sha256 != request.proposed_sha256
    {
        return Err(Error::Conflict);
    }
    if !plan.changed {
        return Ok(ApplyReport {
            operation: plan.operation,
            source: plan.source,
            profile_ids: plan.profile_ids,
            profile_names: plan.profile_names,
            changed: false,
            backup_id: None,
            hosts_sha256: plan.current_sha256,
        });
    }

    write_system_hosts(&storage.paths().hosts_file, &plan.current, &plan.proposed)?;
    Ok(ApplyReport {
        operation: plan.operation,
        source: plan.source,
        profile_ids: plan.profile_ids,
        profile_names: plan.profile_names,
        changed: true,
        backup_id: None,
        hosts_sha256: plan.proposed_sha256,
    })
}

fn validate_request(request: &HostsWriteRequest) -> Result<()> {
    if request.protocol_version != HELPER_PROTOCOL_VERSION {
        return Err(Error::Invalid(format!(
            "unsupported helper protocol version {}",
            request.protocol_version
        )));
    }
    if request.operation != "apply_profiles" && request.operation != "restore_backup" {
        return Err(Error::Invalid(format!(
            "unsupported privileged operation {:?}",
            request.operation
        )));
    }
    if request.source.is_empty() || request.source.len() > 64 * 1024 {
        return Err(Error::Invalid("invalid privileged plan source".into()));
    }
    if request.profile_ids.len() != request.profile_names.len()
        || request.profile_ids.len() > 1024
        || request
            .profile_ids
            .iter()
            .any(|id| id.is_empty() || id.len() > 128 || id.chars().any(char::is_control))
        || request.profile_names.iter().any(|name| {
            name.is_empty() || name.chars().count() > 64 || name.chars().any(char::is_control)
        })
    {
        return Err(Error::Invalid(
            "invalid privileged Profile identities".into(),
        ));
    }
    if request.operation == "restore_backup"
        && (!request.profile_ids.is_empty() || !request.profile_names.is_empty())
    {
        return Err(Error::Invalid(
            "restore plans cannot contain Profile identities".into(),
        ));
    }
    let unique_ids = request
        .profile_ids
        .iter()
        .collect::<std::collections::HashSet<_>>();
    if unique_ids.len() != request.profile_ids.len() {
        return Err(Error::Invalid(
            "duplicate Profile identity in privileged plan".into(),
        ));
    }
    if !is_sha256(&request.plan_id)
        || !is_sha256(&request.current_sha256)
        || !is_sha256(&request.proposed_sha256)
    {
        return Err(Error::Invalid("invalid privileged plan hash".into()));
    }
    if request.proposed.len() > MAX_HOSTS_BYTES {
        return Err(Error::Invalid(format!(
            "proposed hosts exceeds the {MAX_HOSTS_BYTES}-byte helper limit"
        )));
    }
    if sha256_hex(request.proposed.as_bytes()) != request.proposed_sha256 {
        return Err(Error::Conflict);
    }
    Ok(())
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn extract_managed_block(
    proposed: &str,
    profile_ids: &[String],
    profile_names: &[String],
) -> Result<String> {
    let lines = proposed.lines().collect::<Vec<_>>();
    let starts = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| line.starts_with("# >>> CrabHosts managed"))
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    let ends = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| **line == MANAGED_END)
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    if starts.is_empty() && ends.is_empty() {
        if !profile_ids.is_empty() {
            return Err(Error::UnsafeBlock(
                "a non-empty Profile composition requires a managed block".into(),
            ));
        }
        return Ok(String::new());
    }
    if starts.len() != 1
        || ends.len() != 1
        || ends[0] <= starts[0]
        || lines[starts[0]] != MANAGED_START
    {
        return Err(Error::UnsafeBlock(
            "privileged apply requires zero or one balanced generic managed block".into(),
        ));
    }
    let content = &lines[starts[0] + 1..ends[0]];
    if profile_ids.is_empty() && !content.is_empty() {
        return Err(Error::UnsafeBlock(
            "an empty Profile composition cannot emit managed content".into(),
        ));
    }
    let actual_identities = content
        .iter()
        .filter(|line| line.starts_with("# profile: ") || line.starts_with("# profile-id: "))
        .copied()
        .collect::<Vec<_>>();
    let expected_identities = profile_ids
        .iter()
        .zip(profile_names)
        .flat_map(|(id, name)| [format!("# profile: {name}"), format!("# profile-id: {id}")])
        .collect::<Vec<_>>();
    if actual_identities != expected_identities {
        return Err(Error::UnsafeBlock(
            "managed Profile sections do not match the planned Profile identities".into(),
        ));
    }
    Ok(content.join("\n"))
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;
    use crate::{AppPaths, Document, HostEntry, Profile, plan_profile, plan_profiles};

    fn fixture() -> (tempfile::TempDir, Storage, SystemPlan) {
        let directory = tempdir().unwrap();
        let hosts = directory.path().join("hosts");
        std::fs::write(&hosts, "127.0.0.1 localhost\n").unwrap();
        let storage = Storage::new(AppPaths::new(directory.path().join("home"), hosts));
        let mut profile = Profile::new("dev").unwrap();
        profile.entries =
            vec![HostEntry::new("127.0.0.1", vec!["api.local".into()], true, None).unwrap()];
        let document = Document {
            schema_version: 1,
            revision: 0,
            profiles: vec![profile],
        };
        let plan = plan_profile(&storage, &document, "dev").unwrap();
        (directory, storage, plan)
    }

    #[test]
    fn helper_reconstructs_and_applies_a_valid_plan() {
        let (_directory, storage, plan) = fixture();
        let request = plan.write_request().unwrap();
        let report = execute_privileged_write(&storage, &request).unwrap();
        assert!(report.changed);
        assert_eq!(report.backup_id, None);
        let content = storage.read_hosts().unwrap();
        assert!(content.contains("127.0.0.1 localhost"));
        assert!(content.contains("127.0.0.1\tapi.local"));
    }

    #[test]
    fn helper_can_remove_the_managed_block_for_an_empty_composition() {
        let (_directory, storage, plan) = fixture();
        execute_privileged_write(&storage, &plan.write_request().unwrap()).unwrap();
        assert!(storage.read_hosts().unwrap().contains("api.local"));
        let empty = plan_profiles(&storage, &Document::default(), &[]).unwrap();
        let report = execute_privileged_write(&storage, &empty.write_request().unwrap()).unwrap();
        assert!(report.changed);
        assert!(report.profile_ids.is_empty());
        let content = storage.read_hosts().unwrap();
        assert!(!content.contains("CrabHosts managed"));
        assert!(!content.contains("api.local"));
        assert!(content.contains("127.0.0.1 localhost"));
    }

    #[test]
    fn protocol_two_restore_still_restores_a_complete_snapshot() {
        let (_directory, storage, _plan) = fixture();
        let backup = storage
            .backups()
            .create("127.0.0.1 restored\n", &storage.paths().hosts_file)
            .unwrap();
        let restore = crate::plan_restore(&storage, &backup.id).unwrap();
        let request = restore.write_request().unwrap();
        assert_eq!(request.protocol_version, HELPER_PROTOCOL_VERSION);
        assert!(request.profile_ids.is_empty());
        let report = execute_privileged_write(&storage, &request).unwrap();
        assert!(report.changed);
        assert_eq!(storage.read_hosts().unwrap(), "127.0.0.1 restored\n");
    }

    #[test]
    fn helper_rejects_stale_current_content() {
        let (_directory, storage, plan) = fixture();
        std::fs::write(storage.paths().hosts_file.as_path(), "127.0.0.1 changed\n").unwrap();
        let error = execute_privileged_write(&storage, &plan.write_request().unwrap()).unwrap_err();
        assert!(matches!(error, Error::Conflict));
        assert_eq!(storage.read_hosts().unwrap(), "127.0.0.1 changed\n");
    }

    #[test]
    fn helper_rejects_managed_content_from_an_unplanned_profile() {
        let (_directory, storage, plan) = fixture();
        let mut request = plan.write_request().unwrap();
        request.proposed = request
            .proposed
            .replace("# profile: dev", "# profile: injected");
        request.proposed_sha256 = sha256_hex(request.proposed.as_bytes());
        let error = execute_privileged_write(&storage, &request).unwrap_err();
        assert!(matches!(error, Error::UnsafeBlock(_)));
        assert_eq!(storage.read_hosts().unwrap(), "127.0.0.1 localhost\n");
    }

    #[test]
    fn helper_rejects_changes_outside_the_managed_block() {
        let (_directory, storage, plan) = fixture();
        let mut request = plan.write_request().unwrap();
        request.proposed = request.proposed.replace("localhost", "tampered.local");
        request.proposed_sha256 = sha256_hex(request.proposed.as_bytes());
        request.plan_id = sha256_hex(
            format!(
                "{}\0{}\0{}\0{}\0{}",
                request.operation,
                request.source,
                request.profile_ids.join("\0"),
                "127.0.0.1 localhost\n",
                request.proposed
            )
            .as_bytes(),
        );
        let error = execute_privileged_write(&storage, &request).unwrap_err();
        assert!(matches!(error, Error::UnsafeBlock(_)));
        assert_eq!(storage.read_hosts().unwrap(), "127.0.0.1 localhost\n");
    }

    #[test]
    fn helper_request_rejects_unknown_fields() {
        let (_directory, _storage, plan) = fixture();
        let mut value = serde_json::to_value(plan.write_request().unwrap()).unwrap();
        value["target_path"] = serde_json::Value::String("/tmp/unsafe".into());
        assert!(serde_json::from_value::<HostsWriteRequest>(value).is_err());
    }

    #[test]
    fn helper_rejects_a_mismatched_plan_id() {
        let (_directory, storage, plan) = fixture();
        let mut request = plan.write_request().unwrap();
        request.plan_id = "0".repeat(64);
        let error = execute_privileged_write(&storage, &request).unwrap_err();
        assert!(matches!(error, Error::Conflict));
        assert_eq!(storage.read_hosts().unwrap(), "127.0.0.1 localhost\n");
    }
}
