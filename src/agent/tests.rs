use super::*;

struct TestWorkload(ManagedWorkload);

impl Drop for TestWorkload {
    fn drop(&mut self) {
        self.0.runner.stop_all();
    }
}

fn app(name: &str, replicas: u32) -> TestWorkload {
    TestWorkload(ManagedWorkload::new(WorkloadSpec {
        name: name.into(),
        program: "/bin/sleep".into(),
        args: vec!["10".into()],
        replicas,
    }))
}

#[test]
fn construction_keeps_settings_without_starting_processes() {
    let before = Instant::now();
    let mut app = app("api", 2);
    assert_eq!(app.0.spec.name, "api");
    assert_eq!(app.0.spec.replicas, 2);
    assert_eq!(app.0.runner.running_count(), 0);
    assert!(app.0.retry_at >= before && app.0.retry_at <= Instant::now());
}

#[test]
fn ticks_start_missing_copies_and_stop_extra_copies() {
    let keep_alive = AtomicBool::new(true);
    let mut app = app("api", 2);
    app.0.tick(&keep_alive);
    assert_eq!(app.0.runner.running_count(), 2);
    app.0.tick(&keep_alive);
    assert_eq!(app.0.runner.running_count(), 2);
    app.0.spec.replicas = 1;
    app.0.tick(&keep_alive);
    assert_eq!(app.0.runner.running_count(), 1);
    app.0.spec.replicas = 0;
    app.0.tick(&keep_alive);
    assert_eq!(app.0.runner.running_count(), 0);
}

#[test]
fn stopping_and_restarting_one_app_does_not_change_another() {
    let keep_alive = AtomicBool::new(true);
    let mut first = app("api", 2);
    let mut second = app("scraper", 1);
    first.0.tick(&keep_alive);
    second.0.tick(&keep_alive);
    let stopped = first.0.stop();
    assert_eq!(stopped.desired, 0);
    assert_eq!(stopped.running, 0);
    first.0.tick(&keep_alive);
    assert_eq!(first.0.runner.running_count(), 0);
    assert_eq!(second.0.runner.running_count(), 1);
    first.0.scale(1).unwrap();
    first.0.tick(&keep_alive);
    assert_eq!(first.0.runner.running_count(), 1);
    assert_eq!(second.0.runner.running_count(), 1);
    assert_eq!(second.0.spec.replicas, 1);
}

#[test]
fn status_and_scale_report_current_counts_before_the_next_tick() {
    let keep_alive = AtomicBool::new(true);
    let mut app = app("api", 2);
    let initial = app.0.status();
    assert_eq!(initial.workload_name.as_deref(), Some("api"));
    assert_eq!(initial.desired, 2);
    assert_eq!(initial.running, 0);
    app.0.tick(&keep_alive);
    let scaled = app.0.scale(1).unwrap();
    assert_eq!(scaled.desired, 1);
    assert_eq!(scaled.running, 2);
    app.0.tick(&keep_alive);
    assert_eq!(app.0.status().running, 1);
}

#[test]
fn scale_accepts_boundaries_and_rejects_invalid_counts_without_changes() {
    let mut app = app("api", 1);
    assert_eq!(app.0.scale(0).unwrap().desired, 0);
    assert_eq!(app.0.scale(MAX_REPLICAS).unwrap().desired, MAX_REPLICAS);
    let retry_at = app.0.retry_at;
    let error = app.0.scale(MAX_REPLICAS + 1).unwrap_err();
    assert_eq!(
        error,
        format!("Replica count cannot exceed {}", MAX_REPLICAS)
    );
    assert_eq!(app.0.spec.replicas, MAX_REPLICAS);
    assert_eq!(app.0.status().running, 0);
    assert_eq!(app.0.retry_at, retry_at);
}

#[test]
fn failed_launch_waits_five_seconds_without_delaying_another_app() {
    let keep_alive = AtomicBool::new(true);
    let directory = tempfile::tempdir().unwrap();
    let mut failed = app("broken", 1);
    failed.0.spec.program = directory
        .path()
        .join("missing-program")
        .to_str()
        .unwrap()
        .into();
    let before = Instant::now();
    failed.0.tick(&keep_alive);
    assert_eq!(failed.0.runner.running_count(), 0);
    assert!(failed.0.retry_at >= before + Duration::from_secs(5));
    assert!(failed.0.retry_at <= Instant::now() + Duration::from_secs(5));

    // a fixed app must still wait to retry
    failed.0.spec.program = "/bin/sleep".into();
    failed.0.tick(&keep_alive);
    assert_eq!(failed.0.runner.running_count(), 0);
    let mut healthy = app("healthy", 1);
    healthy.0.tick(&keep_alive);
    assert_eq!(healthy.0.runner.running_count(), 1);

    // skip the wait in this test
    failed.0.retry_at = Instant::now();
    failed.0.tick(&keep_alive);
    assert_eq!(failed.0.runner.running_count(), 1);
    assert_eq!(healthy.0.runner.running_count(), 1);
}

#[test]
fn launch_delay_does_not_prevent_stopping_extra_copies() {
    let keep_alive = AtomicBool::new(true);
    let mut app = app("api", 2);
    app.0.tick(&keep_alive);
    app.0.retry_at = Instant::now() + Duration::from_secs(60);
    app.0.spec.replicas = 0;
    app.0.tick(&keep_alive);
    assert_eq!(app.0.runner.running_count(), 0);
}

#[test]
fn shutdown_flag_prevents_start_and_stop_actions() {
    let keep_alive = AtomicBool::new(false);
    let mut app = app("api", 1);
    app.0.tick(&keep_alive);
    assert_eq!(app.0.runner.running_count(), 0);
    keep_alive.store(true, Ordering::SeqCst);
    app.0.tick(&keep_alive);
    keep_alive.store(false, Ordering::SeqCst);
    app.0.spec.replicas = 0;
    app.0.tick(&keep_alive);
    assert_eq!(app.0.runner.running_count(), 1);
}
