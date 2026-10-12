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

    let result = Command::new(env!("CARGO_BIN_EXE_fleet"))
        .arg("identity")
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
        std::fs::read_to_string(key_path).expect("read damaged file"),
        "damaged key"
    );
}
