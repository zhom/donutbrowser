use super::*;
use crate::proxy_storage::process_start_time;

#[cfg(unix)]
fn short_lived_child() -> Child {
  Command::new("sh")
    .args(["-c", "sleep 0.2"])
    .spawn()
    .unwrap()
}

#[cfg(windows)]
fn short_lived_child() -> Child {
  Command::new("cmd")
    .args(["/C", "ping -n 2 127.0.0.1 >NUL"])
    .spawn()
    .unwrap()
}

#[test]
fn supervisor_child_is_reaped_after_exit() {
  let child = short_lived_child();
  let pid = child.id();
  let start_time = resolve_process_start_time(pid).unwrap();
  spawn_supervisor_reaper(child).join().unwrap();
  assert!(!process_identity_matches(pid, Some(start_time)));
}

fn readiness_config(port: u16) -> XrayWorkerConfig {
  let mut config = XrayWorkerConfig::new(
    "readiness".to_string(),
    None,
    "vless://unused".to_string(),
    port,
    "local-user".to_string(),
    "local-password".to_string(),
  );
  let pid = std::process::id();
  config.xray_pid = Some(pid);
  config.xray_pid_start_time = process_start_time(pid);
  config
}

#[test]
fn parses_macos_major_versions_for_sidecar_compatibility() {
  assert_eq!(parse_macos_major_version("11.7.10\n"), Some(11));
  assert_eq!(parse_macos_major_version("12.0"), Some(12));
  assert_eq!(parse_macos_major_version("15.5.1"), Some(15));
  assert_eq!(parse_macos_major_version("unknown"), None);
}

#[tokio::test]
async fn readiness_requires_the_expected_authenticated_socks_endpoint() {
  let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
    .await
    .unwrap();
  let config = readiness_config(listener.local_addr().unwrap().port());
  let expected_username = config.username.clone();
  let expected_password = config.password.clone();
  let server = tokio::spawn(async move {
    let (mut stream, _) = listener.accept().await.unwrap();
    let mut greeting = [0_u8; 3];
    stream.read_exact(&mut greeting).await.unwrap();
    assert_eq!(greeting, [5, 1, 2]);
    stream.write_all(&[5, 2]).await.unwrap();

    let mut header = [0_u8; 2];
    stream.read_exact(&mut header).await.unwrap();
    assert_eq!(header[0], 1);
    let mut username = vec![0_u8; header[1] as usize];
    stream.read_exact(&mut username).await.unwrap();
    let password_len = stream.read_u8().await.unwrap();
    let mut password = vec![0_u8; password_len as usize];
    stream.read_exact(&mut password).await.unwrap();
    assert_eq!(username, expected_username.as_bytes());
    assert_eq!(password, expected_password.as_bytes());
    stream.write_all(&[1, 0]).await.unwrap();
  });

  assert!(authenticated_socks_ready(&config).await);
  server.await.unwrap();
}

#[tokio::test]
async fn readiness_rejects_an_unrelated_listener_on_the_reserved_port() {
  let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
    .await
    .unwrap();
  let config = readiness_config(listener.local_addr().unwrap().port());
  let server = tokio::spawn(async move {
    let (mut stream, _) = listener.accept().await.unwrap();
    let mut greeting = [0_u8; 3];
    stream.read_exact(&mut greeting).await.unwrap();
    stream.write_all(&[5, 0]).await.unwrap();
  });

  assert!(!authenticated_socks_ready(&config).await);
  server.await.unwrap();
}

#[test]
fn browser_identity_is_persisted_on_the_exact_worker() {
  let temp = tempfile::tempdir().unwrap();
  let _cache_guard = crate::app_dirs::set_test_cache_dir(temp.path().to_path_buf());
  let id = format!("xray-browser-owner-{}", uuid::Uuid::new_v4());
  let config = XrayWorkerConfig::new(
    id.clone(),
    Some("profile".to_string()),
    "vless://unused".to_string(),
    1080,
    "local-user".to_string(),
    "local-password".to_string(),
  );
  save_xray_worker_config(&config).unwrap();

  let browser_pid = std::process::id();
  assert!(set_browser_pid(&id, browser_pid));
  let saved = get_xray_worker_config(&id).unwrap();
  assert_eq!(saved.browser_pid, Some(browser_pid));
  assert_eq!(
    saved.browser_pid_start_time,
    process_start_time(browser_pid)
  );

  assert!(delete_xray_worker_config(&id));
}

#[test]
fn reusable_worker_requires_and_persists_the_exact_live_owner() {
  let temp = tempfile::tempdir().unwrap();
  let _cache_guard = crate::app_dirs::set_test_cache_dir(temp.path().to_path_buf());
  let id = format!("xray-worker-lease-{}", uuid::Uuid::new_v4());
  let mut config = XrayWorkerConfig::new(
    id.clone(),
    Some("profile".to_string()),
    "vless://unused".to_string(),
    1080,
    "local-user".to_string(),
    "local-password".to_string(),
  );
  config.browser_pid = Some(u32::MAX);
  config.browser_pid_start_time = Some(1);
  save_xray_worker_config(&config).unwrap();

  let owner_pid = std::process::id();
  let owner_start_time = process_start_time(owner_pid).unwrap();
  assert!(!worker_is_leased_to(&config, owner_pid, owner_start_time));
  assert!(persist_browser_identity(
    &mut config,
    owner_pid,
    owner_start_time
  ));
  assert!(worker_is_leased_to(&config, owner_pid, owner_start_time));
  let saved = get_xray_worker_config(&id).unwrap();
  assert_eq!(saved.browser_pid, Some(owner_pid));
  assert_eq!(saved.browser_pid_start_time, Some(owner_start_time));

  assert!(delete_xray_worker_config(&id));
}
