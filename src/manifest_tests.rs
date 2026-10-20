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
