use std::fs;

use crabhosts_core::{
    AppPaths, Document, Error, HostEntry, Profile, Storage, apply_plan, plan_profile,
    plan_profiles, plan_restore,
};

fn fixture() -> (tempfile::TempDir, Storage, Document) {
    let directory = tempfile::tempdir().unwrap();
    let hosts = directory.path().join("hosts");
    fs::write(&hosts, "127.0.0.1 localhost\n").unwrap();
    let paths = AppPaths::new(directory.path().join("config"), hosts);
    let storage = Storage::new(paths);
    let mut document = Document::default();
    let mut profile = Profile::new("dev").unwrap();
    profile.entries = vec![
        HostEntry::new("10.0.0.2", vec!["api.local".into()], true, None).unwrap(),
        HostEntry::new("10.0.0.3", vec!["off.local".into()], false, None).unwrap(),
    ];
    document.add_profile(profile).unwrap();
    (directory, storage, document)
}

#[test]
fn apply_preserves_unmanaged_lines_and_restores_atomically() {
    let (_directory, storage, document) = fixture();
    let plan = plan_profile(&storage, &document, "dev").unwrap();
    assert!(plan.changed);
    let applied = apply_plan(&storage, &plan).unwrap();
    let backup_id = applied.backup_id.unwrap();
    let content = storage.read_hosts().unwrap();
    assert!(content.contains("127.0.0.1 localhost"));
    assert!(content.contains("10.0.0.2\tapi.local"));
    assert!(!content.contains("off.local"));

    fs::write(&storage.paths().hosts_file, "# external change\n").unwrap();
    let restore = plan_restore(&storage, &backup_id).unwrap();
    apply_plan(&storage, &restore).unwrap();
    assert_eq!(storage.read_hosts().unwrap(), "127.0.0.1 localhost\n");
    assert_eq!(storage.backups().list().unwrap().len(), 2);
}

#[test]
fn stale_plan_cannot_overwrite_a_concurrent_change() {
    let (_directory, storage, document) = fixture();
    let plan = plan_profile(&storage, &document, "dev").unwrap();
    fs::write(&storage.paths().hosts_file, "changed elsewhere\n").unwrap();
    assert!(matches!(apply_plan(&storage, &plan), Err(Error::Conflict)));
    assert_eq!(storage.read_hosts().unwrap(), "changed elsewhere\n");
}

#[test]
fn no_op_apply_and_restore_still_reject_stale_current_content() {
    let (_directory, storage, document) = fixture();
    let first = plan_profile(&storage, &document, "dev").unwrap();
    apply_plan(&storage, &first).unwrap();
    let no_op = plan_profile(&storage, &document, "dev").unwrap();
    assert!(!no_op.changed);
    fs::write(&storage.paths().hosts_file, "changed elsewhere\n").unwrap();
    assert!(matches!(apply_plan(&storage, &no_op), Err(Error::Conflict)));

    fs::write(&storage.paths().hosts_file, "127.0.0.1 localhost\n").unwrap();
    let backup = storage
        .backups()
        .create("127.0.0.1 localhost\n", &storage.paths().hosts_file)
        .unwrap();
    let restore = plan_restore(&storage, &backup.id).unwrap();
    assert!(!restore.changed);
    fs::write(&storage.paths().hosts_file, "changed again\n").unwrap();
    assert!(matches!(
        apply_plan(&storage, &restore),
        Err(Error::Conflict)
    ));
    assert_eq!(storage.read_hosts().unwrap(), "changed again\n");
}

#[test]
fn profile_edits_do_not_touch_system_hosts() {
    let (_directory, storage, mut document) = fixture();
    storage.save(&mut document).unwrap();
    assert_eq!(storage.read_hosts().unwrap(), "127.0.0.1 localhost\n");
    assert_eq!(document.revision, 1);
}

#[test]
fn active_set_is_bound_into_the_plan_id() {
    let (_directory, storage, mut document) = fixture();
    let mut work = Profile::new("work").unwrap();
    work.entries = vec![HostEntry::new("10.0.0.3", vec!["work.local".into()], true, None).unwrap()];
    document.add_profile(work).unwrap();
    let dev_id = document.profile("dev").unwrap().id.clone();
    let work_id = document.profile("work").unwrap().id.clone();
    let dev_plan = plan_profiles(&storage, &document, std::slice::from_ref(&dev_id)).unwrap();
    let combined = plan_profiles(&storage, &document, &[dev_id, work_id]).unwrap();
    assert_ne!(dev_plan.plan_id, combined.plan_id);
    assert_eq!(combined.operation, "apply_profiles");
    assert_eq!(combined.profile_names, ["dev", "work"]);
}

#[test]
fn activation_recovers_legacy_block_and_round_trips() {
    let (_directory, storage, mut document) = fixture();
    let dev_id = document.profile("dev").unwrap().id.clone();
    fs::write(
        &storage.paths().hosts_file,
        "127.0.0.1 localhost\n# >>> CrabHosts managed (profile: dev)\n10.0.0.2 api.local\n# <<< CrabHosts managed\n",
    )
    .unwrap();
    let mut activation = storage.load_activation(&document).unwrap();
    assert_eq!(activation.active_profile_ids, [dev_id]);
    storage.save_activation(&mut activation).unwrap();
    assert_eq!(activation.revision, 1);
    assert!(storage.paths().activation_file.exists());
    assert_eq!(storage.load_activation(&document).unwrap(), activation);

    let mut work = Profile::new("work").unwrap();
    work.entries = vec![HostEntry::new("10.0.0.3", vec!["work.local".into()], true, None).unwrap()];
    let work_id = work.id.clone();
    document.add_profile(work).unwrap();
    activation.set_active(&work_id, true).unwrap();
    storage.save_activation(&mut activation).unwrap();
    let migration = plan_profiles(&storage, &document, &activation.active_profile_ids).unwrap();
    apply_plan(&storage, &migration).unwrap();
    let content = storage.read_hosts().unwrap();
    assert!(content.contains("# >>> CrabHosts managed\n"));
    assert!(!content.contains("(profile: dev)"));
    assert!(content.contains("# profile-id: "));
    assert!(content.contains("api.local"));
    assert!(content.contains("work.local"));
}

#[test]
fn corrupt_activation_is_not_replaced_with_an_empty_set() {
    let (_directory, storage, document) = fixture();
    fs::create_dir_all(&storage.paths().config_dir).unwrap();
    for invalid in [
        "{broken",
        "{}",
        r#"{"schema_version":1,"revision":0}"#,
        r#"{"revision":0,"active_profile_ids":[]}"#,
    ] {
        fs::write(&storage.paths().activation_file, invalid).unwrap();
        assert!(
            storage.load_activation(&document).is_err(),
            "invalid activation was accepted: {invalid}"
        );
        assert_eq!(
            fs::read_to_string(&storage.paths().activation_file).unwrap(),
            invalid
        );
    }
}

#[test]
fn activation_transactions_serialize_concurrent_writers() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Barrier};
    use std::time::Duration;

    let (_directory, storage, mut document) = fixture();
    for name in ["alpha", "beta", "gamma", "delta"] {
        document.add_profile(Profile::new(name).unwrap()).unwrap();
    }
    let ids = document
        .profiles
        .iter()
        .skip(1)
        .map(|profile| profile.id.clone())
        .collect::<Vec<_>>();
    storage
        .save_activation(&mut crabhosts_core::ActivationState::default())
        .unwrap();

    let barrier = Arc::new(Barrier::new(ids.len()));
    let concurrent = Arc::new(AtomicUsize::new(0));
    let maximum = Arc::new(AtomicUsize::new(0));
    let mut threads = Vec::new();
    for id in ids {
        let storage = storage.clone();
        let document = document.clone();
        let barrier = Arc::clone(&barrier);
        let concurrent = Arc::clone(&concurrent);
        let maximum = Arc::clone(&maximum);
        threads.push(std::thread::spawn(move || {
            barrier.wait();
            storage
                .update_activation(&document, |activation| {
                    let running = concurrent.fetch_add(1, Ordering::SeqCst) + 1;
                    maximum.fetch_max(running, Ordering::SeqCst);
                    std::thread::sleep(Duration::from_millis(20));
                    let result = activation.set_active(&id, true);
                    concurrent.fetch_sub(1, Ordering::SeqCst);
                    result
                })
                .unwrap();
        }));
    }
    for thread in threads {
        thread.join().unwrap();
    }
    let activation = storage.load_activation(&document).unwrap();
    assert_eq!(activation.active_profile_ids.len(), 4);
    assert_eq!(maximum.load(Ordering::SeqCst), 1);
}
