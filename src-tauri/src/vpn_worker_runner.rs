use crate::proxy_storage::is_process_running;
use crate::vpn_worker_storage::{
  delete_vpn_worker_config, find_vpn_worker_by_vpn_id, generate_vpn_worker_id,
  get_vpn_worker_config, list_vpn_worker_configs, save_vpn_worker_config, VpnWorkerConfig,
};
use std::process::Stdio;

pub async fn start_vpn_worker(vpn_id: &str) -> Result<VpnWorkerConfig, Box<dyn std::error::Error>> {
  // Check if a VPN worker for this vpn_id already exists and is running
  if let Some(existing) = find_vpn_worker_by_vpn_id(vpn_id) {
    if let Some(pid) = existing.pid {
      if is_process_running(pid) {
        return Ok(existing);
      }
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

  let vpn_type_str = match vpn_config.vpn_type {
    crate::vpn::VpnType::WireGuard => "wireguard",
    crate::vpn::VpnType::OpenVPN => "openvpn",
  };

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

  // Spawn detached VPN worker process
  let exe = std::env::current_exe()?;

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
    cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);

    let child = cmd.spawn()?;
    let pid = child.id();

    let mut config_with_pid = config.clone();
    config_with_pid.pid = Some(pid);
    config_with_pid.local_port = Some(local_port);
    save_vpn_worker_config(&config_with_pid)?;

    drop(child);
  }

  // Wait for the worker to update config with local_url
  tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

  let mut attempts = 0;
  let max_attempts = 100; // 10 seconds max

  loop {
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    if let Some(updated_config) = get_vpn_worker_config(&id) {
      if let Some(ref local_url) = updated_config.local_url {
        if !local_url.is_empty() {
          if let Some(port) = updated_config.local_port {
            if let Ok(Ok(_)) = tokio::time::timeout(
              tokio::time::Duration::from_millis(100),
              tokio::net::TcpStream::connect(("127.0.0.1", port)),
            )
            .await
            {
              return Ok(updated_config);
            }
          }
        }
      }
    }

    attempts += 1;
    if attempts >= max_attempts {
      if let Some(config) = get_vpn_worker_config(&id) {
        let process_running = config.pid.map(is_process_running).unwrap_or(false);
        // Clean up on failure
        delete_vpn_worker_config(&id);
        return Err(
          format!(
            "VPN worker failed to start in time. pid={:?}, process_running={}, local_url={:?}",
            config.pid, process_running, config.local_url
          )
          .into(),
        );
      }
      delete_vpn_worker_config(&id);
      return Err("VPN worker config not found after spawn".into());
    }
  }
}

pub async fn stop_vpn_worker(id: &str) -> Result<bool, Box<dyn std::error::Error>> {
  let config = get_vpn_worker_config(id);

  if let Some(config) = config {
    if let Some(pid) = config.pid {
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
        use std::process::Command;
        let _ = Command::new("taskkill")
          .args(["/F", "/PID", &pid.to_string()])
          .output();
      }

      tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
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
