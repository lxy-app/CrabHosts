use std::process::ExitCode;

use clap::{Parser, Subcommand};
use crabhosts_core::{
    Error, HostEntry, Profile, Result, Storage, parse_hosts, plan_profile, plan_restore,
};
use crabhosts_system_client::SystemClient;
use serde::Serialize;
use serde_json::{Value, json};

#[derive(Debug, Parser)]
#[command(
    name = "crabhosts",
    version,
    about = "Safe, AI-friendly hosts profile manager"
)]
struct Cli {
    /// Emit a stable JSON envelope for automation.
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Show local installation and storage status.
    Status,
    /// Manage reusable hosts profiles without modifying the system.
    Profile {
        #[command(subcommand)]
        command: ProfileCommand,
    },
    /// Preview, apply, or restore system hosts changes.
    System {
        #[command(subcommand)]
        command: SystemCommand,
    },
    /// List automatic system hosts backups.
    Backup {
        #[command(subcommand)]
        command: BackupCommand,
    },
    /// Inspect the optional macOS privileged helper without changing the system.
    Helper {
        #[command(subcommand)]
        command: HelperCommand,
    },
}

#[derive(Debug, Subcommand)]
enum ProfileCommand {
    List,
    Show {
        name: String,
    },
    Create {
        name: String,
        /// Import active mappings from the current system hosts file.
        #[arg(long)]
        from_system: bool,
    },
    Delete {
        name: String,
        #[arg(long)]
        yes: bool,
    },
    /// Add an entry or merge hostnames into an existing entry with the same IP.
    Add {
        profile: String,
        ip: String,
        #[arg(required = true)]
        hostnames: Vec<String>,
        #[arg(long)]
        disabled: bool,
        #[arg(long)]
        comment: Option<String>,
    },
    Remove {
        profile: String,
        hostname: String,
    },
    Enable {
        profile: String,
        hostname: String,
    },
    Disable {
        profile: String,
        hostname: String,
    },
}

#[derive(Debug, Subcommand)]
enum SystemCommand {
    Diff {
        profile: String,
    },
    Apply {
        profile: String,
        #[arg(long, conflicts_with = "yes")]
        dry_run: bool,
        #[arg(long)]
        yes: bool,
        /// Bind this approved write to an earlier dry-run plan ID.
        #[arg(long, requires = "yes")]
        expected_plan: Option<String>,
    },
    Restore {
        backup_id: String,
        #[arg(long, conflicts_with = "yes")]
        dry_run: bool,
        #[arg(long)]
        yes: bool,
        /// Bind this approved restore to an earlier dry-run plan ID.
        #[arg(long, requires = "yes")]
        expected_plan: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
enum BackupCommand {
    List,
}

#[derive(Debug, Subcommand)]
enum HelperCommand {
    /// Check connectivity, version, and protocol compatibility.
    Status,
}

struct Output {
    command: &'static str,
    data: Value,
    human: String,
}

impl Output {
    fn new(command: &'static str, data: impl Serialize, human: impl Into<String>) -> Self {
        let data = serde_json::to_value(data).expect("command data must be JSON serializable");
        Self {
            command,
            data,
            human: human.into(),
        }
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let json_output = cli.json;
    match run(cli) {
        Ok(output) => {
            if json_output {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&json!({
                        "ok": true,
                        "command": output.command,
                        "data": output.data,
                    }))
                    .expect("JSON envelope serialization cannot fail")
                );
            } else if !output.human.is_empty() {
                println!("{}", output.human);
            }
            ExitCode::SUCCESS
        }
        Err(error) => {
            if json_output {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&json!({
                        "ok": false,
                        "error": {"code": error.code(), "message": error.to_string()}
                    }))
                    .expect("JSON error serialization cannot fail")
                );
            } else {
                eprintln!("error[{}]: {error}", error.code());
                if error.is_permission_denied() {
                    eprintln!(
                        "hint: rerun with the OS-appropriate elevated privileges; no privilege escalation is performed implicitly"
                    );
                }
            }
            ExitCode::from(exit_code(&error))
        }
    }
}

fn run(cli: Cli) -> Result<Output> {
    let storage = Storage::from_env()?;
    match cli.command {
        Command::Status => status(&storage),
        Command::Profile { command } => profile(&storage, command),
        Command::System { command } => system(&storage, command),
        Command::Backup {
            command: BackupCommand::List,
        } => {
            let backups = storage.backups().list()?;
            let human = if backups.is_empty() {
                "No backups.".to_owned()
            } else {
                backups
                    .iter()
                    .map(|backup| {
                        format!(
                            "{}\t{}\t{} bytes",
                            backup.id, backup.created_at, backup.size
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            };
            Ok(Output::new("backup.list", backups, human))
        }
        Command::Helper {
            command: HelperCommand::Status,
        } => helper_status(&storage),
    }
}

fn helper_status(storage: &Storage) -> Result<Output> {
    if !cfg!(target_os = "macos")
        || std::env::var_os(crabhosts_core::HOSTS_FILE_ENV).is_some()
        || storage.paths().hosts_file != std::path::Path::new("/etc/hosts")
    {
        return Ok(Output::new(
            "helper.status",
            json!({"mode":"direct","ready":true}),
            "Direct storage mode; no privileged helper is used.".to_owned(),
        ));
    }

    #[cfg(target_os = "macos")]
    {
        let status = crabhosts_system_client::helper_status()?;
        if status.helper_version != env!("CARGO_PKG_VERSION") {
            return Err(Error::HelperProtocol(format!(
                "helper version {} does not match CLI version {}",
                status.helper_version,
                env!("CARGO_PKG_VERSION")
            )));
        }
        Ok(Output::new(
            "helper.status",
            json!({"mode":"helper","ready":true,"helper":status}),
            format!(
                "CrabHosts Helper v{} is ready (protocol {}).",
                status.helper_version, status.protocol_version
            ),
        ))
    }

    #[cfg(not(target_os = "macos"))]
    {
        unreachable!("non-macOS returned direct storage status above")
    }
}

fn status(storage: &Storage) -> Result<Output> {
    let document = storage.load()?;
    let entry_count = document
        .profiles
        .iter()
        .map(|profile| profile.entries.len())
        .sum::<usize>();
    let data = json!({
        "version": env!("CARGO_PKG_VERSION"),
        "schema_version": document.schema_version,
        "revision": document.revision,
        "profile_count": document.profiles.len(),
        "entry_count": entry_count,
        "config_dir": storage.paths().config_dir,
        "database_file": storage.paths().database_file,
        "hosts_file": storage.paths().hosts_file,
        "hosts_file_exists": storage.paths().hosts_file.exists(),
    });
    let human = format!(
        "CrabHosts {}\nProfiles: {}\nEntries: {}\nConfig: {}\nSystem hosts: {}",
        env!("CARGO_PKG_VERSION"),
        document.profiles.len(),
        entry_count,
        storage.paths().config_dir.display(),
        storage.paths().hosts_file.display()
    );
    Ok(Output::new("status", data, human))
}

fn profile(storage: &Storage, command: ProfileCommand) -> Result<Output> {
    match command {
        ProfileCommand::List => {
            let document = storage.load()?;
            let human = if document.profiles.is_empty() {
                "No profiles. Create one with `crabhosts profile create <name>`.".to_owned()
            } else {
                document
                    .profiles
                    .iter()
                    .map(|profile| format!("{}\t{} entries", profile.name, profile.entries.len()))
                    .collect::<Vec<_>>()
                    .join("\n")
            };
            Ok(Output::new("profile.list", document.profiles, human))
        }
        ProfileCommand::Show { name } => {
            let document = storage.load()?;
            let profile = document.profile(&name)?;
            Ok(Output::new(
                "profile.show",
                profile,
                render_profile(profile),
            ))
        }
        ProfileCommand::Create { name, from_system } => {
            let mut document = storage.load()?;
            let mut profile = Profile::new(name)?;
            if from_system {
                profile.entries = parse_hosts(&storage.read_hosts()?);
            }
            let imported_count = profile.entries.len();
            document.add_profile(profile.clone())?;
            storage.save(&mut document)?;
            Ok(Output::new(
                "profile.create",
                json!({"profile": profile, "revision": document.revision}),
                format!(
                    "Created profile {:?} with {} entries (revision {}).",
                    profile.name, imported_count, document.revision
                ),
            ))
        }
        ProfileCommand::Delete { name, yes } => {
            if !yes {
                return Err(Error::ConfirmationRequired);
            }
            let mut document = storage.load()?;
            document.remove_profile(&name)?;
            storage.save(&mut document)?;
            Ok(Output::new(
                "profile.delete",
                json!({"name": name, "revision": document.revision}),
                format!("Deleted profile {name:?}; the system hosts file was not changed."),
            ))
        }
        ProfileCommand::Add {
            profile,
            ip,
            hostnames,
            disabled,
            comment,
        } => {
            let mut document = storage.load()?;
            let new_entry = HostEntry::new(&ip, hostnames, !disabled, comment)?;
            let entry_id = document
                .profile_mut(&profile)?
                .add_or_merge_entry(new_entry)?;
            let entry = document
                .profile(&profile)?
                .entries
                .iter()
                .find(|entry| entry.id == entry_id)
                .cloned()
                .expect("inserted or merged entry must exist");
            storage.save(&mut document)?;
            Ok(Output::new(
                "profile.add",
                json!({"profile": profile, "entry": entry, "revision": document.revision}),
                format!(
                    "Updated profile {profile:?} (revision {}).",
                    document.revision
                ),
            ))
        }
        ProfileCommand::Remove { profile, hostname } => {
            let mut document = storage.load()?;
            let removed = document.profile_mut(&profile)?.remove_hostname(&hostname)?;
            storage.save(&mut document)?;
            Ok(Output::new(
                "profile.remove",
                json!({"profile": profile, "hostname": hostname, "removed": removed, "revision": document.revision}),
                format!(
                    "Removed {hostname:?} from {removed} entr(y/ies); system hosts was not changed."
                ),
            ))
        }
        ProfileCommand::Enable { profile, hostname } => {
            set_enabled(storage, profile, hostname, true)
        }
        ProfileCommand::Disable { profile, hostname } => {
            set_enabled(storage, profile, hostname, false)
        }
    }
}

fn set_enabled(
    storage: &Storage,
    profile: String,
    hostname: String,
    enabled: bool,
) -> Result<Output> {
    let mut document = storage.load()?;
    let changed = document
        .profile_mut(&profile)?
        .set_hostname_enabled(&hostname, enabled)?;
    storage.save(&mut document)?;
    let command = if enabled {
        "profile.enable"
    } else {
        "profile.disable"
    };
    Ok(Output::new(
        command,
        json!({"profile": profile, "hostname": hostname, "enabled": enabled, "changed": changed, "revision": document.revision}),
        format!("Updated {changed} entr(y/ies); system hosts was not changed."),
    ))
}

fn system(storage: &Storage, command: SystemCommand) -> Result<Output> {
    match command {
        SystemCommand::Diff { profile } => {
            let document = storage.load()?;
            let plan = plan_profile(storage, &document, &profile)?;
            let human = plan_human(&plan.diff, plan.changed);
            Ok(Output::new("system.diff", plan, human))
        }
        SystemCommand::Apply {
            profile,
            dry_run,
            yes,
            expected_plan,
        } => {
            require_confirmation(dry_run, yes)?;
            let document = storage.load()?;
            let plan = plan_profile(storage, &document, &profile)?;
            verify_expected_plan(&plan.plan_id, expected_plan.as_deref())?;
            if dry_run {
                let human = plan_human(&plan.diff, plan.changed);
                Ok(Output::new("system.apply.dry_run", plan, human))
            } else {
                let report = SystemClient::new().apply_plan(storage, &plan)?;
                let human = if report.changed {
                    format!(
                        "Applied profile {profile:?}. Backup: {}",
                        report.backup_id.as_deref().unwrap_or("none")
                    )
                } else {
                    "No changes; no backup was needed.".to_owned()
                };
                Ok(Output::new("system.apply", report, human))
            }
        }
        SystemCommand::Restore {
            backup_id,
            dry_run,
            yes,
            expected_plan,
        } => {
            require_confirmation(dry_run, yes)?;
            let plan = plan_restore(storage, &backup_id)?;
            verify_expected_plan(&plan.plan_id, expected_plan.as_deref())?;
            if dry_run {
                let human = plan_human(&plan.diff, plan.changed);
                Ok(Output::new("system.restore.dry_run", plan, human))
            } else {
                let report = SystemClient::new().apply_plan(storage, &plan)?;
                let human = if report.changed {
                    format!(
                        "Restored backup {backup_id:?}. Pre-restore backup: {}",
                        report.backup_id.as_deref().unwrap_or("none")
                    )
                } else {
                    "No changes; the current system hosts already matches the backup.".to_owned()
                };
                Ok(Output::new("system.restore", report, human))
            }
        }
    }
}

fn verify_expected_plan(actual: &str, expected: Option<&str>) -> Result<()> {
    if expected.is_some_and(|expected| expected != actual) {
        Err(Error::Conflict)
    } else {
        Ok(())
    }
}

fn require_confirmation(dry_run: bool, yes: bool) -> Result<()> {
    if dry_run || yes {
        Ok(())
    } else {
        Err(Error::ConfirmationRequired)
    }
}

fn plan_human(diff: &str, changed: bool) -> String {
    if changed {
        diff.to_owned()
    } else {
        "No changes.".to_owned()
    }
}

fn render_profile(profile: &Profile) -> String {
    let mut lines = vec![format!("Profile: {} ({})", profile.name, profile.id)];
    for entry in &profile.entries {
        lines.push(format!(
            "{}\t{}",
            if entry.enabled { "[on] " } else { "[off]" },
            entry.render()
        ));
    }
    if profile.entries.is_empty() {
        lines.push("No entries.".to_owned());
    }
    lines.join("\n")
}

fn exit_code(error: &Error) -> u8 {
    match error {
        Error::ConfirmationRequired => 3,
        error if error.is_permission_denied() => 4,
        Error::Conflict => 5,
        Error::Invalid(_)
        | Error::NotFound(_)
        | Error::AlreadyExists(_)
        | Error::UnsafeBlock(_) => 2,
        _ => 1,
    }
}
