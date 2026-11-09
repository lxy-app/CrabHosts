use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use serde_json::Value;

fn run(config: &Path, hosts: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_crabhosts"))
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
