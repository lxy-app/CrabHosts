//! Shared system-write client used by the desktop application and CLI.
//!
//! Isolated tests that set `CRABHOSTS_HOSTS_FILE` keep using the direct core writer.
//! Production macOS writes use the signed, launchd-registered privileged helper.

use crabhosts_core::{ApplyReport, Error, HOSTS_FILE_ENV, Result, Storage, SystemPlan, apply_plan};

#[cfg(target_os = "macos")]
mod macos;

#[cfg(target_os = "macos")]
pub use macos::{
    HelperRegistrationStatus, helper_registration_status, helper_status, register_helper,
    serve_helper,
};

#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClient;

impl SystemClient {
    pub fn new() -> Self {
        Self
    }

    pub fn apply_plan(&self, storage: &Storage, plan: &SystemPlan) -> Result<ApplyReport> {
        if use_direct_writer(storage) {
            return apply_plan(storage, plan);
        }

        #[cfg(target_os = "macos")]
        {
            apply_with_writer(storage, plan, macos::write)
        }

        #[cfg(not(target_os = "macos"))]
        {
            apply_plan(storage, plan)
        }
    }
}

#[cfg(target_os = "macos")]
fn use_direct_writer(storage: &Storage) -> bool {
    // The production helper never honors this variable. It is intentionally handled only by
    // unprivileged clients so isolated tests and demos can keep using temporary hosts files.
    std::env::var_os(HOSTS_FILE_ENV).is_some()
        || storage.paths().hosts_file != std::path::Path::new("/etc/hosts")
}

#[cfg(not(target_os = "macos"))]
fn use_direct_writer(_storage: &Storage) -> bool {
    let _ = HOSTS_FILE_ENV;
    true
}

fn apply_with_writer<F>(storage: &Storage, plan: &SystemPlan, write: F) -> Result<ApplyReport>
where
    F: FnOnce(crabhosts_core::HostsWriteRequest) -> Result<ApplyReport>,
{
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

    // Backups stay in the invoking user's configuration directory. The root helper never writes
    // into that directory and never receives a user-controlled backup path.
    let backup = storage
        .backups()
        .create(&plan.current, &storage.paths().hosts_file)?;
    let result = write(plan.write_request()?);
    let mut report = match result {
        Ok(report) => report,
        Err(error) => {
            // A reply can be lost after the helper has committed. Reconcile by exact content,
            // never by retrying the write. If the actual state does not match, preserve the error.
            if matches!(storage.read_hosts(), Ok(actual) if actual == plan.proposed) {
                ApplyReport {
                    operation: plan.operation.clone(),
                    source: plan.source.clone(),
                    changed: true,
                    backup_id: None,
                    hosts_sha256: plan.proposed_sha256.clone(),
                }
            } else {
                return Err(error);
            }
        }
    };
    report.backup_id = Some(backup.id);
    Ok(report)
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;
    use crabhosts_core::{
        AppPaths, Document, HostEntry, Profile, execute_privileged_write, plan_profile,
    };

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
    fn successful_helper_write_uses_the_user_backup() {
        let (_directory, storage, plan) = fixture();
        let report = apply_with_writer(&storage, &plan, |request| {
            execute_privileged_write(&storage, &request)
        })
        .unwrap();
        let backup_id = report.backup_id.unwrap();
        let backups = storage.backups().list().unwrap();
        assert_eq!(backups.len(), 1);
        assert_eq!(backups[0].id, backup_id);
        let (_, backup) = storage.backups().load(&backup_id).unwrap();
        assert_eq!(backup, "127.0.0.1 localhost\n");
        assert!(storage.read_hosts().unwrap().contains("api.local"));
    }

    #[test]
    fn lost_helper_reply_is_reconciled_without_retrying_the_write() {
        let (_directory, storage, plan) = fixture();
        let report = apply_with_writer(&storage, &plan, |request| {
            execute_privileged_write(&storage, &request).unwrap();
            Err(Error::HelperUnavailable("simulated lost reply".into()))
        })
        .unwrap();
        assert!(report.changed);
        assert!(report.backup_id.is_some());
        assert!(storage.read_hosts().unwrap().contains("api.local"));
    }

    #[test]
    fn helper_failure_leaves_hosts_unchanged_after_backup() {
        let (_directory, storage, plan) = fixture();
        let error = apply_with_writer(&storage, &plan, |_request| {
            Err(Error::HelperUnavailable("test failure".into()))
        })
        .unwrap_err();
        assert!(matches!(error, Error::HelperUnavailable(_)));
        assert_eq!(storage.read_hosts().unwrap(), "127.0.0.1 localhost\n");
        assert_eq!(storage.backups().list().unwrap().len(), 1);
    }

    #[test]
    fn backup_failure_prevents_the_helper_call() {
        let (_directory, storage, plan) = fixture();
        std::fs::create_dir_all(storage.paths().config_dir.as_path()).unwrap();
        std::fs::write(&storage.paths().backups_dir, "not a directory").unwrap();
        let error = apply_with_writer(&storage, &plan, |_request| {
            panic!("backup failure must not reach the helper")
        })
        .unwrap_err();
        assert!(matches!(error, Error::Io(_)));
        assert_eq!(storage.read_hosts().unwrap(), "127.0.0.1 localhost\n");
    }

    #[test]
    fn stale_plan_is_rejected_before_backup_or_helper_call() {
        let (_directory, storage, plan) = fixture();
        std::fs::write(&storage.paths().hosts_file, "127.0.0.1 changed\n").unwrap();
        let error = apply_with_writer(&storage, &plan, |_request| {
            panic!("stale plan must not reach the helper")
        })
        .unwrap_err();
        assert!(matches!(error, Error::Conflict));
        assert!(storage.backups().list().unwrap().is_empty());
        assert_eq!(storage.read_hosts().unwrap(), "127.0.0.1 changed\n");
    }
}
