use orbit_core::OrbitRuntime;
use tempfile::tempdir;

#[test]
fn watch_runner_coalesces_burst_events_with_queue_one() {
    let dir = tempdir().expect("tempdir");
    let runtime = OrbitRuntime::from_data_root(dir.path()).expect("runtime");

    let db_path = dir.path().join("orbit.db");
    let store = orbit_store::Store::open(&db_path).expect("store");

    let watched_path = dir.path().join("watched.txt");
    std::fs::write(&watched_path, "seed").expect("seed watched file");

    let output_file = dir.path().join("watch-output.txt");
    let watch_command = format!("printf 'run\\n' >> {}", output_file.to_string_lossy());

    store
        .with_transaction(|tx| {
            let _watch = tx.insert_watch(&watched_path.to_string_lossy(), &watch_command, 500)?;
            Ok(())
        })
        .expect("insert watch");

    let mut source = orbit_core::watch::VecWatchEventSource::new(vec![
        orbit_core::watch::WatchEvent::new(watched_path.to_string_lossy().to_string(), 0),
        orbit_core::watch::WatchEvent::new(watched_path.to_string_lossy().to_string(), 100),
        orbit_core::watch::WatchEvent::new(watched_path.to_string_lossy().to_string(), 200),
        orbit_core::watch::WatchEvent::new(watched_path.to_string_lossy().to_string(), 800),
    ]);

    let runs = runtime
        .run_watch_with_source(&mut source, Some(4))
        .expect("watch run");

    assert_eq!(runs, 2, "debounce + queue-1 should yield exactly two runs");

    let output = std::fs::read_to_string(&output_file).expect("output file");
    assert_eq!(output.lines().count(), 2);
}
