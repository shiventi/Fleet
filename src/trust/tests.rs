use super::*;

fn peer(name: &str, public_key: &str) -> TrustedPeer {
    TrustedPeer {
        name: String::from(name),
        public_key: String::from(public_key),
    }
}

fn public_key() -> String {
    rcgen::KeyPair::generate()
        .expect("generate test identity")
        .public_key_pem()
}

#[test]
fn empty_store_does_not_trust_unknown_keys() {
    assert!(!TrustStore::new().contains("unknown-key"));
}

#[test]
fn added_peers_are_found_by_key_not_name() {
    let mut store = TrustStore::new();
    let first = public_key();
    let second = public_key();
    store.add(peer("ovh", &first)).expect("add first peer");
    store.add(peer("pi", &second)).expect("add second peer");

    assert!(store.contains(&first));
    assert!(store.contains(&second));
    assert!(!store.contains("ovh"));
    assert!(!store.contains("unknown-key"));
}

#[test]
fn blank_names_and_keys_are_rejected() {
    let mut store = TrustStore::new();
    let key = public_key();
    for (name, key) in [
        ("", key.as_str()),
        (" \t", key.as_str()),
        ("ovh", ""),
        ("ovh", " \n"),
    ] {
        assert!(store.add(peer(name, key)).is_err());
    }
    assert!(store.peers.is_empty());
}

#[test]
fn duplicate_names_and_keys_do_not_change_the_store() {
    let mut store = TrustStore::new();
    let first = public_key();
    let second = public_key();
    store.add(peer("ovh", &first)).expect("add original peer");

    assert!(store.add(peer("pi", &first)).is_err());
    assert!(store.add(peer("ovh", &second)).is_err());
    assert_eq!(store.peers.len(), 1);
    assert!(store.contains(&first));
    assert!(!store.contains(&second));
}

#[test]
fn formatting_differences_do_not_allow_duplicate_keys() {
    let key = public_key();
    let formatted = format!("\n{}\n", key.replace('\n', "\r\n"));
    let mut store = TrustStore::new();
    store
        .add(peer("ovh", &formatted))
        .expect("add formatted key");
    assert!(store.contains(&key));
    assert!(store.add(peer("pi", &key)).is_err());
    assert_eq!(store.peers()[0].public_key, key);
}

#[test]
fn private_malformed_multiple_and_unsupported_keys_are_rejected() {
    let key = rcgen::KeyPair::generate().expect("test key");
    let public = key.public_key_pem();
    let private = key.serialize_pem();
    let unsupported = rcgen::KeyPair::generate_for(&rcgen::PKCS_ED25519)
        .expect("unsupported test key")
        .public_key_pem();
    let mut store = TrustStore::new();
    for contents in [
        private.clone(),
        "not a key".into(),
        public.repeat(2),
        format!("{public}{private}"),
        unsupported,
        "A".repeat(MAX_KEY_BYTES + 1),
    ] {
        assert!(store.add(peer("bad", &contents)).is_err());
    }
    assert!(store.peers().is_empty());
}

#[test]
fn peer_names_cannot_contain_terminal_controls_or_whitespace() {
    let key = public_key();
    for name in ["with space", " leading", "line\nbreak", "\x1b[31m"] {
        assert!(TrustStore::new().add(peer(name, &key)).is_err());
    }
    assert!(TrustStore::new().add(peer(&"a".repeat(65), &key)).is_err());
}

#[test]
fn json_loading_checks_keys_and_duplicates() {
    let key = public_key();
    let mut store = TrustStore::new();
    store.add(peer("ovh", &key)).expect("add peer");
    let loaded =
        TrustStore::from_json(&store.to_json().expect("encode JSON")).expect("decode JSON");
    assert!(loaded.contains(&key));
    let duplicate = serde_json::to_string(&vec![peer("ovh", &key), peer("pi", &key)])
        .expect("encode duplicates");
    assert!(TrustStore::from_json(&duplicate).is_err());
    assert!(TrustStore::from_json(r#"[{"name":"bad","public_key":"not a key"}]"#).is_err());
    assert!(TrustStore::from_json(&" ".repeat(MAX_STORE_BYTES + 1)).is_err());
}

#[cfg(unix)]
#[test]
fn peers_persist_and_removal_survives_reloading() {
    use std::os::unix::fs::PermissionsExt;
    let directory = tempfile::tempdir().expect("temporary directory");
    std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700))
        .expect("private directory");
    let path = directory.path().join("peers.json");
    let mut store = TrustStore::load(&path).expect("missing file is empty");
    assert!(store.peers().is_empty());
    let key = public_key();
    store.add(peer("pi", &key)).expect("add peer");
    store.save(&path).expect("save peer");
    let mut loaded = TrustStore::load(&path).expect("reload peer");
    assert!(loaded.contains(&key));
    assert!(!loaded.remove("missing"));
    assert!(loaded.remove("pi"));
    loaded.save(&path).expect("save removal");
    assert!(
        TrustStore::load(&path)
            .expect("reload removal")
            .peers()
            .is_empty()
    );
    assert_eq!(
        std::fs::metadata(path)
            .expect("permissions")
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
}
