use crabhosts_core::{HostEntry, Profile};

#[test]
fn disabling_one_hostname_does_not_disable_its_siblings() {
    let mut profile = Profile::new("dev").unwrap();
    profile.entries = vec![
        HostEntry::new(
            "127.0.0.1",
            vec!["api.local".into(), "admin.local".into()],
            true,
            None,
        )
        .unwrap(),
    ];

    assert_eq!(
        profile.set_hostname_enabled("admin.local", false).unwrap(),
        1
    );
    assert_eq!(profile.entries.len(), 2);
    assert!(profile.entries[0].enabled);
    assert_eq!(profile.entries[0].hostnames, ["api.local"]);
    assert!(!profile.entries[1].enabled);
    assert_eq!(profile.entries[1].hostnames, ["admin.local"]);
    assert_eq!(profile.render_enabled(), "127.0.0.1\tapi.local\n");

    assert_eq!(
        profile.set_hostname_enabled("admin.local", false).unwrap(),
        0
    );
    assert_eq!(
        profile.set_hostname_enabled("admin.local", true).unwrap(),
        1
    );
    assert!(profile.entries[1].enabled);
}

#[test]
fn adding_an_entry_merges_same_ip_and_enabled_state() {
    let mut profile = Profile::new("dev").unwrap();
    let first = HostEntry::new(
        "127.0.0.1",
        vec!["api.local".into()],
        true,
        Some("keep me".into()),
    )
    .unwrap();
    let first_id = profile.add_or_merge_entry(first).unwrap();

    let merged = HostEntry::new(
        "127.0.0.1",
        vec!["API.local".into(), "web.local".into()],
        true,
        None,
    )
    .unwrap();
    assert_eq!(profile.add_or_merge_entry(merged).unwrap(), first_id);
    assert_eq!(profile.entries.len(), 1);
    assert_eq!(profile.entries[0].hostnames, ["api.local", "web.local"]);
    assert_eq!(profile.entries[0].comment.as_deref(), Some("keep me"));

    let disabled = HostEntry::new("127.0.0.1", vec!["off.local".into()], false, None).unwrap();
    profile.add_or_merge_entry(disabled).unwrap();
    assert_eq!(profile.entries.len(), 2);
    assert!(!profile.entries[1].enabled);
}

#[test]
fn removing_the_last_hostname_drops_the_empty_entry() {
    let mut profile = Profile::new("dev").unwrap();
    profile.entries =
        vec![HostEntry::new("127.0.0.1", vec!["api.local".into()], true, None).unwrap()];
    assert_eq!(profile.remove_hostname("API.LOCAL.").unwrap(), 1);
    assert!(profile.entries.is_empty());
}
