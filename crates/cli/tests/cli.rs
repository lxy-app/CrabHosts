use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use serde_json::Value;

fn run(config: &Path, hosts: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_crab"))
        .args(args)
        .env("CRABHOSTS_HOME", config)
        .env("CRABHOSTS_HOSTS_FILE", hosts)
        .output()
        .unwrap()
}

fn json(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn json_workflow_is_non_interactive_and_safe() {
    let directory = tempfile::tempdir().unwrap();
    let config = directory.path().join("config");
    let hosts = directory.path().join("hosts");
    fs::write(&hosts, "127.0.0.1 localhost\n").unwrap();

    let created = run(&config, &hosts, &["--json", "profile", "create", "dev"]);
    assert!(created.status.success(), "{created:?}");
    assert_eq!(json(&created)["ok"], true);

    let added = run(
        &config,
        &hosts,
        &[
            "--json",
            "profile",
            "add",
            "dev",
            "10.0.0.2",
            "api.local",
            "admin.local",
        ],
    );
    assert!(added.status.success(), "{added:?}");

    let preview = run(
        &config,
        &hosts,
        &["--json", "system", "apply", "dev", "--dry-run"],
    );
    assert!(preview.status.success(), "{preview:?}");
    let preview_json = json(&preview);
    assert_eq!(preview_json["data"]["changed"], true);
    let plan_id = preview_json["data"]["plan_id"].as_str().unwrap().to_owned();
    assert_eq!(fs::read_to_string(&hosts).unwrap(), "127.0.0.1 localhost\n");

    let unconfirmed = run(&config, &hosts, &["--json", "system", "apply", "dev"]);
    assert_eq!(unconfirmed.status.code(), Some(3));
    assert_eq!(json(&unconfirmed)["error"]["code"], "confirmation_required");

    let stale = run(
        &config,
        &hosts,
        &[
            "--json",
            "system",
            "apply",
            "dev",
            "--yes",
            "--expected-plan",
            "stale-plan",
        ],
    );
    assert_eq!(stale.status.code(), Some(5));
    assert_eq!(json(&stale)["error"]["code"], "conflict");
    assert_eq!(fs::read_to_string(&hosts).unwrap(), "127.0.0.1 localhost\n");

    let applied = run(
        &config,
        &hosts,
        &[
            "--json",
            "system",
            "apply",
            "dev",
            "--yes",
            "--expected-plan",
            &plan_id,
        ],
    );
    assert!(applied.status.success(), "{applied:?}");
    let content = fs::read_to_string(&hosts).unwrap();
    assert!(content.contains("127.0.0.1 localhost"));
    assert!(content.contains("10.0.0.2\tapi.local admin.local"));
    assert!(json(&applied)["data"]["backup_id"].is_string());

    let helper = run(&config, &hosts, &["--json", "helper", "status"]);
    assert!(helper.status.success(), "{helper:?}");
    let helper = json(&helper);
    assert_eq!(helper["data"]["mode"], "direct");
    assert_eq!(helper["data"]["ready"], true);
}

#[test]
fn multi_profile_activation_composes_without_replacing_other_profiles() {
    let directory = tempfile::tempdir().unwrap();
    let config = directory.path().join("config");
    let hosts = directory.path().join("hosts");
    fs::write(&hosts, "127.0.0.1 localhost\n").unwrap();

    for (profile, ip, hostname) in [
        ("dev", "10.0.0.2", "api.dev"),
        ("work", "10.0.0.3", "api.work"),
    ] {
        assert!(
            run(&config, &hosts, &["--json", "profile", "create", profile])
                .status
                .success()
        );
        assert!(
            run(
                &config,
                &hosts,
                &["--json", "profile", "add", profile, ip, hostname],
            )
            .status
            .success()
        );
    }

    // The legacy argument is additive and dry-run does not persist activation.
    let preview = run(
        &config,
        &hosts,
        &["--json", "system", "apply", "dev", "--dry-run"],
    );
    assert!(preview.status.success(), "{preview:?}");
    assert!(!config.join("activation.json").exists());
    let plan_id = json(&preview)["data"]["plan_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let applied = run(
        &config,
        &hosts,
        &[
            "--json",
            "system",
            "apply",
            "dev",
            "--yes",
            "--expected-plan",
            &plan_id,
        ],
    );
    assert!(applied.status.success(), "{applied:?}");
    assert!(fs::read_to_string(&hosts).unwrap().contains("api.dev"));

    let current_preview = run(&config, &hosts, &["--json", "system", "apply", "--dry-run"]);
    let current_plan_id = json(&current_preview)["data"]["plan_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let activated = run(&config, &hosts, &["--json", "profile", "activate", "work"]);
    assert!(activated.status.success(), "{activated:?}");
    assert!(!fs::read_to_string(&hosts).unwrap().contains("api.work"));

    // Changing only the active set invalidates the earlier approval.
    let stale = run(
        &config,
        &hosts,
        &[
            "--json",
            "system",
            "apply",
            "--yes",
            "--expected-plan",
            &current_plan_id,
        ],
    );
    assert_eq!(stale.status.code(), Some(5), "{stale:?}");
    assert!(!fs::read_to_string(&hosts).unwrap().contains("api.work"));

    // Reset the candidate, then verify that the legacy apply argument adds B to active A.
    assert!(
        run(
            &config,
            &hosts,
            &["--json", "profile", "deactivate", "work"],
        )
        .status
        .success()
    );
    let preview = run(
        &config,
        &hosts,
        &["--json", "system", "apply", "work", "--dry-run"],
    );
    assert!(preview.status.success(), "{preview:?}");
    let preview_json = json(&preview);
    assert_eq!(
        preview_json["data"]["profile_names"],
        serde_json::json!(["dev", "work"])
    );
    let plan_id = preview_json["data"]["plan_id"].as_str().unwrap().to_owned();
    let applied = run(
        &config,
        &hosts,
        &[
            "--json",
            "system",
            "apply",
            "work",
            "--yes",
            "--expected-plan",
            &plan_id,
        ],
    );
    assert!(applied.status.success(), "{applied:?}");
    assert_eq!(json(&applied)["data"]["activation_persisted"], true);
    let content = fs::read_to_string(&hosts).unwrap();
    assert!(content.contains("api.dev"));
    assert!(content.contains("api.work"));

    assert!(
        run(&config, &hosts, &["--json", "profile", "deactivate", "dev"],)
            .status
            .success()
    );
    assert!(fs::read_to_string(&hosts).unwrap().contains("api.dev"));
    let preview = run(&config, &hosts, &["--json", "system", "apply", "--dry-run"]);
    let plan_id = json(&preview)["data"]["plan_id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(
        run(
            &config,
            &hosts,
            &[
                "--json",
                "system",
                "apply",
                "--yes",
                "--expected-plan",
                &plan_id,
            ],
        )
        .status
        .success()
    );
    let content = fs::read_to_string(&hosts).unwrap();
    assert!(!content.contains("api.dev"));
    assert!(content.contains("api.work"));

    // An empty active set removes only CrabHosts' block, not unmanaged hosts lines.
    assert!(
        run(
            &config,
            &hosts,
            &["--json", "profile", "deactivate", "work"],
        )
        .status
        .success()
    );
    let preview = run(&config, &hosts, &["--json", "system", "apply", "--dry-run"]);
    let plan_id = json(&preview)["data"]["plan_id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(
        run(
            &config,
            &hosts,
            &[
                "--json",
                "system",
                "apply",
                "--yes",
                "--expected-plan",
                &plan_id,
            ],
        )
        .status
        .success()
    );
    let content = fs::read_to_string(&hosts).unwrap();
    assert!(content.contains("127.0.0.1 localhost"));
    assert!(!content.contains("CrabHosts managed"));
    assert!(!content.contains("api.work"));
}

#[test]
fn legacy_apply_reports_activation_persistence_failure_explicitly() {
    let directory = tempfile::tempdir().unwrap();
    let config = directory.path().join("config");
    let hosts = directory.path().join("hosts");
    fs::write(&hosts, "127.0.0.1 localhost\n").unwrap();
    for (profile, ip, hostname) in [
        ("dev", "10.0.0.2", "api.dev"),
        ("work", "10.0.0.3", "api.work"),
    ] {
        assert!(
            run(&config, &hosts, &["--json", "profile", "create", profile])
                .status
                .success()
        );
        assert!(
            run(
                &config,
                &hosts,
                &["--json", "profile", "add", profile, ip, hostname],
            )
            .status
            .success()
        );
    }
    let show = run(&config, &hosts, &["--json", "profile", "show", "dev"]);
    let dev_id = json(&show)["data"]["id"].as_str().unwrap().to_owned();
    fs::write(
        config.join("activation.json"),
        format!(
            "{{\n  \"schema_version\": 1,\n  \"revision\": 0,\n  \"active_profile_ids\": [\n    \"{dev_id}\"\n  ]\n}}\n"
        ),
    )
    .unwrap();
    // The lock cannot be opened, but reads, backups, and the hosts write still work.
    fs::create_dir(config.join("activation.lock")).unwrap();

    let preview = run(
        &config,
        &hosts,
        &["--json", "system", "apply", "work", "--dry-run"],
    );
    let plan_id = json(&preview)["data"]["plan_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let applied = run(
        &config,
        &hosts,
        &[
            "--json",
            "system",
            "apply",
            "work",
            "--yes",
            "--expected-plan",
            &plan_id,
        ],
    );
    assert!(applied.status.success(), "{applied:?}");
    let output = json(&applied);
    assert_eq!(output["data"]["activation_persisted"], false);
    assert!(output["data"]["activation_warning"].is_string());
    assert_eq!(
        output["data"]["active_profile_ids"],
        serde_json::json!([dev_id])
    );
    let content = fs::read_to_string(&hosts).unwrap();
    assert!(content.contains("api.dev"));
    assert!(content.contains("api.work"));
}

#[test]
fn active_hostname_conflicts_are_structured_and_do_not_write() {
    let directory = tempfile::tempdir().unwrap();
    let config = directory.path().join("config");
    let hosts = directory.path().join("hosts");
    let original = "127.0.0.1 localhost\n";
    fs::write(&hosts, original).unwrap();

    for (profile, ip) in [("dev", "10.0.0.2"), ("work", "10.0.0.3")] {
        assert!(
            run(&config, &hosts, &["--json", "profile", "create", profile])
                .status
                .success()
        );
        assert!(
            run(
                &config,
                &hosts,
                &["--json", "profile", "add", profile, ip, "api.local"],
            )
            .status
            .success()
        );
        assert!(
            run(&config, &hosts, &["--json", "profile", "activate", profile],)
                .status
                .success()
        );
    }

    let conflict = run(&config, &hosts, &["--json", "system", "diff"]);
    assert_eq!(conflict.status.code(), Some(2), "{conflict:?}");
    let error = json(&conflict);
    assert_eq!(error["error"]["code"], "profile_conflict");
    assert_eq!(
        error["error"]["details"]["conflicts"][0]["hostname"],
        "api.local"
    );
    assert_eq!(fs::read_to_string(&hosts).unwrap(), original);
}

#[test]
fn public_binary_and_clap_name_are_crab() {
    let version = Command::new(env!("CARGO_BIN_EXE_crab"))
        .arg("--version")
        .output()
        .unwrap();
    assert!(version.status.success(), "{version:?}");
    assert_eq!(
        String::from_utf8(version.stdout).unwrap(),
        format!("crab {}\n", env!("CARGO_PKG_VERSION"))
    );

    let help = Command::new(env!("CARGO_BIN_EXE_crab"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(help.status.success(), "{help:?}");
    let help = String::from_utf8(help.stdout).unwrap();
    assert!(help.contains("Usage: crab"), "{help}");
    assert!(!help.contains("Usage: crabhosts"), "{help}");
}
