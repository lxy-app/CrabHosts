use crabhosts_core::{Document, HostEntry, Profile};

#[test]
fn rejects_host_file_injection_through_fields() {
    assert!(HostEntry::new("127.0.0.1", vec!["good.local\nbad".into()], true, None).is_err());
    assert!(
        HostEntry::new(
            "127.0.0.1",
            vec!["good.local".into()],
            true,
            Some("line one\nline two".into()),
        )
        .is_err()
    );
    assert!(Profile::new("bad\nprofile").is_err());
}

#[test]
fn normalizes_addresses_hostnames_and_duplicate_input() {
    let entry = HostEntry::new(
        "0:0:0:0:0:0:0:1",
        vec!["DEV.LOCAL.".into(), "dev.local".into()],
        true,
        None,
    )
    .unwrap();
    assert_eq!(entry.ip, "::1");
    assert_eq!(entry.hostnames, ["dev.local"]);
}

#[test]
fn default_document_uses_the_current_schema() {
    let document = Document::default();
    assert_eq!(document.schema_version, 1);
    document.validate().unwrap();
}

#[cfg(unix)]
#[test]
fn atomic_write_refuses_to_replace_symbolic_links() {
    use std::os::unix::fs::symlink;

    let directory = tempfile::tempdir().unwrap();
    let target = directory.path().join("target");
    let link = directory.path().join("link");
    std::fs::write(&target, "original").unwrap();
    symlink(&target, &link).unwrap();

    assert!(crabhosts_core::atomic_write(&link, b"replacement").is_err());
    assert_eq!(std::fs::read_to_string(target).unwrap(), "original");
}
