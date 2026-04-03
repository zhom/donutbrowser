use crate::proxy_storage::{
  delete_proxy_config, generate_proxy_id, get_proxy_config, is_process_running, list_proxy_configs,
  save_proxy_config, ProxyConfig,
};
use std::path::{Path, PathBuf};
use std::process::Stdio;
lazy_static::lazy_static! {
  static ref PROXY_PROCESSES: std::sync::Mutex<std::collections::HashMap<String, u32>> =
    std::sync::Mutex::new(std::collections::HashMap::new());
}

fn target_binary_name(base_name: &str) -> Option<String> {
  let target = std::env::var("TARGET").ok()?;

  #[cfg(windows)]
  {
    Some(format!("{base_name}-{target}.exe"))
  }

  #[cfg(not(windows))]
  {
    Some(format!("{base_name}-{target}"))
  }
}

fn unsuffixed_binary_name(base_name: &str) -> String {
  #[cfg(windows)]
  {
    match base_name {
      "donut-proxy" => "donut-proxy.exe".to_string(),
      "donut-daemon" => "donut-daemon.exe".to_string(),
      _ => String::new(),
    }
  }

  #[cfg(not(windows))]
  {
    base_name.to_string()
  }
}

fn binary_matches_prefix(path: &Path, base_name: &str) -> bool {
  let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
    return false;
  };

  #[cfg(windows)]
  {
    file_name.starts_with(&format!("{base_name}-")) && file_name.ends_with(".exe")
  }

  #[cfg(not(windows))]
  {
    file_name.starts_with(&format!("{base_name}-"))
  }
}

fn push_candidate_dir(dirs: &mut Vec<PathBuf>, dir: Option<PathBuf>) {
  if let Some(dir) = dir {
    if !dirs.iter().any(|existing| existing == &dir) {
      dirs.push(dir);
    }
  }
}

pub(crate) fn find_sidecar_executable(
  base_name: &str,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
  let current_exe = std::env::current_exe()?;
  let current_dir = current_exe
    .parent()
    .ok_or("Failed to get parent directory of current executable")?;

  if current_exe
    .file_stem()
    .and_then(|stem| stem.to_str())
    .is_some_and(|stem| stem == base_name)
  {
    return Ok(current_exe);
  }

  let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
  let mut search_dirs = Vec::new();

  push_candidate_dir(&mut search_dirs, Some(current_dir.to_path_buf()));
  push_candidate_dir(
    &mut search_dirs,
    current_dir.parent().map(std::path::Path::to_path_buf),
  );
  push_candidate_dir(
    &mut search_dirs,
    current_dir
      .parent()
      .and_then(|parent| parent.parent())
      .map(Path::to_path_buf),
  );
  push_candidate_dir(&mut search_dirs, Some(current_dir.join("binaries")));
  push_candidate_dir(
    &mut search_dirs,
    current_dir.parent().map(|parent| parent.join("binaries")),
  );
  push_candidate_dir(
    &mut search_dirs,
    current_dir
      .parent()
      .and_then(|parent| parent.parent())
      .map(|parent| parent.join("binaries")),
  );
  push_candidate_dir(&mut search_dirs, Some(manifest_dir.join("binaries")));
  push_candidate_dir(
    &mut search_dirs,
    Some(manifest_dir.join("target").join("debug")),
  );
  push_candidate_dir(
    &mut search_dirs,
    Some(manifest_dir.join("target").join("release")),
  );

  let mut exact_names = vec![unsuffixed_binary_name(base_name)];
  if let Some(target_name) = target_binary_name(base_name) {
    exact_names.push(target_name);
  }

  for dir in &search_dirs {
    for name in &exact_names {
      if name.is_empty() {
        continue;
      }

      let candidate = dir.join(name);
      if candidate.exists() {
        return Ok(candidate);
      }
    }

    if let Ok(entries) = std::fs::read_dir(dir) {
      for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() && binary_matches_prefix(&path, base_name) {
          return Ok(path);
        }
      }
    }
  }

  Err(
    format!(
      "Failed to locate '{}' executable. Searched in: {}",
      base_name,
      search_dirs
        .iter()
        .map(|dir| dir.display().to_string())
        .collect::<Vec<_>>()
        .join(", ")
    )
    .into(),
  )
}

pub async fn start_proxy_process(
  upstream_url: Option<String>,
  port: Option<u16>,
) -> Result<ProxyConfig, Box<dyn std::error::Error>> {
  start_proxy_process_with_profile(upstream_url, port, None, Vec::new(), None).await
}

pub async fn start_proxy_process_with_profile(
  upstream_url: Option<String>,
  port: Option<u16>,
  profile_id: Option<String>,
  bypass_rules: Vec<String>,
  blocklist_file: Option<String>,
) -> Result<ProxyConfig, Box<dyn std::error::Error>> {
  let id = generate_proxy_id();
  let upstream = upstream_url.unwrap_or_else(|| "DIRECT".to_string());

  // Get available port if not specified
  let local_port = port.unwrap_or_else(|| {
    // Find an available port
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().port()
  });

  let config = ProxyConfig::new(id.clone(), upstream, Some(local_port))
    .with_profile_id(profile_id.clone())
    .with_bypass_rules(bypass_rules)
    .with_blocklist_file(blocklist_file);
  save_proxy_config(&config)?;

  // Log profile_id for debugging
  if let Some(ref pid) = profile_id {
    log::info!("Saved proxy config {} with profile_id: {}", id, pid);
  } else {
    log::info!("Saved proxy config {} without profile_id", id);
  }

  // Spawn proxy worker process in the background using std::process::Command
  // This ensures proper process detachment on Unix systems
  let exe = find_sidecar_executable("donut-proxy")?;

  #[cfg(unix)]
  {
    use std::os::unix::process::CommandExt;
    use std::process::Command as StdCommand;

    let mut cmd = StdCommand::new(&exe);
    cmd.arg("proxy-worker");
    cmd.arg("start");
    cmd.arg("--id");
    cmd.arg(&id);

    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::null());

    // Always log to file for diagnostics (both debug and release builds)
    let log_path = std::env::temp_dir().join(format!("donut-proxy-{}.log", id));
    if let Ok(file) = std::fs::File::create(&log_path) {
      log::info!("Proxy worker stderr will be logged to: {:?}", log_path);
      cmd.stderr(Stdio::from(file));
    } else {
      cmd.stderr(Stdio::null());
    }

    // Properly detach the process on Unix by creating a new session
    unsafe {
      cmd.pre_exec(|| {
        // Create a new process group so the process survives parent exit
        libc::setsid();

        // Set high priority so the proxy is killed last under resource pressure
        // Negative nice value = higher priority. Try -10, fall back to -5 if it fails.
        if libc::setpriority(libc::PRIO_PROCESS, 0, -10) != 0 {
          let _ = libc::setpriority(libc::PRIO_PROCESS, 0, -5);
        }

        Ok(())
      });
    }

    // Spawn detached process
    let child = cmd.spawn()?;
    let pid = child.id();

    // Store PID
    {
      let mut processes = PROXY_PROCESSES.lock().unwrap();
      processes.insert(id.clone(), pid);
    }

    // Update config with PID
    let mut config_with_pid = config.clone();
    config_with_pid.pid = Some(pid);
    save_proxy_config(&config_with_pid)?;

    // Don't wait for the child - it's detached
    drop(child);
  }

  #[cfg(windows)]
  {
    use std::os::windows::io::AsRawHandle;
    use std::os::windows::process::CommandExt;
    use std::process::Command as StdCommand;
    use windows::Win32::Foundation::{CloseHandle, SetHandleInformation, HANDLE, HANDLE_FLAGS};
    use windows::Win32::System::Threading::{
      OpenProcess, SetPriorityClass, ABOVE_NORMAL_PRIORITY_CLASS, PROCESS_SET_INFORMATION,
    };

    // Mark current stdout/stderr as non-inheritable so the spawned worker process
    // does not inherit pipe handles from our parent (prevents blocking when parent exits).
    let stdout_handle = std::io::stdout().as_raw_handle();
    let stderr_handle = std::io::stderr().as_raw_handle();
    const HANDLE_FLAG_INHERIT: u32 = 0x00000001;
    unsafe {
      if !stdout_handle.is_null() {
        let _ = SetHandleInformation(HANDLE(stdout_handle), HANDLE_FLAG_INHERIT, HANDLE_FLAGS(0));
      }
      if !stderr_handle.is_null() {
        let _ = SetHandleInformation(HANDLE(stderr_handle), HANDLE_FLAG_INHERIT, HANDLE_FLAGS(0));
      }
    }

    let mut cmd = StdCommand::new(&exe);
    cmd.arg("proxy-worker");
    cmd.arg("start");
    cmd.arg("--id");
    cmd.arg(&id);

    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::null());

    // Log to file for diagnostics (matching Unix behavior)
    let log_path = std::env::temp_dir().join(format!("donut-proxy-{}.log", id));
    if let Ok(file) = std::fs::File::create(&log_path) {
      log::info!("Proxy worker stderr will be logged to: {:?}", log_path);
      cmd.stderr(Stdio::from(file));
    } else {
      cmd.stderr(Stdio::null());
    }

    // On Windows, use DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP for proper detachment.
    const DETACHED_PROCESS: u32 = 0x00000008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x00000200;
    cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);

    let child = cmd.spawn()?;
    let pid = child.id();

    // Set high priority so the proxy is killed last under resource pressure
    unsafe {
      if let Ok(handle) = OpenProcess(PROCESS_SET_INFORMATION, false, pid) {
        let _ = SetPriorityClass(handle, ABOVE_NORMAL_PRIORITY_CLASS);
        let _ = CloseHandle(handle);
      }
    }

    // Store PID
    {
      let mut processes = PROXY_PROCESSES.lock().unwrap();
      processes.insert(id.clone(), pid);
    }

    // Update config with PID
    let mut config_with_pid = config.clone();
    config_with_pid.pid = Some(pid);
    save_proxy_config(&config_with_pid)?;

    drop(child);
  }

  // Give the process a moment to start up before checking
  tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

  // Wait for the worker to bind to the port and update config
  // Since we pre-allocated the port, the worker should bind immediately
  // We check quickly with short intervals to make startup fast
  let mut attempts = 0;
  let max_attempts = 40; // 4 seconds max (40 * 100ms) - give it more time to start

  loop {
    // Use shorter sleep for faster startup
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    if let Some(updated_config) = get_proxy_config(&id) {
      // Check if local_url is set (worker has bound and updated config)
      if let Some(ref local_url) = updated_config.local_url {
        if !local_url.is_empty() {
          if let Some(port) = updated_config.local_port {
            // Try to connect immediately - port should be ready since we pre-allocated it
            match tokio::time::timeout(
              tokio::time::Duration::from_millis(100),
              tokio::net::TcpStream::connect(("127.0.0.1", port)),
            )
            .await
            {
              Ok(Ok(_stream)) => {
                // Port is listening and accepting connections!
                return Ok(updated_config);
              }
              Ok(Err(_)) | Err(_) => {
                // Port not ready yet, continue waiting
              }
            }
          }
        }
      }
    }

    attempts += 1;
    if attempts >= max_attempts {
      // Try to get the config one more time for better error message
      if let Some(config) = get_proxy_config(&id) {
        // Check if process is still running
        let process_running = config.pid.map(is_process_running).unwrap_or(false);
        return Err(
          format!(
            "Proxy worker failed to start in time. Config: id={}, local_url={:?}, local_port={:?}, pid={:?}, process_running={}",
            config.id, config.local_url, config.local_port, config.pid, process_running
          )
          .into(),
        );
      }
      return Err(
        format!(
          "Proxy worker failed to start in time. Config not found for id: {}",
          id
        )
        .into(),
      );
    }
  }
}

pub async fn stop_proxy_process(id: &str) -> Result<bool, Box<dyn std::error::Error>> {
  let config = get_proxy_config(id);

  if let Some(config) = config {
    if let Some(pid) = config.pid {
      // Kill the process
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

      // Wait a bit for the process to exit
      tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

      // Remove from tracking
      {
        let mut processes = PROXY_PROCESSES.lock().unwrap();
        processes.remove(id);
      }

      // Delete the config file
      delete_proxy_config(id);
      return Ok(true);
    }
  }

  Ok(false)
}

pub async fn stop_all_proxy_processes() -> Result<(), Box<dyn std::error::Error>> {
  let configs = list_proxy_configs();
  for config in configs {
    let _ = stop_proxy_process(&config.id).await;
  }
  Ok(())
}
