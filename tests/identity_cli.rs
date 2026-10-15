#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::process::Command;

fn private_home() -> tempfile::TempDir {
    let home = tempfile::tempdir().expect("create test home");
    std::fs::set_permissions(home.path(), std::fs::Permissions::from_mode(0o700))
        .expect("make test home private");
    home
}

#[test]
fn identity_command_reuses_its_key_and_prints_only_the_public_key() {
    let home = private_home();
    let first = Command::new(env!("CARGO_BIN_EXE_fleet"))
        .arg("identity")
        .env("HOME", home.path())
        .output()
        .expect("run first identity command");
    let second = Command::new(env!("CARGO_BIN_EXE_fleet"))
        .arg("identity")
        .env("HOME", home.path())
        .output()
        .expect("run second identity command");

    assert!(first.status.success());
    assert!(second.status.success());
    assert_eq!(first.stdout, second.stdout);
    let public_key = String::from_utf8(first.stdout).expect("public key text");
    assert!(public_key.starts_with("-----BEGIN PUBLIC KEY-----\n"));
    assert!(public_key.ends_with("-----END PUBLIC KEY-----\n"));
    assert!(!public_key.contains("PRIVATE KEY"));
    assert!(first.stderr.is_empty());

    let key_path = home.path().join(".config/fleet/identity/device-key.pem");
    assert_eq!(
        std::fs::metadata(key_path)
            .expect("key permissions")
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
}

#[test]
fn fingerprint_is_stable_matches_peer_show_and_preserves_saved_files() {
    use p256::pkcs8::{DecodePublicKey, EncodePublicKey};
    use sha2::{Digest, Sha256};
    let home = private_home();
    let public = Command::new(env!("CARGO_BIN_EXE_fleet"))
        .arg("identity")
        .env("HOME", home.path())
        .output()
        .expect("export test identity");
    assert!(public.status.success());
    let public_text = String::from_utf8(public.stdout.clone()).expect("public key text");
    let public_key = p256::PublicKey::from_public_key_pem(&public_text).expect("valid public key");
    let der = public_key.to_public_key_der().expect("public DER");
    let hash = Sha256::digest(der.as_bytes());
    let expected = format!("SHA256:{hash:x}");
    let identity_path = home.path().join(".config/fleet/identity/device-key.pem");
    let before = std::fs::read(&identity_path).expect("saved identity");
    for _ in 0..2 {
        let result = Command::new(env!("CARGO_BIN_EXE_fleet"))
            .args(["identity", "--fingerprint"])
            .env("HOME", home.path())
            .output()
            .expect("fingerprint identity");
        assert!(result.status.success());
        assert!(result.stderr.is_empty());
        assert_eq!(result.stdout, format!("{expected}\n").as_bytes());
    }
    let export = home.path().join("public.pem");
    std::fs::write(&export, &public.stdout).expect("export public key");
    let added = Command::new(env!("CARGO_BIN_EXE_fleet"))
        .args(["peers", "add", "demo"])
        .arg(&export)
        .env("HOME", home.path())
        .output()
        .expect("add test peer");
    assert!(added.status.success());
    let store_path = home
        .path()
        .join(".config/fleet/identity/trusted-peers.json");
    let store_before = std::fs::read(&store_path).expect("saved peers");
    let shown = Command::new(env!("CARGO_BIN_EXE_fleet"))
        .args(["peers", "show", "demo"])
        .env("HOME", home.path())
        .output()
        .expect("show test peer");
    assert!(shown.status.success());
    assert!(shown.stderr.is_empty());
    assert_eq!(
        String::from_utf8(shown.stdout).unwrap(),
        format!("Device name: demo\nFingerprint: {expected}\nPublic key:\n{public_text}")
    );
    assert_eq!(std::fs::read(&identity_path).unwrap(), before);
    assert_eq!(std::fs::read(&store_path).unwrap(), store_before);
}

#[test]
fn identity_command_reports_a_damaged_key_without_replacing_it() {
    let home = private_home();
    let first = Command::new(env!("CARGO_BIN_EXE_fleet"))
        .arg("identity")
        .env("HOME", home.path())
        .output()
        .expect("create identity");
    assert!(first.status.success());
    let key_path = home.path().join(".config/fleet/identity/device-key.pem");
    std::fs::write(&key_path, "damaged key").expect("damage test identity");

    for args in [vec!["identity"], vec!["identity", "--fingerprint"]] {
        let result = Command::new(env!("CARGO_BIN_EXE_fleet"))
            .args(args)
            .env("HOME", home.path())
            .output()
            .expect("run with damaged identity");
        assert!(!result.status.success());
        assert!(result.stdout.is_empty());
        assert!(
            String::from_utf8(result.stderr)
                .expect("error text")
                .contains("Identity error:")
        );
        assert_eq!(
            std::fs::read_to_string(&key_path).expect("read damaged file"),
            "damaged key"
        );
    }
}
