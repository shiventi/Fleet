use super::load_agent_manifest;

fn file(contents: &str) -> tempfile::NamedTempFile {
    let file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(file.path(), contents).unwrap();
    file
}

#[test]
fn loads_distinct_apps_in_order_without_launching_programs() {
    let file = file(
        r#"{"workloads":[
        {"name":"api","program":"/not-launched/api","args":["--port","8000"],"replicas":2},
        {"name":"scraper","program":"/not-launched/scraper","args":[],"replicas":0}
    ]}"#,
    );
    let manifest = load_agent_manifest(file.path()).unwrap();
    assert_eq!(manifest.workloads.len(), 2);
    assert_eq!(manifest.workloads[0].name, "api");
    assert_eq!(manifest.workloads[0].program, "/not-launched/api");
    assert_eq!(manifest.workloads[0].args, ["--port", "8000"]);
    assert_eq!(manifest.workloads[0].replicas, 2);
    assert_eq!(manifest.workloads[1].name, "scraper");
    assert_eq!(manifest.workloads[1].replicas, 0);
}

#[test]
fn loads_old_single_app_files_as_one_entry() {
    let file = file(include_str!("../examples/demo.json"));
    let manifest = load_agent_manifest(file.path()).unwrap();
    assert_eq!(manifest.workloads.len(), 1);
    assert_eq!(manifest.workloads[0].name, "demo-service");
    assert_eq!(manifest.workloads[0].replicas, 1);
    assert_eq!(manifest.workloads[0].program, "/bin/sleep");
}

#[test]
fn loads_the_two_app_example() {
    let file = file(include_str!("../examples/apps.json"));
    let manifest = load_agent_manifest(file.path()).unwrap();
    assert_eq!(manifest.workloads.len(), 2);
    assert_eq!(manifest.workloads[0].name, "demo-fast");
    assert_eq!(manifest.workloads[0].args, ["10"]);
    assert_eq!(manifest.workloads[0].replicas, 1);
    assert_eq!(manifest.workloads[1].name, "demo-slow");
    assert_eq!(manifest.workloads[1].args, ["30"]);
    assert_eq!(manifest.workloads[1].replicas, 2);
}

#[test]
fn invalid_app_lists_do_not_fall_back_to_single_apps() {
    let file = file(r#"{"workloads":null,"name":"api","program":"/bin/sleep","args":[],"replicas":1}"#);
    assert!(load_agent_manifest(file.path()).is_err());
}

#[test]
fn validates_single_app_files_too() {
    for (name, program, replicas) in [(" ", "/bin/sleep", 1), ("api", " ", 1), ("api", "/bin/sleep", 33)] {
        let file = file(&serde_json::json!({"name": name, "program": program, "args": [], "replicas": replicas}).to_string());
        let error = load_agent_manifest(file.path()).unwrap_err();
        assert!(error.contains("Invalid agent manifest"), "{error}");
        assert!(error.contains(&file.path().display().to_string()));
    }
}

#[test]
fn reports_missing_files_with_their_path() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("missing.json");
    let error = load_agent_manifest(&path).unwrap_err();
    assert!(error.contains("Failed to read file"));
    assert!(error.contains(&path.display().to_string()));
}

#[test]
fn reports_bad_json_and_wrong_shapes_with_their_path() {
    for contents in [
        "{",
        "{}",
        r#"{"workloads":null}"#,
        r#"{"workloads":[{"name":"missing-fields"}]}"#,
    ] {
        let file = file(contents);
        let error = load_agent_manifest(file.path()).unwrap_err();
        assert!(error.contains("Failed to parse JSON"), "{error}");
        assert!(error.contains(&file.path().display().to_string()));
    }
}

#[test]
fn returns_validation_errors_instead_of_accepting_invalid_manifests() {
    let valid = serde_json::json!({"name":"api","program":"/bin/sleep","args":["10"],"replicas":1});
    let invalid =
        serde_json::json!({"name":"scraper","program":"/bin/sleep","args":[],"replicas":33});
    for workloads in [
        serde_json::json!([]),
        serde_json::json!([valid, valid]),
        serde_json::json!([valid, invalid]),
    ] {
        let file = file(&serde_json::json!({"workloads":workloads}).to_string());
        let error = load_agent_manifest(file.path()).unwrap_err();
        assert!(error.contains("Invalid agent manifest"), "{error}");
        assert!(error.contains(&file.path().display().to_string()));
    }
}
