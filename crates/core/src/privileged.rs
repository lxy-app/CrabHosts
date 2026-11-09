use serde::{Deserialize, Serialize};

use crate::{
    ApplyReport, Error, Result, Storage, SystemPlan, sha256_hex,
    system::{make_plan, replace_managed_block},
    write_system_hosts,
};

pub const HELPER_PROTOCOL_VERSION: u16 = 1;
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

    if request.operation == "apply_profile" {
        let managed = extract_managed_block(&request.proposed, &request.source)?;
        let expected = replace_managed_block(&current, &request.source, &managed)?;
        if expected != request.proposed {
            return Err(Error::UnsafeBlock(
                "profile apply would modify content outside the CrabHosts managed block".into(),
            ));
        }
    }

    let plan = make_plan(
        &request.operation,
        &request.source,
        current,
        request.proposed.clone(),
        &request.source,
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
            changed: false,
            backup_id: None,
            hosts_sha256: plan.current_sha256,
        });
    }

    write_system_hosts(&storage.paths().hosts_file, &plan.current, &plan.proposed)?;
    Ok(ApplyReport {
        operation: plan.operation,
        source: plan.source,
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
    if request.operation != "apply_profile" && request.operation != "restore_backup" {
        return Err(Error::Invalid(format!(
            "unsupported privileged operation {:?}",
            request.operation
        )));
    }
    if request.source.is_empty() || request.source.len() > 256 {
        return Err(Error::Invalid("invalid privileged plan source".into()));
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

fn extract_managed_block(proposed: &str, profile_name: &str) -> Result<String> {
    let start_marker = format!("# >>> CrabHosts managed (profile: {profile_name})");
    let lines = proposed.lines().collect::<Vec<_>>();
    let starts = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| **line == start_marker)
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    let ends = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| **line == "# <<< CrabHosts managed")
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    if starts.len() != 1 || ends.len() != 1 || ends[0] <= starts[0] {
        return Err(Error::UnsafeBlock(
            "privileged apply requires one balanced managed block".into(),
        ));
    }
    Ok(lines[starts[0] + 1..ends[0]].join("\n"))
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;
    use crate::{AppPaths, Document, HostEntry, Profile, plan_profile};

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
    fn helper_rejects_stale_current_content() {
        let (_directory, storage, plan) = fixture();
        std::fs::write(storage.paths().hosts_file.as_path(), "127.0.0.1 changed\n").unwrap();
        let error = execute_privileged_write(&storage, &plan.write_request().unwrap()).unwrap_err();
        assert!(matches!(error, Error::Conflict));
        assert_eq!(storage.read_hosts().unwrap(), "127.0.0.1 changed\n");
    }

    #[test]
    fn helper_rejects_changes_outside_the_managed_block() {
        let (_directory, storage, plan) = fixture();
        let mut request = plan.write_request().unwrap();
        request.proposed = request.proposed.replace("localhost", "tampered.local");
        request.proposed_sha256 = sha256_hex(request.proposed.as_bytes());
        request.plan_id = sha256_hex(
            format!(
                "{}\0{}\0{}\0{}",
                request.operation, request.source, "127.0.0.1 localhost\n", request.proposed
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
