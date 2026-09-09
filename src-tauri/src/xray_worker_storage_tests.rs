use super::*;

fn test_config(id: &str) -> XrayWorkerConfig {
  XrayWorkerConfig::new(
    id.to_string(),
    Some("profile".to_string()),
    "vless://example".to_string(),
    1080,
    "local-user".to_string(),
    "local-password".to_string(),
  )
}

#[test]
fn local_proxy_settings_use_authenticated_loopback_socks() {
  let config = test_config("id");

  let proxy = config.local_proxy_settings();
  assert_eq!(proxy.proxy_type, "socks5");
  assert_eq!(proxy.host, "127.0.0.1");
  assert_eq!(proxy.port, 1080);
  assert_eq!(proxy.username.as_deref(), Some("local-user"));
  assert_eq!(proxy.password.as_deref(), Some("local-password"));
  assert!(proxy.vless_uri.is_none());
}

#[test]
fn worker_storage_round_trips_updates_lists_and_securely_cleans_runtime_files() {
  let temp = tempfile::tempdir().unwrap();
  let _cache_guard = crate::app_dirs::set_test_cache_dir(temp.path().to_path_buf());
  let id = format!("xray-storage-test-{}", uuid::Uuid::new_v4());
  let mut config = test_config(&id);

  save_xray_worker_config(&config).unwrap();
  assert_eq!(get_xray_worker_config(&id).unwrap().username, "local-user");
  assert_eq!(
    find_xray_worker_by_profile_id("profile").unwrap().id,
    config.id
  );
  assert!(list_xray_worker_configs()
    .iter()
    .any(|candidate| candidate.id == id));

  config.pid = Some(41);
  config.xray_pid = Some(42);
  config.browser_pid = Some(43);
  assert!(update_xray_worker_config(&config));
  let updated = get_xray_worker_config(&id).unwrap();
  assert_eq!(updated.pid, Some(41));
  assert_eq!(updated.xray_pid, Some(42));
  assert_eq!(updated.browser_pid, Some(43));

  let runtime_path = xray_runtime_config_path(&id);
  write_xray_runtime_config(&id, b"{\"runtime\":true}").unwrap();
  let log_path = xray_worker_log_path(&id);
  drop(create_xray_worker_log(&id).unwrap());

  #[cfg(unix)]
  {
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(
      std::fs::metadata(crate::proxy_storage::get_storage_dir())
        .unwrap()
        .permissions()
        .mode()
        & 0o777,
      0o700
    );
    assert_eq!(
      std::fs::metadata(xray_worker_config_path(&id))
        .unwrap()
        .permissions()
        .mode()
        & 0o777,
      0o600
    );
    assert_eq!(
      std::fs::metadata(&runtime_path)
        .unwrap()
        .permissions()
        .mode()
        & 0o777,
      0o600
    );
    assert_eq!(
      std::fs::metadata(&log_path).unwrap().permissions().mode() & 0o777,
      0o600
    );
  }

  assert!(delete_xray_worker_config(&id));
  assert!(get_xray_worker_config(&id).is_none());
  assert!(!runtime_path.exists());
  assert!(!log_path.exists());
  assert!(!update_xray_worker_config(&config));
  assert!(write_xray_runtime_config(&id, b"{}").is_err());
  assert!(create_xray_worker_log(&id).is_err());
}

#[test]
fn fresh_unstarted_workers_have_a_grace_period_but_legacy_entries_are_stale() {
  let fresh = test_config("fresh");
  assert!(!unstarted_worker_is_stale(&fresh));

  let mut legacy = test_config("legacy");
  legacy.created_at = 0;
  assert!(unstarted_worker_is_stale(&legacy));

  legacy.pid = Some(1);
  assert!(!unstarted_worker_is_stale(&legacy));
}

#[test]
fn atomic_state_updates_never_expose_partial_json() {
  let temp = tempfile::tempdir().unwrap();
  let path = temp.path().join("state.json");
  atomic_write_owner_only(&path, br#"{"value":0}"#).unwrap();
  let writer_path = path.clone();
  let writer = std::thread::spawn(move || {
    for value in 1..=500 {
      let content = serde_json::to_vec(&serde_json::json!({ "value": value })).unwrap();
      atomic_write_owner_only(&writer_path, &content).unwrap();
    }
  });

  // Bound the reader on the writer's own lifetime. A completion flag the
  // writer sets last is never set when it panics, which strands this loop
  // reading the last good file forever instead of failing.
  while !writer.is_finished() {
    let content = read_worker_state(&path).expect("state file stays readable while replaced");
    let value: serde_json::Value = serde_json::from_slice(&content).unwrap();
    assert!(value["value"].is_number());
    std::thread::yield_now();
  }
  writer.join().unwrap();
}

#[cfg(unix)]
#[test]
fn atomic_state_write_replaces_a_symlink_without_touching_its_target() {
  use std::os::unix::fs::symlink;

  let temp = tempfile::tempdir().unwrap();
  let victim = temp.path().join("victim");
  let state = temp.path().join("state.json");
  std::fs::write(&victim, "untouched").unwrap();
  symlink(&victim, &state).unwrap();

  atomic_write_owner_only(&state, br#"{"safe":true}"#).unwrap();

  assert_eq!(std::fs::read_to_string(victim).unwrap(), "untouched");
  assert_eq!(
    serde_json::from_slice::<serde_json::Value>(&std::fs::read(state).unwrap()).unwrap()["safe"],
    true
  );
}

#[test]
fn legacy_worker_config_defaults_missing_browser_pid() {
  let value = serde_json::json!({
    "id": "legacy",
    "profile_id": "profile",
    "vless_uri": "vless://example",
    "local_port": 1080,
    "username": "user",
    "password": "password",
    "pid": 1,
    "xray_pid": 2
  });
  let config: XrayWorkerConfig = serde_json::from_value(value).unwrap();
  assert_eq!(config.created_at, 0);
  assert_eq!(config.pid_start_time, None);
  assert_eq!(config.xray_pid_start_time, None);
  assert!(!config.ready);
  assert_eq!(config.browser_pid, None);
  assert_eq!(config.browser_pid_start_time, None);
}
