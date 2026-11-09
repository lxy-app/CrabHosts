use serde::{Deserialize, Serialize};
use similar::TextDiff;

use crate::{Document, Error, Result, Storage, sha256_hex};

const START_PREFIX: &str = "# >>> CrabHosts managed";
const END_MARKER: &str = "# <<< CrabHosts managed";

#[derive(Clone, Debug, Serialize)]
pub struct SystemPlan {
    /// Binds approval to the operation, source, current content, and proposed content.
    pub plan_id: String,
    pub operation: String,
    pub source: String,
    pub changed: bool,
    pub current_sha256: String,
    pub proposed_sha256: String,
    pub diff: String,
    #[serde(skip)]
    pub current: String,
    #[serde(skip)]
    pub proposed: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ApplyReport {
    pub operation: String,
    pub source: String,
    pub changed: bool,
    pub backup_id: Option<String>,
    pub hosts_sha256: String,
}

pub fn plan_profile(storage: &Storage, document: &Document, name: &str) -> Result<SystemPlan> {
    let profile = document.profile(name)?;
    let current = storage.read_hosts()?;
    let proposed = replace_managed_block(&current, name, &profile.render_enabled())?;
    Ok(make_plan("apply_profile", name, current, proposed, name))
}

pub fn plan_restore(storage: &Storage, backup_id: &str) -> Result<SystemPlan> {
    let (_, proposed) = storage.backups().load(backup_id)?;
    let current = storage.read_hosts()?;
    Ok(make_plan(
        "restore_backup",
        backup_id,
        current,
        proposed,
        backup_id,
    ))
}

/// Applies exactly the supplied plan. A concurrent system hosts change aborts the write.
pub fn apply_plan(storage: &Storage, plan: &SystemPlan) -> Result<ApplyReport> {
    if !plan.changed {
        return Ok(ApplyReport {
            operation: plan.operation.clone(),
            source: plan.source.clone(),
            changed: false,
            backup_id: None,
            hosts_sha256: plan.current_sha256.clone(),
        });
    }
    if storage.read_hosts()? != plan.current {
        return Err(Error::Conflict);
    }
    let backup = storage
        .backups()
        .create(&plan.current, &storage.paths().hosts_file)?;
    storage.write_hosts(&plan.proposed)?;
    Ok(ApplyReport {
        operation: plan.operation.clone(),
        source: plan.source.clone(),
        changed: true,
        backup_id: Some(backup.id),
        hosts_sha256: plan.proposed_sha256.clone(),
    })
}

pub(crate) fn make_plan(
    operation: &str,
    source: &str,
    current: String,
    proposed: String,
    diff_label: &str,
) -> SystemPlan {
    let diff = TextDiff::from_lines(&current, &proposed)
        .unified_diff()
        .context_radius(3)
        .header("current-hosts", &format!("crabhosts-{diff_label}"))
        .to_string();
    let plan_id = sha256_hex(format!("{operation}\0{source}\0{current}\0{proposed}").as_bytes());
    SystemPlan {
        plan_id,
        operation: operation.to_owned(),
        source: source.to_owned(),
        changed: current != proposed,
        current_sha256: sha256_hex(current.as_bytes()),
        proposed_sha256: sha256_hex(proposed.as_bytes()),
        diff,
        current,
        proposed,
    }
}

pub(crate) fn replace_managed_block(
    current: &str,
    profile_name: &str,
    managed: &str,
) -> Result<String> {
    let lines = current.lines().collect::<Vec<_>>();
    let starts = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| line.starts_with(START_PREFIX))
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    let ends = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| **line == END_MARKER)
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    if starts.len() > 1 || ends.len() > 1 || starts.len() != ends.len() {
        return Err(Error::UnsafeBlock(
            "expected zero or one balanced pair of CrabHosts markers".into(),
        ));
    }
    if let (Some(start), Some(end)) = (starts.first(), ends.first())
        && end < start
    {
        return Err(Error::UnsafeBlock(
            "end marker precedes start marker".into(),
        ));
    }

    let mut output = Vec::new();
    if let Some(start) = starts.first().copied() {
        output.extend_from_slice(&lines[..start]);
    } else {
        output.extend_from_slice(&lines);
        if !output.is_empty() {
            output.push("");
        }
    }
    let start_marker = format!("# >>> CrabHosts managed (profile: {profile_name})");
    output.push(start_marker.as_str());
    output.extend(managed.lines());
    output.push(END_MARKER);
    if let Some(end) = ends.first().copied() {
        output.extend_from_slice(&lines[end + 1..]);
    }
    let newline = if current.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let mut rendered = output.join(newline);
    rendered.push_str(newline);
    Ok(rendered)
}

#[cfg(test)]
mod tests {
    use super::replace_managed_block;

    #[test]
    fn appends_and_replaces_only_managed_block() {
        let original = "127.0.0.1 localhost\n";
        let updated = replace_managed_block(original, "dev", "10.0.0.2 api.local\n").unwrap();
        assert!(updated.starts_with("127.0.0.1 localhost\n"));
        assert!(updated.contains("10.0.0.2 api.local"));
        let replaced = replace_managed_block(&updated, "work", "10.0.0.3 work.local\n").unwrap();
        assert!(replaced.contains("127.0.0.1 localhost"));
        assert!(!replaced.contains("api.local"));
        assert!(replaced.contains("work.local"));
    }

    #[test]
    fn preserves_crlf_line_endings() {
        let updated =
            replace_managed_block("127.0.0.1 localhost\r\n", "dev", "10.0.0.2 api.local\n")
                .unwrap();
        assert_eq!(
            updated,
            "127.0.0.1 localhost\r\n\r\n# >>> CrabHosts managed (profile: dev)\r\n10.0.0.2 api.local\r\n# <<< CrabHosts managed\r\n"
        );
    }

    #[test]
    fn rejects_unbalanced_markers() {
        assert!(replace_managed_block("# >>> CrabHosts managed\n", "dev", "").is_err());
    }
}
