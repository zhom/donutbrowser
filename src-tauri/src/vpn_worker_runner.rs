use crate::proxy_runner::find_sidecar_executable;
use crate::proxy_storage::is_process_running;
use crate::vpn_worker_storage::{
  delete_vpn_worker_config, find_vpn_worker_by_vpn_id, generate_vpn_worker_id,
  get_vpn_worker_config, list_vpn_worker_configs, save_vpn_worker_config, vpn_worker_config_path,
  VpnWorkerConfig,
};
use std::collections::HashMap;
use std::process::Stdio;
use std::sync::{LazyLock, Mutex};

const VPN_WORKER_POLL_INTERVAL_MS: u64 = 100;
const VPN_WORKER_STARTUP_TIMEOUT_MS: u64 = 30_000;

async fn vpn_worker_accepting_connections(config: &VpnWorkerConfig) -> bool {
  let Some(port) = config.local_port else {
    return false;
  };

  if config
    .local_url
    .as_ref()
    .is_none_or(|local_url| local_url.is_empty())
  {
    return false;
  }

  matches!(
    tokio::time::timeout(
      tokio::time::Duration::from_millis(VPN_WORKER_POLL_INTERVAL_MS),
      tokio::net::TcpStream::connect(("127.0.0.1", port)),
    )
    .await,
    Ok(Ok(_))
  )
}

/// Is this worker's recorded process still the same live process?
///
/// Identity-checked whenever a start time was recorded, so a PID the OS has
/// since recycled reads as dead instead of as a live tunnel. Configs written
/// before `pid_start_time` existed fall back to a bare existence check, so the
/// first run after an upgrade does not declare every surviving worker dead.
/// Mirrors `proxy_storage::browser_owner_is_alive`.
pub fn vpn_worker_alive(config: &VpnWorkerConfig) -> bool {
  let Some(pid) = config.pid else {
    return false;
  };
  match config.pid_start_time {
    Some(start_time) => crate::proxy_storage::process_identity_matches(pid, Some(start_time)),
    None => is_process_running(pid),
  }
}

fn worker_log_path(id: &str) -> std::path::PathBuf {
  std::env::temp_dir().join(format!("donut-vpn-{}.log", id))
}

fn read_worker_log(id: &str) -> String {
  std::fs::read_to_string(worker_log_path(id)).unwrap_or_else(|_| "No log available".to_string())
}

async fn wait_for_vpn_worker_ready(
  id: &str,
) -> Result<VpnWorkerConfig, Box<dyn std::error::Error>> {
  let startup_timeout = tokio::time::Duration::from_millis(VPN_WORKER_STARTUP_TIMEOUT_MS);
  let startup_deadline = tokio::time::Instant::now() + startup_timeout;

  tokio::time::sleep(tokio::time::Duration::from_millis(
    VPN_WORKER_POLL_INTERVAL_MS,
  ))
  .await;

  let mut attempts = 0u32;

  loop {
    tokio::time::sleep(tokio::time::Duration::from_millis(
      VPN_WORKER_POLL_INTERVAL_MS,
    ))
    .await;

    if let Some(updated_config) = get_vpn_worker_config(id) {
      let process_running = vpn_worker_alive(&updated_config);

      if !process_running && attempts > 2 {
        let log_output = read_worker_log(id);
        delete_vpn_worker_config(id);
        return Err(format!("VPN worker process crashed. Log output:\n{}", log_output).into());
      }

      if vpn_worker_accepting_connections(&updated_config).await {
        return Ok(updated_config);
      }
    }

    attempts += 1;
    if tokio::time::Instant::now() >= startup_deadline {
      if let Some(config) = get_vpn_worker_config(id) {
        let process_running = vpn_worker_alive(&config);
        let log_output = read_worker_log(id);
        delete_vpn_worker_config(id);
        return Err(
          format!(
            "VPN worker failed to start within {:.1}s. pid={:?}, process_running={}, local_url={:?}\n\nVPN worker log:\n{}",
            startup_timeout.as_secs_f32(),
            config.pid,
            process_running,
            config.local_url,
            log_output
          )
          .into(),
        );
      }

      delete_vpn_worker_config(id);
      return Err("VPN worker config not found after spawn".into());
    }
  }
}

/// Serializes worker startup for a given VPN, so two concurrent launches cannot
/// both observe "no worker" and both believe they created it. Benign until a
/// launch guard may stop one on failure; then double-ownership means a
/// cancelled launch tears down a tunnel another profile is using. Mirrors
/// `xray_worker_runner::XRAY_START_LOCK`.
static VPN_START_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// How many in-flight launches currently hold a worker for each vpn_id.
///
/// A launch is invisible to `vpn_id_in_use_by_running_browser` until its
/// browser PID is persisted, which happens seconds after the worker is adopted:
/// past the fingerprint gate, the local proxy worker, the decrypted profile
/// copy and the browser spawn. Without this, a sibling launch failing inside
/// that window stopped the shared worker out from under the adopter.
static VPN_LAUNCH_CLAIMS: LazyLock<Mutex<HashMap<String, usize>>> =
  LazyLock::new(|| Mutex::new(HashMap::new()));

/// The critical section is a map bump that cannot panic, so a poisoned lock
/// carries no torn state worth refusing.
fn launch_claims() -> std::sync::MutexGuard<'static, HashMap<String, usize>> {
  VPN_LAUNCH_CLAIMS
    .lock()
    .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// One launch's hold on a VPN worker, taken while `VPN_START_LOCK` is held and
/// released only when the launch scope ends. Strictly RAII: nothing increments
/// the count outside `start_vpn_worker_tracked`, so a panicking launch cannot
/// pin a worker up for good.
pub struct VpnLaunchClaim {
  vpn_id: String,
}

impl VpnLaunchClaim {
  fn take(vpn_id: &str) -> Self {
    *launch_claims().entry(vpn_id.to_string()).or_insert(0) += 1;
    Self {
      vpn_id: vpn_id.to_string(),
    }
  }
}

impl Drop for VpnLaunchClaim {
  fn drop(&mut self) {
    let mut claims = launch_claims();
    if let Some(count) = claims.get_mut(&self.vpn_id) {
      *count = count.saturating_sub(1);
      if *count == 0 {
        claims.remove(&self.vpn_id);
      }
    }
  }
}

fn vpn_id_is_claimed_by_launch(vpn_id: &str) -> bool {
  launch_claims().get(vpn_id).is_some_and(|count| *count > 0)
}

/// A started VPN worker plus whether *this* call spawned it.
pub struct VpnWorkerStart {
  pub config: VpnWorkerConfig,
  /// False when an already-running worker was adopted. Only the creator may
  /// stop it while unwinding a failed launch.
  pub created: bool,
  /// Held for the rest of the launch, so a sibling launch failing before this
  /// one publishes its browser PID cannot stop the worker underneath it.
  pub claim: VpnLaunchClaim,
}

/// Whether any profile with a live browser process is routing through this VPN.
///
/// Extracted from the startup sweep so the launch guard and the sweep agree on
/// what "in use" means instead of each carrying its own copy.
pub fn vpn_id_in_use_by_running_browser(vpn_id: &str) -> bool {
  // A launch that has taken the worker but has not yet persisted its browser
  // PID is invisible to the profile scan below, so consult the claims first.
  if vpn_id_is_claimed_by_launch(vpn_id) {
    return true;
  }
  let Ok(profiles) = crate::profile::ProfileManager::instance().list_profiles() else {
    // Unable to tell — assume in use rather than tear down a live tunnel.
    return true;
  };
  profiles
    .iter()
    .filter(|p| p.process_id.is_some_and(is_process_running))
    .any(|p| p.vpn_id.as_deref() == Some(vpn_id))
}

/// Hold the start lock across an adopt-sensitive section (a launch guard
/// deciding whether to stop a worker it created).
pub async fn lock_vpn_starts() -> tokio::sync::MutexGuard<'static, ()> {
  VPN_START_LOCK.lock().await
}

pub async fn start_vpn_worker(vpn_id: &str) -> Result<VpnWorkerConfig, Box<dyn std::error::Error>> {
  start_vpn_worker_tracked(vpn_id).await.map(|s| s.config)
}

pub async fn start_vpn_worker_tracked(
  vpn_id: &str,
) -> Result<VpnWorkerStart, Box<dyn std::error::Error>> {
  let _start_guard = VPN_START_LOCK.lock().await;
  crate::proxy_runner::ensure_sidecar_version().await?;

  for config in list_vpn_worker_configs() {
    if !vpn_worker_alive(&config) {
      delete_vpn_worker_config(&config.id);
    }
  }

  // Check if a VPN worker for this vpn_id already exists and is running
  if let Some(existing) = find_vpn_worker_by_vpn_id(vpn_id) {
    if vpn_worker_alive(&existing) {
      if vpn_worker_accepting_connections(&existing).await {
        return Ok(VpnWorkerStart {
          config: existing,
          created: false,
          claim: VpnLaunchClaim::take(vpn_id),
        });
      }

      return wait_for_vpn_worker_ready(&existing.id)
        .await
        .map(|config| VpnWorkerStart {
          config,
          created: false,
          claim: VpnLaunchClaim::take(vpn_id),
        });
    }
    // Worker config exists but process is dead, clean up
    delete_vpn_worker_config(&existing.id);
  }

  // Load VPN config from storage to determine type
  let vpn_config = {
    let storage = crate::vpn::VPN_STORAGE
      .lock()
      .map_err(|e| format!("Failed to lock VPN storage: {e}"))?;
    storage
      .load_config(vpn_id)
      .map_err(|e| format!("Failed to load VPN config: {e}"))?
  };

  let vpn_type_str = "wireguard";

  // Write decrypted config to a temp file
  let config_file_path = std::env::temp_dir()
    .join(format!("donut_vpn_{}.conf", vpn_id))
    .to_string_lossy()
    .to_string();

  std::fs::write(&config_file_path, &vpn_config.config_data)?;

  #[cfg(unix)]
  {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(&config_file_path, std::fs::Permissions::from_mode(0o600));
  }

  let id = generate_vpn_worker_id();

  // Find an available port
  let local_port = {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    listener.local_addr()?.port()
  };

  let config = VpnWorkerConfig::new(
    id.clone(),
    vpn_id.to_string(),
    vpn_type_str.to_string(),
    config_file_path,
  );
  save_vpn_worker_config(&config)?;

  let config_json_path = vpn_worker_config_path(&id);

  // Spawn detached VPN worker process
  let exe = find_sidecar_executable("donut-proxy")?;

  #[cfg(unix)]
  {
    use std::os::unix::process::CommandExt;
    use std::process::Command as StdCommand;

    let mut cmd = StdCommand::new(&exe);
    cmd.arg("vpn-worker");
    cmd.arg("start");
    cmd.arg("--id");
    cmd.arg(&id);
    cmd.arg("--port");
    cmd.arg(local_port.to_string());
    cmd.arg("--config-path");
    cmd.arg(&config_json_path);

    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::null());

    let log_path = std::env::temp_dir().join(format!("donut-vpn-{}.log", id));
    if let Ok(file) = std::fs::File::create(&log_path) {
      log::info!("VPN worker stderr will be logged to: {:?}", log_path);
      cmd.stderr(Stdio::from(file));
    } else {
      cmd.stderr(Stdio::null());
    }

    unsafe {
      cmd.pre_exec(|| {
        libc::setsid();
        if libc::setpriority(libc::PRIO_PROCESS, 0, -10) != 0 {
          let _ = libc::setpriority(libc::PRIO_PROCESS, 0, -5);
        }
        Ok(())
      });
    }

    let child = cmd.spawn()?;
    let pid = child.id();

    let mut config_with_pid = config.clone();
    config_with_pid.pid = Some(pid);
    config_with_pid.pid_start_time = crate::proxy_storage::resolve_process_start_time(pid);
    config_with_pid.local_port = Some(local_port);
    save_vpn_worker_config(&config_with_pid)?;

    drop(child);
  }

  #[cfg(windows)]
  {
    use std::os::windows::process::CommandExt;
    use std::process::Command as StdCommand;

    let mut cmd = StdCommand::new(&exe);
    cmd.arg("vpn-worker");
    cmd.arg("start");
    cmd.arg("--id");
    cmd.arg(&id);
    cmd.arg("--port");
    cmd.arg(local_port.to_string());
    cmd.arg("--config-path");
    cmd.arg(&config_json_path);

    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::null());

    let log_path = std::env::temp_dir().join(format!("donut-vpn-{}.log", id));
    if let Ok(file) = std::fs::File::create(&log_path) {
      log::info!("VPN worker stderr will be logged to: {:?}", log_path);
      cmd.stderr(Stdio::from(file));
    } else {
      cmd.stderr(Stdio::null());
    }

    const DETACHED_PROCESS: u32 = 0x00000008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x00000200;
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW);

    let child = cmd.spawn()?;
    let pid = child.id();

    let mut config_with_pid = config.clone();
    config_with_pid.pid = Some(pid);
    config_with_pid.pid_start_time = crate::proxy_storage::resolve_process_start_time(pid);
    config_with_pid.local_port = Some(local_port);
    save_vpn_worker_config(&config_with_pid)?;

    drop(child);
  }

  wait_for_vpn_worker_ready(&id)
    .await
    .map(|config| VpnWorkerStart {
      config,
      created: true,
      claim: VpnLaunchClaim::take(vpn_id),
    })
}

pub async fn stop_vpn_worker(id: &str) -> Result<bool, Box<dyn std::error::Error>> {
  let config = get_vpn_worker_config(id);

  if let Some(config) = config {
    if let Some(pid) = config.pid {
      // Only a PID still pinned to the process this record was written for is
      // ours to signal. A record with no start time predates the pinning and
      // came from an earlier app run, so its PID cannot be verified either.
      if crate::proxy_storage::process_identity_matches(pid, config.pid_start_time) {
        #[cfg(unix)]
        {
          use std::process::Command;
          let _ = Command::new("kill")
            .arg("-TERM")
            .arg(pid.to_string())
            .output();
        }
        #[cfg(windows)]
        {
          use std::os::windows::process::CommandExt;
          use std::process::Command;
          const CREATE_NO_WINDOW: u32 = 0x08000000;
          let _ = Command::new("taskkill")
            .args(["/F", "/PID", &pid.to_string()])
            .creation_flags(CREATE_NO_WINDOW)
            .output();
        }

        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
      } else if is_process_running(pid) {
        // Whatever holds the PID now is either an unrelated process or an
        // unverifiable pre-upgrade worker; the record is forgotten instead. A
        // real worker in the second case lingers until reboot, which beats
        // terminating a stranger.
        log::warn!(
          "Not signalling VPN worker {id}: PID {pid} cannot be pinned to the recorded process (start time {:?}); forgetting the record",
          config.pid_start_time
        );
      }
    }

    // Clean up temp config file
    let _ = std::fs::remove_file(&config.config_file_path);

    delete_vpn_worker_config(id);
    return Ok(true);
  }

  Ok(false)
}

pub async fn stop_vpn_worker_by_vpn_id(vpn_id: &str) -> Result<bool, Box<dyn std::error::Error>> {
  if let Some(config) = find_vpn_worker_by_vpn_id(vpn_id) {
    return stop_vpn_worker(&config.id).await;
  }
  Ok(false)
}

pub async fn stop_all_vpn_workers() -> Result<(), Box<dyn std::error::Error>> {
  let configs = list_vpn_worker_configs();
  for config in configs {
    let _ = stop_vpn_worker(&config.id).await;
  }
  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;

  fn worker(pid: Option<u32>, pid_start_time: Option<u64>) -> VpnWorkerConfig {
    VpnWorkerConfig {
      id: "vpnw_test".to_string(),
      vpn_id: "vpn_test".to_string(),
      vpn_type: "wireguard".to_string(),
      config_file_path: String::new(),
      local_port: None,
      local_url: None,
      pid,
      pid_start_time,
    }
  }

  #[test]
  fn a_recycled_pid_does_not_read_as_a_live_worker() {
    let pid = std::process::id();
    let start_time =
      crate::proxy_storage::process_start_time(pid).expect("current process should be visible");

    assert!(vpn_worker_alive(&worker(Some(pid), Some(start_time))));

    // The same PID with a start time it cannot have: the worker that recorded
    // it is gone and the OS handed its PID to something else.
    assert!(!vpn_worker_alive(&worker(
      Some(pid),
      Some(start_time.saturating_add(1))
    )));

    // Written before the field existed, so bare existence is all the
    // information the record carries. Upgrading must not reap live workers.
    assert!(vpn_worker_alive(&worker(Some(pid), None)));

    assert!(!vpn_worker_alive(&worker(None, None)));
    assert!(!vpn_worker_alive(&worker(None, Some(start_time))));
  }

  #[test]
  fn a_launch_claim_covers_the_worker_until_every_launch_ends() {
    // A vpn_id private to this test, so a parallel test's claims are neither
    // observed here nor disturbed by it.
    let vpn_id = format!("vpn_claim_test_{}", rand::random::<u32>());

    assert!(!vpn_id_is_claimed_by_launch(&vpn_id));

    let creator = VpnLaunchClaim::take(&vpn_id);
    assert!(vpn_id_is_claimed_by_launch(&vpn_id));

    // An adopter joins, then the creator's launch fails: the worker is still
    // covered, which is what stops the creator's guard tearing it down.
    let adopter = VpnLaunchClaim::take(&vpn_id);
    drop(creator);
    assert!(vpn_id_is_claimed_by_launch(&vpn_id));

    // With no launch left holding it, nothing keeps the worker up. A creator
    // whose launch fails alone must still be able to stop what it started.
    drop(adopter);
    assert!(!vpn_id_is_claimed_by_launch(&vpn_id));
  }
}
