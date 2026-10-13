use super::*;
use std::os::unix::fs::PermissionsExt;

fn private_directory() -> tempfile::TempDir {
    let directory = tempfile::tempdir().expect("create test directory");
    std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700))
        .expect("make test directory private");
    directory
}

#[test]
fn saved_identity_keeps_its_public_key() {
    let directory = private_directory();
    let path = directory.path().join("identity.pem");
    let identity = DeviceIdentity::generate().expect("generate identity");

    identity.save_new(&path).expect("save identity");
    let loaded = DeviceIdentity::load(&path).expect("load identity");

    assert_eq!(identity.public_key_pem(), loaded.public_key_pem());
}

#[test]
fn private_writer_creates_and_replaces_owner_only_files() {
    let directory = private_directory();
    let path = directory.path().join("private.txt");
    write_private(&path, b"original", false).expect("create private file");
    write_private(&path, b"updated", true).expect("replace private file");

    assert_eq!(
        read_private(&path, 32).expect("read updated file"),
        "updated"
    );
    assert_eq!(
        std::fs::metadata(&path)
            .expect("read permissions")
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert_eq!(
        std::fs::read_dir(directory.path())
            .expect("list files")
            .count(),
        1
    );
}

#[test]
fn private_writer_can_replace_a_missing_file() {
    let directory = private_directory();
    let path = directory.path().join("private.txt");
    write_private(&path, b"new", true).expect("create missing file");
    assert_eq!(read_private(&path, 32).expect("read new file"), "new");
}

#[test]
fn private_writer_does_not_overwrite_without_permission() {
    let directory = private_directory();
    let path = directory.path().join("private.txt");
    write_private(&path, b"original", false).expect("create file");
    assert_eq!(
        write_private(&path, b"updated", false)
            .expect_err("reject overwrite")
            .kind(),
        io::ErrorKind::AlreadyExists
    );
    assert_eq!(
        read_private(&path, 32).expect("read original file"),
        "original"
    );
}

#[test]
fn private_writer_rejects_replacing_symlinks() {
    let directory = private_directory();
    let target = directory.path().join("target.txt");
    let link = directory.path().join("link.txt");
    write_private(&target, b"original", false).expect("create target");
    std::os::unix::fs::symlink(&target, &link).expect("create symlink");

    assert!(write_private(&link, b"updated", true).is_err());
    assert_eq!(read_private(&target, 32).expect("read target"), "original");
    assert_eq!(std::fs::read_link(&link).expect("symlink remains"), target);

    std::fs::remove_file(&target).expect("make symlink dangling");
    assert!(write_private(&link, b"updated", true).is_err());
    assert_eq!(
        std::fs::read_link(&link).expect("dangling symlink remains"),
        target
    );
    assert!(!target.exists());
}

#[test]
fn private_writer_rejects_replacing_files_with_broad_permissions() {
    let directory = private_directory();
    let path = directory.path().join("private.txt");
    write_private(&path, b"original", false).expect("create file");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644))
        .expect("broaden permissions");

    assert_eq!(
        write_private(&path, b"updated", true)
            .expect_err("reject unsafe file")
            .kind(),
        io::ErrorKind::PermissionDenied
    );
    assert_eq!(
        std::fs::read_to_string(&path).expect("read unchanged file"),
        "original"
    );
    assert_eq!(
        std::fs::metadata(&path)
            .expect("read permissions")
            .permissions()
            .mode()
            & 0o777,
        0o644
    );
}

#[test]
fn private_reader_accepts_the_limit_and_rejects_larger_files() {
    let directory = private_directory();
    let path = directory.path().join("private.txt");
    let mut file = tempfile::NamedTempFile::new_in(directory.path()).expect("create private file");
    file.write_all(b"abcd").expect("write test text");
    file.persist_noclobber(&path).expect("publish private file");

    assert_eq!(read_private(&path, 4).expect("read exact limit"), "abcd");
    assert_eq!(
        read_private(&path, 3)
            .expect_err("reject oversized file")
            .kind(),
        io::ErrorKind::InvalidData
    );
}

#[test]
fn existing_identity_is_not_overwritten() {
    let directory = private_directory();
    let path = directory.path().join("identity.pem");
    let first = DeviceIdentity::generate().expect("generate first identity");
    first.save_new(&path).expect("save first identity");

    let second = DeviceIdentity::generate().expect("generate second identity");
    let error = second
        .save_new(&path)
        .expect_err("must not overwrite identity");
    assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);

    let loaded = DeviceIdentity::load(&path).expect("load original identity");
    assert_eq!(first.public_key_pem(), loaded.public_key_pem());
}

#[test]
fn load_or_create_creates_once_and_reuses_the_identity() {
    let directory = private_directory();
    let path = directory.path().join("identity.pem");

    let first = DeviceIdentity::load_or_create(&path).expect("create identity");
    let second = DeviceIdentity::load_or_create(&path).expect("reuse identity");
    let loaded = DeviceIdentity::load(&path).expect("load saved identity");

    assert_eq!(first.public_key_pem(), second.public_key_pem());
    assert_eq!(first.public_key_pem(), loaded.public_key_pem());
}

#[test]
fn load_or_create_does_not_replace_a_damaged_file() {
    let directory = private_directory();
    let path = directory.path().join("identity.pem");
    DeviceIdentity::load_or_create(&path).expect("create identity");
    std::fs::write(&path, "damaged key").expect("damage test file");

    assert!(DeviceIdentity::load_or_create(&path).is_err());
    assert_eq!(
        std::fs::read_to_string(&path).expect("read test file"),
        "damaged key"
    );
}

#[test]
fn load_or_create_rejects_broad_permissions_without_replacing_the_key() {
    let directory = private_directory();
    let path = directory.path().join("identity.pem");
    let first = DeviceIdentity::load_or_create(&path).expect("create identity");
    assert_eq!(
        std::fs::metadata(&path)
            .expect("read permissions")
            .permissions()
            .mode()
            & 0o077,
        0
    );
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644))
        .expect("broaden permissions");

    assert!(DeviceIdentity::load_or_create(&path).is_err());

    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
        .expect("restore permissions");
    let loaded = DeviceIdentity::load(&path).expect("load original identity");
    assert_eq!(first.public_key_pem(), loaded.public_key_pem());
}

#[test]
fn load_or_create_reports_a_missing_parent_directory() {
    let directory = private_directory();
    let path = directory.path().join("missing").join("identity.pem");

    let error = DeviceIdentity::load_or_create(&path)
        .err()
        .expect("missing directory must fail");
    assert_eq!(error.kind(), io::ErrorKind::NotFound);
    assert!(!path.exists());
}

#[test]
fn default_directory_uses_home_without_creating_a_key() {
    // Use a child process so changing HOME cannot affect other tests.
    if let Some(home) = std::env::var_os("FLEET_TEST_PRIVATE_HOME") {
        let directory = default_directory().expect("prepare default directory");
        assert_eq!(directory, Path::new(&home).join(".config/fleet/identity"));
        assert_eq!(
            std::fs::read_dir(directory).expect("list storage").count(),
            0
        );
        return;
    }
    let home = private_directory();
    let result = std::process::Command::new(std::env::current_exe().expect("test executable"))
        .args([
            "--exact",
            "identity::tests::default_directory_uses_home_without_creating_a_key",
        ])
        .env("HOME", home.path())
        .env("FLEET_TEST_PRIVATE_HOME", home.path())
        .output()
        .expect("run directory test in child process");
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stdout)
    );
}

#[test]
fn preparing_storage_does_not_create_a_key() {
    let home = private_directory();
    let directory = prepare_directory(home.path()).expect("prepare storage");

    assert_eq!(directory, home.path().join(".config/fleet/identity"));
    assert_eq!(
        prepare_directory(home.path()).expect("reuse storage"),
        directory
    );
    assert_eq!(
        std::fs::read_dir(&directory).expect("list storage").count(),
        0
    );
    assert_eq!(
        std::fs::metadata(&directory)
            .expect("directory permissions")
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
}

#[test]
fn preparing_storage_rejects_relative_home_paths() {
    assert_eq!(
        prepare_directory(Path::new("relative-home"))
            .expect_err("reject relative HOME")
            .kind(),
        io::ErrorKind::InvalidInput
    );
}

#[test]
fn default_storage_is_private_and_reuses_the_identity() {
    let home = private_directory();
    let first = DeviceIdentity::load_for_home(home.path()).expect("initialize storage");
    let second = DeviceIdentity::load_for_home(home.path()).expect("reuse storage");
    assert_eq!(first.public_key_pem(), second.public_key_pem());

    let directory = home.path().join(".config/fleet/identity");
    let path = directory.join("device-key.pem");
    assert_eq!(
        std::fs::metadata(directory)
            .expect("directory permissions")
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    assert_eq!(
        std::fs::metadata(path)
            .expect("key permissions")
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
}

#[test]
fn concurrent_initializers_all_use_the_same_complete_key() {
    let home = private_directory();
    let barrier = std::sync::Barrier::new(8);
    let keys = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..8)
            .map(|_| {
                scope.spawn(|| {
                    barrier.wait();
                    DeviceIdentity::load_for_home(home.path())
                        .expect("initialize concurrently")
                        .public_key_pem()
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| handle.join().expect("join initializer"))
            .collect::<Vec<_>>()
    });
    assert!(keys.iter().all(|key| key == &keys[0]));
    let files = std::fs::read_dir(home.path().join(".config/fleet/identity"))
        .expect("list storage")
        .count();
    assert_eq!(files, 1);
}

#[test]
fn incomplete_temporary_files_are_not_loaded_as_identities() {
    let directory = private_directory();
    std::fs::write(directory.path().join("interrupted.tmp"), "incomplete key")
        .expect("simulate interrupted write");
    let path = directory.path().join("identity.pem");
    let identity = DeviceIdentity::load_or_create(&path).expect("create complete key");
    assert_eq!(
        identity.public_key_pem(),
        DeviceIdentity::load(&path)
            .expect("load complete key")
            .public_key_pem()
    );
}

#[test]
fn identity_symlinks_are_rejected_without_overwriting_the_target() {
    let directory = private_directory();
    let target = directory.path().join("original.pem");
    let identity = DeviceIdentity::load_or_create(&target).expect("create original identity");
    let link = directory.path().join("link.pem");
    std::os::unix::fs::symlink(&target, &link).expect("create key symlink");
    assert!(DeviceIdentity::load_or_create(&link).is_err());
    assert_eq!(
        identity.public_key_pem(),
        DeviceIdentity::load(&target)
            .expect("load original key")
            .public_key_pem()
    );
}

#[test]
fn oversized_identity_files_are_rejected() {
    let directory = private_directory();
    let path = directory.path().join("identity.pem");
    DeviceIdentity::load_or_create(&path).expect("create key");
    std::fs::write(&path, vec![b'A'; MAX_KEY_BYTES as usize + 1]).expect("write oversized file");
    let error = DeviceIdentity::load_or_create(&path)
        .err()
        .expect("reject oversized key");
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
}

#[test]
fn unsafe_directories_are_rejected_without_changing_permissions() {
    let home = private_directory();
    DeviceIdentity::load_for_home(home.path()).expect("create storage");
    let directory = home.path().join(".config/fleet/identity");
    std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o755))
        .expect("broaden directory permissions");
    assert!(DeviceIdentity::load_for_home(home.path()).is_err());
    assert_eq!(
        std::fs::metadata(directory)
            .expect("directory permissions")
            .permissions()
            .mode()
            & 0o777,
        0o755
    );
}

#[test]
fn directory_symlinks_are_rejected_before_writing_keys() {
    let home = private_directory();
    let target = private_directory();
    let config = home.path().join(".config");
    ensure_directory(&config, false).expect("create config directory");
    std::os::unix::fs::symlink(target.path(), config.join("fleet"))
        .expect("create directory symlink");
    assert!(DeviceIdentity::load_for_home(home.path()).is_err());
    assert_eq!(
        std::fs::read_dir(target.path())
            .expect("list symlink target")
            .count(),
        0
    );
}
