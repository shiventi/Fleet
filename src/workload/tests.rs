use super::*;

fn spec(name: &str) -> WorkloadSpec {
    WorkloadSpec {
        name: name.into(),
        program: "/bin/sleep".into(),
        args: vec!["10".into()],
        replicas: 1,
    }
}

#[test]
fn validates_replica_limits() {
    let mut workload = spec("test");
    workload.replicas = 0;
    assert!(workload.validate().is_ok());
    workload.replicas = MAX_REPLICAS;
    assert!(workload.validate().is_ok());
    workload.replicas = MAX_REPLICAS + 1;
    assert!(workload.validate().is_err());
}

#[test]
fn accepts_distinct_apps_using_the_same_program() {
    let manifest = AgentManifest {
        workloads: vec![spec("api"), spec("scraper")],
    };
    manifest.validate().unwrap();
}

#[test]
fn rejects_an_empty_app_list() {
    let manifest = AgentManifest {
        workloads: Vec::new(),
    };
    assert_eq!(
        manifest.validate().unwrap_err(),
        "Manifest must contain at least one workload"
    );
}

#[test]
fn rejects_duplicate_names_even_with_different_settings() {
    let mut duplicate = spec("api");
    duplicate.args = vec!["20".into()];
    duplicate.replicas = 2;
    let manifest = AgentManifest {
        workloads: vec![spec("api"), spec("scraper"), duplicate],
    };
    assert!(
        manifest
            .validate()
            .unwrap_err()
            .contains("Duplicate app name api")
    );
}

#[test]
fn checks_each_apps_name_program_and_copy_limit() {
    let mut blank_name = spec(" \t");
    blank_name.replicas = 0;
    let mut blank_program = spec("scraper");
    blank_program.program = " \t".into();
    let mut too_many = spec("scraper");
    too_many.replicas = MAX_REPLICAS + 1;
    for invalid in [blank_name, blank_program, too_many] {
        let manifest = AgentManifest {
            workloads: vec![spec("api"), invalid],
        };
        assert!(manifest.validate().is_err());
    }
}

#[test]
fn allows_zero_copies_and_applies_the_limit_per_app() {
    let mut stopped = spec("stopped");
    stopped.replicas = 0;
    let mut first = spec("first");
    first.replicas = MAX_REPLICAS;
    let mut second = spec("second");
    second.replicas = MAX_REPLICAS;
    let manifest = AgentManifest {
        workloads: vec![stopped, first, second],
    };
    manifest.validate().unwrap();
}
