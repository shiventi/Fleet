use super::*;
use std::fs;
use std::io::Read;

fn spec(files: &[&str]) -> DeploySpec {
    DeploySpec {
        workload: WorkloadSpec {
            name: "hello".into(), program: "python3".into(), args: vec!["-u".into(), "app.py".into()], replicas: 1,
        },
        files: files.iter().map(PathBuf::from).collect(),
    }
}

fn manifest(root: &Path, value: serde_json::Value) -> PathBuf {
    let path = root.join("fleet.json");
    fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
    path
}

#[test]
fn loads_the_hello_example_and_checks_its_files() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/hello/fleet.json");
    let spec = DeploySpec::load_manifest(&path).unwrap();
    assert_eq!(spec.workload.name, "hello");
    assert_eq!(spec.workload.program, "python3");
    assert_eq!(spec.workload.args, ["-u", "app.py"]);
    assert_eq!(spec.workload.replicas, 1);
    assert_eq!(spec.files, [PathBuf::from("app.py")]);
}

#[test]
fn accepts_nested_regular_files_and_resolves_them_from_the_manifest() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("assets")).unwrap();
    fs::write(root.path().join("assets/data.txt"), "data").unwrap();
    let path = manifest(root.path(), serde_json::json!({
        "workload": { "name": "hello", "program": "not-a-real-program", "args": [], "replicas": 1 },
        "files": ["assets/data.txt"]
    }));
    let loaded = DeploySpec::load_manifest(&path).unwrap();
    assert_eq!(loaded.files, [PathBuf::from("assets/data.txt")]);
    assert_eq!(fs::read_to_string(root.path().join("assets/data.txt")).unwrap(), "data");
}

#[test]
fn bare_manifest_names_use_the_current_folder() {
    let file = tempfile::NamedTempFile::new_in(std::env::current_dir().unwrap()).unwrap();
    let name = PathBuf::from(file.path().file_name().unwrap());
    fs::write(file.path(), serde_json::to_vec(&serde_json::json!({
        "workload": { "name": "hello", "program": "python3", "args": [], "replicas": 0 },
        "files": [name]
    })).unwrap()).unwrap();
    let loaded = DeploySpec::load_manifest(&name).unwrap();
    assert_eq!(loaded.files, [name]);
}

#[test]
fn rejects_empty_file_lists_and_invalid_workloads() {
    assert!(spec(&[]).validate().unwrap_err().contains("must include files"));
    let mut app = spec(&["app.py"]);
    app.workload.name = " ".into();
    assert!(app.validate().unwrap_err().contains("name cannot be blank"));
    app.workload.name = "hello".into();
    app.workload.program.clear();
    assert!(app.validate().unwrap_err().contains("program cannot be blank"));
    app.workload.program = "python3".into();
    app.workload.replicas = crate::workload::MAX_REPLICAS + 1;
    assert!(app.validate().unwrap_err().contains("cannot exceed"));
}

#[test]
fn rejects_empty_absolute_and_parent_paths() {
    let root = tempfile::tempdir().unwrap();
    for file in ["", "/app.py", "../app.py", "assets/../../app.py", ".", "./app.py", ".."] {
        let error = spec(&[file]).validate_files(root.path()).unwrap_err();
        assert!(error.contains("Path cannot be empty") || error.contains("must stay inside"), "{file}: {error}");
    }
}

#[test]
fn rejects_missing_files_directories_and_files_used_as_parents() {
    let root = tempfile::tempdir().unwrap();
    let error = spec(&["missing.py"]).validate_files(root.path()).unwrap_err();
    assert!(error.contains("missing.py"));
    fs::create_dir(root.path().join("assets")).unwrap();
    assert!(spec(&["assets"]).validate_files(root.path()).unwrap_err().contains("Not a regular file"));
    fs::write(root.path().join("file.txt"), "data").unwrap();
    assert!(spec(&["file.txt/child"]).validate_files(root.path()).is_err());
    assert!(spec(&["file.txt"]).validate_files(&root.path().join("missing-root")).is_err());
}

#[test]
fn loader_reports_read_parse_and_validation_errors() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("fleet.json");
    let error = DeploySpec::load_manifest(&path).unwrap_err();
    assert!(error.contains("Failed to read"));
    assert!(error.contains(path.to_str().unwrap()));
    fs::write(&path, "not json").unwrap();
    let error = DeploySpec::load_manifest(&path).unwrap_err();
    assert!(error.contains("Invalid JSON"));
    assert!(error.contains(path.to_str().unwrap()));
    for files in [serde_json::json!([]), serde_json::json!(["missing.py"]), serde_json::json!(["../outside.py"])] {
        manifest(root.path(), serde_json::json!({
            "workload": { "name": "hello", "program": "python3", "args": [], "replicas": 1 }, "files": files
        }));
        assert!(DeploySpec::load_manifest(&path).is_err());
    }
    manifest(root.path(), serde_json::json!({ "files": ["app.py"] }));
    assert!(DeploySpec::load_manifest(&path).unwrap_err().contains("Invalid JSON"));
}

#[cfg(unix)]
#[test]
fn rejects_file_parent_and_dangling_symlinks() {
    use std::os::unix::fs::symlink;
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::write(root.path().join("real.py"), "inside").unwrap();
    fs::write(outside.path().join("secret.py"), "outside").unwrap();
    symlink(root.path().join("real.py"), root.path().join("inside-link.py")).unwrap();
    symlink(outside.path().join("secret.py"), root.path().join("outside-link.py")).unwrap();
    symlink(outside.path(), root.path().join("linked-folder")).unwrap();
    symlink(root.path().join("missing"), root.path().join("dangling.py")).unwrap();
    for file in ["inside-link.py", "outside-link.py", "linked-folder/secret.py", "dangling.py"] {
        assert!(spec(&[file]).validate_files(root.path()).unwrap_err().contains("Symlinks are not allowed"));
    }
    assert_eq!(fs::read_to_string(outside.path().join("secret.py")).unwrap(), "outside");
}

#[cfg(unix)]
#[test]
fn rejects_socket_files_without_opening_them() {
    let root = tempfile::tempdir().unwrap();
    let _socket = std::os::unix::net::UnixListener::bind(root.path().join("app.sock")).unwrap();
    assert!(spec(&["app.sock"]).validate_files(root.path()).unwrap_err().contains("Not a regular file"));
}

#[cfg(unix)]
#[test]
fn rejects_pipes_without_opening_them() {
    let root = tempfile::tempdir().unwrap();
    let status = std::process::Command::new("mkfifo").arg(root.path().join("app.pipe")).status().unwrap();
    assert!(status.success());
    assert!(spec(&["app.pipe"]).validate_files(root.path()).unwrap_err().contains("Not a regular file"));
}

#[test]
fn packages_settings_and_only_the_listed_files() {
    let root = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("assets")).unwrap();
    let data = [0, 255, 10, 42];
    fs::write(root.path().join("assets/data.bin"), data).unwrap();
    fs::write(root.path().join(".env"), "not included").unwrap();
    let path = manifest(root.path(), serde_json::json!({
        "workload": { "name": "hello", "program": "python3", "args": ["-u", "app.py"], "replicas": 1 },
        "files": ["assets/data.bin"]
    }));
    let bundle = output.path().join("hello.tar");
    package(&path, &bundle).unwrap();

    let mut archive = tar::Archive::new(File::open(&bundle).unwrap());
    let mut names = Vec::new();
    for entry in archive.entries().unwrap() {
        let mut entry = entry.unwrap();
        assert!(entry.header().entry_type().is_file());
        let name = entry.path().unwrap().into_owned();
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).unwrap();
        if name == Path::new("fleet.json") {
            let decoded: DeploySpec = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(decoded.workload.name, "hello");
            assert_eq!(decoded.workload.args, ["-u", "app.py"]);
            assert_eq!(decoded.files, [PathBuf::from("assets/data.bin")]);
            assert_eq!(entry.header().mode().unwrap(), 0o644);
        } else {
            assert_eq!(name, Path::new("app/assets/data.bin"));
            assert_eq!(bytes, data);
        }
        names.push(name);
    }
    assert_eq!(names, [PathBuf::from("fleet.json"), PathBuf::from("app/assets/data.bin")]);
    assert_eq!(fs::read_dir(output.path()).unwrap().count(), 1);
    assert_eq!(fs::read(root.path().join("assets/data.bin")).unwrap(), data);
}

#[test]
fn packaging_never_overwrites_existing_files_and_cleans_up_temporary_files() {
    let root = tempfile::tempdir().unwrap();
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/hello/fleet.json");
    let output = root.path().join("hello.tar");
    fs::write(&output, "keep this").unwrap();
    assert!(package(&source, &output).is_err());
    assert_eq!(fs::read_to_string(&output).unwrap(), "keep this");
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
}

#[test]
fn bad_sources_do_not_leave_a_bundle_and_missing_output_folders_return_errors() {
    let root = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
    let path = manifest(root.path(), serde_json::json!({
        "workload": { "name": "hello", "program": "python3", "args": [], "replicas": 1 },
        "files": ["missing.py"]
    }));
    assert!(package(&path, &output.path().join("hello.tar")).is_err());
    assert_eq!(fs::read_dir(output.path()).unwrap().count(), 0);
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/hello/fleet.json");
    assert!(package(&source, &output.path().join("missing/hello.tar")).is_err());
    assert_eq!(fs::read_dir(output.path()).unwrap().count(), 0);
}

#[cfg(unix)]
#[test]
fn safe_opens_reject_symlinks_swapped_in_after_preflight() {
    use std::os::unix::fs::symlink;
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::write(root.path().join("app.py"), "inside").unwrap();
    fs::create_dir(root.path().join("assets")).unwrap();
    fs::write(root.path().join("assets/data.txt"), "inside").unwrap();
    fs::write(outside.path().join("data.txt"), "outside").unwrap();
    spec(&["app.py", "assets/data.txt"]).validate_files(root.path()).unwrap();
    let directory = File::open(root.path()).unwrap();
    fs::remove_file(root.path().join("app.py")).unwrap();
    symlink(outside.path().join("data.txt"), root.path().join("app.py")).unwrap();
    assert!(open_source(&directory, Path::new("app.py")).is_err());
    fs::rename(root.path().join("assets"), root.path().join("original-assets")).unwrap();
    symlink(outside.path(), root.path().join("assets")).unwrap();
    assert!(open_source(&directory, Path::new("assets/data.txt")).is_err());
}

#[cfg(unix)]
#[test]
fn safe_opens_stay_with_the_open_folder_if_its_path_is_replaced() {
    use std::os::unix::fs::symlink;
    let root = tempfile::tempdir().unwrap();
    let original = root.path().join("app");
    let replacement = root.path().join("other");
    fs::create_dir(&original).unwrap();
    fs::create_dir(&replacement).unwrap();
    fs::write(original.join("data.txt"), "original").unwrap();
    fs::write(replacement.join("data.txt"), "replacement").unwrap();
    let directory = File::open(&original).unwrap();
    fs::rename(&original, root.path().join("moved")).unwrap();
    symlink(&replacement, &original).unwrap();
    let mut file = open_source(&directory, Path::new("data.txt")).unwrap();
    let mut text = String::new();
    file.read_to_string(&mut text).unwrap();
    assert_eq!(text, "original");
}

#[cfg(unix)]
#[test]
fn safe_opens_reject_non_files_and_unsafe_paths() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("assets")).unwrap();
    let status = std::process::Command::new("mkfifo").arg(root.path().join("pipe")).status().unwrap();
    assert!(status.success());
    let directory = File::open(root.path()).unwrap();
    for path in ["", "../outside", "/outside", ".", "assets", "pipe"] {
        assert!(open_source(&directory, Path::new(path)).is_err(), "accepted {path}");
    }
}

#[cfg(unix)]
#[test]
fn packaging_does_not_follow_an_existing_output_symlink() {
    use std::os::unix::fs::symlink;
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("keep.txt");
    let output = root.path().join("hello.tar");
    fs::write(&target, "keep this").unwrap();
    symlink(&target, &output).unwrap();
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/hello/fleet.json");
    assert!(package(&source, &output).is_err());
    assert_eq!(fs::read_to_string(&target).unwrap(), "keep this");
    assert!(fs::symlink_metadata(&output).unwrap().file_type().is_symlink());
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 2);
}
