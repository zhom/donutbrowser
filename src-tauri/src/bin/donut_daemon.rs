// Donut Browser Daemon - Background process for tray icon and services
// This runs independently of the main Tauri GUI

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use muda::MenuEvent;
use serde::{Deserialize, Serialize};
use tao::event::{Event, StartCause};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tokio::runtime::Runtime;
use tray_icon::TrayIcon;

use donutbrowser_lib::daemon::{autostart, services, tray};

static SHOULD_QUIT: AtomicBool = AtomicBool::new(false);

enum ServiceStatus {
  Ready {
    api_port: Option<u16>,
    mcp_running: bool,
  },
  Failed(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct DaemonState {
  daemon_pid: Option<u32>,
  api_port: Option<u16>,
  mcp_running: bool,
  version: String,
}

fn get_state_path() -> PathBuf {
  autostart::get_data_dir()
    .unwrap_or_else(|| PathBuf::from("."))
    .join("daemon-state.json")
}

fn ensure_data_dir() -> std::io::Result<()> {
  if let Some(data_dir) = autostart::get_data_dir() {
    fs::create_dir_all(&data_dir)?;
  }
  Ok(())
}

fn read_state() -> DaemonState {
  let path = get_state_path();
  if path.exists() {
    if let Ok(content) = fs::read_to_string(&path) {
      if let Ok(state) = serde_json::from_str(&content) {
        return state;
      }
    }
  }
  DaemonState::default()
}

fn write_state(state: &DaemonState) -> std::io::Result<()> {
  let path = get_state_path();
  let content = serde_json::to_string_pretty(state)?;
  fs::write(path, content)
}

fn set_high_priority() {
  #[cfg(unix)]
  {
    // Set high priority so the daemon is killed last under resource pressure
    // Negative nice value = higher priority. Try -10, fall back to -5 if it fails.
    unsafe {
      if libc::setpriority(libc::PRIO_PROCESS, 0, -10) != 0 {
        let _ = libc::setpriority(libc::PRIO_PROCESS, 0, -5);
      }
    }
  }

  #[cfg(windows)]
  {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{
      GetCurrentProcess, SetPriorityClass, ABOVE_NORMAL_PRIORITY_CLASS,
    };

    // Set high priority so the daemon is killed last under resource pressure
    unsafe {
      let handle = GetCurrentProcess();
      let _ = SetPriorityClass(handle, ABOVE_NORMAL_PRIORITY_CLASS);
      // GetCurrentProcess returns a pseudo-handle that doesn't need to be closed,
      // but we do it anyway for consistency
      let _ = CloseHandle(handle);
    }
  }
}

fn run_daemon() {
  // Set high priority so the daemon is less likely to be killed under resource pressure
  set_high_priority();

  // Initialize logging to file for debugging (since stdout/stderr may be redirected)
  let log_path = autostart::get_data_dir()
    .unwrap_or_else(|| std::path::PathBuf::from("."))
    .join("daemon.log");

  let log_file = std::fs::OpenOptions::new()
    .create(true)
    .append(true)
    .open(&log_path);

  env_logger::Builder::from_default_env()
    .filter_level(log::LevelFilter::Info)
    .format_timestamp_millis()
    .target(if let Ok(file) = log_file {
      env_logger::Target::Pipe(Box::new(file))
    } else {
      env_logger::Target::Stderr
    })
    .init();

  if let Err(e) = ensure_data_dir() {
    eprintln!("Failed to create data directory: {}", e);
    process::exit(1);
  }

  log::info!("[daemon] Starting with PID {}", process::id());

  // Create tokio runtime for async operations
  let rt = Runtime::new().expect("Failed to create tokio runtime");

  // Create channel for service status updates
  let (tx, rx) = mpsc::channel::<ServiceStatus>();

  // Spawn services in a background thread so we don't block the event loop
  let rt_handle = rt.handle().clone();
  std::thread::spawn(move || {
    let result = rt_handle.block_on(async { services::DaemonServices::start().await });
    let status = match result {
      Ok(s) => ServiceStatus::Ready {
        api_port: s.api_port,
        mcp_running: s.mcp_running,
      },
      Err(e) => ServiceStatus::Failed(e),
    };
    let _ = tx.send(status);
  });

  // Write initial state (services still starting)
  let state = DaemonState {
    daemon_pid: Some(process::id()),
    api_port: None,
    mcp_running: false,
    version: env!("CARGO_PKG_VERSION").to_string(),
  };
  if let Err(e) = write_state(&state) {
    log::error!("Failed to write state: {}", e);
  }

  // Prepare tray menu and icon (but don't create the tray icon yet)
  // Show "Starting..." state initially
  let tray_menu = tray::TrayMenu::new();
  tray_menu.update_api_status(None);
  tray_menu.update_mcp_status(false);

  let icon = tray::load_icon();
  let menu_channel = MenuEvent::receiver();

  // Create the event loop IMMEDIATELY (critical for macOS tray icon)
  let event_loop = EventLoopBuilder::new().build();

  // Store tray icon in Option - created after event loop starts
  let mut tray_icon: Option<TrayIcon> = None;

  // Run the event loop
  event_loop.run(move |event, _, control_flow| {
    // Use WaitUntil to check for menu events periodically while staying low on CPU
    *control_flow = ControlFlow::WaitUntil(Instant::now() + Duration::from_millis(100));

    match event {
      Event::NewEvents(StartCause::Init) => {
        // Hide from dock on macOS (must be done after event loop starts)
        #[cfg(target_os = "macos")]
        {
          use objc2::MainThreadMarker;
          use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};

          if let Some(mtm) = MainThreadMarker::new() {
            let app = NSApplication::sharedApplication(mtm);
            app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
          }
        }

        // Create tray icon after event loop has started (required for macOS)
        tray_icon = Some(tray::create_tray_icon(icon.clone(), &tray_menu.menu));
        log::info!("[daemon] Tray icon created");
      }
      Event::MainEventsCleared => {
        // Check for service status updates from background thread
        if let Ok(status) = rx.try_recv() {
          match status {
            ServiceStatus::Ready {
              api_port,
              mcp_running,
            } => {
              log::info!("[daemon] Services started successfully");
              tray_menu.update_api_status(api_port);
              tray_menu.update_mcp_status(mcp_running);

              // Update state file
              let mut state = read_state();
              state.api_port = api_port;
              state.mcp_running = mcp_running;
              if let Err(e) = write_state(&state) {
                log::error!("Failed to write state: {}", e);
              }
            }
            ServiceStatus::Failed(e) => {
              log::error!("Failed to start services: {}", e);
              // Keep tray icon running, show error state
              tray_menu.update_api_status(None);
              tray_menu.update_mcp_status(false);
            }
          }
        }

        // Process menu events
        while let Ok(event) = menu_channel.try_recv() {
          if event.id == tray_menu.open_item.id() || event.id == tray_menu.preferences_item.id() {
            tray::open_gui();
          } else if event.id == tray_menu.quit_item.id() {
            log::info!("[daemon] Quit requested");
            SHOULD_QUIT.store(true, Ordering::SeqCst);
          }
        }

        // Use swap to only run cleanup once
        if SHOULD_QUIT.swap(false, Ordering::SeqCst) {
          // Cleanup
          let mut state = read_state();
          state.daemon_pid = None;
          let _ = write_state(&state);
          log::info!("[daemon] Exiting");
          *control_flow = ControlFlow::Exit;
        }
      }
      _ => {}
    }

    // Keep tray_icon alive
    let _ = &tray_icon;

    // Keep runtime alive
    let _ = &rt;
  });
}

fn stop_daemon() {
  let state = read_state();

  if let Some(pid) = state.daemon_pid {
    #[cfg(unix)]
    {
      unsafe {
        libc::kill(pid as i32, libc::SIGTERM);
      }
      eprintln!("Sent stop signal to daemon (PID {})", pid);
    }

    #[cfg(windows)]
    {
      use std::process::Command;
      let _ = Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/F"])
        .output();
      eprintln!("Sent stop signal to daemon (PID {})", pid);
    }
  } else {
    eprintln!("Daemon is not running");
  }
}

fn show_status() {
  let state = read_state();

  if let Some(pid) = state.daemon_pid {
    #[cfg(unix)]
    let is_running = unsafe { libc::kill(pid as i32, 0) == 0 };

    #[cfg(windows)]
    let is_running = {
      use std::process::Command;
      let output = Command::new("tasklist")
        .args(["/FI", &format!("PID eq {}", pid)])
        .output();
      output
        .map(|o| String::from_utf8_lossy(&o.stdout).contains(&pid.to_string()))
        .unwrap_or(false)
    };

    #[cfg(not(any(unix, windows)))]
    let is_running = false;

    if is_running {
      eprintln!("Daemon is running (PID {})", pid);
      if let Some(port) = state.api_port {
        eprintln!("  API: Running on port {}", port);
      } else {
        eprintln!("  API: Stopped");
      }
      eprintln!(
        "  MCP: {}",
        if state.mcp_running {
          "Running"
        } else {
          "Stopped"
        }
      );
    } else {
      eprintln!("Daemon is not running (stale PID in state file)");
    }
  } else {
    eprintln!("Daemon is not running");
  }
}

fn print_usage() {
  eprintln!("Donut Browser Daemon");
  eprintln!();
  eprintln!("Usage: donut-daemon <command>");
  eprintln!();
  eprintln!("Commands:");
  eprintln!("  start       Start the daemon (detaches from terminal)");
  eprintln!("  stop        Stop the running daemon");
  eprintln!("  status      Show daemon status");
  eprintln!("  run         Run in foreground (for debugging)");
  eprintln!("  autostart   Manage autostart settings");
  eprintln!("    enable    Enable autostart on login");
  eprintln!("    disable   Disable autostart on login");
  eprintln!("    status    Show autostart status");
}

fn main() {
  let args: Vec<String> = env::args().collect();

  if args.len() < 2 {
    print_usage();
    process::exit(1);
  }

  match args[1].as_str() {
    "start" => {
      // "start" is now an alias for "run"
      // On macOS, the daemon should be started via launchctl (see daemon_spawn.rs)
      // This command is kept for backward compatibility
      eprintln!("Starting daemon...");
      run_daemon();
    }
    "stop" => {
      stop_daemon();
    }
    "status" => {
      show_status();
    }
    "run" => {
      run_daemon();
    }
    "autostart" => {
      if args.len() < 3 {
        eprintln!("Usage: donut-daemon autostart <enable|disable|status>");
        process::exit(1);
      }
      match args[2].as_str() {
        "enable" => {
          if let Err(e) = autostart::enable_autostart() {
            eprintln!("Failed to enable autostart: {}", e);
            process::exit(1);
          }
          eprintln!("Autostart enabled");
        }
        "disable" => {
          if let Err(e) = autostart::disable_autostart() {
            eprintln!("Failed to disable autostart: {}", e);
            process::exit(1);
          }
          eprintln!("Autostart disabled");
        }
        "status" => {
          if autostart::is_autostart_enabled() {
            eprintln!("Autostart is enabled");
          } else {
            eprintln!("Autostart is disabled");
          }
        }
        _ => {
          eprintln!("Unknown autostart command: {}", args[2]);
          process::exit(1);
        }
      }
    }
    _ => {
      print_usage();
      process::exit(1);
    }
  }
}
