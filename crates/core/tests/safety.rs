use std::fs;

use crabhosts_core::{
    AppPaths, Document, Error, HostEntry, Profile, Storage, apply_plan, plan_profile, plan_restore,
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
fn profile_edits_do_not_touch_system_hosts() {
    let (_directory, storage, mut document) = fixture();
    storage.save(&mut document).unwrap();
    assert_eq!(storage.read_hosts().unwrap(), "127.0.0.1 localhost\n");
    assert_eq!(document.revision, 1);
}
