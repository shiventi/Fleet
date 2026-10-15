#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Command, Output};

fn private_home() -> tempfile::TempDir {
    let home = tempfile::tempdir().expect("temporary home");
    std::fs::set_permissions(home.path(), std::fs::Permissions::from_mode(0o700))
        .expect("private home");
    home
}

fn run(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_fleet"))
        .args(args)
        .env("HOME", home)
        .output()
        .expect("run Fleet")
}

fn public_file(home: &Path, filename: &str) -> std::path::PathBuf {
    let path = home.join(filename);
    let key = rcgen::KeyPair::generate().expect("generate test key");
    std::fs::write(&path, key.public_key_pem()).expect("export public key");
    path
}

fn saved_path(home: &Path) -> std::path::PathBuf {
    home.join(".config/fleet/identity/trusted-peers.json")
}

#[test]
fn lists_adds_reloads_and_removes_peers_without_creating_an_identity() {
    let home = private_home();
    let empty = run(home.path(), &["peers", "list"]);
    assert!(empty.status.success());
    assert_eq!(empty.stdout, b"No approved peers.\n");
    let key = public_file(home.path(), "pi.pem");
    let added = run(home.path(), &["peers", "add", "pi", key.to_str().unwrap()]);
    assert!(
        added.status.success(),
        "{}",
        String::from_utf8_lossy(&added.stderr)
    );
    let list = run(home.path(), &["peers", "list"]);
    assert!(list.status.success());
    assert_eq!(list.stdout, b"pi\n");
    let path = saved_path(home.path());
    let contents = std::fs::read(&path).expect("saved peers");
    let duplicate = run(
        home.path(),
        &["peers", "add", "other-name", key.to_str().unwrap()],
    );
    assert!(!duplicate.status.success());
    assert_eq!(std::fs::read(&path).unwrap(), contents);
    assert!(
        !run(home.path(), &["peers", "remove", "missing"])
            .status
            .success()
    );
    assert_eq!(std::fs::read(&path).unwrap(), contents);
    assert!(
        run(home.path(), &["peers", "remove", "pi"])
            .status
            .success()
    );
    assert_eq!(
        run(home.path(), &["peers", "list"]).stdout,
        b"No approved peers.\n"
    );
    for filename in ["trusted-peers.json", "trusted-peers.lock"] {
        let file = home.path().join(".config/fleet/identity").join(filename);
        assert_eq!(
            std::fs::metadata(file).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    assert!(
        !home
            .path()
            .join(".config/fleet/identity/device-key.pem")
            .exists()
    );
}

#[test]
fn shows_only_the_requested_peers_public_key_without_changing_the_list() {
    let home = private_home();
    let first = public_file(home.path(), "pi.pem");
    let second = public_file(home.path(), "ovh.pem");
    for (name, path) in [("pi", &first), ("ovh", &second)] {
        assert!(
            run(home.path(), &["peers", "add", name, path.to_str().unwrap()])
                .status
                .success()
        );
    }
    let path = saved_path(home.path());
    let before = std::fs::read(&path).expect("read saved list");
    let result = run(home.path(), &["peers", "show", "pi"]);
    assert!(result.status.success());
    assert!(result.stderr.is_empty());
    let key = std::fs::read_to_string(&first).expect("read public key");
    use p256::pkcs8::{DecodePublicKey, EncodePublicKey};
    use sha2::{Digest, Sha256};
    let der = p256::PublicKey::from_public_key_pem(&key)
        .unwrap()
        .to_public_key_der()
        .unwrap();
    let hash = Sha256::digest(der.as_bytes());
    assert_eq!(
        String::from_utf8(result.stdout).unwrap(),
        format!("Device name: pi\nFingerprint: SHA256:{hash:x}\nPublic key:\n{key}")
    );
    assert_eq!(std::fs::read(&path).unwrap(), before);
}

#[test]
fn showing_a_missing_peer_returns_an_error_without_changing_storage() {
    let home = private_home();
    let empty = run(home.path(), &["peers", "show", "missing"]);
    assert!(!empty.status.success());
    assert!(empty.stdout.is_empty());
    assert!(!saved_path(home.path()).exists());
    let key = public_file(home.path(), "pi.pem");
    assert!(
        run(home.path(), &["peers", "add", "pi", key.to_str().unwrap()])
            .status
            .success()
    );
    let path = saved_path(home.path());
    let before = std::fs::read(&path).expect("read saved list");
    let result = run(home.path(), &["peers", "show", "missing"]);
    assert!(!result.status.success());
    assert!(result.stdout.is_empty());
    assert!(String::from_utf8_lossy(&result.stderr).contains("Device was not found"));
    assert_eq!(std::fs::read(&path).unwrap(), before);
}

#[test]
fn rejects_private_keys_without_printing_or_saving_them() {
    let home = private_home();
    let key = rcgen::KeyPair::generate()
        .expect("test key")
        .serialize_pem();
    let path = home.path().join("private.pem");
    std::fs::write(&path, &key).expect("write test private key");
    let result = run(
        home.path(),
        &["peers", "add", "bad", path.to_str().unwrap()],
    );
    assert!(!result.status.success());
    assert!(result.stdout.is_empty());
    assert!(!String::from_utf8_lossy(&result.stderr).contains(&key));
    assert!(!saved_path(home.path()).exists());
}

#[test]
fn damaged_lists_and_unsafe_permissions_are_not_overwritten() {
    let home = private_home();
    let key = public_file(home.path(), "key.pem");
    assert!(
        run(home.path(), &["peers", "add", "pi", key.to_str().unwrap()])
            .status
            .success()
    );
    let path = saved_path(home.path());
    std::fs::write(&path, b"damaged JSON").expect("damage list");
    for args in [
        vec!["peers", "list"],
        vec!["peers", "remove", "pi"],
        vec!["peers", "add", "ovh", key.to_str().unwrap()],
    ] {
        assert!(!run(home.path(), &args).status.success());
        assert_eq!(std::fs::read(&path).unwrap(), b"damaged JSON");
    }
    std::fs::write(&path, b"[]").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(
        !run(home.path(), &["peers", "add", "pi", key.to_str().unwrap()])
            .status
            .success()
    );
    assert_eq!(std::fs::read(&path).unwrap(), b"[]");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    let lock = path.with_file_name("trusted-peers.lock");
    std::fs::set_permissions(&lock, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(!run(home.path(), &["peers", "list"]).status.success());
    assert_eq!(
        std::fs::metadata(lock).unwrap().permissions().mode() & 0o777,
        0o644
    );
}

#[test]
fn rejects_symlinks_for_the_list_lock_and_public_key() {
    let home = private_home();
    let key = public_file(home.path(), "key.pem");
    assert!(run(home.path(), &["peers", "list"]).status.success());
    let key_link = home.path().join("key-link.pem");
    std::os::unix::fs::symlink(&key, &key_link).unwrap();
    assert!(
        !run(
            home.path(),
            &["peers", "add", "pi", key_link.to_str().unwrap()]
        )
        .status
        .success()
    );
    let target = home.path().join("target.txt");
    std::fs::write(&target, b"unchanged").unwrap();
    for path in [
        saved_path(home.path()),
        saved_path(home.path()).with_file_name("trusted-peers.lock"),
    ] {
        if path.exists() {
            std::fs::remove_file(&path).unwrap();
        }
        std::os::unix::fs::symlink(&target, &path).unwrap();
        assert!(!run(home.path(), &["peers", "list"]).status.success());
        assert_eq!(std::fs::read(&target).unwrap(), b"unchanged");
        assert_eq!(std::fs::read_link(&path).unwrap(), target);
        std::fs::remove_file(&path).unwrap();
    }
}

#[test]
fn rejects_oversized_public_key_files() {
    let home = private_home();
    let path = home.path().join("large.pem");
    std::fs::write(&path, vec![b'A'; 16 * 1024 + 1]).unwrap();
    assert!(
        !run(home.path(), &["peers", "add", "pi", path.to_str().unwrap()])
            .status
            .success()
    );
    assert!(!saved_path(home.path()).exists());
}

#[test]
fn concurrent_updates_keep_every_added_peer() {
    let home = private_home();
    let mut children = Vec::new();
    for index in 0..8 {
        let key = public_file(home.path(), &format!("key-{index}.pem"));
        children.push(
            Command::new(env!("CARGO_BIN_EXE_fleet"))
                .args([
                    "peers",
                    "add",
                    &format!("node-{index}"),
                    key.to_str().unwrap(),
                ])
                .env("HOME", home.path())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .expect("start concurrent command"),
        );
    }
    for child in children {
        let result = child.wait_with_output().expect("wait for command");
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
    let result = run(home.path(), &["peers", "list"]);
    assert!(result.status.success());
    let names = String::from_utf8(result.stdout).unwrap();
    assert_eq!(names.lines().count(), 8);
    for index in 0..8 {
        assert!(names.lines().any(|name| name == format!("node-{index}")));
    }
}
