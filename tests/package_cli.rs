use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn package(manifest: &Path, output: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_fleet"))
        .arg("package").arg("--manifest").arg(manifest).arg("--output").arg(output)
        .output().unwrap()
}

#[test]
fn package_cli_creates_a_bundle_and_refuses_to_overwrite_it() {
    let root = tempfile::tempdir().unwrap();
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/hello/fleet.json");
    let output = root.path().join("hello.tar");
    let result = package(&manifest, &output);
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    assert!(String::from_utf8_lossy(&result.stdout).contains("Bundle saved to"));
    let mut archive = tar::Archive::new(fs::File::open(&output).unwrap());
    let names: Vec<_> = archive.entries().unwrap().map(|entry| entry.unwrap().path().unwrap().into_owned()).collect();
    assert_eq!(names, [Path::new("fleet.json"), Path::new("app/app.py")]);
    let before = fs::read(&output).unwrap();
    let result = package(&manifest, &output);
    assert_eq!(result.status.code(), Some(1));
    assert!(result.stdout.is_empty());
    assert!(String::from_utf8_lossy(&result.stderr).contains("Failed to package app:"));
    assert_eq!(fs::read(&output).unwrap(), before);
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
}

#[test]
fn package_cli_reports_duplicate_and_missing_files_without_creating_output() {
    let root = tempfile::tempdir().unwrap();
    let manifest = root.path().join("fleet.json");
    let output = root.path().join("hello.tar");
    fs::write(root.path().join("app.py"), "hello").unwrap();
    for (files, expected) in [
        (serde_json::json!(["app.py", "app.py"]), "Duplicate file: app.py"),
        (serde_json::json!(["missing.py"]), "missing.py"),
    ] {
        let json = serde_json::json!({
            "workload": { "name": "hello", "program": "python3", "args": [], "replicas": 1 }, "files": files
        });
        fs::write(&manifest, serde_json::to_vec(&json).unwrap()).unwrap();
        let result = package(&manifest, &output);
        assert_eq!(result.status.code(), Some(1));
        assert!(result.stdout.is_empty());
        assert!(String::from_utf8_lossy(&result.stderr).contains(expected));
        assert!(!output.exists());
    }
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 2);
}
