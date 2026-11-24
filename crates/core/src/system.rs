use std::collections::{BTreeMap, HashSet};
use std::net::IpAddr;

use serde::{Deserialize, Serialize};
use similar::TextDiff;

use crate::model::normalize_hostname;
use crate::{Document, Error, Profile, Result, Storage, sha256_hex};

pub(crate) const MANAGED_START: &str = "# >>> CrabHosts managed";
pub(crate) const MANAGED_END: &str = "# <<< CrabHosts managed";
const START_PREFIX: &str = "# >>> CrabHosts managed";

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct ConflictMapping {
    pub profile_id: String,
    pub profile_name: String,
    pub ip: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct HostConflict {
    pub hostname: String,
    pub mappings: Vec<ConflictMapping>,
}

#[derive(Clone, Debug, Serialize)]
pub struct SystemPlan {
    /// Binds approval to the operation, active Profile IDs, current content, and proposed content.
    pub plan_id: String,
    pub operation: String,
    /// Human-readable source summary. Profile IDs are the stable apply identity.
    pub source: String,
    #[serde(default)]
    pub profile_ids: Vec<String>,
    #[serde(default)]
    pub profile_names: Vec<String>,
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
    #[serde(default)]
    pub profile_ids: Vec<String>,
    #[serde(default)]
    pub profile_names: Vec<String>,
    pub changed: bool,
    pub backup_id: Option<String>,
    pub hosts_sha256: String,
}

/// Compatibility helper for explicitly planning one Profile. Multi-profile callers should use
/// `plan_profiles`; this helper never changes the device-local activation set.
pub fn plan_profile(storage: &Storage, document: &Document, name: &str) -> Result<SystemPlan> {
    let profile = document.profile(name)?;
    plan_profiles(storage, document, std::slice::from_ref(&profile.id))
}

/// Plans the complete desired managed block for an ordered set of active Profiles.
pub fn plan_profiles(
    storage: &Storage,
    document: &Document,
    profile_ids: &[String],
) -> Result<SystemPlan> {
    let managed = render_profiles(document, profile_ids)?;
    let current = storage.read_hosts()?;
    let proposed = replace_managed_block(&current, &managed)?;
    let names = profile_ids
        .iter()
        .map(|id| {
            document
                .profiles
                .iter()
                .find(|profile| &profile.id == id)
                .map(|profile| profile.name.clone())
                .ok_or_else(|| Error::NotFound(format!("profile id {id:?}")))
        })
        .collect::<Result<Vec<_>>>()?;
    let source = if names.is_empty() {
        "none".to_owned()
    } else {
        names.join(", ")
    };
    Ok(make_plan(
        "apply_profiles",
        &source,
        current,
        proposed,
        "profiles",
        profile_ids,
        &names,
    ))
}

/// Renders all active Profiles into one managed block and rejects ambiguous hostname mappings.
pub fn render_profiles(document: &Document, profile_ids: &[String]) -> Result<String> {
    let profiles = resolve_profiles(document, profile_ids)?;
    reject_hostname_conflicts(&profiles)?;

    let mut output = String::new();
    let mut seen = HashSet::new();
    for profile in profiles {
        output.push_str("# profile: ");
        output.push_str(&profile.name);
        output.push('\n');
        output.push_str("# profile-id: ");
        output.push_str(&profile.id);
        output.push('\n');

        for entry in profile.enabled_entries() {
            let ip = normalized_ip(&entry.ip)?;
            let mut hostnames = Vec::new();
            for hostname in &entry.hostnames {
                let key = normalize_hostname(hostname)?;
                if seen.insert((ip.clone(), key)) {
                    hostnames.push(hostname.clone());
                }
            }
            if hostnames.is_empty() {
                continue;
            }
            output.push_str(&entry.ip);
            output.push('\t');
            output.push_str(&hostnames.join(" "));
            if let Some(comment) = &entry.comment
                && !comment.is_empty()
            {
                output.push_str("\t# ");
                output.push_str(comment);
            }
            output.push('\n');
        }
    }
    Ok(output)
}

fn normalized_ip(ip: &str) -> Result<String> {
    ip.parse::<IpAddr>()
        .map(|address| address.to_string())
        .map_err(|_| Error::Invalid(format!("invalid IP address {ip:?}")))
}

fn resolve_profiles<'a>(
    document: &'a Document,
    profile_ids: &[String],
) -> Result<Vec<&'a Profile>> {
    let mut seen = HashSet::new();
    profile_ids
        .iter()
        .map(|id| {
            if !seen.insert(id) {
                return Err(Error::Invalid(format!(
                    "duplicate active profile id {id:?}"
                )));
            }
            document
                .profiles
                .iter()
                .find(|profile| &profile.id == id)
                .ok_or_else(|| Error::NotFound(format!("profile id {id:?}")))
        })
        .collect()
}

fn reject_hostname_conflicts(profiles: &[&Profile]) -> Result<()> {
    let mut by_hostname = BTreeMap::<String, Vec<ConflictMapping>>::new();
    for profile in profiles {
        for entry in profile.enabled_entries() {
            let ip = normalized_ip(&entry.ip)?;
            for hostname in &entry.hostnames {
                let hostname = normalize_hostname(hostname)?;
                let mappings = by_hostname.entry(hostname).or_default();
                if !mappings
                    .iter()
                    .any(|mapping| mapping.profile_id == profile.id && mapping.ip == ip)
                {
                    mappings.push(ConflictMapping {
                        profile_id: profile.id.clone(),
                        profile_name: profile.name.clone(),
                        ip: ip.clone(),
                    });
                }
            }
        }
    }

    let conflicts = by_hostname
        .into_iter()
        .filter_map(|(hostname, mappings)| {
            let first_ip = mappings.first()?.ip.clone();
            mappings
                .iter()
                .any(|mapping| mapping.ip != first_ip)
                .then_some(HostConflict { hostname, mappings })
        })
        .collect::<Vec<_>>();
    if conflicts.is_empty() {
        Ok(())
    } else {
        Err(Error::ProfileConflicts(conflicts))
    }
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
        &[],
        &[],
    ))
}

/// Applies exactly the supplied plan and makes a best effort to reject concurrent hosts changes.
pub fn apply_plan(storage: &Storage, plan: &SystemPlan) -> Result<ApplyReport> {
    if storage.read_hosts()? != plan.current {
        return Err(Error::Conflict);
    }
    if !plan.changed {
        return Ok(ApplyReport {
            operation: plan.operation.clone(),
            source: plan.source.clone(),
            profile_ids: plan.profile_ids.clone(),
            profile_names: plan.profile_names.clone(),
            changed: false,
            backup_id: None,
            hosts_sha256: plan.current_sha256.clone(),
        });
    }
    let backup = storage
        .backups()
        .create(&plan.current, &storage.paths().hosts_file)?;
    crate::system_writer::write_system_hosts(
        &storage.paths().hosts_file,
        &plan.current,
        &plan.proposed,
    )?;
    Ok(ApplyReport {
        operation: plan.operation.clone(),
        source: plan.source.clone(),
        profile_ids: plan.profile_ids.clone(),
        profile_names: plan.profile_names.clone(),
        changed: true,
        backup_id: Some(backup.id),
        hosts_sha256: plan.proposed_sha256.clone(),
    })
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn make_plan(
    operation: &str,
    source: &str,
    current: String,
    proposed: String,
    diff_label: &str,
    profile_ids: &[String],
    profile_names: &[String],
) -> SystemPlan {
    let diff = TextDiff::from_lines(&current, &proposed)
        .unified_diff()
        .context_radius(3)
        .header("current-hosts", &format!("crabhosts-{diff_label}"))
        .to_string();
    let plan_id = sha256_hex(
        format!(
            "{operation}\0{source}\0{}\0{current}\0{proposed}",
            profile_ids.join("\0")
        )
        .as_bytes(),
    );
    SystemPlan {
        plan_id,
        operation: operation.to_owned(),
        source: source.to_owned(),
        profile_ids: profile_ids.to_vec(),
        profile_names: profile_names.to_vec(),
        changed: current != proposed,
        current_sha256: sha256_hex(current.as_bytes()),
        proposed_sha256: sha256_hex(proposed.as_bytes()),
        diff,
        current,
        proposed,
    }
}

/// Replaces only the managed span. Bytes before and after that span are preserved exactly.
pub(crate) fn replace_managed_block(current: &str, managed: &str) -> Result<String> {
    let mut offset = 0;
    let lines = current
        .split_inclusive('\n')
        .map(|raw| {
            let start = offset;
            offset += raw.len();
            let text = if let Some(without_lf) = raw.strip_suffix('\n') {
                without_lf.strip_suffix('\r').unwrap_or(without_lf)
            } else {
                raw
            };
            (start, offset, text)
        })
        .collect::<Vec<_>>();
    let starts = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| line.2.starts_with(START_PREFIX))
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    let ends = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| line.2 == MANAGED_END)
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
    if starts.is_empty() && managed.is_empty() {
        return Ok(current.to_owned());
    }

    let (prefix, suffix) = match (starts.first(), ends.first()) {
        (Some(start), Some(end)) => (&current[..lines[*start].0], &current[lines[*end].1..]),
        _ => (current, ""),
    };
    if managed.is_empty() {
        return Ok(format!("{prefix}{suffix}"));
    }

    let newline = if current.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let mut output = String::with_capacity(current.len() + managed.len() + 64);
    output.push_str(prefix);
    if starts.is_empty() && !prefix.is_empty() {
        output.push_str(newline);
        if !prefix.ends_with('\n') {
            output.push_str(newline);
        }
    }
    output.push_str(MANAGED_START);
    output.push_str(newline);
    for line in managed.lines() {
        output.push_str(line);
        output.push_str(newline);
    }
    output.push_str(MANAGED_END);
    output.push_str(newline);
    output.push_str(suffix);
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::HostEntry;

    fn profile(id: &str, name: &str, ip: &str, hostname: &str) -> Profile {
        let mut profile = Profile::new(name).unwrap();
        profile.id = id.into();
        profile.entries = vec![HostEntry::new(ip, vec![hostname.into()], true, None).unwrap()];
        profile
    }

    #[test]
    fn combines_profiles_without_losing_unmanaged_lines() {
        let original = "127.0.0.1 localhost\n";
        let dev = render_profiles(
            &Document {
                profiles: vec![
                    profile("dev-id", "dev", "10.0.0.2", "api.local"),
                    profile("work-id", "work", "10.0.0.3", "work.local"),
                ],
                ..Document::default()
            },
            &["dev-id".into(), "work-id".into()],
        )
        .unwrap();
        assert!(dev.contains("api.local"));
        assert!(dev.contains("work.local"));
        let updated = replace_managed_block(original, &dev).unwrap();
        assert!(updated.starts_with("127.0.0.1 localhost\n"));
        assert!(updated.contains("# profile-id: dev-id"));
        assert!(updated.contains("# profile-id: work-id"));
        let replaced =
            replace_managed_block(&updated, "# profile: work\n10.0.0.3 work.local\n").unwrap();
        assert!(replaced.contains("127.0.0.1 localhost"));
        assert!(!replaced.contains("api.local"));
        assert!(replaced.contains("work.local"));
    }

    #[test]
    fn empty_composition_removes_the_managed_block() {
        let current = "127.0.0.1 localhost\n# >>> CrabHosts managed (profile: dev)\n10.0.0.2 api.local\n# <<< CrabHosts managed\n";
        let updated = replace_managed_block(current, "").unwrap();
        assert!(!updated.contains("CrabHosts managed"));
        assert!(!updated.contains("api.local"));
        assert!(updated.contains("127.0.0.1 localhost"));
        assert_eq!(
            replace_managed_block("127.0.0.1 localhost\n", "").unwrap(),
            "127.0.0.1 localhost\n"
        );
    }

    #[test]
    fn exact_duplicates_are_rendered_once() {
        let document = Document {
            profiles: vec![
                profile("dev-id", "dev", "10.0.0.2", "api.local"),
                profile("work-id", "work", "10.0.0.2", "api.local"),
            ],
            ..Document::default()
        };
        let rendered = render_profiles(&document, &["dev-id".into(), "work-id".into()]).unwrap();
        assert_eq!(rendered.matches("api.local").count(), 1);
    }

    #[test]
    fn conflicting_ips_are_rejected_with_profile_details() {
        let document = Document {
            profiles: vec![
                profile("dev-id", "dev", "10.0.0.2", "api.local"),
                profile("work-id", "work", "10.0.0.3", "api.local"),
            ],
            ..Document::default()
        };
        let error = render_profiles(&document, &["dev-id".into(), "work-id".into()]).unwrap_err();
        let Error::ProfileConflicts(conflicts) = error else {
            panic!("expected profile conflicts")
        };
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].hostname, "api.local");
        assert_eq!(conflicts[0].mappings.len(), 2);
    }

    #[test]
    fn conflict_keys_normalize_hostnames_and_ip_addresses() {
        let mut dev = profile("dev-id", "dev", "10.0.0.2", "api.local");
        dev.entries[0].hostnames[0] = "API.LOCAL.".into();
        let document = Document {
            profiles: vec![dev, profile("work-id", "work", "10.0.0.3", "api.local")],
            ..Document::default()
        };
        assert!(matches!(
            render_profiles(&document, &["dev-id".into(), "work-id".into()]),
            Err(Error::ProfileConflicts(_))
        ));

        let mut dev = profile("dev-id", "dev", "::1", "api.local");
        dev.entries[0].ip = "0:0:0:0:0:0:0:1".into();
        dev.entries[0].hostnames[0] = "API.LOCAL.".into();
        let document = Document {
            profiles: vec![dev, profile("work-id", "work", "::1", "api.local")],
            ..Document::default()
        };
        let rendered = render_profiles(&document, &["dev-id".into(), "work-id".into()])
            .unwrap()
            .to_lowercase();
        assert_eq!(rendered.matches("api.local").count(), 1);
    }

    #[test]
    fn preserves_unmanaged_mixed_line_endings_and_final_bytes() {
        let prefix = "127.0.0.1 localhost\r\n# mixed\n";
        let suffix = "# trailing comment\r\n10.0.0.9 unmanaged.local";
        let current = format!(
            "{prefix}# >>> CrabHosts managed (profile: dev)\n10.0.0.2 api.local\n# <<< CrabHosts managed\n{suffix}"
        );
        let updated = replace_managed_block(
            &current,
            "# profile: work\n# profile-id: work-id\n10.0.0.3 work.local\n",
        )
        .unwrap();
        assert!(updated.starts_with(prefix));
        assert!(updated.ends_with(suffix));
        assert!(updated.contains("work.local"));
        assert!(!updated.contains("api.local"));

        let removed = replace_managed_block(&updated, "").unwrap();
        assert_eq!(removed, format!("{prefix}{suffix}"));
    }

    #[test]
    fn preserves_crlf_line_endings() {
        let updated = replace_managed_block(
            "127.0.0.1 localhost\r\n",
            "# profile: dev\n10.0.0.2 api.local\n",
        )
        .unwrap();
        assert_eq!(
            updated,
            "127.0.0.1 localhost\r\n\r\n# >>> CrabHosts managed\r\n# profile: dev\r\n10.0.0.2 api.local\r\n# <<< CrabHosts managed\r\n"
        );
    }

    #[test]
    fn rejects_unbalanced_markers() {
        assert!(replace_managed_block("# >>> CrabHosts managed\n", "").is_err());
    }
}
