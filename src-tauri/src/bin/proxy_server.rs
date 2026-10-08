use clap::{Arg, Command};
use donutbrowser_lib::proxy_runner::{
  start_proxy_process_with_profile, stop_all_proxy_processes, stop_proxy_process,
};
use donutbrowser_lib::proxy_server::{redacted_upstream, run_proxy_server};
use donutbrowser_lib::proxy_storage::{build_proxy_url, get_proxy_config};
use std::process;

fn set_high_priority() {
  #[cfg(unix)]
  {
    unsafe {
      // Set high priority (negative nice value = higher priority)
      // -10 is a reasonably high priority without being too aggressive
      // This may fail without elevated privileges, which is fine
      let result = libc::setpriority(libc::PRIO_PROCESS, 0, -10);
      if result == 0 {
        log::debug!("Process priority set nice=-10");
      } else {
        // Try a less aggressive priority if -10 fails
        let result = libc::setpriority(libc::PRIO_PROCESS, 0, -5);
        if result == 0 {
          log::debug!("Process priority set nice=-5");
        }
      }
    }
  }

  #[cfg(target_os = "linux")]
  {
    // Lower OOM score so this process is less likely to be killed under memory pressure
    // Valid range is -1000 to 1000, lower = less likely to be killed
    // -500 is a reasonable value that makes us less likely to be killed
    if let Err(e) = std::fs::write("/proc/self/oom_score_adj", "-500") {
      log::debug!("OOM score adjustment failed err=\"{e}\"");
    } else {
      log::debug!("OOM score adjustment set oom_score_adj=-500");
    }
  }

  #[cfg(windows)]
  {
    use windows::Win32::System::Threading::{
      GetCurrentProcess, SetPriorityClass, ABOVE_NORMAL_PRIORITY_CLASS,
    };

    unsafe {
      let process = GetCurrentProcess();
      if SetPriorityClass(process, ABOVE_NORMAL_PRIORITY_CLASS).is_ok() {
        log::debug!("Process priority set class=ABOVE_NORMAL");
      } else {
        log::debug!("Process priority class set failed");
      }
    }
  }
}

#[tokio::main(flavor = "multi_thread")]
async fn main() {
  // Initialize logger to write to stderr (which will be redirected to file).
  //
  // Default filter is Info — Debug pulls in reqwest/hyper internals which
  // make the per-worker log unreadable on a busy browser session and obscure
  // the actual lines we care about (binds, accept errors, upstream failures).
  // RUST_LOG is parsed after the default so it wins: `debug`, or
  // `donutbrowser_lib::proxy_server=trace` for the tunnel code alone.
  // Same line shape as the app log: `[ts][target][LEVEL] message`, with the
  // crate prefix dropped from the target.
  env_logger::Builder::new()
    .filter_level(log::LevelFilter::Info)
    .parse_default_env()
    .format(|buf, record| {
      use std::io::Write;
      let target = record.target();
      let target = target.strip_prefix("donutbrowser_lib::").unwrap_or(target);
      writeln!(
        buf,
        "[{}][{}][{}] {}",
        buf.timestamp_millis(),
        target,
        record.level(),
        record.args()
      )
    })
    .init();

  std::panic::set_hook(Box::new(|panic_info| {
    let location = panic_info
      .location()
      .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
      .unwrap_or_else(|| "-".to_string());
    let message = panic_info
      .payload()
      .downcast_ref::<&str>()
      .map(|s| s.to_string())
      .or_else(|| panic_info.payload().downcast_ref::<String>().cloned())
      .unwrap_or_default();
    log::error!(
      "Worker panicked pid={} at={location} msg={message:?}",
      std::process::id()
    );
  }));

  let matches = Command::new("donut-proxy")
    .version(env!("BUILD_VERSION"))
    .subcommand(
      Command::new("proxy")
        .about("Manage proxy servers")
        .subcommand(
          Command::new("start")
            .about("Start a proxy server")
            .arg(Arg::new("host").long("host").help("Upstream proxy host"))
            .arg(
              Arg::new("proxy-port")
                .long("proxy-port")
                .value_parser(clap::value_parser!(u16))
                .help("Upstream proxy port"),
            )
            .arg(
              Arg::new("type")
                .long("type")
                .help("Proxy type (http, https, httpstls, socks4, socks5, ss)"),
            )
            .arg(
              Arg::new("port")
                .short('p')
                .long("port")
                .value_parser(clap::value_parser!(u16))
                .help("Local port to use (random if not specified)"),
            )
            .arg(
              Arg::new("ignore-certificate")
                .long("ignore-certificate")
                .help("Ignore certificate errors for HTTPS proxies"),
            )
            .arg(
              Arg::new("upstream")
                .short('u')
                .long("upstream")
                .help("Upstream proxy URL (protocol://[username:password@]host:port)"),
            )
            .arg(
              Arg::new("profile-id")
                .long("profile-id")
                .help("ID of the profile this proxy is associated with"),
            )
            .arg(
              Arg::new("bypass-rules")
                .long("bypass-rules")
                .help("JSON array of bypass rules (hostnames, IPs, or regex patterns)"),
            )
            .arg(
              Arg::new("blocklist-file")
                .long("blocklist-file")
                .help("Path to DNS blocklist file (one domain per line)"),
            )
            .arg(
              Arg::new("dns-allowlist-mode")
                .long("dns-allowlist-mode")
                .num_args(0)
                .help("Treat --blocklist-file as an allowlist (block all domains not listed)"),
            )
            .arg(
              Arg::new("no-domain-history")
                .long("no-domain-history")
                .num_args(0)
                .help("Count bytes only; do not record which sites the browser reaches"),
            )
            .arg(
              Arg::new("local-protocol")
                .long("local-protocol")
                .help("Protocol served to the browser: http (default) or socks5"),
            ),
        )
        .subcommand(
          Command::new("stop")
            .about("Stop a proxy server")
            .arg(Arg::new("id").long("id").help("Proxy ID to stop"))
            .arg(
              Arg::new("upstream")
                .long("upstream")
                .help("Stop proxies with this upstream URL"),
            ),
        )
        .subcommand(Command::new("list").about("List all proxy servers")),
    )
    .subcommand(
      Command::new("xray")
        .about("Manage Xray-core workers for share-link proxies (VLESS, VMess, Trojan, Hysteria2)")
        .subcommand(
          Command::new("start")
            .about(
              "Start a worker serving authenticated SOCKS5 on 127.0.0.1. The share link is \
               read from the first line of stdin, never from the command line",
            )
            .arg(
              Arg::new("owner-pid")
                .long("owner-pid")
                .value_parser(clap::value_parser!(u32))
                .required(true)
                .help("The worker exits when this process exits"),
            ),
        )
        .subcommand(
          Command::new("bind")
            .about("Make a worker exit with this process instead of its current owner")
            .arg(Arg::new("id").long("id").required(true).help("Worker ID"))
            .arg(
              Arg::new("pid")
                .long("pid")
                .value_parser(clap::value_parser!(u32))
                .required(true)
                .help("New owner process ID"),
            ),
        )
        .subcommand(
          Command::new("stop")
            .about("Stop a worker")
            .arg(Arg::new("id").long("id").required(true).help("Worker ID")),
        ),
    )
    .subcommand(
      Command::new("proxy-worker")
        .about("Run a proxy worker process (internal use)")
        .arg(
          Arg::new("id")
            .long("id")
            .required(true)
            .help("Proxy configuration ID"),
        )
        .arg(Arg::new("action").required(true).help("Action (start)")),
    )
    .subcommand(
      Command::new("vpn-worker")
        .about("Run a VPN worker process (internal use)")
        .arg(
          Arg::new("id")
            .long("id")
            .required(true)
            .help("VPN worker configuration ID"),
        )
        .arg(
          Arg::new("port")
            .long("port")
            .value_parser(clap::value_parser!(u16))
            .required(true)
            .help("Local SOCKS5 port"),
        )
        .arg(Arg::new("action").required(true).help("Action (start)"))
        .arg(
          Arg::new("config-path")
            .long("config-path")
            .help("Direct path to the VPN worker config JSON file"),
        ),
    )
    .subcommand(
      Command::new("xray-worker")
        .about("Run an Xray-core worker process (internal use)")
        .arg(Arg::new("action").required(true).help("Action (start)"))
        .arg(
          Arg::new("config-path")
            .long("config-path")
            .required(true)
            .help("Direct path to the Xray worker config JSON file"),
        ),
    )
    .get_matches();

  if let Some(proxy_matches) = matches.subcommand_matches("proxy") {
    if let Some(start_matches) = proxy_matches.subcommand_matches("start") {
      let mut upstream_url: Option<String> = None;

      // Build upstream URL from individual components if provided
      if let (Some(host), Some(port), Some(proxy_type)) = (
        start_matches.get_one::<String>("host"),
        start_matches.get_one::<u16>("proxy-port"),
        start_matches.get_one::<String>("type"),
      ) {
        let username = std::env::var("DONUT_PROXY_USERNAME").ok();
        let password = std::env::var("DONUT_PROXY_PASSWORD").ok();
        upstream_url = Some(build_proxy_url(
          proxy_type,
          host,
          *port,
          username.as_deref(),
          password.as_deref(),
        ));
      } else if let Some(upstream) = start_matches.get_one::<String>("upstream") {
        if url::Url::parse(upstream)
          .is_ok_and(|parsed| !parsed.username().is_empty() || parsed.password().is_some())
        {
          eprintln!("Credentialed upstream URLs are not accepted as process arguments");
          process::exit(2);
        }
        upstream_url = Some(upstream.clone());
      }

      let port = start_matches.get_one::<u16>("port").copied();
      let profile_id = start_matches.get_one::<String>("profile-id").cloned();
      let bypass_rules: Vec<String> = start_matches
        .get_one::<String>("bypass-rules")
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_default();
      let blocklist_file = start_matches.get_one::<String>("blocklist-file").cloned();
      let dns_allowlist_mode = start_matches.get_flag("dns-allowlist-mode");
      let local_protocol = start_matches.get_one::<String>("local-protocol").cloned();
      let record_domains = !start_matches.get_flag("no-domain-history");

      match start_proxy_process_with_profile(
        upstream_url,
        port,
        profile_id,
        bypass_rules,
        blocklist_file,
        dns_allowlist_mode,
        local_protocol,
        record_domains,
      )
      .await
      {
        Ok(config) => {
          // Output the configuration as JSON for the Rust side to parse
          // Use println! here because this needs to go to stdout for parsing
          println!(
            "{}",
            serde_json::json!({
              "id": config.id,
              "localPort": config.local_port,
              "localUrl": config.local_url,
              "upstreamUrl": redacted_upstream(&config.upstream_url),
            })
          );
          process::exit(0);
        }
        Err(e) => {
          eprintln!("Failed to start proxy: {}", e);
          process::exit(1);
        }
      }
    } else if let Some(stop_matches) = proxy_matches.subcommand_matches("stop") {
      if let Some(id) = stop_matches.get_one::<String>("id") {
        match stop_proxy_process(id).await {
          Ok(success) => {
            // Use println! here because this needs to go to stdout for parsing
            println!("{}", serde_json::json!({ "success": success }));
            process::exit(0);
          }
          Err(e) => {
            eprintln!("Failed to stop proxy: {}", e);
            process::exit(1);
          }
        }
      } else if let Some(upstream) = stop_matches.get_one::<String>("upstream") {
        // Find proxies with this upstream URL
        let configs = donutbrowser_lib::proxy_storage::list_proxy_configs();
        let matching_configs: Vec<_> = configs
          .iter()
          .filter(|config| config.upstream_url == *upstream)
          .collect();

        if matching_configs.is_empty() {
          eprintln!("No proxies found for {}", upstream);
          process::exit(1);
        }

        for config in matching_configs {
          let _ = stop_proxy_process(&config.id).await;
        }

        // Use println! here because this needs to go to stdout for parsing
        println!("{}", serde_json::json!({ "success": true }));
        process::exit(0);
      } else {
        // Stop all proxies
        match stop_all_proxy_processes().await {
          Ok(_) => {
            // Use println! here because this needs to go to stdout for parsing
            println!("{}", serde_json::json!({ "success": true }));
            process::exit(0);
          }
          Err(e) => {
            eprintln!("Failed to stop all proxies: {}", e);
            process::exit(1);
          }
        }
      }
    } else if proxy_matches.subcommand_matches("list").is_some() {
      let configs = donutbrowser_lib::proxy_storage::list_proxy_configs();
      // Use println! here because this needs to go to stdout for parsing
      println!("{}", serde_json::to_string(&configs).unwrap());
      process::exit(0);
    } else {
      eprintln!("Invalid action. Use 'start', 'stop', or 'list'");
      process::exit(1);
    }
  } else if let Some(xray_matches) = matches.subcommand_matches("xray") {
    if let Some(start_matches) = xray_matches.subcommand_matches("start") {
      let owner_pid = *start_matches
        .get_one::<u32>("owner-pid")
        .expect("owner-pid is required");
      let mut share_link = String::new();
      if std::io::stdin().read_line(&mut share_link).is_err() || share_link.trim().is_empty() {
        eprintln!("No share link on stdin");
        process::exit(2);
      }

      // The detached worker would otherwise inherit the caller's pipes, and a
      // caller that reads to EOF would wait for the worker to exit. Same fix
      // as `proxy start` (proxy_runner.rs).
      #[cfg(windows)]
      {
        use std::os::windows::io::AsRawHandle;
        use windows::Win32::Foundation::{SetHandleInformation, HANDLE, HANDLE_FLAGS};
        const HANDLE_FLAG_INHERIT: u32 = 0x00000001;
        for handle in [
          std::io::stdin().as_raw_handle(),
          std::io::stdout().as_raw_handle(),
          std::io::stderr().as_raw_handle(),
        ] {
          if !handle.is_null() {
            unsafe {
              let _ = SetHandleInformation(HANDLE(handle), HANDLE_FLAG_INHERIT, HANDLE_FLAGS(0));
            }
          }
        }
      }

      match donutbrowser_lib::xray_worker_runner::start_xray_worker_for_owner(
        None,
        share_link.trim(),
        owner_pid,
      )
      .await
      {
        Ok(worker) => {
          // Use println! here because this needs to go to stdout for parsing
          println!(
            "{}",
            serde_json::json!({
              "id": worker.id,
              "localPort": worker.local_port,
              "localUrl": format!("socks5://127.0.0.1:{}", worker.local_port),
              "username": worker.username,
              "password": worker.password,
            })
          );
          process::exit(0);
        }
        Err(e) => {
          eprintln!("Failed to start Xray-core worker: {e}");
          process::exit(1);
        }
      }
    } else if let Some(bind_matches) = xray_matches.subcommand_matches("bind") {
      let id = bind_matches
        .get_one::<String>("id")
        .expect("id is required");
      let pid = *bind_matches.get_one::<u32>("pid").expect("pid is required");
      let success = donutbrowser_lib::xray_worker_runner::set_browser_pid(id, pid);
      // Use println! here because this needs to go to stdout for parsing
      println!("{}", serde_json::json!({ "success": success }));
      process::exit(0);
    } else if let Some(stop_matches) = xray_matches.subcommand_matches("stop") {
      let id = stop_matches
        .get_one::<String>("id")
        .expect("id is required");
      match donutbrowser_lib::xray_worker_runner::stop_xray_worker_now(id) {
        Ok(success) => {
          // Use println! here because this needs to go to stdout for parsing
          println!("{}", serde_json::json!({ "success": success }));
          process::exit(0);
        }
        Err(e) => {
          eprintln!("Failed to stop Xray-core worker: {e}");
          process::exit(1);
        }
      }
    } else {
      eprintln!("Invalid action. Use 'start', 'bind', or 'stop'");
      process::exit(1);
    }
  } else if let Some(worker_matches) = matches.subcommand_matches("proxy-worker") {
    let id = worker_matches
      .get_one::<String>("id")
      .expect("id is required");
    let action = worker_matches
      .get_one::<String>("action")
      .expect("action is required");

    if action == "start" {
      // Set high priority so this process is killed last under resource pressure
      set_high_priority();

      // Retry config loading to handle file system race condition on Windows
      // where the config file may not be immediately visible after being written
      let config = {
        let mut attempts = 0;
        loop {
          if let Some(config) = get_proxy_config(id) {
            break config;
          }
          attempts += 1;
          if attempts >= 10 {
            log::error!(
              "Worker exiting kind=proxy id={id} pid={} reason=config_not_found attempts={attempts}",
              process::id()
            );
            process::exit(1);
          }
          std::thread::sleep(std::time::Duration::from_millis(50));
        }
      };

      // Run the proxy server - this should never return (infinite loop)
      if let Err(e) = run_proxy_server(config).await {
        log::error!(
          "Worker exiting kind=proxy id={id} pid={} reason=error err=\"{e}\"",
          process::id()
        );
        process::exit(1);
      }
      // This should never be reached - run_proxy_server has an infinite loop
      log::error!(
        "Worker exiting kind=proxy id={id} pid={} reason=accept_loop_returned",
        process::id()
      );
      process::exit(1);
    } else {
      eprintln!("Invalid action for proxy-worker. Use 'start'");
      process::exit(1);
    }
  } else if let Some(vpn_matches) = matches.subcommand_matches("vpn-worker") {
    let id = vpn_matches.get_one::<String>("id").expect("id is required");
    let action = vpn_matches
      .get_one::<String>("action")
      .expect("action is required");
    let port = *vpn_matches
      .get_one::<u16>("port")
      .expect("port is required");
    let config_path = vpn_matches.get_one::<String>("config-path");

    if action == "start" {
      set_high_priority();
      let pid = process::id();
      let exit_with = |reason: &str, detail: String| -> ! {
        log::error!("Worker exiting kind=vpn id={id} pid={pid} reason={reason} {detail}");
        process::exit(1);
      };

      let config = if let Some(path) = config_path {
        // Load config directly from the provided path
        match std::fs::read_to_string(path) {
          Ok(content) => match serde_json::from_str::<
            donutbrowser_lib::vpn_worker_storage::VpnWorkerConfig,
          >(&content)
          {
            Ok(config) => config,
            Err(e) => exit_with("config_unparsable", format!("path=\"{path}\" err=\"{e}\"")),
          },
          Err(e) => exit_with("config_unreadable", format!("path=\"{path}\" err=\"{e}\"")),
        }
      } else {
        // Fallback: discover config by ID with retries
        let mut attempts = 0;
        loop {
          if let Some(config) = donutbrowser_lib::vpn_worker_storage::get_vpn_worker_config(id) {
            break config;
          }
          attempts += 1;
          if attempts >= 50 {
            exit_with(
              "config_not_found",
              format!(
                "attempts={attempts} dir=\"{}\"",
                donutbrowser_lib::proxy_storage::get_storage_dir().display()
              ),
            );
          }
          std::thread::sleep(std::time::Duration::from_millis(100));
        }
      };

      // Read the decrypted VPN config from the temp file
      let vpn_config_data = match std::fs::read_to_string(&config.config_file_path) {
        Ok(data) => data,
        Err(e) => exit_with(
          "vpn_config_unreadable",
          format!("vpn={} err=\"{e}\"", config.vpn_id),
        ),
      };

      match config.vpn_type.as_str() {
        "wireguard" => {
          let wg_config = match donutbrowser_lib::vpn::parse_wireguard_config(&vpn_config_data) {
            Ok(c) => c,
            Err(e) => exit_with(
              "wireguard_config_invalid",
              format!("vpn={} err=\"{e}\"", config.vpn_id),
            ),
          };

          let server =
            donutbrowser_lib::vpn::socks5_server::WireGuardSocks5Server::new(wg_config, port);
          if let Err(e) = server
            .run(id.clone(), config_path.map(std::path::PathBuf::from))
            .await
          {
            exit_with("error", format!("vpn={} err=\"{e}\"", config.vpn_id));
          }
        }
        other => exit_with("unknown_vpn_type", format!("vpn_type={other}")),
      }
    } else {
      eprintln!("Invalid action for vpn-worker. Use 'start'");
      process::exit(1);
    }
  } else if let Some(xray_matches) = matches.subcommand_matches("xray-worker") {
    let action = xray_matches
      .get_one::<String>("action")
      .expect("action is required");
    let config_path = xray_matches
      .get_one::<String>("config-path")
      .expect("config-path is required");
    if action != "start" {
      eprintln!("Invalid action for xray-worker. Use 'start'");
      process::exit(1);
    }

    set_high_priority();
    if let Err(error) =
      donutbrowser_lib::xray_worker_runner::run_xray_worker(std::path::Path::new(config_path)).await
    {
      log::error!(
        "Worker exiting kind=xray pid={} reason=error err=\"{error}\"",
        process::id()
      );
      process::exit(1);
    }
  } else {
    eprintln!("No command specified");
    process::exit(1);
  }
}
